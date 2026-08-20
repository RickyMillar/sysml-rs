//! Action and control flow validation checks.
//!
//! Validates control node incoming/outgoing succession counts
//! and action parameter requirements.

use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph};

/// Helper: count successions where this element is the target (incoming).
fn count_incoming_successions(element: &Element, graph: &ModelGraph) -> usize {
    // Look for Succession elements that have this element as their target
    graph
        .elements
        .values()
        .filter(|e| {
            (e.kind == ElementKind::Succession || e.kind.is_subtype_of(ElementKind::Succession))
                && e.props
                    .get("target")
                    .and_then(|v| v.as_ref())
                    .is_some_and(|id| *id == element.id)
        })
        .count()
}

/// Helper: count successions where this element is the source (outgoing).
fn count_outgoing_successions(element: &Element, graph: &ModelGraph) -> usize {
    graph
        .elements
        .values()
        .filter(|e| {
            (e.kind == ElementKind::Succession || e.kind.is_subtype_of(ElementKind::Succession))
                && e.props
                    .get("source")
                    .and_then(|v| v.as_ref())
                    .is_some_and(|id| *id == element.id)
        })
        .count()
}

/// Rule S120: A merge node must have at most one outgoing succession.
pub fn merge_node_one_outgoing(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::MergeNode {
        return None;
    }

    let outgoing = count_outgoing_successions(element, graph);
    if outgoing > 1 {
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::CardinalityViolation {
                member_type: "outgoing succession",
                max: 1,
                actual: outgoing,
            },
            rule_id: "S120",
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rule S121: A decision node must have at most one incoming succession.
pub fn decision_node_one_incoming(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::DecisionNode {
        return None;
    }

    let incoming = count_incoming_successions(element, graph);
    if incoming > 1 {
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::CardinalityViolation {
                member_type: "incoming succession",
                max: 1,
                actual: incoming,
            },
            rule_id: "S121",
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rule S122: A fork node must have at most one incoming succession.
pub fn fork_node_one_incoming(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ForkNode {
        return None;
    }

    let incoming = count_incoming_successions(element, graph);
    if incoming > 1 {
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::CardinalityViolation {
                member_type: "incoming succession",
                max: 1,
                actual: incoming,
            },
            rule_id: "S122",
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rule S123: A join node must have at most one outgoing succession.
pub fn join_node_one_outgoing(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::JoinNode {
        return None;
    }

    let outgoing = count_outgoing_successions(element, graph);
    if outgoing > 1 {
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::CardinalityViolation {
                member_type: "outgoing succession",
                max: 1,
                actual: outgoing,
            },
            rule_id: "S123",
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rule S124: A perform action must be typed by exactly one action definition.
pub fn perform_action_one_type(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::PerformActionUsage {
        return None;
    }

    let typing_count = graph
        .typed_feature_to_typings
        .get(&element.id)
        .map_or(0, |ids| ids.len());

    if typing_count > 1 {
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::Custom {
                message: format!(
                    "a perform action must be typed by exactly one action definition, found {}",
                    typing_count
                ),
            },
            rule_id: "S124",
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rule S125: A send action must have a payload parameter.
///
/// Spec: §8.3.17.15 `validateSendActionParameters` — "A SendActionUsage must
/// have at least three owned input parameters, corresponding to its payload,
/// sender and receiver" (`inputParameters()->size() >= 3`). The first argument
/// is the payload (`deriveSendActionUsagePayloadArgument: payloadArgument =
/// argument(1)`), so a send lacking any payload argument is ill-formed.
///
/// In practice the tree-sitter grammar makes the payload mandatory at the
/// syntax level (`send_action: seq("send", _expression, …)`), so a payload-less
/// send is a parse error and never reaches this validator from real source — a
/// well-formed send always carries its payload as an `Expression` subtree child
/// (its spec slot is Derived). This check is therefore the defensive structural
/// gate for a `SendActionUsage` assembled without a payload (programmatically or
/// via a future representation change): if no payload-bearing child is present,
/// the send violates `validateSendActionParameters` and S125 fires.
pub fn send_action_has_payload(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::SendActionUsage {
        return None;
    }

    // The payload may be represented either as a ParameterMembership or — as the
    // tree-sitter parser lowers it — as the `payloadArgument` expression subtree
    // projected directly under the send. The `via <port>` target is stored as a
    // prop, not a child, so any expression child here is the payload.
    let has_payload = graph.children_of(&element.id).any(|child| {
        child.kind == ElementKind::ParameterMembership
            || child.kind.is_subtype_of(ElementKind::ParameterMembership)
            || child.kind == ElementKind::Expression
            || child.kind.is_subtype_of(ElementKind::Expression)
    });

    if has_payload {
        None
    } else {
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::Custom {
                message: "a send action must have a payload parameter".to_owned(),
            },
            rule_id: "S125",
            is_warning: false,
        }])
    }
}

/// Rule S126: An accept action must have a payload parameter.
pub fn accept_action_has_payload(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::AcceptActionUsage {
        return None;
    }

    // The accept's `payloadParameter` is lowered by the tree-sitter parser as a
    // ReferenceUsage slot (mirroring the accept's name/typing), wrapped in an
    // OwningMembership rather than a ParameterMembership envelope. Accept either
    // representation as satisfying the payload requirement.
    let has_parameter = graph.children_of(&element.id).any(|child| {
        child.kind == ElementKind::ParameterMembership
            || child.kind.is_subtype_of(ElementKind::ParameterMembership)
            || child.kind == ElementKind::ReferenceUsage
            || child.kind.is_subtype_of(ElementKind::ReferenceUsage)
    });

    if has_parameter {
        None
    } else {
        let has_children = graph.children_of(&element.id).next().is_some();
        if has_children {
            Some(vec![SemanticError {
                element_id: element.id.clone(),
                element_name: element.name.clone(),
                kind: SemanticErrorKind::Custom {
                    message: "an accept action must have a payload parameter".to_owned(),
                },
                rule_id: "S126",
                is_warning: false,
            }])
        } else {
            None
        }
    }
}

/// Rule S127: An assignment action must have a target feature.
pub fn assignment_action_has_target(
    element: &Element,
    _graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::AssignmentActionUsage {
        return None;
    }

    // Check for "targetFeature" property
    let has_target = element.props.contains_key("targetFeature");

    if has_target {
        None
    } else {
        // Only flag if the element appears to be elaborated (has a name)
        if element.name.is_some() {
            Some(vec![SemanticError {
                element_id: element.id.clone(),
                element_name: element.name.clone(),
                kind: SemanticErrorKind::Custom {
                    message: "an assignment action must have a target feature".to_owned(),
                },
                rule_id: "S127",
                is_warning: false,
            }])
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Value;
    use sysml_id::ElementId;

    #[test]
    fn merge_node_one_outgoing_passes() {
        let mut graph = ModelGraph::new();
        let merge = Element::new(ElementId::new_v4(), ElementKind::MergeNode).with_name("Merge");
        let merge_id = graph.add_element(merge);

        let target = Element::new(ElementId::new_v4(), ElementKind::ActionUsage).with_name("Next");
        let target_id = graph.add_element(target);

        let succ = Element::new(ElementId::new_v4(), ElementKind::Succession)
            .with_prop("source", Value::Ref(merge_id.clone()))
            .with_prop("target", Value::Ref(target_id));
        graph.add_element(succ);

        let elem = graph.get_element(&merge_id).unwrap();
        assert!(merge_node_one_outgoing(elem, &graph).is_none());
    }

    #[test]
    fn merge_node_two_outgoing_fails() {
        let mut graph = ModelGraph::new();
        let merge = Element::new(ElementId::new_v4(), ElementKind::MergeNode).with_name("Merge");
        let merge_id = graph.add_element(merge);

        for _ in 0..2 {
            let target =
                Element::new(ElementId::new_v4(), ElementKind::ActionUsage).with_name("Next");
            let target_id = graph.add_element(target);

            let succ = Element::new(ElementId::new_v4(), ElementKind::Succession)
                .with_prop("source", Value::Ref(merge_id.clone()))
                .with_prop("target", Value::Ref(target_id));
            graph.add_element(succ);
        }

        let elem = graph.get_element(&merge_id).unwrap();
        let result = merge_node_one_outgoing(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S120");
    }

    #[test]
    fn decision_node_no_incoming_passes() {
        let graph = ModelGraph::new();
        let elem = Element::new(ElementId::new_v4(), ElementKind::DecisionNode).with_name("Dec");

        assert!(decision_node_one_incoming(&elem, &graph).is_none());
    }

    #[test]
    fn send_without_payload_child_raises_s125() {
        // §8.3.17.15 validateSendActionParameters: a send lacking its payload
        // parameter is ill-formed. A payload-less send is a parse error, so this
        // gate exercises the validator against a hand-built malformed graph.
        let mut graph = ModelGraph::new();
        let send =
            Element::new(ElementId::new_v4(), ElementKind::SendActionUsage).with_name("snd");
        let send_id = graph.add_element(send);

        let elem = graph.get_element(&send_id).unwrap();
        let result = send_action_has_payload(elem, &graph);
        assert!(result.is_some(), "a send with no payload must raise a diagnostic");
        assert_eq!(result.unwrap()[0].rule_id, "S125");
    }

    #[test]
    fn send_with_payload_expression_child_is_clean() {
        // A well-formed send carries its payload as an Expression subtree child
        // (the tree-sitter lowering of `send <expr> …`); S125 must not fire.
        let mut graph = ModelGraph::new();
        let send =
            Element::new(ElementId::new_v4(), ElementKind::SendActionUsage).with_name("snd");
        let send_id = graph.add_element(send);

        let payload = Element::new(ElementId::new_v4(), ElementKind::Expression)
            .with_owner(send_id.clone());
        graph.add_element(payload);

        let elem = graph.get_element(&send_id).unwrap();
        assert!(
            send_action_has_payload(elem, &graph).is_none(),
            "a send with a payload expression child is well-formed"
        );
    }
}
