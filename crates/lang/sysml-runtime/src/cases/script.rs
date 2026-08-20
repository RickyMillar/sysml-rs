//! Scripted-event extraction from verification-case action chains.

use sysml_core::{ElementKind, ModelGraph, RelationshipKind};
use sysml_span::Diagnostic;

#[allow(unused_imports)]
use super::*;

// ---------------------------------------------------------------------------
// Scripted event extraction (Phase 9.2)
// ---------------------------------------------------------------------------

/// A scripted event extracted from a verification case's test actions.
#[derive(Debug, Clone)]
pub struct ScriptedEvent {
    /// Step index (ordering from succession chain).
    pub step: usize,
    /// Event name to inject.
    pub event: String,
    /// Target subsystem hint (from action target or send expression).
    pub target_hint: Option<String>,
    /// Delay in milliseconds from simulation start.
    pub delay_ms: f64,
}

/// Extract a timed event script from a verification case's action children.
///
/// Walks the graph to find action usages within the verification case element,
/// reads succession chains (HappensBefore relationships) for ordering,
/// and produces ScriptedEvents suitable for feeding into the orchestrator.
///
/// Each action child with an ElementKind of ActionUsage becomes a ScriptedEvent
/// where the event name = the action's name. Timing is computed as
/// step_index * dt_ms. If the action has no successor, it's the last event.
pub fn extract_event_script(
    case_name: &str,
    graph: &ModelGraph,
    dt_ms: f64,
) -> Result<Vec<ScriptedEvent>, Vec<Diagnostic>> {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        case = case_name,
        element_count = graph.element_count(),
        "extracting event script from verification case"
    );

    // Find the verification case element by name
    let case_elem = graph
        .elements
        .values()
        .find(|e| {
            (e.kind == ElementKind::VerificationCaseDefinition
                || e.kind == ElementKind::VerificationCaseUsage)
                && e.name.as_deref() == Some(case_name)
        })
        .ok_or_else(|| {
            vec![Diagnostic::error(format!(
                "verification case '{}' not found in model",
                case_name
            ))]
        })?;

    let case_id = case_elem.id.clone();

    // Look for a testScript action child first; fall back to direct children
    let script_parent_id = graph
        .children_of(&case_id)
        .find(|c| {
            (c.kind == ElementKind::ActionUsage || c.kind == ElementKind::ActionDefinition)
                && c.name.as_deref() == Some("testScript")
        })
        .map(|c| c.id.clone())
        .unwrap_or_else(|| case_id.clone());

    // Collect all ActionUsage children of the script parent (excluding testScript itself)
    let action_children: Vec<_> = graph
        .children_of(&script_parent_id)
        .filter(|c| c.kind == ElementKind::ActionUsage)
        .collect();

    if action_children.is_empty() {
        return Ok(Vec::new());
    }

    // Build a name -> ElementId map for action children
    let name_to_id: std::collections::HashMap<String, sysml_core::ElementId> = action_children
        .iter()
        .filter_map(|e| e.name.as_ref().map(|n| (n.clone(), e.id.clone())))
        .collect();

    let id_to_name: std::collections::HashMap<sysml_core::ElementId, String> = action_children
        .iter()
        .filter_map(|e| e.name.as_ref().map(|n| (e.id.clone(), n.clone())))
        .collect();

    // Build succession ordering from SuccessionAsUsage siblings
    // These have "source" and "target" props naming the action steps
    let successions: Vec<(String, String)> = graph
        .children_of(&script_parent_id)
        .filter(|c| c.kind == ElementKind::SuccessionAsUsage)
        .filter_map(|succ| {
            let source = succ
                .get_prop("source")
                .and_then(|v| v.as_str())
                .or_else(|| succ.get_prop("unresolved_source").and_then(|v| v.as_str()))
                .map(String::from);
            let target = succ
                .get_prop("target")
                .and_then(|v| v.as_str())
                .or_else(|| succ.get_prop("unresolved_target").and_then(|v| v.as_str()))
                .map(String::from);
            match (source, target) {
                (Some(s), Some(t)) => Some((s, t)),
                _ => None,
            }
        })
        .collect();

    // Also check for Transition relationships created by elaboration
    let elaborated_successions: Vec<(String, String)> = graph
        .relationships
        .values()
        .filter(|r| r.kind == RelationshipKind::Transition)
        .filter_map(|r| {
            let src_name = id_to_name.get(&r.source)?;
            let tgt_name = id_to_name.get(&r.target)?;
            Some((src_name.clone(), tgt_name.clone()))
        })
        .collect();

    // Merge both succession sources
    let all_successions = {
        let mut combined = successions;
        for es in elaborated_successions {
            if !combined.contains(&es) {
                combined.push(es);
            }
        }
        combined
    };

    // Build the ordered chain via topological sort of the succession graph
    let ordered_names = if all_successions.is_empty() {
        // Fall back to declaration order
        action_children
            .iter()
            .filter_map(|e| e.name.clone())
            .collect::<Vec<_>>()
    } else {
        topological_sort_successions(&all_successions, &name_to_id)
    };

    // Map to ScriptedEvents
    let events: Vec<ScriptedEvent> = ordered_names
        .iter()
        .enumerate()
        .map(|(i, name)| ScriptedEvent {
            step: i,
            event: name.clone(),
            target_hint: None,
            delay_ms: i as f64 * dt_ms,
        })
        .collect();

    #[cfg(feature = "tracing")]
    tracing::debug!(
        case = case_name,
        events = events.len(),
        "extracted event script"
    );

    Ok(events)
}

