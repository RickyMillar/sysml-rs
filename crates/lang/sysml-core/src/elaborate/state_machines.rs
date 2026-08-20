//! State machine elaboration.
//!
//! Derives implicit state machine structure from the parsed model:
//! - Tags the first `StateUsage` child of a `StateDefinition` as initial
//! - Creates `Relationship::Transition` from `TransitionUsage` children
//! - Copies entry/do/exit action text to state properties

use super::ElaborationReport;
use crate::resolution::resolved_props;
use crate::{
    CanonicalKey, ElementId, ElementKind, ModelGraph, Relationship, RelationshipKind, Value,
};

/// Elaborate all state machines in the graph.
pub(super) fn elaborate_state_machines(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    tag_parallel_states(graph, report);
    tag_initial_states(graph, report);
    create_transitions_from_usages(graph, report);
    derive_state_sequencing(graph, report);
    tag_final_states(graph, report);
    tag_state_actions(graph, report);
}

/// Derive the implicit `stateSequencing` successions between a state's exclusive
/// substates (SysML Systems Library `States.sysml:71-77`):
///
/// ```text
/// succession stateSequencing first [0..1] exclusiveStates then [0..1] exclusiveStates;
/// assert constraint { notEmpty(exclusiveStates) implies size(stateSequencing) == size(exclusiveStates) - 1 }
/// ```
///
/// The exclusive substates of a (non-parallel) state cannot overlap in time, so
/// the library models a strict temporal sequence over them — a chain of N-1
/// successions for N exclusive states. We materialise that chain on the
/// `ModelGraph` as `Succession` relationships tagged `stateSequencing = true`,
/// linking the exclusive `StateUsage` children in declaration (source-span) order.
///
/// The tag distinguishes these implicit ordering successions from user-declared
/// successions/transitions; consumers that present user edges (e.g. the diagram's
/// Successions compartment) skip tagged ones. Parallel states (whose substates run
/// concurrently and are therefore NOT mutually exclusive) are excluded.
///
/// Additive and idempotent: a `stateSequencing` succession is created only if one
/// does not already link the pair.
fn derive_state_sequencing(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    // Both state definitions and state usages can own exclusive substates.
    let owner_ids: Vec<ElementId> = graph
        .element_ids_by_kind(&ElementKind::StateDefinition)
        .iter()
        .chain(graph.element_ids_by_kind(&ElementKind::StateUsage).iter())
        .cloned()
        .collect();

    for owner_id in owner_ids {
        // Parallel states run their substates concurrently — not mutually
        // exclusive, so no strict sequencing applies.
        let is_parallel = graph
            .get_element(&owner_id)
            .and_then(|e| e.get_prop("isParallel"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_parallel {
            continue;
        }

        // exclusiveStates = the direct StateUsage children, in declaration order.
        let mut exclusive: Vec<(ElementId, usize, Option<String>)> = graph
            .children_of(&owner_id)
            .filter(|e| e.kind == ElementKind::StateUsage)
            .map(|e| {
                let span_start = e.spans.first().map(|s| s.start).unwrap_or(usize::MAX);
                (e.id.clone(), span_start, e.name.clone())
            })
            .collect();
        if exclusive.len() < 2 {
            continue;
        }
        exclusive.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));

        // Link consecutive exclusive states: N states -> N-1 successions.
        for pair in exclusive.windows(2) {
            let src = pair[0].0.clone();
            let tgt = pair[1].0.clone();

            let already_sequenced = graph
                .relationships_by_kind(&RelationshipKind::Succession)
                .any(|r| {
                    r.source == src
                        && r.target == tgt
                        && r.props
                            .get("stateSequencing")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                });
            if already_sequenced {
                continue;
            }

            let src_key = CanonicalKey::root(&src.to_string());
            let tgt_key = CanonicalKey::root(&tgt.to_string());
            let edge_key = CanonicalKey::for_relationship(
                &src_key,
                RelationshipKind::Succession.as_str(),
                &tgt_key,
                0,
            );
            let mut rel =
                Relationship::new_with_key(RelationshipKind::Succession, src, tgt, &edge_key);
            rel.props
                .insert("stateSequencing".into(), Value::Bool(true));
            graph.add_relationship(rel);
            report.state_sequencing_created += 1;
        }
    }
}

