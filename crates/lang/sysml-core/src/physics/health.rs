//! Physics health diagnostics for IDE integration.
//!
//! Provides static analysis diagnostics that run at elaboration time:
//! - PH001: Domain mismatch on flow connections (signal↔signal carrier
//!   mismatches downgrade to info)
//! - PH002: Conservation imbalance (all ports same direction; signal ports
//!   are not counted)
//! - PH003: Signal port classification (incomplete effort/flow pair =
//!   measurement-only signal port, reported as info)
//! - PH004: Direction conflict on flow connection (skipped when either
//!   endpoint is signal-classified)

use std::collections::HashMap;

use sysml_span::{Diagnostic, DiagnosticTier, Span};

use crate::element_ordering::primary_span;
use crate::{Element, ElementId, ElementKind, ModelGraph};

use super::classify::{classify_part_attributes, classify_port_definition, suggest_isq_type};
use super::domain::{BondGraphRole, PhysicsDomainRegistry, VariableRole};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Get the name span or fall back to primary span (first source span or synthetic).
fn name_span_or_primary(element: &Element) -> Span {
    element
        .name_span
        .clone()
        .unwrap_or_else(|| primary_span(element))
}

/// Human-readable conservation law name for a domain.
fn conservation_law_name(domain: &str) -> &'static str {
    match domain {
        "electrical" => "Kirchhoff's Current Law (KCL)",
        "thermal" => "energy balance",
        "hydraulic" => "mass balance",
        "mechanical_translational" | "mechanical_rotational" => "force/torque balance",
        "chemical" => "molar balance",
        _ => "conservation law",
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Run physics health diagnostics on a model graph.
///
/// This is the main entry point for IDE integration. Returns diagnostics
/// for domain mismatches, direction conflicts, and conservation imbalances.
/// Runs as pure static analysis — no execution state needed.
pub fn physics_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    let registry = PhysicsDomainRegistry::from_workspace_graph(graph);
    let mut diags = Vec::new();

    check_flow_domain_mismatches(graph, &registry, &mut diags);
    check_flow_direction_conflicts(graph, &registry, &mut diags);
    check_missing_effort_flow_pairs(graph, &registry, &mut diags);
    check_conservation_imbalances(graph, &registry, &mut diags);
    check_unwired_rci_elements(graph, &registry, &mut diags);
    check_real_typed_physics_attributes(graph, &mut diags);

    diags
}

// ---------------------------------------------------------------------------
// PH001: Domain mismatch on flow connections
// ---------------------------------------------------------------------------

/// Check that flow connections link ports of the same physics domain.
///
/// e.g., connecting an electrical port to a thermal port is almost certainly wrong.
fn check_flow_domain_mismatches(
    graph: &ModelGraph,
    registry: &PhysicsDomainRegistry,
    diags: &mut Vec<Diagnostic>,
) {
    for elem in graph.elements.values() {
        if elem.kind != ElementKind::FlowUsage {
            continue;
        }

        let source_path = elem.get_prop("source").and_then(|v| v.as_str());
        let target_path = elem.get_prop("target").and_then(|v| v.as_str());

        let (source_path, target_path) = match (source_path, target_path) {
            (Some(s), Some(t)) => (s, t),
            _ => continue,
        };

        let source_def = find_port_def_for_endpoint(graph, source_path);
        let target_def = find_port_def_for_endpoint(graph, target_path);

        let (source_def, target_def) = match (source_def, target_def) {
            (Some(s), Some(t)) => (s, t),
            _ => continue,
        };

        let source_class = classify_port_definition(&source_def, graph, registry);
        let target_class = classify_port_definition(&target_def, graph, registry);

        match (source_class.domain, target_class.domain) {
            (Some(sd), Some(td)) if sd != td => {
                // Signal↔signal links don't exchange energy — a differing
                // carrier is a unit/type consistency note, not a coupling
                // error. (Same-carrier signal links never reach this arm.)
                if source_class.is_signal && target_class.is_signal {
                    diags.push(
                        Diagnostic::info(format!(
                            "signal carrier mismatch: flow connects signal port '{}' (carrier {}) to signal port '{}' (carrier {})",
                            source_path, sd, target_path, td,
                        ))
                        .with_code("PH001")
                        .with_tier(DiagnosticTier::Semantic)
                        .with_span(name_span_or_primary(elem))
                        .with_note(
                            "signal links carry measurements, not energy — but both ends \
                             should agree on the carried quantity's domain",
                        ),
                    );
                    continue;
                }

                let mut diag = Diagnostic::warning(format!(
                    "flow connects {} port '{}' to {} port '{}'",
                    sd, source_path, td, target_path,
                ))
                .with_code("PH001")
                .with_tier(DiagnosticTier::Semantic)
                .with_span(name_span_or_primary(elem))
                .with_note(format!(
                    "{} and {} ports cannot exchange energy directly",
                    sd, td,
                ))
                .with_note(
                    "for cross-domain coupling, use a transformer element \
                     (e.g., electric motor, heat exchanger)",
                );

                // Add related spans for source and target ports
                if let Some(src_elem) = find_port_element(graph, source_path) {
                    diag = diag.with_related(
                        name_span_or_primary(src_elem),
                        format!("source port '{}' is {} domain", source_path, sd),
                    );
                }
                if let Some(tgt_elem) = find_port_element(graph, target_path) {
                    diag = diag.with_related(
                        name_span_or_primary(tgt_elem),
                        format!("target port '{}' is {} domain", target_path, td),
                    );
                }

                diags.push(diag);
            }
            _ => {}
        }
    }
}