/// Topological sort of action names based on succession pairs.
///
/// Finds the root (an action that appears as a source but never as a target)
/// and walks the chain. Actions not in the chain are appended at the end.
fn topological_sort_successions(
    successions: &[(String, String)],
    name_to_id: &std::collections::HashMap<String, sysml_core::ElementId>,
) -> Vec<String> {
    use std::collections::{HashMap, HashSet};

    // Build adjacency: source -> target
    let mut next: HashMap<&str, &str> = HashMap::new();
    let mut targets: HashSet<&str> = HashSet::new();
    let mut sources: HashSet<&str> = HashSet::new();

    for (src, tgt) in successions {
        next.insert(src.as_str(), tgt.as_str());
        targets.insert(tgt.as_str());
        sources.insert(src.as_str());
    }

    // Find root: a source that is not a target
    let root = sources.iter().find(|s| !targets.contains(**s)).copied();

    let mut ordered = Vec::new();
    let mut visited: HashSet<&str> = HashSet::new();

    if let Some(start) = root {
        let mut current = start;
        loop {
            if visited.contains(current) {
                break; // Cycle protection
            }
            visited.insert(current);
            ordered.push(current.to_owned());
            match next.get(current) {
                Some(nxt) => current = nxt,
                None => break,
            }
        }
    }

    // Append any remaining actions not in the chain (declaration order)
    for name in name_to_id.keys() {
        if !visited.contains(name.as_str()) {
            ordered.push(name.clone());
        }
    }

    ordered
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind, ModelGraph};

    #[test]
    fn extract_event_script_from_verification_case() {
        let mut graph = ModelGraph::new();

        // Create a verification case definition
        let vc = Element::new_with_kind(ElementKind::VerificationCaseDefinition)
            .with_name("FullBrewCycleTest");
        let vc_id = graph.add_element(vc);

        // Create testScript action child
        let test_script = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("testScript")
            .with_owner(vc_id.clone());
        let ts_id = graph.add_element(test_script);

        // Create action steps as children of testScript
        let power_on = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("powerOn")
            .with_owner(ts_id.clone());
        let _po_id = graph.add_element(power_on);

        let temp_reached = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("tempReached")
            .with_owner(ts_id.clone());
        let _tr_id = graph.add_element(temp_reached);

        let start_brew = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("startBrew")
            .with_owner(ts_id.clone());
        let _sb_id = graph.add_element(start_brew);

        // Create successions: powerOn then tempReached then startBrew
        let succ1 = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(ts_id.clone())
            .with_prop("source", "powerOn")
            .with_prop("target", "tempReached");
        graph.add_element(succ1);

        let succ2 = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(ts_id.clone())
            .with_prop("source", "tempReached")
            .with_prop("target", "startBrew");
        graph.add_element(succ2);

        let events = extract_event_script("FullBrewCycleTest", &graph, 100.0).unwrap();

        assert_eq!(events.len(), 3);
        assert_eq!(events[0].event, "powerOn");
        assert_eq!(events[0].step, 0);
        assert_eq!(events[0].delay_ms, 0.0);

        assert_eq!(events[1].event, "tempReached");
        assert_eq!(events[1].step, 1);
        assert_eq!(events[1].delay_ms, 100.0);

        assert_eq!(events[2].event, "startBrew");
        assert_eq!(events[2].step, 2);
        assert_eq!(events[2].delay_ms, 200.0);
    }

    #[test]
    fn extract_event_script_fallback_to_declaration_order() {
        let mut graph = ModelGraph::new();

        let vc =
            Element::new_with_kind(ElementKind::VerificationCaseDefinition).with_name("SimpleTest");
        let vc_id = graph.add_element(vc);

        // Add actions directly to the case (no testScript wrapper, no successions)
        let a = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("stepA")
            .with_owner(vc_id.clone());
        graph.add_element(a);

        let b = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("stepB")
            .with_owner(vc_id.clone());
        graph.add_element(b);

        let events = extract_event_script("SimpleTest", &graph, 50.0).unwrap();

        assert_eq!(events.len(), 2);
        // Should fall back to declaration order
        let names: Vec<&str> = events.iter().map(|e| e.event.as_str()).collect();
        assert!(names.contains(&"stepA"));
        assert!(names.contains(&"stepB"));
    }

    #[test]
    fn extract_event_script_case_not_found() {
        let graph = ModelGraph::new();
        let result = extract_event_script("NonExistent", &graph, 100.0);
        assert!(result.is_err());
    }
}
