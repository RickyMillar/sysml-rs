//! Succession elaboration.
//!
//! Creates `Relationship::Transition` from `SuccessionAsUsage` children within
//! `ActionDefinition`/`ActionUsage` elements. Successions define ordering
//! between action steps.

use super::ElaborationReport;
use crate::{
    CanonicalKey, ElementId, ElementKind, ModelGraph, Relationship, RelationshipKind, Value,
};

/// Elaborate successions by creating Transition relationships from SuccessionAsUsage elements.
pub(super) fn elaborate_successions(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    // Collect SuccessionAsUsage elements
    let succession_ids = graph
        .element_ids_by_kind(&ElementKind::SuccessionAsUsage)
        .to_vec();
    let succession_infos: Vec<SuccessionInfo> = succession_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .map(|e| SuccessionInfo {
            id: e.id.clone(),
            owner: e.owner.clone(),
            source_name: e
                .get_prop("source")
                .and_then(|v| v.as_str())
                .or_else(|| e.get_prop("unresolved_source").and_then(|v| v.as_str()))
                .map(String::from),
            target_name: e
                .get_prop("target")
                .and_then(|v| v.as_str())
                .or_else(|| e.get_prop("unresolved_target").and_then(|v| v.as_str()))
                .map(String::from),
            source_ref: e.get_prop("source").and_then(|v| v.as_ref()).cloned(),
            target_ref: e.get_prop("target").and_then(|v| v.as_ref()).cloned(),
        })
        .collect();

    for info in succession_infos {
        let source_id = info
            .source_ref
            .filter(|id| graph.get_element(id).is_some())
            .or_else(|| {
                info.source_name
                    .as_deref()
                    .and_then(|name| find_sibling_by_name(graph, &info.owner, name))
            });

        let target_id = info
            .target_ref
            .filter(|id| graph.get_element(id).is_some())
            .or_else(|| {
                info.target_name
                    .as_deref()
                    .and_then(|name| find_sibling_by_name(graph, &info.owner, name))
            });

        if let (Some(src), Some(tgt)) = (source_id, target_id) {
            // Check if relationship already exists
            let already_exists = graph
                .relationships_by_kind(&RelationshipKind::Transition)
                .any(|r| r.source == src && r.target == tgt);

            if !already_exists {
                let src_key = CanonicalKey::root(&src.to_string());
                let tgt_key = CanonicalKey::root(&tgt.to_string());
                let edge_key = CanonicalKey::for_relationship(
                    &src_key,
                    RelationshipKind::Transition.as_str(),
                    &tgt_key,
                    0,
                );
                let mut rel =
                    Relationship::new_with_key(RelationshipKind::Transition, src, tgt, &edge_key);
                // Preserve provenance so diagnostics can point back to the
                // originating SuccessionAsUsage element span.
                rel.props
                    .insert("origin_transition".into(), Value::Ref(info.id.clone()));
                graph.add_relationship(rel);
                report.successions_created += 1;
            }
        }
    }
}

struct SuccessionInfo {
    id: ElementId,
    owner: Option<ElementId>,
    source_name: Option<String>,
    target_name: Option<String>,
    source_ref: Option<ElementId>,
    target_ref: Option<ElementId>,
}