// ---------------------------------------------------------------------------
// PH004: Direction conflicts
// ---------------------------------------------------------------------------

/// Check that flow connections don't link two ports with the same direction
/// (e.g., two "out" ports connected without conjugation).
///
/// Signal-classified endpoints are exempt: signal links are directed
/// per-tick copies with legal fan-out, so same-direction endpoints don't
/// create conflicting effort/flow sources.
fn check_flow_direction_conflicts(
    graph: &ModelGraph,
    registry: &PhysicsDomainRegistry,
    diags: &mut Vec<Diagnostic>,
) {
    for elem in graph.elements.values() {
        if elem.kind != ElementKind::FlowUsage {
            continue;
        }

        let source_path = elem.get_prop("source").and_then(|v| v.as_str());
        let target_path = elem.get_prop("target").and_then(|v| v.as_str());

        let (source_path, target_path) = match (source_path, target_path) {
            (Some(s), Some(t)) => (s, t),
            _ => continue,
        };

        let source_dir = find_port_direction(graph, source_path);
        let target_dir = find_port_direction(graph, target_path);

        let conflicting_dir = match (source_dir.as_deref(), target_dir.as_deref()) {
            (Some("out"), Some("out")) => Some("out"),
            (Some("in"), Some("in")) => Some("in"),
            _ => None,
        };

        if let Some(dir) = conflicting_dir {
            // Skip when either endpoint port is signal-classified.
            let endpoint_is_signal = |path: &str| {
                find_port_def_for_endpoint(graph, path)
                    .map(|def| classify_port_definition(&def, graph, registry).is_signal)
                    .unwrap_or(false)
            };
            if endpoint_is_signal(source_path) || endpoint_is_signal(target_path) {
                continue;
            }

            let mut diag = Diagnostic::warning(format!(
                "flow connects two '{}' ports: '{}' and '{}'",
                dir, source_path, target_path,
            ))
            .with_code("PH004")
            .with_tier(DiagnosticTier::Semantic)
            .with_span(name_span_or_primary(elem))
            .with_note(format!(
                "connecting two '{}' ports creates conflicting {} sources",
                dir,
                if dir == "out" { "effort" } else { "flow" },
            ))
            .with_note("if one port should receive, use conjugated typing: `port x : ~PortDef`");

            if let Some(src_elem) = find_port_element(graph, source_path) {
                diag = diag.with_related(
                    name_span_or_primary(src_elem),
                    format!("'{}' has direction '{}'", source_path, dir),
                );
            }
            if let Some(tgt_elem) = find_port_element(graph, target_path) {
                diag = diag.with_related(
                    name_span_or_primary(tgt_elem),
                    format!("'{}' has direction '{}'", target_path, dir),
                );
            }

            diags.push(diag);
        }
    }
}

// ---------------------------------------------------------------------------
// PH003: Missing effort/flow pairs
// ---------------------------------------------------------------------------

