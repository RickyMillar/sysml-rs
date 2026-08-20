//! Cardinality (member count) validation checks.
//!
//! Validates that containers don't exceed maximum counts for certain member types.

use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph};

/// Helper: count children of a specific kind.
#[allow(clippy::needless_pass_by_value)] // ElementKind is not Copy due to #[non_exhaustive]
fn count_children_of_kind(element: &Element, graph: &ModelGraph, kind: ElementKind) -> usize {
    graph
        .children_of(&element.id)
        .filter(|child| child.kind == kind || child.kind.is_subtype_of(kind.clone()))
        .count()
}

/// Helper: create a cardinality error.
fn cardinality_error(
    element: &Element,
    member_type: &'static str,
    max: usize,
    actual: usize,
    rule_id: &'static str,
) -> SemanticError {
    SemanticError {
        element_id: element.id.clone(),
        element_name: element.name.clone(),
        kind: SemanticErrorKind::CardinalityViolation {
            member_type,
            max,
            actual,
        },
        rule_id,
        is_warning: false,
    }
}

/// Rules S060-S063: At most one SubjectMembership in Requirements/Cases.
pub fn at_most_one_subject(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    let applicable = matches!(
        element.kind,
        ElementKind::RequirementDefinition
            | ElementKind::RequirementUsage
            | ElementKind::CaseDefinition
            | ElementKind::CaseUsage
    ) || element
        .kind
        .is_subtype_of(ElementKind::RequirementDefinition)
        || element.kind.is_subtype_of(ElementKind::RequirementUsage)
        || element.kind.is_subtype_of(ElementKind::CaseDefinition)
        || element.kind.is_subtype_of(ElementKind::CaseUsage);

    if !applicable {
        return None;
    }

    let count = count_children_of_kind(element, graph, ElementKind::SubjectMembership);
    if count > 1 {
        let rule_id = match element.kind {
            ElementKind::RequirementDefinition => "S060",
            ElementKind::RequirementUsage => "S061",
            ElementKind::CaseDefinition => "S062",
            ElementKind::CaseUsage => "S063",
            _ => "S060",
        };
        Some(vec![cardinality_error(
            element,
            "SubjectMembership",
            1,
            count,
            rule_id,
        )])
    } else {
        None
    }
}

/// Rules S064-S065: At most one ObjectiveMembership in Cases.
pub fn at_most_one_objective(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    let applicable = matches!(
        element.kind,
        ElementKind::CaseDefinition | ElementKind::CaseUsage
    ) || element.kind.is_subtype_of(ElementKind::CaseDefinition)
        || element.kind.is_subtype_of(ElementKind::CaseUsage);

    if !applicable {
        return None;
    }

    let count = count_children_of_kind(element, graph, ElementKind::ObjectiveMembership);
    if count > 1 {
        let rule_id = match element.kind {
            ElementKind::CaseDefinition => "S064",
            _ => "S065",
        };
        Some(vec![cardinality_error(
            element,
            "ObjectiveMembership",
            1,
            count,
            rule_id,
        )])
    } else {
        None
    }
}

/// Rules S066-S067: At most one ReturnParameterMembership in Functions/Expressions.
pub fn at_most_one_return_parameter(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    let applicable = matches!(
        element.kind,
        ElementKind::Function | ElementKind::Expression
    ) || element.kind.is_subtype_of(ElementKind::Function)
        || element.kind.is_subtype_of(ElementKind::Expression);

    if !applicable {
        return None;
    }

    let count = count_children_of_kind(element, graph, ElementKind::ReturnParameterMembership);
    if count > 1 {
        let rule_id = match element.kind {
            ElementKind::Function => "S066",
            _ => "S067",
        };
        Some(vec![cardinality_error(
            element,
            "ReturnParameterMembership",
            1,
            count,
            rule_id,
        )])
    } else {
        None
    }
}

