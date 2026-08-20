//! Port elaboration.
//!
//! Derives implicit port properties from `PortUsage` elements:
//! - Resolves `PortDefinition` typing (via `find_feature_type` from resolution)
//! - Tags conjugation state (`isConjugated` property)
//! - Extracts effective direction (`effectiveDirection` property)
//!
//! These properties are consumed downstream by:
//! - `sysml-runtime`'s `compile_ports()` to build `PortRegistry`
//! - Port diagnostics and health checkers (FL010-FL015)
//! - Diagram renderers (port symbols, direction glyphs)
//! - LSP hover info (port definition, features)
//!
//! ## Design
//!
//! Additive and idempotent — only sets properties that don't already exist.
//! Follows the same pattern as `flows.rs` and `connectors.rs`.

use super::ElaborationReport;
use crate::resolution::scoping::chaining::find_feature_type;
use crate::{ElementId, ElementKind, ModelGraph, Value};

/// Elaborate all `PortUsage` elements in the graph.
///
/// For each port:
/// 1. Resolve PortDefinition typing → set `portDefinition` property
/// 2. Detect conjugation → set `isConjugated` property
/// 3. Extract effective direction → set `effectiveDirection` property
pub(super) fn elaborate_ports(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let port_ids: Vec<ElementId> = graph.element_ids_by_kind(&ElementKind::PortUsage).to_vec();

    for port_id in port_ids {
        let changed = elaborate_single_port(&port_id, graph);
        if changed {
            report.ports_elaborated += 1;
        }
    }
}