/// Check classified PortDefinitions for an incomplete effort/flow pair.
///
/// RSC-1.1: a port whose features form an incomplete conjugate pair (only
/// effort or only flow) IS the signal-port shape (Modelica causal signal
/// connector) — signal classification wins over "half-modeled power port",
/// so this now reports the classification as info rather than suggesting
/// the missing conjugate attribute.
fn check_missing_effort_flow_pairs(
    graph: &ModelGraph,
    registry: &PhysicsDomainRegistry,
    diags: &mut Vec<Diagnostic>,
) {
    for elem in graph.elements.values() {
        if elem.kind != ElementKind::PortDefinition {
            continue;
        }

        let name = match &elem.name {
            Some(n) => n.clone(),
            None => continue,
        };

        let classification = classify_port_definition(&name, graph, registry);
        if !classification.is_signal {
            // Unclassified ports and complete (effort + flow) power ports
            // are both fine.
            continue;
        }

        let carrier = classification
            .carrier_domain
            .or(classification.domain)
            .unwrap_or("unknown");
        let carried: Vec<&str> = classification
            .features
            .iter()
            .filter(|f| matches!(f.role, VariableRole::Effort | VariableRole::Flow))
            .map(|f| f.name.as_str())
            .collect();

        // Describe the carried quantity without emitting empty `()` or a bare
        // "unknown quantity ()" when the domain/features are unclassified.
        let detail = match (carrier, carried.is_empty()) {
            ("unknown", true) => String::new(),
            ("unknown", false) => format!(" carrying ({})", carried.join(", ")),
            (c, true) => format!(" carrying {c} quantity"),
            (c, false) => format!(" carrying {c} quantity ({})", carried.join(", ")),
        };

        diags.push(
            Diagnostic::info(format!(
                "port '{name}' classified as signal (measurement-only){detail}"
            ))
            .with_code("PH003")
            .with_tier(DiagnosticTier::Semantic)
            .with_span(name_span_or_primary(elem))
            .with_note(
                "signal ports carry measurements, not power — they are exempt from \
                 conservation-law checks (PH002/PH004)",
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// PH002: Conservation imbalances
// ---------------------------------------------------------------------------

/// Check parts with 3+ ports of the same domain for conservation viability.
///
/// If all ports are the same direction (all in or all out), conservation
/// constraints (KCL, mass balance) cannot be satisfied.
fn check_conservation_imbalances(
    graph: &ModelGraph,
    registry: &PhysicsDomainRegistry,
    diags: &mut Vec<Diagnostic>,
) {
    // Group ports by owner + domain, keeping the owner ElementId for span access
    struct PortInfo {
        name: String,
        direction: Option<String>,
        id: ElementId,
    }
    let mut owner_domain_ports: HashMap<(ElementId, String), Vec<PortInfo>> = HashMap::new();

    for elem in graph.elements.values() {
        if elem.kind != ElementKind::PortUsage {
            continue;
        }

        let port_name = match &elem.name {
            Some(n) => n.clone(),
            None => continue,
        };

        let owner_id = match &elem.owner {
            Some(id) => id.clone(),
            None => continue,
        };

        let direction = elem
            .get_prop("effectiveDirection")
            .and_then(|v| v.as_str())
            .map(String::from);

        let def_name = elem
            .get_prop("portDefinition")
            .and_then(|v| v.as_str())
            .map(String::from);

        let domain = if let Some(ref dn) = def_name {
            let classification = classify_port_definition(dn, graph, registry);
            // Signal ports carry measurements, not conserved flow — they
            // don't participate in KCL/balance counting. A part with ONLY
            // signal ports must produce no PH002.
            if classification.is_signal {
                continue;
            }
            classification.domain.map(String::from)
        } else {
            None
        };

        if let Some(domain) = domain {
            owner_domain_ports
                .entry((owner_id, domain))
                .or_default()
                .push(PortInfo {
                    name: port_name,
                    direction,
                    id: elem.id.clone(),
                });
        }
    }

    for ((owner_id, domain), ports) in &owner_domain_ports {
        if ports.len() < 3 {
            continue;
        }

        let all_in = ports.iter().all(|p| p.direction.as_deref() == Some("in"));
        let all_out = ports.iter().all(|p| p.direction.as_deref() == Some("out"));

        if !all_in && !all_out {
            continue;
        }

        let dir_label = if all_in { "in" } else { "out" };
        let owner_name = graph
            .get_element(owner_id)
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| "<unnamed>".to_owned());
        let law_name = conservation_law_name(&domain);

        let mut diag = Diagnostic::warning(format!(
            "part '{}' has {} {} ports but all are '{}' — {} cannot be satisfied",
            owner_name,
            ports.len(),
            domain,
            dir_label,
            law_name,
        ))
        .with_code("PH002")
        .with_tier(DiagnosticTier::Semantic);

        if let Some(owner_elem) = graph.get_element(owner_id) {
            diag = diag.with_span(name_span_or_primary(owner_elem));
        }

        let missing_dir = if all_in { "outgoing" } else { "incoming" };
        diag = diag.with_note(format!(
            "{} requires at least one {} port on this part",
            law_name, missing_dir,
        ));

        // Add related spans for ports (up to 5 to avoid noise)
        for port in ports.iter().take(5) {
            if let Some(port_elem) = graph.get_element(&port.id) {
                diag = diag.with_related(
                    name_span_or_primary(port_elem),
                    format!("'{}' — direction: {}", port.name, dir_label),
                );
            }
        }
        if ports.len() > 5 {
            diag = diag.with_note(format!("{} additional ports omitted", ports.len() - 5));
        }

        diags.push(diag);
    }
}

// ---------------------------------------------------------------------------
// PH006: Real-typed attributes that could use ISQ types
// ---------------------------------------------------------------------------

/// Suggest ISQ types for attributes named like physics quantities but typed as `Real`.
///
/// For example, `attribute voltage : Real` in a PortDefinition could be
/// `attribute voltage : ISQ::ElectricPotentialValue`, which would unlock
/// automatic physics domain classification and simulation features.
fn check_real_typed_physics_attributes(graph: &ModelGraph, diags: &mut Vec<Diagnostic>) {
    // Only check inside PortDefinition and PartDefinition — don't nag about
    // random `Real` attributes elsewhere
    let physics_contexts: std::collections::HashSet<_> = graph
        .elements
        .values()
        .filter(|e| {
            matches!(
                e.kind,
                ElementKind::PortDefinition | ElementKind::PartDefinition
            )
        })
        .map(|e| e.id.clone())
        .collect();

    for elem in graph.elements.values() {
        if elem.kind != ElementKind::AttributeUsage {
            continue;
        }

        let attr_name = match &elem.name {
            Some(n) => n.clone(),
            None => continue,
        };

        // Only check attributes inside PortDefinition or PartDefinition
        let owner_id = match &elem.owner {
            Some(id) if physics_contexts.contains(id) => id.clone(),
            _ => continue,
        };

        // Check if typed as Real (or ScalarValues::Real, or similar generic type)
        let type_name = get_attribute_type_name(elem, graph);
        let is_generic_real = match type_name.as_deref() {
            Some("Real") | Some("ScalarValues::Real") | Some("Integer") | Some("Number") => true,
            _ => false,
        };

        if !is_generic_real {
            continue;
        }

        // Check if the name matches a known physics quantity
        if let Some((isq_type, description)) = suggest_isq_type(&attr_name) {
            let owner_kind = graph
                .get_element(&owner_id)
                .map(|e| format!("{:?}", e.kind))
                .unwrap_or_default();
            let owner_name = graph
                .get_element(&owner_id)
                .and_then(|e| e.name.clone())
                .unwrap_or_else(|| "<unnamed>".to_owned());

            diags.push(
                Diagnostic::info(format!(
                    "did you mean `{}`? attribute '{}' in '{}' is typed as Real",
                    isq_type, attr_name, owner_name,
                ))
                .with_code("PH006")
                .with_tier(DiagnosticTier::Semantic)
                .with_span(name_span_or_primary(elem))
                .with_note(format!(
                    "{} — using the ISQ type unlocks automatic physics classification",
                    description,
                ))
                .with_note(format!("try: `attribute {} : {};`", attr_name, isq_type,))
                .with_note("ISQ types enable domain detection, conservation laws, and simulation"),
            );
        }
    }
}

/// Get the type name of an attribute from its properties or FeatureTyping children.
fn get_attribute_type_name(elem: &Element, graph: &ModelGraph) -> Option<String> {
    // Check direct property
    if let Some(tn) = elem.get_prop("unresolved_type").and_then(|v| v.as_str()) {
        return Some(tn.to_owned());
    }

    // Check FeatureTyping children
    for child in graph.children_of(&elem.id) {
        if child.kind == ElementKind::FeatureTyping {
            if let Some(tn) = child.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                return Some(tn.to_owned());
            }
        }
    }

    None
}

// ---------------------------------------------------------------------------
// PH005: Unwired R/C/I elements
// ---------------------------------------------------------------------------

/// Detect parts with R/C/I attributes and 2+ same-domain ports but no constraints.
///
/// For example, a `Resistor` part with `resistance : ResistanceValue` and two
/// electrical ports should have a constraint `resistance * current = voltage_in - voltage_out`.
fn check_unwired_rci_elements(
    graph: &ModelGraph,
    registry: &PhysicsDomainRegistry,
    diags: &mut Vec<Diagnostic>,
) {
    for elem in graph.elements.values() {
        if elem.kind != ElementKind::PartDefinition {
            continue;
        }

        let part_name = match &elem.name {
            Some(n) => n.clone(),
            None => continue,
        };

        // Find R/C/I attributes on this part
        let rci_features = classify_part_attributes(&part_name, graph, registry);
        let rci: Vec<_> = rci_features
            .iter()
            .filter(|f| {
                matches!(
                    f.bond_graph_role,
                    Some(BondGraphRole::Resistance)
                        | Some(BondGraphRole::Conductance)
                        | Some(BondGraphRole::Capacitance)
                        | Some(BondGraphRole::Inductance)
                )
            })
            .collect();

        if rci.is_empty() {
            continue;
        }

        // Check if the part has 2+ same-domain ports
        let port_children: Vec<_> = graph
            .children_of(&elem.id)
            .filter(|c| c.kind == ElementKind::PortUsage)
            .collect();

        if port_children.len() < 2 {
            continue;
        }

        // Check if there are any ConstraintUsage children (already wired)
        let has_constraints = graph
            .children_of(&elem.id)
            .any(|c| c.kind == ElementKind::ConstraintUsage);

        if has_constraints {
            continue;
        }

        // Emit PH005 for each R/C/I attribute
        for feature in &rci {
            let role_name = match feature.bond_graph_role {
                Some(BondGraphRole::Resistance) | Some(BondGraphRole::Conductance) => {
                    "R-element (dissipator)"
                }
                Some(BondGraphRole::Capacitance) => "C-element (energy storage, effort)",
                Some(BondGraphRole::Inductance) => "I-element (energy storage, flow)",
                _ => continue,
            };

            let constraint_hint = match feature.bond_graph_role {
                Some(BondGraphRole::Resistance) => {
                    format!(
                        "constraint {{ {} * current = voltage_in - voltage_out }}",
                        feature.name,
                    )
                }
                Some(BondGraphRole::Capacitance) => {
                    format!(
                        "constraint {{ {} * d(voltage)/dt = current }}",
                        feature.name,
                    )
                }
                Some(BondGraphRole::Inductance) => {
                    format!(
                        "constraint {{ {} * d(current)/dt = voltage_in - voltage_out }}",
                        feature.name,
                    )
                }
                _ => "constraint { ... }".to_owned(),
            };

            let mut diag = Diagnostic::info(format!(
                "part '{}' has {} attribute '{}' ({}) but no constraint wiring it to ports",
                part_name, role_name, feature.name, role_name,
            ))
            .with_code("PH005")
            .with_tier(DiagnosticTier::Semantic)
            .with_span(name_span_or_primary(elem))
            .with_note(format!(
                "this part looks like a {} — add a constitutive relation",
                role_name,
            ))
            .with_note(format!("example: `{}`", constraint_hint));

            // Point to the port children
            for port in port_children.iter().take(4) {
                let pname = port.name.as_deref().unwrap_or("<unnamed>");
                diag = diag.with_related(
                    name_span_or_primary(port),
                    format!("port '{}' on this part", pname),
                );
            }

            diags.push(diag);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the PortDefinition name for a flow endpoint path.
fn find_port_def_for_endpoint(graph: &ModelGraph, endpoint_path: &str) -> Option<String> {
    let port_name = endpoint_path.rsplit('.').next()?;

    for elem in graph.elements.values() {
        if elem.kind != ElementKind::PortUsage {
            continue;
        }
        if elem.name.as_deref() != Some(port_name) {
            continue;
        }

        if let Some(def) = elem.get_prop("portDefinition").and_then(|v| v.as_str()) {
            return Some(def.to_owned());
        }

        for child in graph.children_of(&elem.id) {
            if child.kind == ElementKind::FeatureTyping {
                if let Some(type_name) = child.get_prop("unresolved_type").and_then(|v| v.as_str())
                {
                    let is_port_def = graph.elements.values().any(|e| {
                        e.kind == ElementKind::PortDefinition
                            && e.name.as_deref() == Some(type_name)
                    });
                    if is_port_def {
                        return Some(type_name.to_owned());
                    }
                }
            }
        }
    }
    None
}

/// Find the effective direction for a port at the given endpoint path.
fn find_port_direction(graph: &ModelGraph, endpoint_path: &str) -> Option<String> {
    let port_name = endpoint_path.rsplit('.').next()?;

    for elem in graph.elements.values() {
        if elem.kind != ElementKind::PortUsage {
            continue;
        }
        if elem.name.as_deref() != Some(port_name) {
            continue;
        }

        return elem
            .get_prop("effectiveDirection")
            .and_then(|v| v.as_str())
            .map(String::from);
    }
    None
}

/// Find a PortUsage element by endpoint path.
fn find_port_element<'a>(graph: &'a ModelGraph, endpoint_path: &str) -> Option<&'a Element> {
    let port_name = endpoint_path.rsplit('.').next()?;

    graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::PortUsage && e.name.as_deref() == Some(port_name))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate;
    use crate::Element;

    /// Empty graph produces no physics diagnostics.
    #[test]
    fn empty_graph_no_diagnostics() {
        let graph = ModelGraph::new();
        let diags = physics_health_diagnostics(&graph);
        assert!(diags.is_empty());
    }

    /// Well-formed electrical port (effort + flow) produces no diagnostic.
    #[test]
    fn well_formed_port_no_diagnostic() {
        let mut graph = ModelGraph::new();

        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("PhasePort");
        let def_id = graph.add_element(port_def);

        let voltage = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("voltage")
            .with_owner(def_id.clone());
        graph.add_element(voltage);

        let current = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("current")
            .with_owner(def_id);
        graph.add_element(current);

        let diags = physics_health_diagnostics(&graph);
        let pair_diags: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("missing"))
            .collect();
        assert!(
            pair_diags.is_empty(),
            "well-formed port should not have missing-pair warnings: {:?}",
            pair_diags
        );
    }

    /// PH003 (RSC-1.1): a port with only an effort feature is signal-classified
    /// and gets the signal info wording — NOT the old "missing flow feature"
    /// suggestion.
    #[test]
    fn ph003_signal_port_gets_signal_wording() {
        let mut graph = ModelGraph::new();

        let port_def =
            Element::new_with_kind(ElementKind::PortDefinition).with_name("ThermalProbe");
        let def_id = graph.add_element(port_def);

        let temp = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("temperature")
            .with_owner(def_id);
        graph.add_element(temp);

        let diags = physics_health_diagnostics(&graph);
        let ph003: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("PH003"))
            .collect();
        assert_eq!(ph003.len(), 1, "should emit exactly one PH003");
        assert!(
            ph003[0].message.contains("classified as signal"),
            "signal wording expected: {:?}",
            ph003[0].message
        );
        assert!(
            ph003[0].message.contains("thermal"),
            "message should name the carrier domain: {:?}",
            ph003[0].message
        );
        assert!(
            ph003[0].message.contains("temperature"),
            "message should name the carried quantity: {:?}",
            ph003[0].message
        );
        assert!(
            !ph003[0].message.contains("missing"),
            "old half-modeled-power-port wording must be gone: {:?}",
            ph003[0].message
        );
        assert!(
            !ph003[0].notes.iter().any(|n| n.contains("add `attribute")),
            "signal ports must not get the effort/flow suggestion note: {:?}",
            ph003[0].notes
        );
    }

    /// PH003 (RSC-1.1): a complete effort+flow power port emits no PH003 at all.
    #[test]
    fn ph003_silent_on_full_power_port() {
        let mut graph = ModelGraph::new();

        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("ACPhase");
        let def_id = graph.add_element(port_def);
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("voltage")
                .with_owner(def_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("current")
                .with_owner(def_id),
        );

        let diags = physics_health_diagnostics(&graph);
        assert!(
            !diags.iter().any(|d| d.code.as_deref() == Some("PH003")),
            "complete power port must not emit PH003: {:?}",
            diags
        );
    }

    /// PH002: Conservation imbalance gets code, span, and related locations.
    #[test]
    fn ph002_conservation_imbalance_has_code_and_related() {
        let mut graph = ModelGraph::new();

        let port_def =
            Element::new_with_kind(ElementKind::PortDefinition).with_name("ElectricalPort");
        let def_id = graph.add_element(port_def);

        let voltage = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("voltage")
            .with_owner(def_id.clone());
        graph.add_element(voltage);

        let current = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("current")
            .with_owner(def_id);
        graph.add_element(current);

        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("busbar");
        let part_id = graph.add_element(part);

        for name in &["circuitOut1", "circuitOut2", "circuitOut3"] {
            let port = Element::new_with_kind(ElementKind::PortUsage)
                .with_name(*name)
                .with_owner(part_id.clone());
            let port_id = graph.add_element(port);

            let typing = Element::new_with_kind(ElementKind::FeatureTyping)
                .with_prop("unresolved_type", "ElectricalPort")
                .with_owner(port_id.clone());
            graph.add_element(typing);
        }

        elaborate(&mut graph);

        // Manually set directions and portDefinition
        for elem in graph.elements.values_mut() {
            if elem.kind == ElementKind::PortUsage {
                elem.set_prop("effectiveDirection", crate::Value::String("out".to_owned()));
                elem.set_prop(
                    "portDefinition",
                    crate::Value::String("ElectricalPort".to_owned()),
                );
            }
        }

        let diags = physics_health_diagnostics(&graph);
        let ph002: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("PH002"))
            .collect();
        assert_eq!(ph002.len(), 1, "should emit exactly one PH002");
        assert!(
            ph002[0].message.contains("KCL"),
            "message should mention KCL: {:?}",
            ph002[0].message
        );
        assert!(!ph002[0].notes.is_empty(), "should have guidance notes");
        assert!(
            !ph002[0].related.is_empty(),
            "should have related port locations"
        );
    }

    /// PH002 (RSC-1.1): a part with 3 signal in-ports (measurement-only,
    /// flow-quantity-only port def) must produce NO PH002 — signal ports
    /// don't participate in KCL counting.
    #[test]
    fn ph002_skips_parts_with_only_signal_ports() {
        let mut graph = ModelGraph::new();

        // Signal port def: only a flow-role quantity (current), no effort.
        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("SensePort");
        let def_id = graph.add_element(port_def);
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("rmsCurrent")
                .with_owner(def_id),
        );

        // Firmware-style part with three 'in' signal ports.
        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("firmware");
        let part_id = graph.add_element(part);
        for name in &["senseIn1", "senseIn2", "senseIn3"] {
            graph.add_element(
                Element::new_with_kind(ElementKind::PortUsage)
                    .with_name(*name)
                    .with_owner(part_id.clone())
                    .with_prop("effectiveDirection", "in")
                    .with_prop("portDefinition", "SensePort"),
            );
        }

        let diags = physics_health_diagnostics(&graph);
        assert!(
            !diags.iter().any(|d| d.code.as_deref() == Some("PH002")),
            "part with only signal ports must not emit PH002: {:?}",
            diags
        );
    }

    /// PH004 (RSC-1.1): direction conflicts are skipped when either endpoint
    /// port is signal-classified (fan-out / same-direction is normal for
    /// signal links).
    #[test]
    fn ph004_skips_signal_endpoints() {
        let mut graph = ModelGraph::new();

        // Signal port def: effort-only thermal probe.
        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("ProbePort");
        let def_id = graph.add_element(port_def);
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("temperature")
                .with_owner(def_id),
        );

        // Two 'out' signal ports connected by a flow — would be PH004 for
        // power ports, but both are signal-classified.
        graph.add_element(
            Element::new_with_kind(ElementKind::PortUsage)
                .with_name("probeA")
                .with_prop("effectiveDirection", "out")
                .with_prop("portDefinition", "ProbePort"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::PortUsage)
                .with_name("probeB")
                .with_prop("effectiveDirection", "out")
                .with_prop("portDefinition", "ProbePort"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FlowUsage)
                .with_prop("source", "probeA")
                .with_prop("target", "probeB"),
        );

        let diags = physics_health_diagnostics(&graph);
        assert!(
            !diags.iter().any(|d| d.code.as_deref() == Some("PH004")),
            "signal endpoints must not trigger PH004: {:?}",
            diags
        );
    }

    /// PH004 still fires for genuine power ports with conflicting directions.
    #[test]
    fn ph004_still_fires_for_power_ports() {
        let mut graph = ModelGraph::new();

        // Full power port def (effort + flow).
        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("PowerPort");
        let def_id = graph.add_element(port_def);
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("voltage")
                .with_owner(def_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("current")
                .with_owner(def_id),
        );

        graph.add_element(
            Element::new_with_kind(ElementKind::PortUsage)
                .with_name("feedA")
                .with_prop("effectiveDirection", "out")
                .with_prop("portDefinition", "PowerPort"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::PortUsage)
                .with_name("feedB")
                .with_prop("effectiveDirection", "out")
                .with_prop("portDefinition", "PowerPort"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FlowUsage)
                .with_prop("source", "feedA")
                .with_prop("target", "feedB"),
        );

        let diags = physics_health_diagnostics(&graph);
        assert!(
            diags.iter().any(|d| d.code.as_deref() == Some("PH004")),
            "power-port direction conflict must still emit PH004: {:?}",
            diags
        );
    }

    /// PH001 (RSC-1.1): signal↔signal links with differing carriers downgrade
    /// to an info "signal carrier mismatch" instead of a cross-domain warning.
    #[test]
    fn ph001_signal_carrier_mismatch_is_info() {
        let mut graph = ModelGraph::new();

        // Electrical-carrier signal port (flow-only).
        let elec_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("AmpSense");
        let elec_id = graph.add_element(elec_def);
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("rmsCurrent")
                .with_owner(elec_id),
        );

        // Thermal-carrier signal port (effort-only).
        let therm_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("TempSense");
        let therm_id = graph.add_element(therm_def);
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("temperature")
                .with_owner(therm_id),
        );

        graph.add_element(
            Element::new_with_kind(ElementKind::PortUsage)
                .with_name("ampOut")
                .with_prop("portDefinition", "AmpSense"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::PortUsage)
                .with_name("tempIn")
                .with_prop("portDefinition", "TempSense"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::FlowUsage)
                .with_prop("source", "ampOut")
                .with_prop("target", "tempIn"),
        );

        let diags = physics_health_diagnostics(&graph);
        let ph001: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("PH001"))
            .collect();
        assert_eq!(ph001.len(), 1, "should emit exactly one PH001: {:?}", diags);
        assert_eq!(
            ph001[0].severity,
            sysml_span::Severity::Info,
            "signal↔signal carrier mismatch must be info, not warning"
        );
        assert!(
            ph001[0].message.contains("signal carrier mismatch"),
            "message should say carrier mismatch: {:?}",
            ph001[0].message
        );
    }

    /// PH005: Part with R/C/I attribute and ports but no constraint.
    #[test]
    fn ph005_unwired_resistance_detected() {
        let mut graph = ModelGraph::new();

        // Create a PartDefinition with a resistance attribute
        let part_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("Resistor");
        let part_id = graph.add_element(part_def);

        // Resistance attribute typed as ResistanceValue
        let resistance = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("resistance")
            .with_owner(part_id.clone());
        let res_id = graph.add_element(resistance);

        let res_typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_prop("unresolved_type", "ResistanceValue")
            .with_owner(res_id);
        graph.add_element(res_typing);

        // Two ports on the part
        let port_in = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("phaseIn")
            .with_owner(part_id.clone());
        graph.add_element(port_in);

        let port_out = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("phaseOut")
            .with_owner(part_id);
        graph.add_element(port_out);

        let diags = physics_health_diagnostics(&graph);
        let ph005: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("PH005"))
            .collect();
        assert_eq!(ph005.len(), 1, "should emit exactly one PH005");
        assert!(
            ph005[0].message.contains("resistance"),
            "message should mention the resistance attribute: {:?}",
            ph005[0].message
        );
        assert!(
            ph005[0].message.contains("R-element"),
            "message should mention R-element: {:?}",
            ph005[0].message
        );
        assert!(
            !ph005[0].notes.is_empty(),
            "should have constraint suggestion notes"
        );
        assert!(
            ph005[0].notes.iter().any(|n| n.contains("constraint")),
            "note should suggest a constraint expression"
        );
    }

    /// PH005: Part with R/C/I attribute but has a constraint — no diagnostic.
    #[test]
    fn ph005_wired_resistance_no_diagnostic() {
        let mut graph = ModelGraph::new();

        let part_def =
            Element::new_with_kind(ElementKind::PartDefinition).with_name("WiredResistor");
        let part_id = graph.add_element(part_def);

        let resistance = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("resistance")
            .with_owner(part_id.clone());
        let res_id = graph.add_element(resistance);

        let res_typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_prop("unresolved_type", "ResistanceValue")
            .with_owner(res_id);
        graph.add_element(res_typing);

        let port_in = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("phaseIn")
            .with_owner(part_id.clone());
        graph.add_element(port_in);

        let port_out = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("phaseOut")
            .with_owner(part_id.clone());
        graph.add_element(port_out);

        // Add a ConstraintUsage child — this part IS wired
        let constraint = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("ohmsLaw")
            .with_owner(part_id);
        graph.add_element(constraint);

        let diags = physics_health_diagnostics(&graph);
        let ph005: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("PH005"))
            .collect();
        assert!(
            ph005.is_empty(),
            "part with ConstraintUsage should NOT emit PH005: {:?}",
            ph005,
        );
    }

    /// PH006: Real-typed attribute with physics name suggests ISQ type.
    #[test]
    fn ph006_suggests_isq_for_real_voltage() {
        let mut graph = ModelGraph::new();

        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("SimplePort");
        let def_id = graph.add_element(port_def);

        // Attribute named "voltage" but typed as Real — should trigger PH006
        let voltage = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("voltage")
            .with_prop("unresolved_type", "Real")
            .with_owner(def_id);
        graph.add_element(voltage);

        let diags = physics_health_diagnostics(&graph);
        let ph006: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("PH006"))
            .collect();
        assert_eq!(ph006.len(), 1, "should emit PH006 for voltage : Real");
        assert!(
            ph006[0].message.contains("did you mean"),
            "message should use 'did you mean' phrasing: {:?}",
            ph006[0].message
        );
        assert!(
            ph006[0].message.contains("ElectricPotentialValue"),
            "should suggest the ISQ type: {:?}",
            ph006[0].message
        );
    }

    /// PH006: Fuzzy match — ratedVoltage contains "voltage".
    #[test]
    fn ph006_fuzzy_matches_compound_names() {
        let mut graph = ModelGraph::new();

        let part_def = Element::new_with_kind(ElementKind::PartDefinition).with_name("Breaker");
        let def_id = graph.add_element(part_def);

        let attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("ratedVoltage")
            .with_prop("unresolved_type", "Real")
            .with_owner(def_id.clone());
        graph.add_element(attr);

        let attr2 = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("contactResistance")
            .with_prop("unresolved_type", "Real")
            .with_owner(def_id);
        graph.add_element(attr2);

        let diags = physics_health_diagnostics(&graph);
        let ph006: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("PH006"))
            .collect();
        assert_eq!(
            ph006.len(),
            2,
            "should match both ratedVoltage and contactResistance"
        );
    }

    /// P-RA2 Slice 1: PH001 and PH006 emissions are tagged with the Semantic tier.
    ///
    /// One representative test per direction — exhaustive per-code tests would
    /// just restate the .with_tier(...) line. If a future emission site forgets
    /// the tag, the regression will surface here or in the P-RA3 gate tests.
    #[test]
    fn ph001_and_ph006_carry_semantic_tier() {
        // --- PH006 (cheap to construct): voltage : Real inside a PortDefinition ---
        let mut graph = ModelGraph::new();
        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("SimplePort");
        let def_id = graph.add_element(port_def);
        let voltage = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("voltage")
            .with_prop("unresolved_type", "Real")
            .with_owner(def_id);
        graph.add_element(voltage);

        let diags = physics_health_diagnostics(&graph);
        let ph006 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("PH006"))
            .expect("PH006 should fire for voltage : Real");
        assert_eq!(
            ph006.tier,
            sysml_span::DiagnosticTier::Semantic,
            "PH006 must be tagged Semantic so the readiness gate releases it post-resolution"
        );

        // --- PH001 (needs two domains and a flow connection) ---
        let mut g = ModelGraph::new();

        // electrical port: voltage + current
        let elec_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("ElecPort");
        let elec_id = g.add_element(elec_def);
        g.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("voltage")
                .with_owner(elec_id.clone()),
        );
        g.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("current")
                .with_owner(elec_id),
        );

        // thermal port: temperature + heatFlow
        let therm_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("ThermPort");
        let therm_id = g.add_element(therm_def);
        g.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("temperature")
                .with_owner(therm_id.clone()),
        );
        g.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("heatFlow")
                .with_owner(therm_id),
        );

        // two PortUsages, one of each domain
        let src = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("src")
            .with_prop("portDefinition", "ElecPort");
        g.add_element(src);
        let tgt = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("tgt")
            .with_prop("portDefinition", "ThermPort");
        g.add_element(tgt);

        // flow connecting them (source/target paths resolve via bare port name)
        let flow = Element::new_with_kind(ElementKind::FlowUsage)
            .with_prop("source", "src")
            .with_prop("target", "tgt");
        g.add_element(flow);

        let diags = physics_health_diagnostics(&g);
        let ph001 = diags
            .iter()
            .find(|d| d.code.as_deref() == Some("PH001"))
            .expect("PH001 should fire for electrical->thermal flow");
        assert_eq!(
            ph001.tier,
            sysml_span::DiagnosticTier::Semantic,
            "PH001 must be tagged Semantic"
        );
    }

    /// PH006: ISQ-typed attribute does NOT trigger suggestion.
    #[test]
    fn ph006_no_suggestion_for_isq_typed() {
        let mut graph = ModelGraph::new();

        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("GoodPort");
        let def_id = graph.add_element(port_def);

        // Already using ISQ type — no PH006
        let voltage = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("voltage")
            .with_prop("unresolved_type", "ElectricPotentialValue")
            .with_owner(def_id);
        graph.add_element(voltage);

        let diags = physics_health_diagnostics(&graph);
        let ph006: Vec<_> = diags
            .iter()
            .filter(|d| d.code.as_deref() == Some("PH006"))
            .collect();
        assert!(
            ph006.is_empty(),
            "ISQ-typed attributes should NOT trigger PH006"
        );
    }
}