/// Tag the first `StateUsage` child of each `StateDefinition` as initial,
/// unless a state already has the `initial` property set.
fn tag_initial_states(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    // Collect state definitions
    let state_def_ids: Vec<ElementId> = graph
        .element_ids_by_kind(&ElementKind::StateDefinition)
        .to_vec();

    for def_id in state_def_ids {
        // Check if any child StateUsage already has initial=true
        // Collect children sorted by span position (or name as fallback)
        // to ensure deterministic ordering regardless of HashSet iteration
        let mut children: Vec<(ElementId, usize, Option<String>)> = graph
            .children_of(&def_id)
            .filter(|e| e.kind == ElementKind::StateUsage)
            .map(|e| {
                let span_start = e.spans.first().map(|s| s.start).unwrap_or(usize::MAX);
                (e.id.clone(), span_start, e.name.clone())
            })
            .collect();

        if children.is_empty() {
            continue;
        }

        // Sort by span position first, then by name for hand-built graphs
        children.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));

        let has_initial = children.iter().any(|(id, _, _)| {
            graph
                .get_element(id)
                .and_then(|e| e.get_prop("initial"))
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
        });

        if !has_initial {
            // Tag the first StateUsage child as initial
            if let Some((first_id, _, _)) = children.first() {
                if let Some(elem) = graph.get_element_mut(first_id) {
                    elem.set_prop("initial", true);
                    report.initial_states_tagged += 1;
                }
            }
        }
    }
}

