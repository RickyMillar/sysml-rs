#![allow(clippy::indexing_slicing)]
use std::collections::{HashMap, HashSet, VecDeque};

use sysml_core::element_ordering::{primary_span, sort_elements_by_source_order};
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, RelationshipKind};
use sysml_span::Diagnostic;
#[cfg(test)]
use sysml_span::Span;

use crate::statemachine::health::{
    configuration_assignment_diagnostic, configuration_attribute_targets,
};

/// The set of element kinds that count as action steps inside an ActionDefinition.
const ACTION_STEP_KINDS: &[ElementKind] = &[
    ElementKind::ActionUsage,
    ElementKind::PerformActionUsage,
    ElementKind::SendActionUsage,
    ElementKind::AcceptActionUsage,
    ElementKind::AssignmentActionUsage,
    ElementKind::IfActionUsage,
    ElementKind::WhileLoopActionUsage,
    ElementKind::ForLoopActionUsage,
    ElementKind::TerminateActionUsage,
    ElementKind::DecisionNode,
    ElementKind::MergeNode,
    ElementKind::ForkNode,
    ElementKind::JoinNode,
];

/// Diagnose action health issues across all action definitions in a graph.
///
/// This pass is intended for editor diagnostics and preflight checks before
/// action compilation and execution.
pub fn action_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    let mut defs: Vec<&Element> = graph
        .elements_by_kind(&ElementKind::ActionDefinition)
        .collect();
    sort_elements_by_source_order(&mut defs);

    let config_targets = configuration_attribute_targets(graph);

    let mut diagnostics = Vec::new();
    for def in defs {
        diagnostics.extend(analyze_action(graph, def, &config_targets));
    }
    diagnostics
}

