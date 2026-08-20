//! Port classification: ISQ-typed and heuristic fallback.
//!
//! Given a `PortDefinition` element in a `ModelGraph`, classifies its features
//! (children) by physical domain and variable role using either ISQ dimension
//! vectors (high confidence) or name-based heuristics (lower confidence).

use std::collections::HashMap;

use super::dimension::DimensionVector;
use super::domain::{BondGraphRole, PhysicsDomainRegistry, VariableRole};
use crate::metadata::has_metadata_typed;
use crate::{ElementKind, ModelGraph};
use sysml_span::Diagnostic;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of classifying a port definition's features by physics domain.
#[derive(Clone, Debug)]
pub struct PortClassification {
    /// The inferred physics domain (e.g., "electrical", "thermal"), or `None` if
    /// classification failed entirely.
    pub domain: Option<&'static str>,
    /// Per-feature classification results.
    pub features: Vec<ClassifiedFeature>,
    /// Overall confidence level for this classification.
    pub confidence: ClassificationConfidence,
    /// Informational/warning diagnostics produced during classification.
    pub diagnostics: Vec<Diagnostic>,
    /// `true` when this port carries measurement-style signals rather than
    /// power: its classified features form an *incomplete* effort/flow
    /// conjugate pair for their domain (only-effort or only-flow). This is
    /// the Modelica causal-signal-connector shape — e.g. a sensor port whose
    /// item carries an RMS current reading plus bookkeeping siblings
    /// (timestamps, validity flags). A port with BOTH effort and flow
    /// features of one domain is a power port (`is_signal == false`).
    pub is_signal: bool,
    /// For signal ports, the physics domain of the carried quantity (kept
    /// for unit/type checking across signal links). `None` for power ports.
    pub carrier_domain: Option<&'static str>,
}

/// A single feature (child usage) within a port, with its inferred role and dimension.
#[derive(Clone, Debug)]
pub struct ClassifiedFeature {
    /// The feature's name (from `element.name`).
    pub name: String,
    /// Inferred variable role (Effort, Flow, Parameter, etc.).
    pub role: VariableRole,
    /// ISQ dimension vector, if resolved.
    pub dimension: Option<DimensionVector>,
    /// Bond graph role (R/C/I/Effort/Flow), if dimension arithmetic resolved it.
    pub bond_graph_role: Option<BondGraphRole>,
}

/// Confidence level of a port classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClassificationConfidence {
    /// The signal/power role was explicitly declared via `@Signal` /
    /// `@SignalPort` / `@PowerPort` metadata on the port definition
    /// (RSC-1.2). Declared wins over every inference tier.
    Declared,
    /// All features were typed with ISQ types that resolved to known dimensions.
    ISQTyped,
    /// At least one feature was classified by name heuristic rather than ISQ type.
    NameHeuristic,
    /// No features could be classified.
    Unknown,
}

// ---------------------------------------------------------------------------
// Name-based heuristic table
// ---------------------------------------------------------------------------

/// Static lookup table for name-based classification.
/// Each entry is (substring, domain, role).
const NAME_HEURISTICS: &[(&str, &str, VariableRole)] = &[
    ("current", "electrical", VariableRole::Flow),
    ("voltage", "electrical", VariableRole::Effort),
    ("resistance", "electrical", VariableRole::Parameter),
    ("temperature", "thermal", VariableRole::Effort),
    ("temp", "thermal", VariableRole::Effort),
    ("heat", "thermal", VariableRole::Flow),
    ("thermal", "thermal", VariableRole::Parameter),
    ("pressure", "hydraulic", VariableRole::Effort),
    ("flow_rate", "hydraulic", VariableRole::Flow),
    ("force", "mechanical_translational", VariableRole::Flow),
    ("velocity", "mechanical_translational", VariableRole::Effort),
    ("speed", "mechanical_translational", VariableRole::Effort),
];

/// ISQ type suggestions for common attribute names.
///
/// When a user types `attribute voltage : Real`, we can suggest the ISQ type
/// that would unlock physics classification and simulation features.
/// Each entry is `(name_pattern, isq_type, description)`.
const ISQ_SUGGESTIONS: &[(&str, &str, &str)] = &[
    // Electrical
    (
        "voltage",
        "ISQ::ElectricPotentialValue",
        "electrical effort — enables voltage propagation",
    ),
    (
        "potential",
        "ISQ::ElectricPotentialValue",
        "electrical effort",
    ),
    ("emf", "ISQ::ElectricPotentialValue", "electromotive force"),
    // NOTE: "current" is ambiguous (electric current vs "current level/state").
    // Only match as suffix: loadCurrent, ratedCurrent — not currentLevel, currentState.
    ("amperage", "ISQ::ElectricCurrentValue", "electrical flow"),
    (
        "resistance",
        "ISQ::ResistanceValue",
        "R-element — enables V=IR constitutive relation",
    ),
    ("impedance", "ISQ::ImpedanceValue", "complex resistance"),
    (
        "capacitance",
        "ISQ::CapacitanceValue",
        "C-element — enables energy storage modeling",
    ),
    (
        "inductance",
        "ISQ::InductanceValue",
        "I-element — enables energy storage modeling",
    ),
    ("charge", "ISQ::ElectricChargeValue", "electric charge"),
    // Thermal
    (
        "temperature",
        "ISQ::ThermodynamicTemperatureValue",
        "thermal effort — enables temperature propagation",
    ),
    (
        "heatflow",
        "ISQ::HeatFlowRateValue",
        "thermal flow — enables energy balance",
    ),
    ("heat_flow", "ISQ::HeatFlowRateValue", "thermal flow"),
    ("heatrate", "ISQ::HeatFlowRateValue", "thermal flow"),
    // Hydraulic
    (
        "pressure",
        "ISQ::PressureValue",
        "hydraulic effort — enables pressure propagation",
    ),
    (
        "massflow",
        "ISQ::MassFlowRateValue",
        "hydraulic flow — enables mass balance",
    ),
    ("mass_flow", "ISQ::MassFlowRateValue", "hydraulic flow"),
    ("flowrate", "ISQ::VolumeFlowRateValue", "volume flow rate"),
    ("flow_rate", "ISQ::VolumeFlowRateValue", "volume flow rate"),
    // Mechanical
    (
        "force",
        "ISQ::ForceValue",
        "mechanical flow — enables force balance",
    ),
    ("velocity", "ISQ::SpeedValue", "mechanical effort"),
    ("torque", "ISQ::TorqueValue", "rotational mechanical flow"),
    (
        "angular",
        "ISQ::AngularVelocityValue",
        "rotational mechanical effort",
    ),
    // General
    ("frequency", "ISQ::FrequencyValue", "frequency quantity"),
    ("length", "ISQ::LengthValue", "length quantity"),
    ("density", "ISQ::DensityValue", "density quantity"),
    (
        "acceleration",
        "ISQ::AccelerationValue",
        "acceleration quantity",
    ),
];