/// Rules S068-S069: At most one entry, one do, one exit action in States.
pub fn at_most_one_state_subaction(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    let applicable = matches!(
        element.kind,
        ElementKind::StateDefinition | ElementKind::StateUsage
    );

    if !applicable {
        return None;
    }

    let rule_id = match element.kind {
        ElementKind::StateDefinition => "S068",
        _ => "S069",
    };

    // Count state subactions by kind. The tree-sitter parser lowers
    // `entry/do/exit action …` to an `ActionUsage` member wrapped in a
    // `StateSubactionMembership` (carrying the `kind` discriminator) and mirrors
    // that kind onto the member as a `stateSubactionKind` prop
    // (`process_state_subaction`). `children_of` returns the member ActionUsage
    // (not the wrapping membership), so the prop is the shape that fires here;
    // the membership branch is kept for graphs built with the membership as a
    // direct child. Count both so the rule fires on real parsed input
    // (§8.3.18.5/6).
    let mut entry_count = 0usize;
    let mut do_count = 0usize;
    let mut exit_count = 0usize;

    for child in graph.children_of(&element.id) {
        let kind_str = match child.kind {
            ElementKind::ActionUsage => {
                child.props.get("stateSubactionKind").and_then(|v| v.as_str())
            }
            ElementKind::StateSubactionMembership => {
                child.props.get("kind").and_then(|v| v.as_str())
            }
            _ => None,
        };
        match kind_str {
            Some("entry") => entry_count += 1,
            Some("do") => do_count += 1,
            Some("exit") => exit_count += 1,
            _ => {}
        }
    }

    let mut errors = Vec::new();
    if entry_count > 1 {
        errors.push(cardinality_error(
            element,
            "entry action",
            1,
            entry_count,
            rule_id,
        ));
    }
    if do_count > 1 {
        errors.push(cardinality_error(
            element,
            "do action",
            1,
            do_count,
            rule_id,
        ));
    }
    if exit_count > 1 {
        errors.push(cardinality_error(
            element,
            "exit action",
            1,
            exit_count,
            rule_id,
        ));
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rules S070-S071: At most one ViewRenderingMembership in Views.
pub fn at_most_one_view_rendering(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    let applicable = matches!(
        element.kind,
        ElementKind::ViewDefinition | ElementKind::ViewUsage
    );

    if !applicable {
        return None;
    }

    let count = count_children_of_kind(element, graph, ElementKind::ViewRenderingMembership);
    if count > 1 {
        let rule_id = match element.kind {
            ElementKind::ViewDefinition => "S070",
            _ => "S071",
        };
        Some(vec![cardinality_error(
            element,
            "ViewRenderingMembership",
            1,
            count,
            rule_id,
        )])
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_id::ElementId;

    #[test]
    fn one_subject_passes() {
        let mut graph = ModelGraph::new();
        let req =
            Element::new(ElementId::new_v4(), ElementKind::RequirementDefinition).with_name("Req");
        let req_id = graph.add_element(req);

        let subject = Element::new(ElementId::new_v4(), ElementKind::SubjectMembership)
            .with_owner(req_id.clone());
        graph.add_element(subject);

        let req_elem = graph.get_element(&req_id).unwrap();
        assert!(at_most_one_subject(req_elem, &graph).is_none());
    }

    #[test]
    fn two_subjects_fails() {
        let mut graph = ModelGraph::new();
        let req =
            Element::new(ElementId::new_v4(), ElementKind::RequirementDefinition).with_name("Req");
        let req_id = graph.add_element(req);

        let subject1 = Element::new(ElementId::new_v4(), ElementKind::SubjectMembership)
            .with_owner(req_id.clone());
        graph.add_element(subject1);

        let subject2 = Element::new(ElementId::new_v4(), ElementKind::SubjectMembership)
            .with_owner(req_id.clone());
        graph.add_element(subject2);

        let req_elem = graph.get_element(&req_id).unwrap();
        let result = at_most_one_subject(req_elem, &graph);
        assert!(result.is_some());
        let errors = result.unwrap();
        assert_eq!(errors[0].rule_id, "S060");
        assert!(matches!(
            errors[0].kind,
            SemanticErrorKind::CardinalityViolation {
                max: 1,
                actual: 2,
                ..
            }
        ));
    }

    #[test]
    fn state_with_one_entry_passes() {
        let mut graph = ModelGraph::new();
        let state =
            Element::new(ElementId::new_v4(), ElementKind::StateDefinition).with_name("MyState");
        let state_id = graph.add_element(state);

        let entry = Element::new(ElementId::new_v4(), ElementKind::StateSubactionMembership)
            .with_owner(state_id.clone())
            .with_prop("kind", "entry");
        graph.add_element(entry);

        let state_elem = graph.get_element(&state_id).unwrap();
        assert!(at_most_one_state_subaction(state_elem, &graph).is_none());
    }

    #[test]
    fn state_with_two_entries_fails() {
        let mut graph = ModelGraph::new();
        let state =
            Element::new(ElementId::new_v4(), ElementKind::StateDefinition).with_name("MyState");
        let state_id = graph.add_element(state);

        let entry1 = Element::new(ElementId::new_v4(), ElementKind::StateSubactionMembership)
            .with_owner(state_id.clone())
            .with_prop("kind", "entry");
        graph.add_element(entry1);

        let entry2 = Element::new(ElementId::new_v4(), ElementKind::StateSubactionMembership)
            .with_owner(state_id.clone())
            .with_prop("kind", "entry");
        graph.add_element(entry2);

        let state_elem = graph.get_element(&state_id).unwrap();
        let result = at_most_one_state_subaction(state_elem, &graph);
        assert!(result.is_some());
    }
}
