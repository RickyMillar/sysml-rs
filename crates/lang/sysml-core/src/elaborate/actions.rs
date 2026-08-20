//! Action elaboration.
//!
//! Normalizes properties on control-flow action elements so the action runner
//! and health diagnostics can find them in consistent locations. The parser may
//! store conditions, collections, targets, etc. under different property names
//! depending on the syntax variant.

use super::ElaborationReport;
use crate::expression_pretty::pretty_print_owner;
use crate::resolution::resolved_props;
use crate::{ElementId, ElementKind, ModelGraph, Value};

/// Elaborate actions: normalize condition/collection/target/receiver properties.
pub(super) fn elaborate_actions(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    elaborate_if_actions(graph, report);
    elaborate_loop_actions(graph, report);
    elaborate_send_actions(graph, report);
    elaborate_accept_actions(graph, report);
    elaborate_assignment_actions(graph, report);
    resolve_assignment_targets(graph);
    resolve_terminate_targets(graph);
}

/// Stamp `resolvedAssignmentTarget` (`Value::Ref`) on each
/// `AssignmentActionUsage` whose `<target> = …` LHS name resolves to a feature
/// in scope. ADDITIVE (leaves `target`/`targetFeature` strings intact) and
/// self-contained: lets the semantic-token emitter colour the assignment target
/// reference site by the resolved feature's kind. The name resolves in the
/// assignment's OWNER scope (`resolve_name` walks up to the enclosing part). A
/// miss stamps nothing — the emitter falls back to UNRESOLVED.
fn resolve_assignment_targets(graph: &mut ModelGraph) {
    let to_tag: Vec<(ElementId, ElementId)> = graph
        .element_ids_by_kind(&ElementKind::AssignmentActionUsage)
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop(resolved_props::ASSIGNMENT_TARGET).is_none())
        .filter_map(|e| {
            let name = e
                .get_prop("targetFeature")
                .and_then(|v| v.as_str())
                .or_else(|| e.get_prop("target").and_then(|v| v.as_str()))?
                .to_owned();
            // Dotted paths are chain refs (B.1.2 territory) — skip.
            if name.contains('.') || name.contains("::") {
                return None;
            }
            // An assignment target names a VALUE feature (attribute/part/port),
            // never another action. Resolve with that kind exclusion: plain
            // `resolve_name` would nondeterministically return this very
            // AssignmentActionUsage (it shares the target's name) via its
            // hash-ordered global fallback. Walk the owner scope up, preferring
            // a non-action feature.
            let start = e.owner.clone().unwrap_or_else(|| e.id.clone());
            let target = resolve_value_feature(graph, &start, &name)?;
            if target == e.id {
                return None;
            }
            Some((e.id.clone(), target))
        })
        .collect();

    for (id, target) in to_tag {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.props
                .insert(resolved_props::ASSIGNMENT_TARGET.into(), Value::Ref(target));
        }
    }
}

/// Stamp `resolvedTerminatedOccurrence` (`Value::Ref`) on each
/// `TerminateActionUsage` whose `terminate <ref>;` argument name resolves to a
/// feature in scope. ADDITIVE (leaves the parser's `unresolved_target` string
/// intact). Spec shape: the argument is a `NodeParameterMember`
/// (ParameterMembership) owning a `NodeParameter` (ReferenceUsage) whose
/// `FeatureBinding` (FeatureValue) expression names the terminated occurrence
/// (SysML.xtext `TerminateNode`/`NodeParameterMember`/`FeatureBinding`; vocab
/// `terminatedOccurrenceArgument`, SysML-vocab.ttl). Unlike an assignment
/// target, the terminated occurrence may legitimately BE an occurrence/action
/// usage, so only the terminate node itself is excluded. Dotted feature chains
/// (`proc.wf`) are chain refs (B.1.2 territory) — skipped, like assignment.
/// A miss stamps nothing.
fn resolve_terminate_targets(graph: &mut ModelGraph) {
    let to_tag: Vec<(ElementId, ElementId)> = graph
        .element_ids_by_kind(&ElementKind::TerminateActionUsage)
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop(resolved_props::TERMINATED_OCCURRENCE).is_none())
        .filter_map(|e| {
            let name = e
                .get_prop("unresolved_target")
                .and_then(|v| v.as_str())?
                .to_owned();
            if name.contains('.') || name.contains("::") {
                return None;
            }
            let start = e.owner.clone().unwrap_or_else(|| e.id.clone());
            let self_id = e.id.clone();
            let target = resolve_named_feature(graph, &start, &|c: &crate::Element| {
                c.name.as_deref() == Some(name.as_str()) && c.id != self_id
            })?;
            Some((e.id.clone(), target))
        })
        .collect();

    for (id, target) in to_tag {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.props.insert(
                resolved_props::TERMINATED_OCCURRENCE.into(),
                Value::Ref(target),
            );
        }
    }
}

