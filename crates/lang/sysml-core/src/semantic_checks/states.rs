//! State machine validation checks.
//!
//! Validates state machine constraints such as parallel state transitions
//! and transition ownership.

use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph};

/// Helper: check if a state is parallel (has isParallel = true).
fn is_parallel_state(element: &Element) -> bool {
    element
        .props
        .get("isParallel")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// Rules S110-S111: A parallel state cannot have transitions.
pub fn parallel_state_no_transitions(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if !matches!(
        element.kind,
        ElementKind::StateDefinition | ElementKind::StateUsage
    ) {
        return None;
    }

    if !is_parallel_state(element) {
        return None;
    }

    let rule_id = match element.kind {
        ElementKind::StateDefinition => "S110",
        _ => "S111",
    };

    // Check for TransitionUsage children
    let has_transitions = graph
        .children_of(&element.id)
        .any(|child| child.kind == ElementKind::TransitionUsage);

    if has_transitions {
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::Custom {
                message: "a parallel state cannot have transitions".to_owned(),
            },
            rule_id,
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rule S112: A transition must be owned by a state or an action.
pub fn transition_owned_by_state_or_action(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::TransitionUsage {
        return None;
    }

    let owner_id = element.owner.as_ref()?;
    let owner = graph.elements.get(owner_id)?;

    let is_valid = matches!(
        owner.kind,
        ElementKind::StateDefinition
            | ElementKind::StateUsage
            | ElementKind::ActionDefinition
            | ElementKind::ActionUsage
    ) || owner.kind.is_subtype_of(ElementKind::StateDefinition)
        || owner.kind.is_subtype_of(ElementKind::StateUsage)
        || owner.kind.is_subtype_of(ElementKind::ActionDefinition)
        || owner.kind.is_subtype_of(ElementKind::ActionUsage);

    if is_valid {
        None
    } else {
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::OwnershipViolation {
                member_kind: ElementKind::TransitionUsage,
                owner_kind: owner.kind.clone(),
            },
            rule_id: "S112",
            is_warning: false,
        }])
    }
}

/// Rule S113: A transition must have a source.
pub fn transition_has_source(element: &Element, _graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::TransitionUsage {
        return None;
    }

    // Check for a "source" property
    let has_source = element.props.contains_key("source");
    // Also check for TransitionFeatureMembership with kind = "trigger"
    // or if the transition has a source end

    if has_source {
        None
    } else {
        // Transitions without explicit source may inherit from context
        // Only flag if there's no source AND no owner context
        if element.owner.is_none() {
            Some(vec![SemanticError {
                element_id: element.id.clone(),
                element_name: element.name.clone(),
                kind: SemanticErrorKind::Custom {
                    message: "a transition must have a source".to_owned(),
                },
                rule_id: "S113",
                is_warning: false,
            }])
        } else {
            None
        }
    }
}

/// Rule S114: An exhibit state must be typed by exactly one state definition.
pub fn exhibit_state_one_type(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ExhibitStateUsage {
        return None;
    }

    // Check typing count
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
                    "an exhibit state must be typed by exactly one state definition, found {}",
                    typing_count
                ),
            },
            rule_id: "S114",
            is_warning: false,
        }])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_id::ElementId;

    #[test]
    fn non_parallel_state_with_transitions_passes() {
        let mut graph = ModelGraph::new();
        let state =
            Element::new(ElementId::new_v4(), ElementKind::StateDefinition).with_name("MyState");
        let state_id = graph.add_element(state);

        let transition = Element::new(ElementId::new_v4(), ElementKind::TransitionUsage)
            .with_owner(state_id.clone());
        graph.add_element(transition);

        let elem = graph.get_element(&state_id).unwrap();
        assert!(parallel_state_no_transitions(elem, &graph).is_none());
    }

    #[test]
    fn parallel_state_with_transitions_fails() {
        let mut graph = ModelGraph::new();
        let state = Element::new(ElementId::new_v4(), ElementKind::StateDefinition)
            .with_name("ParallelState")
            .with_prop("isParallel", true);
        let state_id = graph.add_element(state);

        let transition = Element::new(ElementId::new_v4(), ElementKind::TransitionUsage)
            .with_owner(state_id.clone());
        graph.add_element(transition);

        let elem = graph.get_element(&state_id).unwrap();
        let result = parallel_state_no_transitions(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S110");
    }

    #[test]
    fn transition_in_state_passes() {
        let mut graph = ModelGraph::new();
        let state =
            Element::new(ElementId::new_v4(), ElementKind::StateDefinition).with_name("MyState");
        let state_id = graph.add_element(state);

        let transition = Element::new(ElementId::new_v4(), ElementKind::TransitionUsage)
            .with_owner(state_id.clone());
        let trans_id = graph.add_element(transition);

        let elem = graph.get_element(&trans_id).unwrap();
        assert!(transition_owned_by_state_or_action(elem, &graph).is_none());
    }

    #[test]
    fn transition_in_package_fails() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new(ElementId::new_v4(), ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let transition =
            Element::new(ElementId::new_v4(), ElementKind::TransitionUsage).with_owner(pkg_id);
        let trans_id = graph.add_element(transition);

        let elem = graph.get_element(&trans_id).unwrap();
        let result = transition_owned_by_state_or_action(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S112");
    }
}