/// Elaborate a single PortUsage element.
///
/// Returns true if any property was set.
fn elaborate_single_port(port_id: &ElementId, graph: &mut ModelGraph) -> bool {
    // Phase 1: Gather data (immutable borrow)
    let (needs_def, needs_conj, needs_dir, def_name, is_conj, direction) = {
        let Some(port) = graph.get_element(port_id) else {
            return false;
        };
        if port.kind != ElementKind::PortUsage {
            return false;
        }

        let needs_def = port.get_prop("portDefinition").is_none();
        let needs_conj = port.get_prop("isConjugated").is_none();
        let needs_dir = port.get_prop("effectiveDirection").is_none();

        if !needs_def && !needs_conj && !needs_dir {
            return false; // already elaborated
        }

        // Resolve PortDefinition name
        let def_name = if needs_def {
            resolve_port_definition_name(port_id, graph)
        } else {
            None
        };

        // Detect conjugation (always check — needed for direction calculation)
        let is_conj = if needs_conj {
            is_conjugated_port(port_id, graph)
        } else {
            // Already elaborated — read the existing value
            port.get_prop("isConjugated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        };

        // Extract direction
        let direction = if needs_dir {
            extract_effective_direction(port_id, is_conj, graph)
        } else {
            None
        };

        (
            needs_def, needs_conj, needs_dir, def_name, is_conj, direction,
        )
    };

    // Phase 2: Apply changes (mutable borrow)
    let mut changed = false;

    if needs_def {
        if let Some(name) = def_name {
            if let Some(elem) = graph.get_element_mut(port_id) {
                elem.set_prop("portDefinition", Value::String(name));
                changed = true;
            }
        }
    }

    if needs_conj && is_conj {
        if let Some(elem) = graph.get_element_mut(port_id) {
            elem.set_prop("isConjugated", Value::Bool(true));
            changed = true;
        }
    }

    if needs_dir {
        if let Some(dir) = direction {
            if let Some(elem) = graph.get_element_mut(port_id) {
                elem.set_prop("effectiveDirection", Value::String(dir));
                changed = true;
            }
        }
    }

    changed
}

/// Resolve the PortDefinition name for a PortUsage.
///
/// Uses `find_feature_type()` from the resolution module (O(1) reverse index).
fn resolve_port_definition_name(port_id: &ElementId, graph: &ModelGraph) -> Option<String> {
    let def_id = find_feature_type(graph, port_id)?;
    let def_elem = graph.get_element(&def_id)?;
    def_elem.name.clone()
}

/// Check if a port has conjugated typing (~PortDef).
///
/// Checks:
/// 1. `isConjugated` marker property already set
/// 2. Outgoing relationships pointing to ConjugatedPortTyping/ConjugatedPortDefinition
fn is_conjugated_port(port_id: &ElementId, graph: &ModelGraph) -> bool {
    // Check marker property
    if let Some(port) = graph.get_element(port_id) {
        if port.get_prop("isConjugated").and_then(|v| v.as_bool()) == Some(true) {
            return true;
        }
    }

    // Check outgoing relationships
    for rel in graph.outgoing(port_id) {
        if let Some(target) = graph.get_element(&rel.target) {
            if target.kind == ElementKind::ConjugatedPortTyping
                || target.kind == ElementKind::ConjugatedPortDefinition
            {
                return true;
            }
        }
    }

    // Check owned children: the parser encodes usage-level `: ~P` as an
    // owned ConjugatedPortTyping child (carrying `isConjugated=true` and
    // the `unresolved_type`), not as a relationship edge — see
    // create_conjugated_port_typing_with_key in sysml-parser-trait.
    for child in graph.children_of(port_id) {
        if child.kind == ElementKind::ConjugatedPortTyping
            || child.kind == ElementKind::ConjugatedPortDefinition
        {
            return true;
        }
    }
    false
}

/// Extract the effective direction for a port, accounting for conjugation.
///
/// Resolution order:
/// 1. PortUsage's own `direction` property (set by parser for `in port x`)
/// 2. PortDefinition's children directions (e.g., `port def P { in item x }` → "in")
/// 3. PortDefinition name heuristic (e.g., `PhaseInPort` → "in")
/// 4. Falls back to "undirected"
///
/// If conjugated, reverses the direction (in↔out).
fn extract_effective_direction(
    port_id: &ElementId,
    is_conjugated: bool,
    graph: &ModelGraph,
) -> Option<String> {
    let port = graph.get_element(port_id)?;

    // Step 1: Check PortUsage's own direction property
    let own_dir = port.get_prop("direction").and_then(|v| v.as_str());

    let base_dir = if let Some(dir) = own_dir {
        if dir != "undirected" {
            dir.to_owned()
        } else {
            infer_direction_from_definition(port_id, graph)
                .unwrap_or_else(|| "undirected".to_owned())
        }
    } else {
        // No direction property at all — try definition inference
        infer_direction_from_definition(port_id, graph).unwrap_or_else(|| "undirected".to_owned())
    };

    let effective = if is_conjugated {
        match base_dir.as_str() {
            "in" => "out".to_owned(),
            "out" => "in".to_owned(),
            other => other.to_owned(),
        }
    } else {
        base_dir
    };

    Some(effective)
}

/// Resolve a PortUsage to its PortDefinition element, with fallbacks.
///
/// Tries in order:
/// 1. `find_feature_type()` reverse index (fast, works after resolution)
/// 2. FeatureTyping children with `unresolved_type` (works before resolution)
fn find_port_definition_element(port_id: &ElementId, graph: &ModelGraph) -> Option<ElementId> {
    // Fast path: resolved type via reverse index
    if let Some(def_id) = find_feature_type(graph, port_id) {
        let def = graph.get_element(&def_id)?;
        if def.kind == ElementKind::PortDefinition {
            return Some(def_id);
        }
    }

    // Fallback: walk FeatureTyping children for unresolved_type
    for child in graph.children_of(port_id) {
        if child.kind == ElementKind::FeatureTyping
            || child.kind.is_subtype_of(ElementKind::FeatureTyping)
        {
            if let Some(type_name) = child.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                if let Some(def) = graph.elements.values().find(|e| {
                    e.kind == ElementKind::PortDefinition && e.name.as_deref() == Some(type_name)
                }) {
                    return Some(def.id.clone());
                }
            }
        }
    }

    None
}

/// Infer port direction from the PortDefinition's children.
///
/// Examines the direction properties on the definition's children
/// (ItemUsage, AttributeUsage, etc.):
/// - All "in" → port is "in"
/// - All "out" → port is "out"
/// - Mixed → "inout"
/// - No directed children → try name-based heuristic
fn infer_direction_from_definition(port_id: &ElementId, graph: &ModelGraph) -> Option<String> {
    let def_id = find_port_definition_element(port_id, graph)?;

    // Scan children for direction properties
    let mut has_in = false;
    let mut has_out = false;

    for child in graph.children_of(&def_id) {
        if let Some(dir) = child.get_prop("direction").and_then(|v| v.as_str()) {
            match dir {
                "in" => has_in = true,
                "out" => has_out = true,
                "inout" => {
                    has_in = true;
                    has_out = true;
                }
                _ => {}
            }
        }
    }

    match (has_in, has_out) {
        (true, false) => return Some("in".to_owned()),
        (false, true) => return Some("out".to_owned()),
        (true, true) => return Some("inout".to_owned()),
        (false, false) => {} // Fall through to name heuristic
    }

    // Name-based heuristic on the definition name
    let def = graph.get_element(&def_id)?;
    let def_name = def.name.as_deref()?;
    infer_direction_from_def_name(def_name)
}

/// Infer port direction from the PortDefinition's name.
///
/// Naming conventions: `PhaseInPort` → "in", `PhaseOutPort` → "out".
fn infer_direction_from_def_name(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    // Check suffix patterns first (most specific)
    if lower.ends_with("inport") || lower.ends_with("_in") {
        Some("in".to_owned())
    } else if lower.ends_with("outport") || lower.ends_with("_out") {
        Some("out".to_owned())
    // Check contains patterns (less specific)
    } else if lower.contains("in") && !lower.contains("out") {
        // Only match if "in" appears as a word boundary-ish pattern
        // e.g., "PhaseIn" but not "Point" or "Internal"
        if lower.ends_with("in") || lower.contains("input") || lower.contains("_in") {
            Some("in".to_owned())
        } else {
            None
        }
    } else if lower.contains("out") && !lower.contains("in") {
        if lower.ends_with("out") || lower.contains("output") || lower.contains("_out") {
            Some("out".to_owned())
        } else {
            None
        }
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate;
    use crate::Element;

    #[test]
    fn elaborate_port_sets_effective_direction() {
        let mut graph = ModelGraph::new();

        let port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("waterOut")
            .with_prop("direction", "out");
        let port_id = graph.add_element(port);

        let report = elaborate(&mut graph);
        assert!(report.ports_elaborated >= 1);

        let elem = graph.get_element(&port_id).unwrap();
        assert_eq!(
            elem.get_prop("effectiveDirection").and_then(|v| v.as_str()),
            Some("out")
        );
    }

    #[test]
    fn elaborate_conjugated_port_reverses_direction() {
        let mut graph = ModelGraph::new();

        let port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("waterIn")
            .with_prop("direction", "in")
            .with_prop("isConjugated", true);
        let port_id = graph.add_element(port);

        elaborate(&mut graph);

        let elem = graph.get_element(&port_id).unwrap();
        // Conjugated: in → out
        assert_eq!(
            elem.get_prop("effectiveDirection").and_then(|v| v.as_str()),
            Some("out")
        );
    }

    #[test]
    fn elaborate_port_undirected_by_default() {
        let mut graph = ModelGraph::new();

        let port = Element::new_with_kind(ElementKind::PortUsage).with_name("dataPort");
        let port_id = graph.add_element(port);

        elaborate(&mut graph);

        let elem = graph.get_element(&port_id).unwrap();
        assert_eq!(
            elem.get_prop("effectiveDirection").and_then(|v| v.as_str()),
            Some("undirected")
        );
    }

    #[test]
    fn elaborate_port_idempotent() {
        let mut graph = ModelGraph::new();

        let port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("port1")
            .with_prop("direction", "out");
        let port_id = graph.add_element(port);

        let report1 = elaborate(&mut graph);
        let report2 = elaborate(&mut graph);

        // Second pass should not re-elaborate (properties already set)
        assert!(report1.ports_elaborated >= 1);
        assert_eq!(report2.ports_elaborated, 0);

        let elem = graph.get_element(&port_id).unwrap();
        assert_eq!(
            elem.get_prop("effectiveDirection").and_then(|v| v.as_str()),
            Some("out")
        );
    }

    #[test]
    fn does_not_elaborate_non_port_elements() {
        let mut graph = ModelGraph::new();

        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("notAPort")
            .with_prop("direction", "in");
        graph.add_element(part);

        let report = elaborate(&mut graph);
        assert_eq!(report.ports_elaborated, 0);
    }

    /// Port direction inferred from PortDefinition children.
    ///
    /// `port def PhaseInPort { in item power : ACPower; }` → port typed as
    /// PhaseInPort should get effectiveDirection = "in".
    #[test]
    fn elaborate_port_direction_from_definition_children() {
        let mut graph = ModelGraph::new();

        // Create PortDefinition with an "in" child
        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("PhaseInPort");
        let def_id = graph.add_element(port_def);

        let item = Element::new_with_kind(ElementKind::ItemUsage)
            .with_name("power")
            .with_prop("direction", "in");
        graph.add_element(item.with_owner(def_id.clone()));

        // Create PortUsage typed as PhaseInPort (via FeatureTyping child)
        let port = Element::new_with_kind(ElementKind::PortUsage).with_name("phaseIn");
        let port_id = graph.add_element(port);

        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_prop("unresolved_type", "PhaseInPort");
        graph.add_element(typing.with_owner(port_id.clone()));

        elaborate(&mut graph);

        let elem = graph.get_element(&port_id).unwrap();
        assert_eq!(
            elem.get_prop("effectiveDirection").and_then(|v| v.as_str()),
            Some("in"),
            "PortUsage typed as PhaseInPort (with 'in item') should be direction 'in'"
        );
    }

    /// Port direction inferred from PortDefinition with "out" children.
    #[test]
    fn elaborate_port_direction_from_definition_out_children() {
        let mut graph = ModelGraph::new();

        let port_def =
            Element::new_with_kind(ElementKind::PortDefinition).with_name("PowerOutPort");
        let def_id = graph.add_element(port_def);

        let item = Element::new_with_kind(ElementKind::ItemUsage)
            .with_name("power")
            .with_prop("direction", "out");
        graph.add_element(item.with_owner(def_id.clone()));

        let port = Element::new_with_kind(ElementKind::PortUsage).with_name("powerOut");
        let port_id = graph.add_element(port);

        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_prop("unresolved_type", "PowerOutPort");
        graph.add_element(typing.with_owner(port_id.clone()));

        elaborate(&mut graph);

        let elem = graph.get_element(&port_id).unwrap();
        assert_eq!(
            elem.get_prop("effectiveDirection").and_then(|v| v.as_str()),
            Some("out")
        );
    }

    /// Port definition with mixed direction children → inout.
    #[test]
    fn elaborate_port_direction_mixed_children_is_inout() {
        let mut graph = ModelGraph::new();

        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("ControlPort");
        let def_id = graph.add_element(port_def);

        let cmd = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("command")
            .with_prop("direction", "in");
        graph.add_element(cmd.with_owner(def_id.clone()));

        let status = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("status")
            .with_prop("direction", "out");
        graph.add_element(status.with_owner(def_id.clone()));

        let port = Element::new_with_kind(ElementKind::PortUsage).with_name("control");
        let port_id = graph.add_element(port);

        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_prop("unresolved_type", "ControlPort");
        graph.add_element(typing.with_owner(port_id.clone()));

        elaborate(&mut graph);

        let elem = graph.get_element(&port_id).unwrap();
        assert_eq!(
            elem.get_prop("effectiveDirection").and_then(|v| v.as_str()),
            Some("inout")
        );
    }

    /// Conjugation reverses the direction inferred from definition.
    #[test]
    fn elaborate_conjugated_port_reverses_definition_direction() {
        let mut graph = ModelGraph::new();

        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("PhaseInPort");
        let def_id = graph.add_element(port_def);

        let item = Element::new_with_kind(ElementKind::ItemUsage)
            .with_name("power")
            .with_prop("direction", "in");
        graph.add_element(item.with_owner(def_id.clone()));

        let port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("phaseOut")
            .with_prop("isConjugated", true);
        let port_id = graph.add_element(port);

        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_prop("unresolved_type", "PhaseInPort");
        graph.add_element(typing.with_owner(port_id.clone()));

        elaborate(&mut graph);

        let elem = graph.get_element(&port_id).unwrap();
        // Conjugated: in → out
        assert_eq!(
            elem.get_prop("effectiveDirection").and_then(|v| v.as_str()),
            Some("out"),
            "Conjugated PhaseInPort should have effectiveDirection 'out'"
        );
    }

    /// Name-based direction heuristic when definition has no directed children.
    #[test]
    fn elaborate_port_direction_from_def_name_heuristic() {
        let mut graph = ModelGraph::new();

        // PortDefinition with no directed children but suggestive name
        let port_def = Element::new_with_kind(ElementKind::PortDefinition).with_name("PhaseInPort");
        let def_id = graph.add_element(port_def);

        // Undirected child (no direction property)
        let item = Element::new_with_kind(ElementKind::ItemUsage).with_name("power");
        graph.add_element(item.with_owner(def_id.clone()));

        let port = Element::new_with_kind(ElementKind::PortUsage).with_name("phaseIn");
        let port_id = graph.add_element(port);

        let typing = Element::new_with_kind(ElementKind::FeatureTyping)
            .with_prop("unresolved_type", "PhaseInPort");
        graph.add_element(typing.with_owner(port_id.clone()));

        elaborate(&mut graph);

        let elem = graph.get_element(&port_id).unwrap();
        assert_eq!(
            elem.get_prop("effectiveDirection").and_then(|v| v.as_str()),
            Some("in"),
            "PhaseInPort name heuristic should infer 'in'"
        );
    }
}