/// Resolve a bare feature name to a VALUE feature (never an action usage),
/// walking the owner scope chain then falling back to a global search — with a
/// deterministic tie-break so co-named features resolve reproducibly (`children_of`
/// and `elements` are hash-ordered). Excludes `AssignmentActionUsage` so an
/// assignment target does not resolve to itself or a peer assignment.
fn resolve_value_feature(graph: &ModelGraph, start: &ElementId, name: &str) -> Option<ElementId> {
    let acceptable = |e: &crate::Element| {
        e.name.as_deref() == Some(name) && e.kind != ElementKind::AssignmentActionUsage
    };
    resolve_named_feature(graph, start, &acceptable)
}

/// Shared scope walk for the reference-site resolvers above: accept the first
/// `acceptable` element found on the owner chain (children scanned per level),
/// falling back to a global search — deterministic min-`ElementId` tie-break at
/// every level (`children_of` and `elements` are hash-ordered).
fn resolve_named_feature(
    graph: &ModelGraph,
    start: &ElementId,
    acceptable: &dyn Fn(&crate::Element) -> bool,
) -> Option<ElementId> {
    // Walk up the owner chain; at each level scan owned children.
    let mut cursor = Some(start.clone());
    while let Some(scope) = cursor {
        let mut hit: Option<ElementId> = None;
        for child in graph.children_of(&scope) {
            if acceptable(child) {
                // Deterministic: smallest ElementId wins on co-name collisions.
                match &hit {
                    Some(best) if &child.id >= best => {}
                    _ => hit = Some(child.id.clone()),
                }
            }
        }
        if let Some(id) = hit {
            return Some(id);
        }
        cursor = graph.get_element(&scope).and_then(|e| e.owner.clone());
    }
    // Global fallback, same exclusion + deterministic tie-break.
    graph
        .elements
        .values()
        .filter(|e| acceptable(e))
        .map(|e| e.id.clone())
        .min()
}

/// For `IfActionUsage`: tag `condition` from `unresolved_value` or condition child.
fn elaborate_if_actions(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let if_ids = graph
        .element_ids_by_kind(&ElementKind::IfActionUsage)
        .to_vec();
    let to_elaborate: Vec<(ElementId, String)> = if_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("condition").is_none())
        .filter_map(|e| {
            // AST-first: pretty-print the condition subtree if present.
            if let Some(val) = pretty_print_owner(e, graph) {
                return Some((e.id.clone(), val));
            }
            // Legacy string-prop fallback for hand-crafted graphs.
            if let Some(val) = e.get_prop("unresolved_value").and_then(|v| v.as_str()) {
                return Some((e.id.clone(), val.to_owned()));
            }
            // Try condition child
            let condition = find_condition_child(graph, &e.id)?;
            Some((e.id.clone(), condition))
        })
        .collect();

    for (id, condition) in to_elaborate {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("condition", Value::String(condition));
            report.actions_elaborated += 1;
        }
    }
}

/// For `WhileLoopActionUsage`/`ForLoopActionUsage`: tag condition/collection/body.
fn elaborate_loop_actions(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    // While loops: need condition
    let while_ids = graph
        .element_ids_by_kind(&ElementKind::WhileLoopActionUsage)
        .to_vec();
    let while_to_elaborate: Vec<(ElementId, String)> = while_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("condition").is_none())
        .filter_map(|e| {
            let val = pretty_print_owner(e, graph)
                .or_else(|| {
                    e.get_prop("unresolved_value")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
                .or_else(|| find_condition_child(graph, &e.id))?;
            Some((e.id.clone(), val))
        })
        .collect();

    for (id, condition) in while_to_elaborate {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("condition", Value::String(condition));
            report.actions_elaborated += 1;
        }
    }

    // For loops: need collection
    let for_ids = graph
        .element_ids_by_kind(&ElementKind::ForLoopActionUsage)
        .to_vec();
    let for_to_elaborate: Vec<(ElementId, String)> = for_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("collection").is_none())
        .filter_map(|e| {
            let val = e
                .get_prop("unresolved_collection")
                .and_then(|v| v.as_str())
                .or_else(|| e.get_prop("unresolved_reference").and_then(|v| v.as_str()))?;
            Some((e.id.clone(), val.to_owned()))
        })
        .collect();

    for (id, collection) in for_to_elaborate {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("collection", Value::String(collection));
            report.actions_elaborated += 1;
        }
    }
}