/// Find a sibling element by name (child of the same owner).
/// Delegates to the shared `resolve_name` helper, with a fallback
/// that resolves bare control-flow keywords ("fork", "join", "merge",
/// "decide") to the matching anonymous control node sibling.
fn find_sibling_by_name(
    graph: &ModelGraph,
    owner: &Option<ElementId>,
    name: &str,
) -> Option<ElementId> {
    if let Some(id) = super::resolve_name(graph, owner, name) {
        return Some(id);
    }

    // Fallback: resolve bare control-flow keywords to anonymous control nodes.
    // Users write `first grind then join;` where "join" is the keyword, not a
    // named element. Map the keyword to the corresponding ElementKind and find
    // the first matching sibling.
    let kind = match name {
        "fork" => Some(ElementKind::ForkNode),
        "join" => Some(ElementKind::JoinNode),
        "merge" => Some(ElementKind::MergeNode),
        "decide" => Some(ElementKind::DecisionNode),
        _ => None,
    };

    if let (Some(kind), Some(owner_id)) = (kind, owner.as_ref()) {
        return graph
            .children_of(owner_id)
            .find(|e| e.kind == kind)
            .map(|e| e.id.clone());
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate;
    use crate::Element;

    #[test]
    fn creates_transition_from_succession() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("MyAction");
        let action_id = graph.add_element(action);

        let step1 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("step1")
            .with_owner(action_id.clone());
        let step1_id = graph.add_element(step1);

        let step2 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("step2")
            .with_owner(action_id.clone());
        let step2_id = graph.add_element(step2);

        let succession = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(action_id)
            .with_prop("source", "step1")
            .with_prop("target", "step2");
        graph.add_element(succession);

        let report = elaborate(&mut graph);

        assert_eq!(report.successions_created, 1);

        let transitions: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Transition)
            .collect();
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].source, step1_id);
        assert_eq!(transitions[0].target, step2_id);
    }

    #[test]
    fn idempotent_succession() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("MyAction");
        let action_id = graph.add_element(action);

        let step1 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("a")
            .with_owner(action_id.clone());
        graph.add_element(step1);

        let step2 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("b")
            .with_owner(action_id.clone());
        graph.add_element(step2);

        let succession = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(action_id)
            .with_prop("source", "a")
            .with_prop("target", "b");
        graph.add_element(succession);

        let r1 = elaborate(&mut graph);
        assert_eq!(r1.successions_created, 1);

        let r2 = elaborate(&mut graph);
        assert_eq!(r2.successions_created, 0);
    }

    #[test]
    fn keyword_fallback_resolves_fork() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("A");
        let action_id = graph.add_element(action);

        // Anonymous fork with synthetic name
        let fork = Element::new_with_kind(ElementKind::ForkNode)
            .with_name("$fork_0")
            .with_owner(action_id.clone());
        let fork_id = graph.add_element(fork);

        let step = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("grind")
            .with_owner(action_id.clone());
        let step_id = graph.add_element(step);

        // Succession: fork → grind (using synthetic name)
        let succ = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(action_id)
            .with_prop("source", "$fork_0")
            .with_prop("target", "grind");
        graph.add_element(succ);

        let report = elaborate(&mut graph);
        assert_eq!(report.successions_created, 1);

        let transitions: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Transition)
            .collect();
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].source, fork_id);
        assert_eq!(transitions[0].target, step_id);
    }

    #[test]
    fn keyword_fallback_resolves_join_by_keyword() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("A");
        let action_id = graph.add_element(action);

        let step = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("grind")
            .with_owner(action_id.clone());
        let step_id = graph.add_element(step);

        // Anonymous join (no synthetic name — user wrote `first grind then join;`)
        let join = Element::new_with_kind(ElementKind::JoinNode).with_owner(action_id.clone());
        let join_id = graph.add_element(join);

        // Succession: grind → join (using keyword "join" as target)
        let succ = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(action_id)
            .with_prop("source", "grind")
            .with_prop("target", "join");
        graph.add_element(succ);

        let report = elaborate(&mut graph);
        assert_eq!(
            report.successions_created, 1,
            "keyword 'join' should resolve to the JoinNode sibling"
        );

        let transitions: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Transition)
            .collect();
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].source, step_id);
        assert_eq!(transitions[0].target, join_id);
    }

    #[test]
    fn fork_fanout_creates_parallel_transitions() {
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("A");
        let action_id = graph.add_element(action);

        let fork = Element::new_with_kind(ElementKind::ForkNode)
            .with_name("$fork_0")
            .with_owner(action_id.clone());
        let fork_id = graph.add_element(fork);

        let grind = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("grind")
            .with_owner(action_id.clone());
        let grind_id = graph.add_element(grind);

        let heat = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("heat")
            .with_owner(action_id.clone());
        let heat_id = graph.add_element(heat);

        // Two successions from fork: parallel fan-out
        let succ1 = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(action_id.clone())
            .with_prop("source", "$fork_0")
            .with_prop("target", "grind");
        graph.add_element(succ1);

        let succ2 = Element::new_with_kind(ElementKind::SuccessionAsUsage)
            .with_owner(action_id)
            .with_prop("source", "$fork_0")
            .with_prop("target", "heat");
        graph.add_element(succ2);

        let report = elaborate(&mut graph);
        assert_eq!(report.successions_created, 2);

        let transitions: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Transition)
            .collect();
        assert_eq!(transitions.len(), 2);
        // Both transitions should source from the fork
        assert!(transitions.iter().all(|t| t.source == fork_id));
        let targets: Vec<_> = transitions.iter().map(|t| t.target.clone()).collect();
        assert!(targets.contains(&grind_id));
        assert!(targets.contains(&heat_id));
    }
}