/// Create `Relationship::Transition` from `TransitionUsage` children of state definitions.
///
/// TransitionUsage elements in the parsed model are children of StateDefinition
/// or StateUsage elements. We extract source/target information from:
/// 1. Explicit `source`/`target` properties (already set by parser)
/// 2. `unresolved_source`/`unresolved_target` string properties (name-based)
/// 3. Owner context: if inside a state, the owner is the implicit source
/// 4. Children that indicate the target (e.g., SuccessionStateUsage children)
#[allow(clippy::print_stderr)] // debug output gated behind SYSML_DEBUG_TRANSITIONS env var
fn create_transitions_from_usages(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    // Collect TransitionUsage elements
    let transition_ids = graph
        .element_ids_by_kind(&ElementKind::TransitionUsage)
        .to_vec();
    let transition_infos: Vec<TransitionInfo> = transition_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .map(|e| TransitionInfo {
            id: e.id.clone(),
            name: e.name.clone(),
            owner: e.owner.clone(),
            source_prop: e.get_prop("source").cloned(),
            target_prop: e.get_prop("target").cloned(),
            unresolved_source: e
                .get_prop("unresolved_source")
                .and_then(|v| v.as_str())
                .map(String::from),
            unresolved_target: e
                .get_prop("unresolved_target")
                .and_then(|v| v.as_str())
                .map(String::from),
            // Trigger/guard/effect are real child usages wrapped in
            // TransitionFeatureMembership(kind); the textual forms are
            // derived from the children (one home: transition_feature_text).
            event: graph.transition_feature_text(&e.id, "trigger"),
            trigger_port: e
                .get_prop("trigger_port")
                .and_then(|v| v.as_str())
                .map(String::from),
            guard: graph.transition_feature_text(&e.id, "guard"),
            effect: graph.transition_feature_text(&e.id, "effect"),
        })
        .collect();

    for info in transition_infos {
        // Try to resolve source
        let source_id = resolve_state_ref(
            graph,
            &info.owner,
            &info.source_prop,
            &info.unresolved_source,
        )
        .or_else(|| {
            // If inside a state usage, the owner is the implicit source.
            info.owner.as_ref().and_then(|owner_id| {
                let owner = graph.get_element(owner_id)?;
                if owner.kind == ElementKind::StateUsage {
                    Some(owner_id.clone())
                } else if owner.kind == ElementKind::StateDefinition {
                    // For transitions directly owned by a StateDefinition (e.g.
                    // `accept X then Y;` at the top level of a state def), infer
                    // the source as the nearest preceding StateUsage sibling.
                    let transition_span_start = graph
                        .get_element(&info.id)
                        .and_then(|e| e.spans.first())
                        .map(|s| s.start)
                        .unwrap_or(usize::MAX);
                    let mut best: Option<(usize, sysml_id::ElementId)> = None;
                    for sibling in graph.children_of(owner_id) {
                        if sibling.kind == ElementKind::StateUsage {
                            if let Some(span) = sibling.spans.first() {
                                if span.start < transition_span_start {
                                    match &best {
                                        Some((best_start, _)) if span.start > *best_start => {
                                            best = Some((span.start, sibling.id.clone()));
                                        }
                                        None => {
                                            best = Some((span.start, sibling.id.clone()));
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }
                    best.map(|(_, id)| id)
                } else {
                    None
                }
            })
        });

        // Try to resolve target
        let target_id = resolve_state_ref(
            graph,
            &info.owner,
            &info.target_prop,
            &info.unresolved_target,
        )
        .or_else(|| {
            // Look for child elements that indicate the target
            // (e.g., a SuccessionStateUsage or ReferenceUsage child)
            find_transition_target_from_children(graph, &info.id)
        });

        #[cfg(debug_assertions)]
        if std::env::var("SYSML_DEBUG_TRANSITIONS").is_ok() {
            eprintln!(
                "    resolved: source={:?}, target={:?}",
                source_id
                    .as_ref()
                    .and_then(|id| graph.get_element(id))
                    .and_then(|e| e.name.clone()),
                target_id
                    .as_ref()
                    .and_then(|id| graph.get_element(id))
                    .and_then(|e| e.name.clone()),
            );
        }

        // Additive: stamp the resolved source/target state refs onto the
        // TransitionUsage itself, leaving the existing `source`/`target` STRING
        // props untouched for their existing readers. The semantic-token emitter
        // reads these `Value::Ref`s to colour the transition's source/target
        // reference-site names by the resolved state's kind (Phase B.2). This is
        // stamped independently of the `Relationship::Transition` minted below —
        // each side is recorded whenever it resolves, even if the relationship
        // already exists or the other side is unresolved.
        if source_id.is_some() || target_id.is_some() {
            if let Some(elem) = graph.get_element_mut(&info.id) {
                if let Some(src) = source_id.as_ref() {
                    elem.props.insert(
                        resolved_props::TRANSITION_SOURCE.into(),
                        Value::Ref(src.clone()),
                    );
                }
                if let Some(tgt) = target_id.as_ref() {
                    elem.props.insert(
                        resolved_props::TRANSITION_TARGET.into(),
                        Value::Ref(tgt.clone()),
                    );
                }
            }
        }

        if let (Some(src), Some(tgt)) = (source_id, target_id) {
            // Check if this transition relationship already exists
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
                // originating TransitionUsage element span.
                rel.props
                    .insert("origin_transition".into(), Value::Ref(info.id.clone()));

                // Track whether the event came from a real trigger (accept action)
                // vs just the transition's name. Guard-only transitions have no
                // real trigger — the orchestrator uses this to detect them.
                let has_trigger = info.event.is_some();
                rel.props
                    .insert("has_trigger".into(), Value::Bool(has_trigger));
                if let Some(event) = info.event.as_ref().or(info.name.as_ref()) {
                    rel.props
                        .insert("event".into(), Value::String(event.clone()));
                }
                if let Some(trigger_port) = &info.trigger_port {
                    rel.props
                        .insert("trigger_port".into(), Value::String(trigger_port.clone()));
                }
                if let Some(guard) = &info.guard {
                    rel.props
                        .insert("guard".into(), Value::String(guard.clone()));
                }
                if let Some(effect) = &info.effect {
                    rel.props
                        .insert("action".into(), Value::String(effect.clone()));
                }

                graph.add_relationship(rel);
                report.transitions_created += 1;
            }
        }
    }
}

/// Info extracted from a TransitionUsage element for processing.
struct TransitionInfo {
    id: ElementId,
    name: Option<String>,
    owner: Option<ElementId>,
    source_prop: Option<Value>,
    target_prop: Option<Value>,
    unresolved_source: Option<String>,
    unresolved_target: Option<String>,
    event: Option<String>,
    trigger_port: Option<String>,
    guard: Option<String>,
    effect: Option<String>,
}

/// Resolve a state reference from a property value or unresolved name.
fn resolve_state_ref(
    graph: &ModelGraph,
    owner: &Option<ElementId>,
    prop: &Option<Value>,
    unresolved_name: &Option<String>,
) -> Option<ElementId> {
    // Try resolved Ref first
    if let Some(value) = prop {
        if let Some(id) = value.as_ref() {
            if graph.get_element(id).is_some() {
                return Some(id.clone());
            }
        }
        // Try as string name
        if let Some(name) = value.as_str() {
            if let Some(id) = find_state_by_name_in_scope(graph, owner, name) {
                return Some(id);
            }
            return find_state_by_name(graph, name);
        }
    }

    // Try unresolved name
    if let Some(name) = unresolved_name {
        if let Some(id) = find_state_by_name_in_scope(graph, owner, name) {
            return Some(id);
        }
        return find_state_by_name(graph, name);
    }

    None
}

/// Find a StateUsage element by name anywhere in the graph.
fn find_state_by_name(graph: &ModelGraph, name: &str) -> Option<ElementId> {
    let normalized = normalize_state_name(name);
    if normalized.is_empty() {
        return None;
    }

    graph
        .elements
        .values()
        .find(|e| {
            if e.kind != ElementKind::StateUsage {
                return false;
            }
            let Some(elem_name) = e.name.as_deref() else {
                return false;
            };
            elem_name == name || normalize_state_name(elem_name) == normalized
        })
        .map(|e| e.id.clone())
}

/// Find a state by name near a transition owner, preferring the enclosing state definition.
fn find_state_by_name_in_scope(
    graph: &ModelGraph,
    owner: &Option<ElementId>,
    name: &str,
) -> Option<ElementId> {
    let normalized = normalize_state_name(name);
    if normalized.is_empty() {
        return None;
    }

    let enclosing_state_def = find_enclosing_state_definition(graph, owner)?;
    graph
        .children_of(&enclosing_state_def)
        .filter(|e| e.kind == ElementKind::StateUsage)
        .find(|e| {
            e.name
                .as_deref()
                .map(|n| normalize_state_name(n) == normalized)
                .unwrap_or(false)
        })
        .map(|e| e.id.clone())
}

fn find_enclosing_state_definition(
    graph: &ModelGraph,
    owner: &Option<ElementId>,
) -> Option<ElementId> {
    let mut current = owner.clone()?;
    loop {
        let element = graph.get_element(&current)?;
        if element.kind == ElementKind::StateDefinition {
            return Some(current);
        }
        current = element.owner.clone()?;
    }
}

fn normalize_state_name(name: &str) -> String {
    let mut text = name.trim().trim_end_matches(';').trim().to_owned();
    if let Some(rest) = text.strip_prefix("first ") {
        text = rest.trim().to_owned();
    }
    if let Some(rest) = text.strip_prefix("then ") {
        text = rest.trim().to_owned();
    }
    if text.contains(' ') {
        if let Some(last) = text.split_whitespace().last() {
            text = last.to_owned();
        }
    }
    if text.contains("::") {
        if let Some(last) = text.rsplit("::").next() {
            text = last.to_owned();
        }
    }
    text.trim_matches('\'').trim_matches('"').trim().to_owned()
}

/// Look for the transition target in the children of a TransitionUsage.
///
/// The parser may create child elements (like StateUsage with succession semantics
/// or ReferenceUsage) that indicate the target.
fn find_transition_target_from_children(
    graph: &ModelGraph,
    transition_id: &ElementId,
) -> Option<ElementId> {
    // Look for StateUsage children (e.g., SuccessionStateUsage mapped to StateUsage)
    for child in graph.children_of(transition_id) {
        if child.kind == ElementKind::StateUsage {
            // The child state might have a typing that references the real target
            if let Some(type_name) = child.get_prop("unresolved_type").and_then(|v| v.as_str()) {
                if let Some(id) = find_state_by_name(graph, type_name) {
                    return Some(id);
                }
            }
            // Or the child's name itself is the target state name
            if let Some(name) = &child.name {
                if let Some(id) = find_state_by_name(graph, name) {
                    if id != *transition_id {
                        return Some(id);
                    }
                }
            }
        }

        // Check ReferenceUsage children that might reference a state
        if child.kind == ElementKind::ReferenceUsage {
            if let Some(name) = &child.name {
                if let Some(id) = find_state_by_name(graph, name) {
                    return Some(id);
                }
            }
        }

        // Check for unresolved references in children's typings
        for typing_child in graph.children_of(&child.id) {
            if typing_child.kind == ElementKind::FeatureTyping
                || typing_child.kind.is_subtype_of(ElementKind::FeatureTyping)
            {
                if let Some(name) = typing_child
                    .get_prop("unresolved_type")
                    .and_then(|v| v.as_str())
                {
                    if let Some(id) = find_state_by_name(graph, name) {
                        return Some(id);
                    }
                }
            }
        }
    }

    None
}

/// Copy entry/do/exit action expressions from ActionUsage children to state properties.
///
/// The parser tags ActionUsage children with `stateSubactionKind` = "entry"/"do"/"exit".
/// This function copies the action's `unresolved_value` to the parent state's
/// `entry`/`do_action`/`exit` property.
fn tag_state_actions(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    // Collect state element IDs
    let mut state_ids = Vec::new();
    state_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::StateUsage));
    state_ids.extend_from_slice(graph.element_ids_by_kind(&ElementKind::StateDefinition));

    for state_id in state_ids {
        // Collect action info from children
        let action_info: Vec<(String, Option<String>)> = graph
            .children_of(&state_id)
            .filter(|e| e.kind == ElementKind::ActionUsage)
            .filter_map(|e| {
                let kind = e.get_prop("stateSubactionKind")?.as_str()?.to_owned();
                let value = crate::expression_pretty::pretty_print_owner(e, graph)
                    .or_else(|| {
                        e.get_prop("unresolved_value")
                            .and_then(|v| v.as_str())
                            .map(String::from)
                    })
                    .or_else(|| e.name.clone());
                Some((kind, value))
            })
            .collect();

        for (kind, value) in action_info {
            let prop_name = match kind.as_str() {
                "entry" => "entry",
                "do" => "do_action",
                "exit" => "exit",
                _ => continue,
            };

            // Only set if not already present (additive)
            if let Some(state) = graph.get_element_mut(&state_id) {
                if state.get_prop(prop_name).is_none() {
                    if let Some(v) = value {
                        state.set_prop(prop_name, v);
                    } else {
                        // Even without a value, mark that the action exists
                        state.set_prop(prop_name, "");
                    }
                    report.state_actions_tagged += 1;
                }
            }
        }
    }
}

/// Tag states that have no outgoing transitions or are named "done" as final.
///
/// A StateUsage is considered final if:
/// 1. It has no outgoing `Relationship::Transition` (no transition with this state as source), OR
/// 2. Its name is "done" (SysML v2 convention)
///
/// Only sets `final=true` if not already set (additive/idempotent).
/// Also marks inferred finals with `final_inferred=true` so downstream
/// diagnostics can distinguish inferred vs explicitly-modeled final states.
fn tag_final_states(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    // Collect all StateUsage IDs that are children of a StateDefinition
    let state_def_ids: Vec<ElementId> = graph
        .element_ids_by_kind(&ElementKind::StateDefinition)
        .to_vec();

    let mut state_usage_ids: Vec<ElementId> = Vec::new();
    for def_id in &state_def_ids {
        for child in graph.children_of(def_id) {
            if child.kind == ElementKind::StateUsage {
                state_usage_ids.push(child.id.clone());
            }
        }
    }

    // Collect outgoing transition sources for quick lookup
    let sources_with_outgoing: std::collections::HashSet<ElementId> = graph
        .relationships_by_kind(&RelationshipKind::Transition)
        .map(|r| r.source.clone())
        .collect();

    for state_id in state_usage_ids {
        let (already_final, is_done_name, has_outgoing) = {
            let Some(elem) = graph.get_element(&state_id) else {
                continue;
            };
            let already_final = elem
                .get_prop("final")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let is_done_name = elem.name.as_deref() == Some("done");
            let has_outgoing = sources_with_outgoing.contains(&state_id);
            (already_final, is_done_name, has_outgoing)
        };

        if already_final {
            continue;
        }

        if !has_outgoing || is_done_name {
            if let Some(elem) = graph.get_element_mut(&state_id) {
                elem.set_prop("final", true);
                elem.set_prop("final_inferred", true);
                report.final_states_tagged += 1;
            }
        }
    }
}

/// Tag StateDefinitions that have a parallel region structure as `isParallel=true`.
///
/// A StateDefinition is considered parallel if:
/// 1. It has the keyword `parallel` set as a property, OR
/// 2. It has multiple StateUsage children that each contain their own StateUsage
///    children (regions pattern)
///
/// Only sets `isParallel=true` if not already set (additive/idempotent).
fn tag_parallel_states(graph: &mut ModelGraph, _report: &mut ElaborationReport) {
    let state_def_ids: Vec<ElementId> = graph
        .element_ids_by_kind(&ElementKind::StateDefinition)
        .to_vec();

    for def_id in state_def_ids {
        // Skip if already tagged
        let already_parallel = graph
            .get_element(&def_id)
            .and_then(|e| e.get_prop("isParallel"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if already_parallel {
            continue;
        }

        // Check if the keyword "parallel" is set
        let has_parallel_keyword = graph
            .get_element(&def_id)
            .and_then(|e| e.get_prop("keyword"))
            .and_then(|v| v.as_str())
            .map(|s| s == "parallel")
            .unwrap_or(false);

        if has_parallel_keyword {
            if let Some(elem) = graph.get_element_mut(&def_id) {
                elem.set_prop("isParallel", true);
            }
            continue;
        }

        // Detect regions pattern: multiple StateUsage children each with their own StateUsage children
        let child_state_ids: Vec<ElementId> = graph
            .children_of(&def_id)
            .filter(|e| e.kind == ElementKind::StateUsage)
            .map(|e| e.id.clone())
            .collect();

        if child_state_ids.len() >= 2 {
            let regions_with_substates = child_state_ids
                .iter()
                .filter(|child_id| {
                    graph
                        .children_of(child_id)
                        .any(|gc| gc.kind == ElementKind::StateUsage)
                })
                .count();

            // If at least 2 children have their own sub-states, it's a parallel regions pattern
            if regions_with_substates >= 2 {
                if let Some(elem) = graph.get_element_mut(&def_id) {
                    elem.set_prop("isParallel", true);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate;
    use crate::Element;
    use sysml_span::Span;

    #[test]
    fn tags_first_state_as_initial() {
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("SM");
        let sm_id = graph.add_element(sm);

        // Use spans to ensure deterministic child ordering (idle first)
        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(sm_id.clone())
            .with_span(Span::new("test", 0, 10));
        let s1_id = graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("active")
            .with_owner(sm_id)
            .with_span(Span::new("test", 10, 20));
        let _s2_id = graph.add_element(s2);

        let report = elaborate(&mut graph);

        assert_eq!(report.initial_states_tagged, 1);
        let s1_elem = graph.get_element(&s1_id).unwrap();
        assert_eq!(
            s1_elem.get_prop("initial").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    /// Build a state def with `n` exclusive StateUsage substates (spans in order).
    fn state_def_with_substates(n: usize, parallel: bool) -> (ModelGraph, ElementId) {
        let mut graph = ModelGraph::new();
        let mut def = Element::new_with_kind(ElementKind::StateDefinition).with_name("SM");
        if parallel {
            def.set_prop("isParallel", true);
        }
        let def_id = graph.add_element(def);
        for i in 0..n {
            let s = Element::new_with_kind(ElementKind::StateUsage)
                .with_name(format!("s{i}"))
                .with_owner(def_id.clone())
                .with_span(Span::new("test", i * 10, i * 10 + 5));
            graph.add_element(s);
        }
        (graph, def_id)
    }

    fn count_state_sequencing(graph: &ModelGraph) -> usize {
        graph
            .relationships_by_kind(&RelationshipKind::Succession)
            .filter(|r| {
                r.props
                    .get("stateSequencing")
                    .and_then(Value::as_bool)
                    .unwrap_or(false)
            })
            .count()
    }

    #[test]
    fn derives_n_minus_one_state_sequencing_successions() {
        let (mut graph, _) = state_def_with_substates(3, false);
        let report = elaborate(&mut graph);
        // 3 exclusive substates -> 2 stateSequencing successions (States.sysml:77).
        assert_eq!(report.state_sequencing_created, 2);
        assert_eq!(count_state_sequencing(&graph), 2);
    }

    #[test]
    fn parallel_state_has_no_state_sequencing() {
        let (mut graph, _) = state_def_with_substates(3, true);
        let _ = elaborate(&mut graph);
        // Parallel substates run concurrently — not mutually exclusive, no sequencing.
        assert_eq!(count_state_sequencing(&graph), 0);
    }

    #[test]
    fn single_substate_has_no_state_sequencing() {
        let (mut graph, _) = state_def_with_substates(1, false);
        let _ = elaborate(&mut graph);
        assert_eq!(count_state_sequencing(&graph), 0);
    }

    #[test]
    fn state_sequencing_is_idempotent() {
        let (mut graph, _) = state_def_with_substates(3, false);
        let _ = elaborate(&mut graph);
        let after_first = count_state_sequencing(&graph);
        // Re-elaborating must not duplicate the derived successions.
        let report2 = elaborate(&mut graph);
        assert_eq!(report2.state_sequencing_created, 0, "second pass adds none");
        assert_eq!(count_state_sequencing(&graph), after_first);
    }

    #[test]
    fn does_not_retag_if_initial_exists() {
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("SM");
        let sm_id = graph.add_element(sm);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(sm_id.clone());
        let _s1_id = graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("active")
            .with_owner(sm_id)
            .with_prop("initial", true);
        let s2_id = graph.add_element(s2);

        let report = elaborate(&mut graph);

        // Should not tag anything new — s2 already marked initial
        assert_eq!(report.initial_states_tagged, 0);
        let s2_elem = graph.get_element(&s2_id).unwrap();
        assert_eq!(
            s2_elem.get_prop("initial").and_then(|v| v.as_bool()),
            Some(true)
        );
    }

    #[test]
    fn creates_transition_from_usage_with_names() {
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("SM");
        let sm_id = graph.add_element(sm);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(sm_id.clone());
        let s1_id = graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("active")
            .with_owner(sm_id.clone());
        let s2_id = graph.add_element(s2);

        // TransitionUsage with string source/target names; the trigger is a
        // real AcceptActionUsage child wrapped in a
        // TransitionFeatureMembership(kind=trigger) whose `text` prop carries
        // the canonical trigger string (SysML v2 §8.3.18.8).
        let t = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(sm_id)
            .with_prop("source", "idle")
            .with_prop("target", "active");
        let t_id = graph.add_element(t);
        graph.add_transition_feature(
            &t_id,
            "trigger",
            Element::new_with_kind(ElementKind::AcceptActionUsage).with_prop("text", "start"),
        );

        let report = elaborate(&mut graph);

        assert!(report.transitions_created >= 1);

        // Verify transition relationship was created
        let transitions: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Transition)
            .collect();
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].source, s1_id);
        assert_eq!(transitions[0].target, s2_id);
        assert_eq!(
            transitions[0].props.get("event").and_then(|v| v.as_str()),
            Some("start")
        );
    }

    #[test]
    fn stamps_resolved_transition_source_target_refs() {
        // Phase B.2: elaboration stamps ADDITIVE `resolvedTransitionSource`/
        // `resolvedTransitionTarget` (Value::Ref) on the TransitionUsage, leaving
        // the existing `source`/`target` STRING props untouched. The semantic-
        // token emitter reads these to colour the reference-site names.
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("SM");
        let sm_id = graph.add_element(sm);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(sm_id.clone());
        let s1_id = graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("active")
            .with_owner(sm_id.clone());
        let s2_id = graph.add_element(s2);

        let t = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(sm_id)
            .with_prop("source", "idle")
            .with_prop("target", "active");
        let t_id = graph.add_element(t);

        let _ = elaborate(&mut graph);

        let t_elem = graph.get_element(&t_id).unwrap();
        assert_eq!(
            t_elem
                .get_prop(crate::resolution::resolved_props::TRANSITION_SOURCE)
                .and_then(|v| v.as_ref()),
            Some(&s1_id),
            "resolvedTransitionSource must point at the source state"
        );
        assert_eq!(
            t_elem
                .get_prop(crate::resolution::resolved_props::TRANSITION_TARGET)
                .and_then(|v| v.as_ref()),
            Some(&s2_id),
            "resolvedTransitionTarget must point at the target state"
        );
        // Existing string props preserved — additive, no in-place retype.
        assert_eq!(
            t_elem.get_prop("source").and_then(|v| v.as_str()),
            Some("idle")
        );
        assert_eq!(
            t_elem.get_prop("target").and_then(|v| v.as_str()),
            Some("active")
        );
    }

    #[test]
    fn creates_transition_from_owner_context() {
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("SM");
        let sm_id = graph.add_element(sm);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(sm_id.clone());
        let s1_id = graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("active")
            .with_owner(sm_id);
        let s2_id = graph.add_element(s2);

        // TransitionUsage inside idle state, target=active (unresolved)
        let t = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(s1_id.clone())
            .with_prop("unresolved_target", "active");
        graph.add_element(t);

        let report = elaborate(&mut graph);

        assert!(report.transitions_created >= 1);

        let transitions: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Transition)
            .collect();
        assert!(!transitions.is_empty());
        // Source should be idle (from owner context)
        assert_eq!(transitions[0].source, s1_id);
        assert_eq!(transitions[0].target, s2_id);
    }

    #[test]
    fn tags_entry_exit_actions() {
        let mut graph = ModelGraph::new();

        let s = Element::new_with_kind(ElementKind::StateUsage).with_name("idle");
        let s_id = graph.add_element(s);

        let entry = Element::new_with_kind(ElementKind::ActionUsage)
            .with_owner(s_id.clone())
            .with_prop("stateSubactionKind", "entry")
            .with_prop("unresolved_value", "log('entering')");
        graph.add_element(entry);

        let exit = Element::new_with_kind(ElementKind::ActionUsage)
            .with_owner(s_id.clone())
            .with_prop("stateSubactionKind", "exit")
            .with_prop("unresolved_value", "log('exiting')");
        graph.add_element(exit);

        let report = elaborate(&mut graph);

        assert_eq!(report.state_actions_tagged, 2);

        let state = graph.get_element(&s_id).unwrap();
        assert_eq!(
            state.get_prop("entry").and_then(|v| v.as_str()),
            Some("log('entering')")
        );
        assert_eq!(
            state.get_prop("exit").and_then(|v| v.as_str()),
            Some("log('exiting')")
        );
    }

    #[test]
    fn idempotent_elaboration() {
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("SM");
        let sm_id = graph.add_element(sm);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(sm_id.clone());
        graph.add_element(s1);

        // First elaboration
        let r1 = elaborate(&mut graph);
        assert_eq!(r1.initial_states_tagged, 1);

        // Second elaboration — should not change anything
        let r2 = elaborate(&mut graph);
        assert_eq!(r2.initial_states_tagged, 0);
        assert_eq!(r2.total(), 0);
    }

    #[test]
    fn tags_final_state_no_outgoing_transitions() {
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("SM");
        let sm_id = graph.add_element(sm);

        let s_idle = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(sm_id.clone())
            .with_span(Span::new("test", 0, 10));
        let idle_id = graph.add_element(s_idle);

        let s_active = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("active")
            .with_owner(sm_id.clone())
            .with_span(Span::new("test", 10, 20));
        let active_id = graph.add_element(s_active);

        let s_done = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("done")
            .with_owner(sm_id.clone())
            .with_span(Span::new("test", 20, 30));
        let done_id = graph.add_element(s_done);

        // Transitions: idle -> active -> done (done has no outgoing)
        let t1 = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(sm_id.clone())
            .with_prop("source", "idle")
            .with_prop("target", "active");
        graph.add_element(t1);

        let t2 = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(sm_id)
            .with_prop("source", "active")
            .with_prop("target", "done");
        graph.add_element(t2);

        let report = elaborate(&mut graph);

        // "done" should be tagged final (no outgoing transitions AND named "done")
        assert!(report.final_states_tagged >= 1);
        let done_elem = graph.get_element(&done_id).unwrap();
        assert_eq!(
            done_elem.get_prop("final").and_then(|v| v.as_bool()),
            Some(true)
        );

        // "idle" has an outgoing transition, should NOT be final
        let idle_elem = graph.get_element(&idle_id).unwrap();
        assert!(
            idle_elem.get_prop("final").is_none()
                || idle_elem.get_prop("final").and_then(|v| v.as_bool()) != Some(true)
        );

        // "active" has an outgoing transition, should NOT be final
        let active_elem = graph.get_element(&active_id).unwrap();
        assert!(
            active_elem.get_prop("final").is_none()
                || active_elem.get_prop("final").and_then(|v| v.as_bool()) != Some(true)
        );
    }

    #[test]
    fn tags_parallel_state_with_regions() {
        let mut graph = ModelGraph::new();

        // Create a state definition with two region states, each with substates
        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("ParallelSM");
        let sm_id = graph.add_element(sm);

        // Region 1
        let region1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("region1")
            .with_owner(sm_id.clone());
        let region1_id = graph.add_element(region1);

        let r1_sub1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("r1_idle")
            .with_owner(region1_id.clone());
        graph.add_element(r1_sub1);

        let r1_sub2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("r1_active")
            .with_owner(region1_id);
        graph.add_element(r1_sub2);

        // Region 2
        let region2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("region2")
            .with_owner(sm_id.clone());
        let region2_id = graph.add_element(region2);

        let r2_sub1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("r2_idle")
            .with_owner(region2_id.clone());
        graph.add_element(r2_sub1);

        let r2_sub2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("r2_active")
            .with_owner(region2_id);
        graph.add_element(r2_sub2);

        let report = elaborate(&mut graph);

        // ParallelSM should be tagged isParallel=true
        let sm_elem = graph.get_element(&sm_id).unwrap();
        assert_eq!(
            sm_elem.get_prop("isParallel").and_then(|v| v.as_bool()),
            Some(true),
            "StateDefinition with two region states containing substates should be tagged isParallel"
        );

        // Verify idempotent: second elaboration shouldn't change anything
        let _ = report;
        let r2 = elaborate(&mut graph);
        // isParallel already set, should not re-tag
        let sm_elem2 = graph.get_element(&sm_id).unwrap();
        assert_eq!(
            sm_elem2.get_prop("isParallel").and_then(|v| v.as_bool()),
            Some(true)
        );
        // The second run should not have changed isParallel (already set)
        let _ = r2;
    }

    #[test]
    fn resolves_transition_names_with_prefix_tokens() {
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("Toggle");
        let sm_id = graph.add_element(sm);

        let off = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Off")
            .with_owner(sm_id.clone());
        let off_id = graph.add_element(off);

        let on = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("On")
            .with_owner(sm_id.clone());
        let on_id = graph.add_element(on);

        // Simulates parser outputs that include transition keywords in unresolved names.
        let t = Element::new_with_kind(ElementKind::TransitionUsage)
            .with_owner(sm_id)
            .with_prop("source", "first Off")
            .with_prop("target", "then On");
        graph.add_element(t);

        let report = elaborate(&mut graph);
        assert!(report.transitions_created >= 1);

        let transitions: Vec<_> = graph
            .relationships_by_kind(&RelationshipKind::Transition)
            .collect();
        assert_eq!(transitions.len(), 1);
        assert_eq!(transitions[0].source, off_id);
        assert_eq!(transitions[0].target, on_id);
    }
}