/// For `SendActionUsage`: normalize `target` and `payload` properties.
fn elaborate_send_actions(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let send_ids = graph
        .element_ids_by_kind(&ElementKind::SendActionUsage)
        .to_vec();
    let to_elaborate: Vec<(ElementId, Option<String>, Option<String>)> = send_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("target").is_none() || e.get_prop("payload").is_none())
        .map(|e| {
            let target = if e.get_prop("target").is_none() {
                e.get_prop("unresolved_target")
                    .and_then(|v| v.as_str())
                    .or_else(|| e.get_prop("unresolved_reference").and_then(|v| v.as_str()))
                    .map(String::from)
            } else {
                None
            };
            let payload = if e.get_prop("payload").is_none() {
                e.get_prop("unresolved_payload")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            } else {
                None
            };
            (e.id.clone(), target, payload)
        })
        .filter(|(_, t, p)| t.is_some() || p.is_some())
        .collect();

    for (id, target, payload) in to_elaborate {
        let mut changed = false;
        if let Some(tgt) = target {
            if let Some(elem) = graph.get_element_mut(&id) {
                elem.set_prop("target", Value::String(tgt));
                changed = true;
            }
        }
        if let Some(pld) = payload {
            if let Some(elem) = graph.get_element_mut(&id) {
                elem.set_prop("payload", Value::String(pld));
                changed = true;
            }
        }
        if changed {
            report.actions_elaborated += 1;
        }
    }
}

/// For `AcceptActionUsage`: normalize `receiver` property.
fn elaborate_accept_actions(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let accept_ids = graph
        .element_ids_by_kind(&ElementKind::AcceptActionUsage)
        .to_vec();
    let to_elaborate: Vec<(ElementId, String)> = accept_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| e.get_prop("receiver").is_none())
        .filter_map(|e| {
            let val = e
                .get_prop("unresolved_receiver")
                .and_then(|v| v.as_str())
                .or_else(|| e.get_prop("unresolved_reference").and_then(|v| v.as_str()))?;
            Some((e.id.clone(), val.to_owned()))
        })
        .collect();

    for (id, receiver) in to_elaborate {
        if let Some(elem) = graph.get_element_mut(&id) {
            elem.set_prop("receiver", Value::String(receiver));
            report.actions_elaborated += 1;
        }
    }
}

/// For `AssignmentActionUsage`: normalize `targetFeature` from `unresolved_target`.
fn elaborate_assignment_actions(graph: &mut ModelGraph, report: &mut ElaborationReport) {
    let assign_ids = graph
        .element_ids_by_kind(&ElementKind::AssignmentActionUsage)
        .to_vec();
    let to_elaborate: Vec<(ElementId, Option<String>, Option<String>)> = assign_ids
        .iter()
        .filter_map(|id| graph.get_element(id))
        .filter(|e| {
            e.get_prop("targetFeature").is_none() || e.get_prop("valueExpression").is_none()
        })
        .map(|e| {
            let target = if e.get_prop("targetFeature").is_none() {
                e.get_prop("unresolved_target")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            } else {
                None
            };
            let value = if e.get_prop("valueExpression").is_none() {
                pretty_print_owner(e, graph).or_else(|| {
                    e.get_prop("unresolved_value")
                        .and_then(|v| v.as_str())
                        .map(String::from)
                })
            } else {
                None
            };
            (e.id.clone(), target, value)
        })
        .filter(|(_, t, v)| t.is_some() || v.is_some())
        .collect();

    for (id, target, value) in to_elaborate {
        let mut changed = false;
        if let Some(tgt) = target {
            if let Some(elem) = graph.get_element_mut(&id) {
                elem.set_prop("targetFeature", Value::String(tgt));
                changed = true;
            }
        }
        if let Some(val) = value {
            if let Some(elem) = graph.get_element_mut(&id) {
                elem.set_prop("valueExpression", Value::String(val));
                changed = true;
            }
        }
        if changed {
            report.actions_elaborated += 1;
        }
    }
}