/// Suffix-only suggestions for ambiguous words.
/// These only match when they appear at the END of the name.
/// e.g., `loadCurrent` matches, `currentLevel` does not.
const ISQ_SUFFIX_ONLY: &[(&str, &str, &str)] = &[
    (
        "current",
        "ISQ::ElectricCurrentValue",
        "electrical flow — enables KCL at junctions",
    ),
    ("mass", "ISQ::MassValue", "mass quantity"),
    ("power", "ISQ::PowerValue", "power quantity"),
    ("energy", "ISQ::EnergyValue", "energy quantity"),
    ("speed", "ISQ::SpeedValue", "mechanical effort"),
    ("volume", "ISQ::VolumeValue", "volume quantity"),
    ("area", "ISQ::AreaValue", "area quantity"),
];

/// Suggest an ISQ type for an attribute name typed as `Real`.
///
/// Uses word-boundary-aware matching: `ratedVoltage` matches `voltage` because
/// "Voltage" starts a new camelCase word. Ambiguous words like `current` only
/// match as suffixes — `loadCurrent` matches but `currentLevel` does not.
///
/// Returns `(isq_type, description)` or `None`.
pub fn suggest_isq_type(attr_name: &str) -> Option<(&'static str, &'static str)> {
    let lower = attr_name.to_lowercase();

    // First check the main table (word-boundary matching)
    for &(pattern, isq_type, desc) in ISQ_SUGGESTIONS {
        if matches_at_word_boundary(&lower, attr_name, pattern) {
            return Some((isq_type, desc));
        }
    }

    // Then check suffix-only patterns (exact or suffix match only)
    for &(pattern, isq_type, desc) in ISQ_SUFFIX_ONLY {
        if lower == pattern || lower.ends_with(pattern) {
            // Verify word boundary before the suffix
            if lower == pattern {
                return Some((isq_type, desc));
            }
            let prefix_len = lower.len() - pattern.len();
            let bytes = attr_name.as_bytes();
            if prefix_len > 0 {
                let before = bytes[prefix_len - 1];
                let at = bytes[prefix_len];
                if before == b'_' || (before.is_ascii_lowercase() && at.is_ascii_uppercase()) {
                    return Some((isq_type, desc));
                }
            }
        }
    }

    None
}

/// Check if `pattern` appears at a word boundary in the attribute name.
///
/// A word boundary is: start of string, after `_`, or at a camelCase transition
/// (lowercase followed by uppercase).
fn matches_at_word_boundary(lower_name: &str, original_name: &str, pattern: &str) -> bool {
    // Exact match
    if lower_name == pattern {
        return true;
    }

    // Suffix match (e.g., "ratedVoltage" ends with "voltage")
    if lower_name.ends_with(pattern) {
        let prefix_len = lower_name.len() - pattern.len();
        if prefix_len == 0 {
            return true;
        }
        // Check for word boundary at the junction
        let bytes = original_name.as_bytes();
        if prefix_len < bytes.len() {
            let before = bytes[prefix_len - 1];
            let at = bytes[prefix_len];
            // Underscore boundary: contact_resistance
            if before == b'_' {
                return true;
            }
            // camelCase boundary: contactResistance (lowercase before, uppercase at)
            if before.is_ascii_lowercase() && at.is_ascii_uppercase() {
                return true;
            }
        }
        return false;
    }

    // Check for pattern at any word boundary position (not just suffix)
    if let Some(pos) = lower_name.find(pattern) {
        let end_pos = pos + pattern.len();

        // Pattern must start at a word boundary
        let starts_at_boundary = pos == 0
            || original_name.as_bytes().get(pos - 1) == Some(&b'_')
            || (pos > 0
                && original_name.as_bytes()[pos - 1].is_ascii_lowercase()
                && original_name.as_bytes()[pos].is_ascii_uppercase());

        // Pattern must end at a word boundary (end of string, or next char is uppercase/underscore)
        let ends_at_boundary = end_pos >= lower_name.len()
            || original_name.as_bytes().get(end_pos) == Some(&b'_')
            || original_name
                .as_bytes()
                .get(end_pos)
                .map_or(false, |b| b.is_ascii_uppercase());

        if starts_at_boundary && ends_at_boundary {
            return true;
        }
    }

    false
}