fn analyze_action(
    graph: &ModelGraph,
    action_def: &Element,
    config_targets: &HashMap<String, ElementId>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let action_name = action_def
        .name
        .clone()
        .unwrap_or_else(|| "<anonymous>".to_owned());

    // Collect action step children
    let mut steps: Vec<&Element> = graph
        .children_of(&action_def.id)
        .filter(|e| ACTION_STEP_KINDS.contains(&e.kind))
        .collect();
    sort_elements_by_source_order(&mut steps);

    // AX001: No action steps at all.
    //
    // Per spec, an ActionDefinition without steps is a legal *atomic*
    // action — execution treats it as a single indivisible performance.
    // The diagnostic is therefore info-severity preflight context, never
    // an error, and is suppressed entirely for signature-style leaf
    // actions (parameters / docs / typing only).
    if steps.is_empty() {
        let children: Vec<_> = graph.children_of(&action_def.id).collect();
        // Benign children: any relationship kind (covers FeatureTyping,
        // Specialization, Subclassification, memberships, and the implied
        // edges minted by workspace elaboration — kind whitelists drift,
        // the predicate doesn't) plus parameter/doc usage kinds.
        let is_signature_only = !children.is_empty()
            && children.iter().all(|c| {
                c.kind.is_relationship()
                    || matches!(
                        c.kind,
                        ElementKind::AttributeUsage
                            | ElementKind::ReferenceUsage
                            | ElementKind::ItemUsage
                            | ElementKind::PortUsage
                            // Metadata annotations (e.g. `@ToolExecution` on an
                            // ODE dynamics action) are signature decoration, not
                            // decomposition steps — AX001 must not fire on them.
                            | ElementKind::MetadataUsage
                            | ElementKind::Comment
                            | ElementKind::Documentation
                            | ElementKind::TextualRepresentation
                    )
            });
        if !is_signature_only {
            diagnostics.push(
                Diagnostic::info(format!(
                    "action '{}' is atomic (no steps) — it will not decompose during execution",
                    action_name
                ))
                .with_code("AX001")
                .with_span(primary_span(action_def))
                .with_note(
                    "to make it composite, add action steps inside the body, e.g. `action step1;`",
                ),
            );
        }
        return diagnostics;
    }

    let step_ids: HashSet<ElementId> = steps.iter().map(|s| s.id.clone()).collect();

    // Gather succession/transition edges within this action's steps
    let mut edges_in_action = Vec::new();
    for rel in graph.relationships_by_kind(&RelationshipKind::Transition) {
        if step_ids.contains(&rel.source) && step_ids.contains(&rel.target) {
            edges_in_action.push(rel);
        }
    }

    // Also check for SuccessionAsUsage children (created by `then` keyword).
    // These are element-level control flow that the RelationshipKind::Transition
    // check above doesn't catch.
    let has_succession_children = graph
        .children_of(&action_def.id)
        .any(|c| c.kind == ElementKind::SuccessionAsUsage);

    // AX002: No explicit control flow edges
    if edges_in_action.is_empty() && !has_succession_children {
        diagnostics.push(
            Diagnostic::warning(format!(
                "action '{}' has no explicit control flow; steps will execute sequentially",
                action_name
            ))
            .with_code("AX002")
            .with_span(primary_span(action_def)),
        );
        // No edges means we can't do reachability analysis, so return early
        // after checking individual node diagnostics below.
    }

    // Build adjacency map for reachability
    let mut adjacency: HashMap<ElementId, Vec<ElementId>> = HashMap::new();
    for rel in &edges_in_action {
        adjacency
            .entry(rel.source.clone())
            .or_default()
            .push(rel.target.clone());
    }

    // AX003: DecisionNode with no outgoing edges
    for step in &steps {
        if step.kind == ElementKind::DecisionNode {
            let out_degree = adjacency.get(&step.id).map_or(0, |targets| targets.len());
            if out_degree == 0 {
                let node_name = step.name.clone().unwrap_or_else(|| "<unnamed>".to_owned());
                diagnostics.push(
                    Diagnostic::error(format!(
                        "decision node '{}' in '{}' has no outgoing branches",
                        node_name, action_name
                    ))
                    .with_code("AX003")
                    .with_span(primary_span(step))
                    .with_note("add successions, e.g. 'first checkResult then pass; first checkResult then fail;'"),
                );
            }
        }
    }

    // AX004: ForkNode without matching JoinNode
    let has_fork = steps.iter().any(|s| s.kind == ElementKind::ForkNode);
    let has_join = steps.iter().any(|s| s.kind == ElementKind::JoinNode);
    if has_fork && !has_join {
        diagnostics.push(
            Diagnostic::warning(format!(
                "action '{}' has fork node without matching join",
                action_name
            ))
            .with_code("AX004")
            .with_span(primary_span(action_def))
            .with_note("add a `join node` to reconverge parallel branches"),
        );
    }

    // AX005: Unreachable action steps (BFS from entry points)
    // Entry points are steps with no incoming edges — these are the true starts
    // of the control flow, regardless of source position.
    if !edges_in_action.is_empty() {
        let has_incoming: HashSet<&ElementId> = edges_in_action.iter().map(|r| &r.target).collect();
        // Entry points: no incoming edges AND at least one outgoing edge.
        // A completely disconnected node (no edges at all) is NOT an entry point.
        let entry_ids: Vec<&ElementId> = steps
            .iter()
            .map(|s| &s.id)
            .filter(|id| !has_incoming.contains(id) && adjacency.contains_key(*id))
            .collect();

        // BFS from ALL entry points (or fall back to steps[0] if every node
        // has an incoming edge, e.g. a cycle).
        let mut reachable: HashSet<ElementId> = HashSet::new();
        let seeds: Vec<&ElementId> = if entry_ids.is_empty() {
            vec![&steps[0].id]
        } else {
            entry_ids
        };
        for seed in &seeds {
            reachable.extend(compute_reachable(&adjacency, seed));
        }

        for step in &steps {
            if seeds.contains(&&step.id) {
                continue;
            }
            if !reachable.contains(&step.id) {
                let step_name = step.name.clone().unwrap_or_else(|| step.id.to_string());
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "action step '{}' in '{}' is unreachable",
                        step_name, action_name
                    ))
                    .with_code("AX005")
                    .with_span(primary_span(step))
                    .with_note(format!(
                        "connect to control flow, e.g. 'first previousStep then {};'",
                        step_name
                    )),
                );
            }
        }
    }

    // AX006: PerformActionUsage referencing unknown action
    // Check TypeOf relationships from PerformActionUsage elements
    for step in &steps {
        if step.kind == ElementKind::PerformActionUsage {
            let ref_name = step.name.clone().unwrap_or_else(|| "<unnamed>".to_owned());
            // Check if there's a TypeOf relationship pointing to a known action
            let has_valid_ref = graph
                .relationships_by_kind(&RelationshipKind::TypeOf)
                .any(|rel| rel.source == step.id && graph.get_element(&rel.target).is_some());
            if !has_valid_ref {
                // Check unresolved_type/typing props before reporting —
                // the type reference exists but hasn't been resolved yet
                let has_unresolved =
                    step.get_prop("unresolved_type").is_some() || step.get_prop("typing").is_some();
                if has_unresolved {
                    continue;
                }
                // Check for FeatureTyping element children (created by the parser
                // for `perform action cal : Calibrate` — the typing relationship
                // is an element, not a graph-level Relationship)
                let has_typing_child = graph
                    .children_of(&step.id)
                    .any(|c| c.kind == ElementKind::FeatureTyping);
                if has_typing_child {
                    continue;
                }
                // Fall back: check if there's an ActionDefinition with the same name
                let name_found = graph
                    .elements_by_kind(&ElementKind::ActionDefinition)
                    .any(|e| e.name.as_deref() == Some(&ref_name));
                if !name_found {
                    diagnostics.push(
                        Diagnostic::error(format!(
                            "action '{}' references unknown action '{}'",
                            action_name, ref_name
                        ))
                        .with_code("AX006")
                        .with_span(primary_span(step))
                        .with_note("ensure the action is defined or imported in scope"),
                    );
                }
            }
        }
    }

    // AX007: IfActionUsage with no condition expression
    for step in &steps {
        if step.kind == ElementKind::IfActionUsage
            && step.get_prop("condition").is_none()
            && sysml_core::expression_pretty::pretty_print_owner(step, graph).is_none()
        {
            let step_name = step.name.clone().unwrap_or_else(|| "<unnamed>".to_owned());
            diagnostics.push(
                Diagnostic::error(format!(
                    "if action '{}' in '{}' has no condition expression",
                    step_name, action_name
                ))
                .with_code("AX007")
                .with_span(primary_span(step))
                .with_note("add a boolean condition, e.g. `if (expr)`"),
            );
        }
    }

    // AX008: WhileLoopActionUsage with no condition expression
    for step in &steps {
        if step.kind == ElementKind::WhileLoopActionUsage
            && step.get_prop("condition").is_none()
            && sysml_core::expression_pretty::pretty_print_owner(step, graph).is_none()
        {
            let step_name = step.name.clone().unwrap_or_else(|| "<unnamed>".to_owned());
            diagnostics.push(
                Diagnostic::error(format!(
                    "while loop '{}' in '{}' has no condition expression",
                    step_name, action_name
                ))
                .with_code("AX008")
                .with_span(primary_span(step))
                .with_note("add a boolean condition, e.g. `while (expr)`"),
            );
        }
    }

    // AX009: ForLoopActionUsage with no collection reference
    for step in &steps {
        if step.kind == ElementKind::ForLoopActionUsage
            && step.get_prop("collection").is_none()
            && step.get_prop("unresolved_collection").is_none()
            && step.get_prop("unresolved_reference").is_none()
        {
            let step_name = step.name.clone().unwrap_or_else(|| "<unnamed>".to_owned());
            diagnostics.push(
                Diagnostic::error(format!(
                    "for loop '{}' in '{}' has no collection reference",
                    step_name, action_name
                ))
                .with_code("AX009")
                .with_span(primary_span(step))
                .with_note("add a collection, e.g. `for x in items`"),
            );
        }
    }

    // AX010: SendActionUsage with no target endpoint
    for step in &steps {
        if step.kind == ElementKind::SendActionUsage
            && step.get_prop("target").is_none()
            && step.get_prop("unresolved_target").is_none()
            && step.get_prop("unresolved_reference").is_none()
        {
            let step_name = step.name.clone().unwrap_or_else(|| "<unnamed>".to_owned());
            diagnostics.push(
                Diagnostic::warning(format!(
                    "send action '{}' in '{}' has no target endpoint",
                    step_name, action_name
                ))
                .with_code("AX010")
                .with_span(primary_span(step))
                .with_note("add `to <port>` to specify the message target"),
            );
        }
    }

    // AX011: AcceptActionUsage with no receiver
    for step in &steps {
        if step.kind == ElementKind::AcceptActionUsage
            && step.get_prop("receiver").is_none()
            && step.get_prop("unresolved_receiver").is_none()
            && step.get_prop("unresolved_reference").is_none()
        {
            let step_name = step.name.clone().unwrap_or_else(|| "<unnamed>".to_owned());
            diagnostics.push(
                Diagnostic::warning(format!(
                    "accept action '{}' in '{}' has no receiver",
                    step_name, action_name
                ))
                .with_code("AX011")
                .with_span(primary_span(step))
                .with_note("add `via <port>` to specify where to receive messages"),
            );
        }
    }

    // AX012: PerformActionUsage with no behavior
    for step in &steps {
        if step.kind == ElementKind::PerformActionUsage {
            let has_type_of = graph
                .relationships_by_kind(&RelationshipKind::TypeOf)
                .any(|rel| rel.source == step.id);
            let step_name = step.name.clone().unwrap_or_else(|| "<unnamed>".to_owned());
            let name_matches_def = graph
                .elements_by_kind(&ElementKind::ActionDefinition)
                .any(|e| e.name.as_deref() == Some(step_name.as_str()));
            // Also check for FeatureTyping element children (created by parser
            // for `perform action cal : Calibrate` — typing is a child element)
            let has_typing_child = graph
                .children_of(&step.id)
                .any(|c| c.kind == ElementKind::FeatureTyping);
            if !has_type_of
                && !name_matches_def
                && !has_typing_child
                && step.get_prop("unresolved_type").is_none()
                && step.get_prop("unresolved_reference").is_none()
            {
                diagnostics.push(
                    Diagnostic::warning(format!(
                        "perform action '{}' in '{}' has no behavior",
                        step_name, action_name
                    ))
                    .with_code("AX012")
                    .with_span(primary_span(step))
                    .with_note("add a typing, e.g. 'perform myAction : SomeAction;'"),
                );
            }
        }
    }

    // AX013: AssignmentActionUsage with no targetFeature
    for step in &steps {
        if step.kind == ElementKind::AssignmentActionUsage
            && step.get_prop("targetFeature").is_none()
            && step.get_prop("unresolved_target").is_none()
        {
            let step_name = step.name.clone().unwrap_or_else(|| "<unnamed>".to_owned());
            diagnostics.push(
                Diagnostic::warning(format!(
                    "assignment action '{}' in '{}' has no target feature",
                    step_name, action_name
                ))
                .with_code("AX013")
                .with_span(primary_span(step))
                .with_note("assignment actions require a target: `assign <target> = <value>;`"),
            );
        }
    }

    // VR001: assignment action writes a configuration-like attribute (a
    // part-owned attribute with a literal default). Tier-1 variability lint
    // (RSC-1.4 / gap G10) — the target map and diagnostic shape are shared
    // with the state-machine pass in `statemachine/health.rs`.
    for step in &steps {
        if step.kind != ElementKind::AssignmentActionUsage {
            continue;
        }
        let Some(target) = step
            .get_prop("targetFeature")
            .and_then(|v| v.as_str())
            .or_else(|| step.get_prop("target").and_then(|v| v.as_str()))
            .or(step.name.as_deref())
        else {
            continue;
        };
        let context = format!("assignment in action '{}'", action_name);
        if let Some(diag) = configuration_assignment_diagnostic(
            graph,
            config_targets,
            target,
            primary_span(step),
            &context,
        ) {
            diagnostics.push(diag);
        }
    }

    diagnostics
}