/// Find a condition expression from constraint children of an action element.
fn find_condition_child(graph: &ModelGraph, parent_id: &ElementId) -> Option<String> {
    for child in graph.children_of(parent_id) {
        if child.kind == ElementKind::ConstraintUsage
            || child.kind == ElementKind::AssertConstraintUsage
        {
            if let Some(val) = pretty_print_owner(child, graph) {
                return Some(val);
            }
            if let Some(val) = child
                .get_prop("constraint")
                .or_else(|| child.get_prop("expr"))
                .and_then(|v| v.as_str())
            {
                return Some(val.to_owned());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::elaborate::elaborate;
    use crate::Element;

    #[test]
    fn stamps_resolved_assignment_target_ref() {
        // B.2b: resolve_assignment_targets stamps resolvedAssignmentTarget
        // (Value::Ref) at the attribute the `<target> = …` LHS resolves to, so
        // the assignment target reference site can be coloured by its kind.
        let mut graph = ModelGraph::new();

        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Drive");
        let part_id = graph.add_element(part);

        let attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("V_applied")
            .with_owner(part_id.clone());
        let attr_id = graph.add_element(attr);

        let assign = Element::new_with_kind(ElementKind::AssignmentActionUsage)
            .with_name("V_applied")
            .with_owner(part_id.clone())
            .with_prop("target", "V_applied")
            .with_prop("targetFeature", "V_applied");
        let assign_id = graph.add_element(assign);

        let _ = elaborate(&mut graph);

        let a = graph.get_element(&assign_id).unwrap();
        assert_eq!(
            a.get_prop(crate::resolution::resolved_props::ASSIGNMENT_TARGET)
                .and_then(|v| v.as_ref()),
            Some(&attr_id),
            "resolvedAssignmentTarget must point at the assigned attribute"
        );
        // Existing string props preserved (additive).
        assert_eq!(
            a.get_prop("target").and_then(|v| v.as_str()),
            Some("V_applied")
        );
    }

    #[test]
    fn stamps_resolved_terminated_occurrence_ref() {
        // `terminate eng;` — the argument names an in-scope occurrence
        // feature; the resolver stamps resolvedTerminatedOccurrence
        // (Value::Ref) at it, ADDITIVE to the parser's unresolved_target
        // string (SysML.xtext TerminateNode/NodeParameterMember; vocab
        // terminatedOccurrenceArgument).
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("Drive");
        let action_id = graph.add_element(action);

        let eng = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("eng")
            .with_owner(action_id.clone());
        let eng_id = graph.add_element(eng);

        let term = Element::new_with_kind(ElementKind::TerminateActionUsage)
            .with_owner(action_id.clone())
            .with_prop("unresolved_target", "eng");
        let term_id = graph.add_element(term);

        let _ = elaborate(&mut graph);

        let t = graph.get_element(&term_id).unwrap();
        assert_eq!(
            t.get_prop(crate::resolution::resolved_props::TERMINATED_OCCURRENCE)
                .and_then(|v| v.as_ref()),
            Some(&eng_id),
            "resolvedTerminatedOccurrence must point at the terminated occurrence"
        );
        // Parser capture preserved (additive).
        assert_eq!(
            t.get_prop("unresolved_target").and_then(|v| v.as_str()),
            Some("eng")
        );
    }

    #[test]
    fn terminate_target_may_resolve_to_an_action_usage() {
        // The terminated occurrence may legitimately be an action usage —
        // only the terminate node itself is excluded from resolution.
        let mut graph = ModelGraph::new();

        let action = Element::new_with_kind(ElementKind::ActionDefinition).with_name("Mission");
        let action_id = graph.add_element(action);

        let sub = Element::new_with_kind(ElementKind::ActionUsage)
            .with_name("monitor")
            .with_owner(action_id.clone());
        let sub_id = graph.add_element(sub);

        let term = Element::new_with_kind(ElementKind::TerminateActionUsage)
            .with_owner(action_id.clone())
            .with_prop("unresolved_target", "monitor");
        let term_id = graph.add_element(term);

        let _ = elaborate(&mut graph);

        assert_eq!(
            graph
                .get_element(&term_id)
                .unwrap()
                .get_prop(crate::resolution::resolved_props::TERMINATED_OCCURRENCE)
                .and_then(|v| v.as_ref()),
            Some(&sub_id),
        );
    }

    #[test]
    fn terminate_dotted_target_is_skipped_honestly() {
        // Dotted chains are B.1.2 territory — no ref is stamped (never a
        // wrong one), the verbatim capture stays.
        let mut graph = ModelGraph::new();

        let term = Element::new_with_kind(ElementKind::TerminateActionUsage)
            .with_prop("unresolved_target", "proc.wf");
        let term_id = graph.add_element(term);

        let _ = elaborate(&mut graph);

        let t = graph.get_element(&term_id).unwrap();
        assert!(
            t.get_prop(crate::resolution::resolved_props::TERMINATED_OCCURRENCE)
                .is_none(),
            "dotted chain must not resolve in the bare-name arm"
        );
    }

    #[test]
    fn tags_if_condition_from_unresolved_value() {
        let mut graph = ModelGraph::new();

        let if_action = Element::new_with_kind(ElementKind::IfActionUsage)
            .with_name("checkTemp")
            .with_prop("unresolved_value", "temp > 100");
        let if_id = graph.add_element(if_action);

        let report = elaborate(&mut graph);

        assert!(report.actions_elaborated >= 1);
        let elem = graph.get_element(&if_id).unwrap();
        assert_eq!(
            elem.get_prop("condition").and_then(|v| v.as_str()),
            Some("temp > 100")
        );
    }

    #[test]
    fn does_not_overwrite_existing_condition() {
        let mut graph = ModelGraph::new();

        let if_action = Element::new_with_kind(ElementKind::IfActionUsage)
            .with_name("checkTemp")
            .with_prop("condition", "temp > 200")
            .with_prop("unresolved_value", "temp > 100");
        let if_id = graph.add_element(if_action);

        elaborate(&mut graph);

        let elem = graph.get_element(&if_id).unwrap();
        assert_eq!(
            elem.get_prop("condition").and_then(|v| v.as_str()),
            Some("temp > 200")
        );
    }

    #[test]
    fn tags_while_condition() {
        let mut graph = ModelGraph::new();

        let while_action = Element::new_with_kind(ElementKind::WhileLoopActionUsage)
            .with_name("pumpLoop")
            .with_prop("unresolved_value", "pressure < 50");
        let while_id = graph.add_element(while_action);

        let report = elaborate(&mut graph);

        assert!(report.actions_elaborated >= 1);
        let elem = graph.get_element(&while_id).unwrap();
        assert_eq!(
            elem.get_prop("condition").and_then(|v| v.as_str()),
            Some("pressure < 50")
        );
    }

    #[test]
    fn tags_for_collection() {
        let mut graph = ModelGraph::new();

        let for_action = Element::new_with_kind(ElementKind::ForLoopActionUsage)
            .with_name("processItems")
            .with_prop("unresolved_collection", "inventory.items");
        let for_id = graph.add_element(for_action);

        let report = elaborate(&mut graph);

        assert!(report.actions_elaborated >= 1);
        let elem = graph.get_element(&for_id).unwrap();
        assert_eq!(
            elem.get_prop("collection").and_then(|v| v.as_str()),
            Some("inventory.items")
        );
    }

    #[test]
    fn tags_send_target_and_payload() {
        let mut graph = ModelGraph::new();

        let send = Element::new_with_kind(ElementKind::SendActionUsage)
            .with_name("sendMsg")
            .with_prop("unresolved_target", "controller")
            .with_prop("unresolved_payload", "reading");
        let send_id = graph.add_element(send);

        let report = elaborate(&mut graph);

        assert!(report.actions_elaborated >= 1);
        let elem = graph.get_element(&send_id).unwrap();
        assert_eq!(
            elem.get_prop("target").and_then(|v| v.as_str()),
            Some("controller")
        );
        assert_eq!(
            elem.get_prop("payload").and_then(|v| v.as_str()),
            Some("reading")
        );
    }

    #[test]
    fn tags_accept_receiver() {
        let mut graph = ModelGraph::new();

        let accept = Element::new_with_kind(ElementKind::AcceptActionUsage)
            .with_name("receiveMsg")
            .with_prop("unresolved_receiver", "sensor");
        let accept_id = graph.add_element(accept);

        let report = elaborate(&mut graph);

        assert!(report.actions_elaborated >= 1);
        let elem = graph.get_element(&accept_id).unwrap();
        assert_eq!(
            elem.get_prop("receiver").and_then(|v| v.as_str()),
            Some("sensor")
        );
    }

    #[test]
    fn idempotent() {
        let mut graph = ModelGraph::new();

        let if_action = Element::new_with_kind(ElementKind::IfActionUsage)
            .with_name("check")
            .with_prop("unresolved_value", "x > 0");
        graph.add_element(if_action);

        let send = Element::new_with_kind(ElementKind::SendActionUsage)
            .with_name("notify")
            .with_prop("unresolved_target", "monitor");
        graph.add_element(send);

        let r1 = elaborate(&mut graph);
        assert!(r1.actions_elaborated > 0);

        let r2 = elaborate(&mut graph);
        assert_eq!(r2.actions_elaborated, 0, "second elaborate should be no-op");
    }
}