/// Attempt to classify a feature by its name using substring matching.
///
/// Returns the domain and variable role if a heuristic matches, or `None`.
pub fn classify_by_name(feature_name: &str) -> Option<(&'static str, VariableRole)> {
    let lower = feature_name.to_lowercase();
    for &(pattern, domain, ref role) in NAME_HEURISTICS {
        if lower.contains(pattern) {
            return Some((domain, role.clone()));
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Port classification
// ---------------------------------------------------------------------------

/// Classify a `PortDefinition` by inspecting its child features' types and names.
///
/// Algorithm:
/// 1. Find the `PortDefinition` element by name.
/// 2. Collect its `ItemUsage` and `AttributeUsage` children.
/// 3. For each child, try ISQ-based classification via the registry, then
///    nested type walking (ItemDefinition → AttributeUsage children), then
///    fall back to name heuristics.
/// 4. Determine the overall domain by majority vote among classified features.
pub fn classify_port_definition(
    port_def_name: &str,
    graph: &ModelGraph,
    registry: &PhysicsDomainRegistry,
) -> PortClassification {
    let mut diagnostics = Vec::new();
    let mut features = Vec::new();
    let mut used_heuristic = false;
    let mut any_classified = false;

    // (a) Find the PortDefinition element by name.
    let port_def_id = graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::PortDefinition && e.name.as_deref() == Some(port_def_name))
        .map(|e| e.id.clone());

    let port_def_id = match port_def_id {
        Some(id) => id,
        None => {
            diagnostics.push(Diagnostic::warning(format!(
                "PortDefinition '{}' not found in model graph.",
                port_def_name
            )));
            return PortClassification {
                domain: None,
                features: Vec::new(),
                confidence: ClassificationConfidence::Unknown,
                diagnostics,
                is_signal: false,
                carrier_domain: None,
            };
        }
    };

    // (b) Get children that are ItemUsage or AttributeUsage.
    let child_features: Vec<_> = graph
        .elements
        .values()
        .filter(|e| {
            e.owner.as_ref() == Some(&port_def_id)
                && (e.kind == ElementKind::ItemUsage || e.kind == ElementKind::AttributeUsage)
        })
        .collect();

    // (c) Domain vote counter.
    let mut domain_votes: HashMap<&'static str, usize> = HashMap::new();

    // (d) Classify each feature.
    for child in &child_features {
        let feat_name = match &child.name {
            Some(n) => n.clone(),
            None => continue,
        };

        // Read type name from props — check direct properties first, then
        // FeatureTyping children (parser stores type on FeatureTyping relationship).
        let type_name = child
            .get_prop("typeName")
            .or_else(|| child.get_prop("unresolved_type"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                // Look for FeatureTyping child with unresolved_type
                graph
                    .elements
                    .values()
                    .find(|e| {
                        e.owner.as_ref() == Some(&child.id) && e.kind == ElementKind::FeatureTyping
                    })
                    .and_then(|ft| ft.get_prop("unresolved_type"))
                    .and_then(|v| v.as_str())
            });

        // Try ISQ classification first.
        if let Some(tn) = type_name {
            if tn != "Real" {
                if let Some(dim) = registry.dimension_for_type(tn) {
                    // Use category-aware classification if the ISQ type is known
                    let classification = if let Some(entry) = super::isq_types::lookup_isq_type(tn)
                    {
                        registry.classify_dimension_with_hint(dim, entry.2)
                    } else {
                        registry.classify_dimension(dim)
                    };
                    if let Some((domain, role)) = classification {
                        // Also compute the bond graph role for richer classification
                        let bg_role = domain.classify_bond_graph_role(dim);
                        *domain_votes.entry(domain.name).or_insert(0) += 1;
                        any_classified = true;
                        features.push(ClassifiedFeature {
                            name: feat_name,
                            role,
                            dimension: Some(*dim),
                            bond_graph_role: Some(bg_role),
                        });
                        continue;
                    }

                    // Fallthrough: classify_dimension only matches effort/flow.
                    // Try full bond graph classification (R/C/I/Power).
                    if let Some(dim) = registry.dimension_for_type(tn) {
                        let full = if let Some(entry) = super::isq_types::lookup_isq_type(tn) {
                            registry.classify_dimension_full_with_hint(dim, entry.2)
                        } else {
                            registry.classify_dimension_full(dim)
                        };
                        if let Some((domain, bg_role)) = full {
                            *domain_votes.entry(domain.name).or_insert(0) += 1;
                            any_classified = true;
                            features.push(ClassifiedFeature {
                                name: feat_name,
                                role: bg_role.to_variable_role(),
                                dimension: Some(*dim),
                                bond_graph_role: Some(bg_role),
                            });
                            continue;
                        }
                    }
                }

                // Nested type walking: look up the ItemDefinition and walk its
                // AttributeUsage children to find classifiable names.
                let nested = classify_nested_type(tn, graph, registry, 0);
                if !nested.is_empty() {
                    for (nested_feat, domain, role, dim, bg_role) in &nested {
                        *domain_votes.entry(domain).or_insert(0) += 1;
                        any_classified = true;
                        if dim.is_none() {
                            used_heuristic = true;
                        }
                        features.push(ClassifiedFeature {
                            name: nested_feat.clone(),
                            role: role.clone(),
                            dimension: *dim,
                            bond_graph_role: *bg_role,
                        });
                    }
                    continue;
                }
            }
        }

        // Fallback: name heuristic on the feature name itself.
        if let Some((domain, role)) = classify_by_name(&feat_name) {
            *domain_votes.entry(domain).or_insert(0) += 1;
            any_classified = true;
            used_heuristic = true;
            features.push(ClassifiedFeature {
                name: feat_name,
                role,
                dimension: None,
                bond_graph_role: None,
            });
        }
    }

    // (e) Determine overall domain by majority vote. Ties break
    // deterministically by domain name (HashMap iteration order is not
    // stable across runs).
    let domain = domain_votes
        .iter()
        .max_by_key(|(name, count)| (**count, std::cmp::Reverse(*name)))
        .map(|(name, _)| *name);

    // (f) If heuristic was used, push an informational diagnostic.
    if used_heuristic {
        if let Some(d) = domain {
            diagnostics.push(Diagnostic::info(format!(
                "Port '{}' classified as {} by name heuristic. Consider using ISQ types.",
                port_def_name, d
            )));
        }
    }

    let confidence = if !any_classified {
        ClassificationConfidence::Unknown
    } else if used_heuristic {
        ClassificationConfidence::NameHeuristic
    } else {
        ClassificationConfidence::ISQTyped
    };

    // (g) Signal classification: a classified port whose features form an
    // incomplete effort/flow conjugate pair (exactly one of {effort, flow}
    // present) carries a measurement signal, not power. Bookkeeping siblings
    // (timestamps, validity flags, strings) never classify as effort/flow,
    // so they don't affect this. The quantity's domain is retained as the
    // carrier for unit/type checking across signal links.
    let has_effort = features.iter().any(|f| f.role == VariableRole::Effort);
    let has_flow = features.iter().any(|f| f.role == VariableRole::Flow);
    let is_signal = domain.is_some() && (has_effort != has_flow);
    let carrier_domain = if is_signal { domain } else { None };

    // (h) RSC-1.2 declared-role override: `@Signal` / `@SignalPort` metadata
    // on the port definition forces signal classification; `@PowerPort`
    // forces power. Declared wins over inferred (the Modelica `flow`-keyword
    // lesson, audit G7). Matching is by metadata type name only — bare or
    // qualified, last-`::`-segment — via `metadata::has_metadata_typed`;
    // no stdlib metadata def is required.
    let declared_signal = has_metadata_typed(graph, &port_def_id, "Signal")
        || has_metadata_typed(graph, &port_def_id, "SignalPort");
    let declared_power = has_metadata_typed(graph, &port_def_id, "PowerPort");

    let (is_signal, carrier_domain, confidence) = match (declared_signal, declared_power) {
        (true, true) => {
            // Contradictory declaration — fail loud, keep inference.
            diagnostics.push(Diagnostic::warning(format!(
                "Port '{}' declares both @Signal and @PowerPort metadata; \
                 ignoring both and keeping the inferred classification.",
                port_def_name
            )));
            (is_signal, carrier_domain, confidence)
        }
        (true, false) => (true, domain, ClassificationConfidence::Declared),
        (false, true) => (false, None, ClassificationConfidence::Declared),
        (false, false) => (is_signal, carrier_domain, confidence),
    };

    PortClassification {
        domain,
        features,
        confidence,
        diagnostics,
        is_signal,
        carrier_domain,
    }
}

/// Classify a PartDefinition's attributes by ISQ type.
///
/// Walks the PartDefinition's AttributeUsage children, resolves their ISQ types
/// via the registry, and returns features with bond graph roles (R/C/I).
/// Used by PH005 to detect unwired R/C/I elements.
pub fn classify_part_attributes(
    part_def_name: &str,
    graph: &ModelGraph,
    registry: &PhysicsDomainRegistry,
) -> Vec<ClassifiedFeature> {
    let mut features = Vec::new();

    // Find the PartDefinition element
    let part_def = graph.elements.values().find(|e| {
        e.kind == ElementKind::PartDefinition && e.name.as_deref() == Some(part_def_name)
    });

    let part_def = match part_def {
        Some(e) => e,
        None => return features,
    };

    // Walk children looking for AttributeUsage with ISQ types
    for child in graph.children_of(&part_def.id) {
        if child.kind != ElementKind::AttributeUsage {
            continue;
        }

        let feat_name = match &child.name {
            Some(n) => n.clone(),
            None => continue,
        };

        // Get the type name from the child or its FeatureTyping children
        let type_name = child
            .get_prop("unresolved_type")
            .and_then(|v| v.as_str())
            .map(String::from)
            .or_else(|| {
                graph.children_of(&child.id).find_map(|ft| {
                    if ft.kind == ElementKind::FeatureTyping {
                        ft.get_prop("unresolved_type")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    } else {
                        None
                    }
                })
            });

        let type_name = match type_name {
            Some(tn) => tn,
            None => continue,
        };

        // Try ISQ type lookup
        if let Some(entry) = super::isq_types::lookup_isq_type(&type_name) {
            let dim = &entry.1;
            let classification = registry.classify_dimension_with_hint(dim, entry.2);
            let full = registry.classify_dimension_full_with_hint(dim, entry.2);
            let bond_graph_role = full.map(|(_, bgr)| bgr);

            if let Some((_domain, role)) = classification {
                features.push(ClassifiedFeature {
                    name: feat_name,
                    role,
                    dimension: Some(dim.clone()),
                    bond_graph_role,
                });
            } else if let Some((_domain, bgr)) = full {
                features.push(ClassifiedFeature {
                    name: feat_name,
                    role: bgr.to_variable_role(),
                    dimension: Some(dim.clone()),
                    bond_graph_role: Some(bgr),
                });
            }
        }
    }

    features
}

/// Walk into an ItemDefinition (or AttributeDefinition) to classify its child
/// attributes by ISQ type or name heuristic.
///
/// For example, `ACPhase :> ACPower` has children `voltage : Real` and
/// `current : Real`. We walk into `ACPhase`, find those attributes, and
/// classify them by name heuristic → electrical.
///
/// Returns a vec of `(feature_name, domain, role, dimension)` tuples.
/// Limits recursion to `MAX_NESTED_DEPTH` levels to prevent cycles.
const MAX_NESTED_DEPTH: usize = 3;

fn classify_nested_type(
    type_name: &str,
    graph: &ModelGraph,
    registry: &PhysicsDomainRegistry,
    depth: usize,
) -> Vec<(
    String,
    &'static str,
    VariableRole,
    Option<DimensionVector>,
    Option<BondGraphRole>,
)> {
    if depth >= MAX_NESTED_DEPTH {
        return Vec::new();
    }

    let mut results = Vec::new();

    // Find the ItemDefinition or AttributeDefinition by name
    let def_id = graph
        .elements
        .values()
        .find(|e| {
            matches!(
                e.kind,
                ElementKind::ItemDefinition | ElementKind::AttributeDefinition
            ) && e.name.as_deref() == Some(type_name)
        })
        .map(|e| e.id.clone());

    let def_id = match def_id {
        Some(id) => id,
        None => return results,
    };

    // Walk children: AttributeUsage and ItemUsage
    let children: Vec<_> = graph
        .elements
        .values()
        .filter(|e| {
            e.owner.as_ref() == Some(&def_id)
                && matches!(e.kind, ElementKind::AttributeUsage | ElementKind::ItemUsage)
        })
        .collect();

    for child in &children {
        let name = match &child.name {
            Some(n) => n.clone(),
            None => continue,
        };

        // Try ISQ type on the child's type — check props then FeatureTyping children
        let child_type = child
            .get_prop("typeName")
            .or_else(|| child.get_prop("unresolved_type"))
            .and_then(|v| v.as_str())
            .or_else(|| {
                graph
                    .elements
                    .values()
                    .find(|e| {
                        e.owner.as_ref() == Some(&child.id) && e.kind == ElementKind::FeatureTyping
                    })
                    .and_then(|ft| ft.get_prop("unresolved_type"))
                    .and_then(|v| v.as_str())
            });

        if let Some(ct) = child_type {
            if ct != "Real" {
                if let Some(dim) = registry.dimension_for_type(ct) {
                    // Use category-aware classification when the ISQ type is
                    // known (mirrors classify_port_definition) — without the
                    // hint, degenerate dimension arithmetic misclassifies
                    // (e.g. DurationValue timestamps as luminous C-elements).
                    let isq_entry = super::isq_types::lookup_isq_type(ct);
                    let classification = if let Some(entry) = isq_entry {
                        registry.classify_dimension_with_hint(dim, entry.2)
                    } else {
                        registry.classify_dimension(dim)
                    };
                    if let Some((domain, role)) = classification {
                        let bg_role = domain.classify_bond_graph_role(dim);
                        results.push((name, domain.name, role, Some(*dim), Some(bg_role)));
                        continue;
                    }
                    // Fallthrough: try full bond graph classification
                    let full = if let Some(entry) = isq_entry {
                        registry.classify_dimension_full_with_hint(dim, entry.2)
                    } else {
                        registry.classify_dimension_full(dim)
                    };
                    if let Some((domain, bg_role)) = full {
                        results.push((
                            name,
                            domain.name,
                            bg_role.to_variable_role(),
                            Some(*dim),
                            Some(bg_role),
                        ));
                        continue;
                    }
                }
                // Recurse into the child's type
                let nested = classify_nested_type(ct, graph, registry, depth + 1);
                if !nested.is_empty() {
                    results.extend(nested);
                    continue;
                }
            }
        }

        // Name heuristic on the attribute name
        if let Some((domain, role)) = classify_by_name(&name) {
            results.push((name, domain, role, None, None));
        }
    }

    // Also check superclasses: if the definition has a Subclassification,
    // walk the parent type too (e.g., ACPhase :> ACPower).
    let superclass_names: Vec<String> = graph
        .elements
        .values()
        .filter(|e| e.owner.as_ref() == Some(&def_id) && e.kind == ElementKind::Subclassification)
        .filter_map(|e| {
            e.get_prop("unresolved_superclassifier")
                .and_then(|v| v.as_str())
                .map(|s| s.to_owned())
        })
        .collect();

    for super_name in superclass_names {
        let super_results = classify_nested_type(&super_name, graph, registry, depth + 1);
        // Only add features not already found (avoid duplicates from override)
        for r in super_results {
            if !results.iter().any(|(n, _, _, _, _)| n == &r.0) {
                results.push(r);
            }
        }
    }

    results
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{Element, ElementId, Value};

    #[test]
    fn classify_by_name_rms_current() {
        let result = classify_by_name("rms_current");
        assert_eq!(result, Some(("electrical", VariableRole::Flow)));
    }

    #[test]
    fn classify_by_name_temperature_k() {
        let result = classify_by_name("temperature_k");
        assert_eq!(result, Some(("thermal", VariableRole::Effort)));
    }

    #[test]
    fn classify_by_name_power_is_ambiguous() {
        // "power" is not in the heuristic table
        assert_eq!(classify_by_name("power"), None);
    }

    #[test]
    fn classify_by_name_random_thing() {
        assert_eq!(classify_by_name("random_thing"), None);
    }

    #[test]
    fn classify_port_definition_with_name_heuristic() {
        let mut graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        // Create a PortDefinition named "ElectricalPort".
        let port_id = ElementId::new_v4();
        graph.add_element(
            Element::new(port_id.clone(), ElementKind::PortDefinition).with_name("ElectricalPort"),
        );

        // Add an ItemUsage child named "current" typed as "Real".
        let item_id = ElementId::new_v4();
        graph.add_element(
            Element::new(item_id, ElementKind::ItemUsage)
                .with_owner(port_id.clone())
                .with_name("current")
                .with_prop("typeName", Value::String("Real".into())),
        );

        let result = classify_port_definition("ElectricalPort", &graph, &registry);

        assert_eq!(result.confidence, ClassificationConfidence::NameHeuristic);
        assert_eq!(result.domain, Some("electrical"));
        assert_eq!(result.features.len(), 1);
        assert_eq!(result.features[0].name, "current");
        assert_eq!(result.features[0].role, VariableRole::Flow);
        assert!(result.features[0].dimension.is_none());
        // Should have an info diagnostic about heuristic usage.
        assert!(
            result.diagnostics.iter().any(|d| {
                let msg = format!("{}", d);
                msg.contains("heuristic")
            }),
            "expected a diagnostic mentioning heuristic, got: {:?}",
            result.diagnostics
        );
    }

    /// Regression (multi-circuit protection fixture): a measurement item carrying
    /// one electrical quantity plus a timestamp must classify electrical — the
    /// DurationValue timestamp used to vote "luminous" via degenerate R/C/I
    /// arithmetic and win the tie.
    #[test]
    fn measurement_item_with_timestamp_classifies_electrical() {
        let mut graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        // item def CurrentSenseMeasurement { sense_ma; timestamp_us }
        let item_def_id = ElementId::new_v4();
        graph.add_element(
            Element::new(item_def_id.clone(), ElementKind::ItemDefinition)
                .with_name("CurrentSenseMeasurement"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(item_def_id.clone())
                .with_name("sense_ma")
                .with_prop("typeName", Value::String("ElectricCurrentValue".into())),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(item_def_id)
                .with_name("timestamp_us")
                .with_prop("typeName", Value::String("DurationValue".into())),
        );

        // port def CurrentSensePort { out item reading : CurrentSenseMeasurement }
        let port_id = ElementId::new_v4();
        graph.add_element(
            Element::new(port_id.clone(), ElementKind::PortDefinition)
                .with_name("CurrentSensePort"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::ItemUsage)
                .with_owner(port_id)
                .with_name("reading")
                .with_prop(
                    "typeName",
                    Value::String("CurrentSenseMeasurement".into()),
                ),
        );

        let result = classify_port_definition("CurrentSensePort", &graph, &registry);
        assert_eq!(
            result.domain,
            Some("electrical"),
            "timestamp attribute must not flip the domain: {:?}",
            result.features
        );
        // RSC-1.1: flow-only quantity + bookkeeping sibling = signal port.
        assert!(
            result.is_signal,
            "measurement-only port (flow quantity + timestamp) must classify as signal: {:?}",
            result.features
        );
        assert_eq!(
            result.carrier_domain,
            Some("electrical"),
            "signal port keeps the quantity's domain as carrier"
        );
    }

    /// RSC-1.1: a measurement item port carrying an RMS current reading plus
    /// a timestamp classifies as a signal port with an electrical carrier —
    /// the incomplete (flow-only) conjugate pair is the signal condition.
    #[test]
    fn measurement_item_port_is_signal_with_electrical_carrier() {
        let mut graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        // item def CurrentMeasurement { rms : ElectricCurrentValue; timestamp_us : DurationValue }
        let item_def_id = ElementId::new_v4();
        graph.add_element(
            Element::new(item_def_id.clone(), ElementKind::ItemDefinition)
                .with_name("CurrentMeasurement"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(item_def_id.clone())
                .with_name("rms")
                .with_prop("typeName", Value::String("ElectricCurrentValue".into())),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(item_def_id)
                .with_name("timestamp_us")
                .with_prop("typeName", Value::String("DurationValue".into())),
        );

        // port def CurrentSensePort { out item reading : CurrentMeasurement }
        let port_id = ElementId::new_v4();
        graph.add_element(
            Element::new(port_id.clone(), ElementKind::PortDefinition)
                .with_name("CurrentSensePort"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::ItemUsage)
                .with_owner(port_id)
                .with_name("reading")
                .with_prop("typeName", Value::String("CurrentMeasurement".into())),
        );

        let result = classify_port_definition("CurrentSensePort", &graph, &registry);
        assert!(result.is_signal, "flow-only measurement port is a signal");
        assert_eq!(result.carrier_domain, Some("electrical"));
        assert_eq!(result.domain, Some("electrical"));
    }

    /// RSC-1.1: a full power port (both effort AND flow features of one
    /// domain, ACPhase-style voltage+current) is NOT a signal port.
    #[test]
    fn full_effort_flow_port_is_not_signal() {
        let mut graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        let port_id = ElementId::new_v4();
        graph.add_element(
            Element::new(port_id.clone(), ElementKind::PortDefinition).with_name("ACPhasePort"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(port_id.clone())
                .with_name("voltage")
                .with_prop("typeName", Value::String("Real".into())),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(port_id)
                .with_name("current")
                .with_prop("typeName", Value::String("Real".into())),
        );

        let result = classify_port_definition("ACPhasePort", &graph, &registry);
        assert_eq!(result.domain, Some("electrical"));
        assert!(
            !result.is_signal,
            "complete effort/flow conjugate pair stays a power port: {:?}",
            result.features
        );
        assert_eq!(result.carrier_domain, None);
    }

    #[test]
    fn classify_port_definition_not_found() {
        let graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        let result = classify_port_definition("NonExistent", &graph, &registry);
        assert_eq!(result.confidence, ClassificationConfidence::Unknown);
        assert_eq!(result.domain, None);
        assert!(result.features.is_empty());
    }

    /// Test: Nested type walking classifies ACPhase → {voltage, current} → electrical.
    #[test]
    fn classify_port_with_nested_item_type() {
        let mut graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        // Create ItemDefinition "ACPower" with voltage + current attributes
        let acpower_id = ElementId::new_v4();
        graph.add_element(
            Element::new(acpower_id.clone(), ElementKind::ItemDefinition).with_name("ACPower"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(acpower_id.clone())
                .with_name("voltage")
                .with_prop("typeName", Value::String("Real".into())),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(acpower_id.clone())
                .with_name("current")
                .with_prop("typeName", Value::String("Real".into())),
        );

        // Create ItemDefinition "ACPhase" :> ACPower (inherits voltage, current)
        let acphase_id = ElementId::new_v4();
        graph.add_element(
            Element::new(acphase_id.clone(), ElementKind::ItemDefinition).with_name("ACPhase"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::Subclassification)
                .with_owner(acphase_id.clone())
                .with_prop(
                    "unresolved_superclassifier",
                    Value::String("ACPower".into()),
                ),
        );

        // Create PortDefinition "PhasePort" with item "power : ACPhase"
        let port_id = ElementId::new_v4();
        graph.add_element(
            Element::new(port_id.clone(), ElementKind::PortDefinition).with_name("PhasePort"),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::ItemUsage)
                .with_owner(port_id.clone())
                .with_name("power")
                .with_prop("typeName", Value::String("ACPhase".into())),
        );

        let result = classify_port_definition("PhasePort", &graph, &registry);

        assert_eq!(
            result.domain,
            Some("electrical"),
            "PhasePort should be classified as electrical"
        );
        assert!(
            result.features.len() >= 2,
            "should find voltage and current features"
        );
        assert_eq!(result.confidence, ClassificationConfidence::NameHeuristic);

        let feat_names: Vec<_> = result.features.iter().map(|f| f.name.as_str()).collect();
        assert!(feat_names.contains(&"voltage"), "should find voltage");
        assert!(feat_names.contains(&"current"), "should find current");
    }

    /// Helper: attach an ANONYMOUS MetadataUsage + FeatureTyping child to an
    /// element, mirroring the tree-sitter parser's `@Type` lowering
    /// (dispatch.rs "metadata_usage" arm, commits bc34d833 + 00f6e550):
    /// the MetadataUsage carries no name; the type reference rides on an
    /// owned FeatureTyping child's `unresolved_type` prop.
    fn attach_metadata(graph: &mut ModelGraph, owner: &ElementId, type_ref: &str) {
        let meta_id = ElementId::new_v4();
        graph.add_element(
            Element::new(meta_id.clone(), ElementKind::MetadataUsage).with_owner(owner.clone()),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::FeatureTyping)
                .with_owner(meta_id.clone())
                .with_prop("typedFeature", Value::Ref(meta_id))
                .with_prop("unresolved_type", Value::String(type_ref.into())),
        );
    }

    /// Helper: a power-shaped port def (complete voltage+current pair).
    fn power_shaped_port(graph: &mut ModelGraph, name: &str) -> ElementId {
        let port_id = ElementId::new_v4();
        graph.add_element(
            Element::new(port_id.clone(), ElementKind::PortDefinition).with_name(name),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(port_id.clone())
                .with_name("voltage")
                .with_prop("typeName", Value::String("Real".into())),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(port_id.clone())
                .with_name("current")
                .with_prop("typeName", Value::String("Real".into())),
        );
        port_id
    }

    /// Helper: a measurement-shaped port def (flow-only — inferred signal).
    fn measurement_shaped_port(graph: &mut ModelGraph, name: &str) -> ElementId {
        let port_id = ElementId::new_v4();
        graph.add_element(
            Element::new(port_id.clone(), ElementKind::PortDefinition).with_name(name),
        );
        graph.add_element(
            Element::new(ElementId::new_v4(), ElementKind::AttributeUsage)
                .with_owner(port_id.clone())
                .with_name("measuredCurrent")
                .with_prop("typeName", Value::String("Real".into())),
        );
        port_id
    }

    /// RSC-1.2 (1): a full effort/flow pair would infer power, but `@Signal`
    /// metadata forces signal classification. Declared > inferred.
    #[test]
    fn declared_signal_overrides_inferred_power() {
        let mut graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        let port_id = power_shaped_port(&mut graph, "DeclaredSignalPort");
        attach_metadata(&mut graph, &port_id, "Signal");

        let result = classify_port_definition("DeclaredSignalPort", &graph, &registry);
        assert!(
            result.is_signal,
            "@Signal must force is_signal=true over the inferred power pair: {:?}",
            result.features
        );
        assert_eq!(result.confidence, ClassificationConfidence::Declared);
        assert_eq!(
            result.carrier_domain,
            Some("electrical"),
            "declared signal keeps the inferred domain as carrier"
        );
        assert_eq!(result.domain, Some("electrical"));
    }

    /// RSC-1.2 (2): a measurement-style (flow-only) port would infer signal,
    /// but `@PowerPort` metadata forces power classification.
    #[test]
    fn declared_power_overrides_inferred_signal() {
        let mut graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        let port_id = measurement_shaped_port(&mut graph, "DeclaredPowerPort");
        attach_metadata(&mut graph, &port_id, "PowerPort");

        let result = classify_port_definition("DeclaredPowerPort", &graph, &registry);
        assert!(
            !result.is_signal,
            "@PowerPort must force is_signal=false over the inferred signal shape"
        );
        assert_eq!(result.confidence, ClassificationConfidence::Declared);
        assert_eq!(result.carrier_domain, None);
    }

    /// RSC-1.2: qualified metadata references match on the last `::` segment
    /// (`@SimExtensions::SignalPort` == `@SignalPort`).
    #[test]
    fn declared_signal_qualified_name_matches() {
        let mut graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        let port_id = power_shaped_port(&mut graph, "QualifiedSignalPort");
        attach_metadata(&mut graph, &port_id, "SimExtensions::SignalPort");

        let result = classify_port_definition("QualifiedSignalPort", &graph, &registry);
        assert!(result.is_signal);
        assert_eq!(result.confidence, ClassificationConfidence::Declared);
    }

    /// RSC-1.2: contradictory `@Signal` + `@PowerPort` declarations are
    /// rejected loudly — inference is kept and a warning is emitted.
    #[test]
    fn conflicting_declared_roles_keep_inference_and_warn() {
        let mut graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        let port_id = power_shaped_port(&mut graph, "ConflictedPort");
        attach_metadata(&mut graph, &port_id, "Signal");
        attach_metadata(&mut graph, &port_id, "PowerPort");

        let result = classify_port_definition("ConflictedPort", &graph, &registry);
        assert!(
            !result.is_signal,
            "conflicting declarations fall back to the inferred power pair"
        );
        assert_ne!(
            result.confidence,
            ClassificationConfidence::Declared,
            "conflicting declarations must not claim Declared confidence"
        );
        assert!(
            result
                .diagnostics
                .iter()
                .any(|d| format!("{}", d).contains("both @Signal and @PowerPort")),
            "expected a conflict warning, got: {:?}",
            result.diagnostics
        );
    }

    /// RSC-1.2 (3): unrelated metadata does not trigger the override —
    /// inference is unchanged.
    #[test]
    fn unrelated_metadata_leaves_inference_unchanged() {
        let mut graph = ModelGraph::new();
        let registry = PhysicsDomainRegistry::new();

        let port_id = power_shaped_port(&mut graph, "AnnotatedPowerPort");
        attach_metadata(&mut graph, &port_id, "Safety");

        let result = classify_port_definition("AnnotatedPowerPort", &graph, &registry);
        assert!(!result.is_signal, "unrelated metadata must not flip roles");
        assert_eq!(result.confidence, ClassificationConfidence::NameHeuristic);
    }

    #[test]
    fn classify_port_definition_isq_typed() {
        let mut graph = ModelGraph::new();
        let mut registry = PhysicsDomainRegistry::new();

        // Register an ISQ type.
        let current_dim = DimensionVector {
            current: 1,
            ..Default::default()
        };
        registry.register_type("ElectricCurrentValue".into(), current_dim.clone());

        // Create port definition.
        let port_id = ElementId::new_v4();
        graph.add_element(
            Element::new(port_id.clone(), ElementKind::PortDefinition).with_name("SensorPort"),
        );

        // Add an AttributeUsage child typed with the ISQ type.
        let attr_id = ElementId::new_v4();
        graph.add_element(
            Element::new(attr_id, ElementKind::AttributeUsage)
                .with_owner(port_id.clone())
                .with_name("measuredCurrent")
                .with_prop("typeName", Value::String("ElectricCurrentValue".into())),
        );

        let result = classify_port_definition("SensorPort", &graph, &registry);

        assert_eq!(result.confidence, ClassificationConfidence::ISQTyped);
        assert_eq!(result.domain, Some("electrical"));
        assert_eq!(result.features.len(), 1);
        assert_eq!(result.features[0].role, VariableRole::Flow);
        assert_eq!(result.features[0].dimension, Some(current_dim));
        // No heuristic diagnostic expected.
        assert!(
            !result
                .diagnostics
                .iter()
                .any(|d| format!("{}", d).contains("heuristic")),
            "should not have heuristic diagnostic for ISQ-typed classification"
        );
    }
}