fn compute_reachable(
    adjacency: &HashMap<ElementId, Vec<ElementId>>,
    start: &ElementId,
) -> HashSet<ElementId> {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    visited.insert(start.clone());
    queue.push_back(start.clone());

    while let Some(current) = queue.pop_front() {
        if let Some(next_nodes) = adjacency.get(&current) {
            for next in next_nodes {
                if visited.insert(next.clone()) {
                    queue.push_back(next.clone());
                }
            }
        }
    }

    visited
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, Relationship};

    #[test]
    fn reports_action_without_steps() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("EmptyAction")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        graph.add_element(action);

        let diagnostics = action_health_diagnostics(&graph);
        let ax001 = diagnostics
            .iter()
            .find(|d| d.code.as_deref() == Some("AX001") && d.message.contains("EmptyAction"))
            .expect("AX001 fires for a fully empty action def");
        // Per spec an ActionDefinition without steps is a legal atomic
        // action — AX001 is informational, never an error.
        assert_eq!(
            ax001.severity,
            sysml_span::Severity::Info,
            "AX001 must be info severity (atomic actions are legal)"
        );
    }

    /// Leaf action defs (params + docs only) are signature-style atomic
    /// actions — no AX001 at all, even when workspace elaboration mints
    /// implied relationship children (e.g. Subclassification from the
    /// implicit-generalization pass). Regression for the coffee-machine
    /// fixture where the old kind-whitelist missed Subclassification.
    #[test]
    fn leaf_action_with_params_and_implied_subclassification_is_silent() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("GrindBeans")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let doc = Element::new_with_kind(ElementKind::Documentation).with_owner(action_id.clone());
        graph.add_element(doc);

        let beans = Element::new_with_kind(ElementKind::ItemUsage)
            .with_name("beans")
            .with_owner(action_id.clone());
        graph.add_element(beans);

        // Implied base-type edge minted by workspace elaboration (IG-1).
        let subclass = Element::new_with_kind(ElementKind::Subclassification).with_owner(action_id);
        graph.add_element(subclass);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("AX001")),
            "signature-only leaf action must not emit AX001, got: {diagnostics:?}"
        );
    }

    #[test]
    fn reports_no_control_flow() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("NoFlow")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let step1 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Step1")
            .with_owner(action_id.clone())
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(step1);

        let step2 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Step2")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 4, 5));
        graph.add_element(step2);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX002")));
    }

    #[test]
    fn reports_decision_node_no_outgoing() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("WithDecision")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let step1 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Start")
            .with_owner(action_id.clone())
            .with_span(Span::new("file:///test.sysml", 2, 3));
        let step1_id = graph.add_element(step1);

        let decision = Element::new_with_kind(ElementKind::DecisionNode)
            .with_name("Check")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 4, 5));
        let decision_id = graph.add_element(decision);

        // Add succession from step1 to decision, but no outgoing from decision
        let edge = Relationship::new(RelationshipKind::Transition, step1_id, decision_id);
        graph.add_relationship(edge);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX003") && d.message.contains("Check")));
    }

    #[test]
    fn reports_fork_without_join() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("ForkOnly")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let step1 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Start")
            .with_owner(action_id.clone())
            .with_span(Span::new("file:///test.sysml", 2, 3));
        let step1_id = graph.add_element(step1);

        let fork = Element::new_with_kind(ElementKind::ForkNode)
            .with_name("Split")
            .with_owner(action_id.clone())
            .with_span(Span::new("file:///test.sysml", 4, 5));
        let fork_id = graph.add_element(fork);

        let branch1 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("BranchA")
            .with_owner(action_id.clone())
            .with_span(Span::new("file:///test.sysml", 6, 7));
        let branch1_id = graph.add_element(branch1);

        let branch2 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("BranchB")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 8, 9));
        let branch2_id = graph.add_element(branch2);

        graph.add_relationship(Relationship::new(
            RelationshipKind::Transition,
            step1_id,
            fork_id.clone(),
        ));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Transition,
            fork_id.clone(),
            branch1_id,
        ));
        graph.add_relationship(Relationship::new(
            RelationshipKind::Transition,
            fork_id,
            branch2_id,
        ));

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX004")));
    }

    #[test]
    fn reports_unreachable_step() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("HasIsland")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let step1 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Start")
            .with_owner(action_id.clone())
            .with_span(Span::new("file:///test.sysml", 2, 3));
        let step1_id = graph.add_element(step1);

        let step2 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("End")
            .with_owner(action_id.clone())
            .with_span(Span::new("file:///test.sysml", 4, 5));
        let step2_id = graph.add_element(step2);

        let island = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Island")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 6, 7));
        graph.add_element(island);

        // Only connect step1 -> step2, leaving Island unreachable
        graph.add_relationship(Relationship::new(
            RelationshipKind::Transition,
            step1_id,
            step2_id,
        ));

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX005") && d.message.contains("Island")));
    }

    #[test]
    fn reports_perform_unknown_action() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("Caller")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let perform = Element::new_with_kind(ElementKind::PerformActionUsage)
            .with_name("DoMissing")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(perform);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX006") && d.message.contains("DoMissing")));
    }

    #[test]
    fn reports_if_no_condition() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("TestAction")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let if_step = Element::new_with_kind(ElementKind::IfActionUsage)
            .with_name("noCondition")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(if_step);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX007")));
    }

    #[test]
    fn reports_while_no_condition() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("TestAction")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let while_step = Element::new_with_kind(ElementKind::WhileLoopActionUsage)
            .with_name("noCondLoop")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(while_step);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX008")));
    }

    #[test]
    fn reports_for_no_collection() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("TestAction")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let for_step = Element::new_with_kind(ElementKind::ForLoopActionUsage)
            .with_name("noCollection")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(for_step);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX009")));
    }

    #[test]
    fn reports_send_no_target() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("TestAction")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let send_step = Element::new_with_kind(ElementKind::SendActionUsage)
            .with_name("noTarget")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(send_step);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX010")));
    }

    #[test]
    fn reports_accept_no_receiver() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("TestAction")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let accept_step = Element::new_with_kind(ElementKind::AcceptActionUsage)
            .with_name("noReceiver")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(accept_step);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX011")));
    }

    #[test]
    fn reports_perform_no_behavior() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("TestAction")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let perform_step = Element::new_with_kind(ElementKind::PerformActionUsage)
            .with_name("orphanPerform")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(perform_step);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX012")));
    }

    #[test]
    fn reports_assignment_no_target() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("TestAction")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let assign_step = Element::new_with_kind(ElementKind::AssignmentActionUsage)
            .with_name("noTarget")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(assign_step);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(diagnostics
            .iter()
            .any(|d| d.code.as_deref() == Some("AX013")));
    }

    #[test]
    fn assignment_with_target_no_ax013() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("TestAction")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let assign_step = Element::new_with_kind(ElementKind::AssignmentActionUsage)
            .with_name("hasTarget")
            .with_owner(action_id)
            .with_prop("targetFeature", "x")
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(assign_step);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("AX013")),
            "assignment with targetFeature should not trigger AX013"
        );
    }

    #[test]
    fn healthy_action_reports_no_errors() {
        let mut graph = ModelGraph::new();
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("Healthy")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);

        let step1 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Step1")
            .with_owner(action_id.clone())
            .with_span(Span::new("file:///test.sysml", 2, 3));
        let step1_id = graph.add_element(step1);

        let step2 = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("Step2")
            .with_owner(action_id)
            .with_span(Span::new("file:///test.sysml", 4, 5));
        let step2_id = graph.add_element(step2);

        graph.add_relationship(Relationship::new(
            RelationshipKind::Transition,
            step1_id,
            step2_id,
        ));

        let diagnostics = action_health_diagnostics(&graph);
        assert!(
            diagnostics.is_empty(),
            "expected no diagnostics, got: {:?}",
            diagnostics
        );
    }

    // === VR001 — variability lint (shared with statemachine/health.rs) ===

    #[test]
    fn vr001_fires_on_assignment_to_part_config_attribute() {
        let mut graph = ModelGraph::new();
        let part = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Breaker")
            .with_span(Span::new("file:///test.sysml", 100, 110));
        let part_id = graph.add_element(part);
        let attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("ratedVoltage")
            .with_owner(part_id)
            .with_prop("isDefault", true)
            .with_prop("value", sysml_core::Value::Float(230.0))
            .with_span(Span::new("file:///test.sysml", 111, 120));
        graph.add_element(attr);

        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("Configure")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);
        let assign = Element::new_with_kind(ElementKind::AssignmentActionUsage)
            .with_owner(action_id)
            .with_prop("targetFeature", "ratedVoltage")
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(assign);

        let diagnostics = action_health_diagnostics(&graph);
        let hits: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.code.as_deref() == Some("VR001"))
            .collect();
        assert_eq!(hits.len(), 1, "expected one VR001, got: {:?}", diagnostics);
        assert!(hits[0].message.contains("'ratedVoltage'"));
        assert_eq!(hits[0].severity, sysml_span::Severity::Warning);
    }

    /// Assignments to action-local variables (counters, flags) are normal —
    /// the local declaration suppresses the name even when a same-named
    /// configuration attribute exists elsewhere.
    #[test]
    fn vr001_silent_for_action_local_variable() {
        let mut graph = ModelGraph::new();
        let part = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Pump")
            .with_span(Span::new("file:///test.sysml", 100, 110));
        let part_id = graph.add_element(part);
        let attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("retries")
            .with_owner(part_id)
            .with_prop("isDefault", true)
            .with_prop("value", sysml_core::Value::Int(3))
            .with_span(Span::new("file:///test.sysml", 111, 120));
        graph.add_element(attr);

        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("Retry")
            .with_span(Span::new("file:///test.sysml", 0, 1));
        let action_id = graph.add_element(action);
        // Action-local variable shadows the part attribute name.
        let local = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("retries")
            .with_owner(action_id.clone());
        graph.add_element(local);
        let assign = Element::new_with_kind(ElementKind::AssignmentActionUsage)
            .with_owner(action_id)
            .with_prop("targetFeature", "retries")
            .with_span(Span::new("file:///test.sysml", 2, 3));
        graph.add_element(assign);

        let diagnostics = action_health_diagnostics(&graph);
        assert!(
            !diagnostics
                .iter()
                .any(|d| d.code.as_deref() == Some("VR001")),
            "action-local variable assignment must not fire VR001, got: {:?}",
            diagnostics
        );
    }
}
