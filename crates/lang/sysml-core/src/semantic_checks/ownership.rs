//! Ownership context validation checks.
//!
//! Validates that membership elements appear in the correct ownership context.

use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph};

/// Helper: check that an element's owner matches one of the expected kinds.
fn check_owner_kind(
    element: &Element,
    graph: &ModelGraph,
    allowed_kinds: &[ElementKind],
    rule_id: &'static str,
) -> Option<Vec<SemanticError>> {
    let owner_id = element.owner.as_ref()?;
    let owner = graph.elements.get(owner_id)?;

    let is_valid = allowed_kinds
        .iter()
        .any(|allowed| owner.kind == *allowed || owner.kind.is_subtype_of(allowed.clone()));

    if is_valid {
        None
    } else {
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::OwnershipViolation {
                member_kind: element.kind.clone(),
                owner_kind: owner.kind.clone(),
            },
            rule_id,
            is_warning: false,
        }])
    }
}

/// Rule S040: ParameterMembership is only allowed in Behaviors and Steps.
pub fn parameter_membership_owning_type(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ParameterMembership {
        return None;
    }
    check_owner_kind(
        element,
        graph,
        &[
            ElementKind::Behavior,
            ElementKind::Step,
            ElementKind::Function,
            ElementKind::Expression,
            ElementKind::ActionDefinition,
            ElementKind::ActionUsage,
            ElementKind::CalculationDefinition,
            ElementKind::CalculationUsage,
            ElementKind::StateDefinition,
            ElementKind::StateUsage,
            ElementKind::TransitionUsage,
        ],
        "S040",
    )
}

/// Rule S041: ReturnParameterMembership is only in Functions and Expressions.
pub fn return_parameter_membership_owning_type(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ReturnParameterMembership {
        return None;
    }
    check_owner_kind(
        element,
        graph,
        &[
            ElementKind::Function,
            ElementKind::Expression,
            ElementKind::CalculationDefinition,
            ElementKind::CalculationUsage,
            ElementKind::ConstraintDefinition,
            ElementKind::ConstraintUsage,
        ],
        "S041",
    )
}

/// Rule S042: StateSubactionMembership only in StateDefinition/StateUsage.
pub fn state_subaction_owned_by_state(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::StateSubactionMembership {
        return None;
    }
    check_owner_kind(
        element,
        graph,
        &[ElementKind::StateDefinition, ElementKind::StateUsage],
        "S042",
    )
}

/// Rule S043: SubjectMembership only in Requirements and Cases.
pub fn subject_membership_in_req_or_case(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::SubjectMembership {
        return None;
    }
    check_owner_kind(
        element,
        graph,
        &[
            ElementKind::RequirementDefinition,
            ElementKind::RequirementUsage,
            ElementKind::CaseDefinition,
            ElementKind::CaseUsage,
        ],
        "S043",
    )
}

/// Rule S044: ObjectiveMembership only in Cases.
pub fn objective_membership_in_case(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ObjectiveMembership {
        return None;
    }
    check_owner_kind(
        element,
        graph,
        &[
            ElementKind::CaseDefinition,
            ElementKind::CaseUsage,
            ElementKind::AnalysisCaseDefinition,
            ElementKind::AnalysisCaseUsage,
            ElementKind::VerificationCaseDefinition,
            ElementKind::VerificationCaseUsage,
            ElementKind::UseCaseDefinition,
            ElementKind::UseCaseUsage,
        ],
        "S044",
    )
}

/// Rule S045: ActorMembership only in Requirements and Cases.
pub fn actor_membership_in_req_or_case(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ActorMembership {
        return None;
    }
    check_owner_kind(
        element,
        graph,
        &[
            ElementKind::RequirementDefinition,
            ElementKind::RequirementUsage,
            ElementKind::CaseDefinition,
            ElementKind::CaseUsage,
            ElementKind::UseCaseDefinition,
            ElementKind::UseCaseUsage,
        ],
        "S045",
    )
}

/// Rule S046: StakeholderMembership only in Requirements.
pub fn stakeholder_membership_in_requirement(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::StakeholderMembership {
        return None;
    }
    check_owner_kind(
        element,
        graph,
        &[
            ElementKind::RequirementDefinition,
            ElementKind::RequirementUsage,
            ElementKind::ConcernDefinition,
            ElementKind::ConcernUsage,
        ],
        "S046",
    )
}

/// Rule S047: RequirementConstraintMembership only in Requirements.
pub fn requirement_constraint_in_requirement(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::RequirementConstraintMembership {
        return None;
    }
    check_owner_kind(
        element,
        graph,
        &[
            ElementKind::RequirementDefinition,
            ElementKind::RequirementUsage,
        ],
        "S047",
    )
}

/// Rule S048: ViewRenderingMembership only in Views.
pub fn view_rendering_in_view(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ViewRenderingMembership {
        return None;
    }
    check_owner_kind(
        element,
        graph,
        &[ElementKind::ViewDefinition, ElementKind::ViewUsage],
        "S048",
    )
}

/// Rule S049: TransitionFeatureMembership only in TransitionUsage.
pub fn transition_feature_in_transition(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::TransitionFeatureMembership {
        return None;
    }
    check_owner_kind(element, graph, &[ElementKind::TransitionUsage], "S049")
}

/// Rule S050: VariantMembership must be owned by a variation.
pub fn variant_membership_in_variation(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::VariantMembership {
        return None;
    }

    let owner_id = element.owner.as_ref()?;
    let owner = graph.elements.get(owner_id)?;

    // Check if owner has isVariation=true
    let is_variation = owner
        .props
        .get("isVariation")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if is_variation {
        None
    } else {
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::OwnershipViolation {
                member_kind: ElementKind::VariantMembership,
                owner_kind: owner.kind.clone(),
            },
            rule_id: "S050",
            is_warning: false,
        }])
    }
}

/// Rule S051: ResultExpressionMembership only in Functions and Expressions.
pub fn result_expression_in_function_or_expression(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ResultExpressionMembership {
        return None;
    }
    check_owner_kind(
        element,
        graph,
        &[
            ElementKind::Function,
            ElementKind::Expression,
            ElementKind::CalculationDefinition,
            ElementKind::CalculationUsage,
            ElementKind::ConstraintDefinition,
            ElementKind::ConstraintUsage,
        ],
        "S051",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_id::ElementId;

    #[test]
    fn state_subaction_in_state_passes() {
        let mut graph = ModelGraph::new();
        let state =
            Element::new(ElementId::new_v4(), ElementKind::StateDefinition).with_name("MyState");
        let state_id = graph.add_element(state);

        let membership = Element::new(ElementId::new_v4(), ElementKind::StateSubactionMembership)
            .with_owner(state_id);
        let mem_id = graph.add_element(membership);

        let elem = graph.get_element(&mem_id).unwrap();
        assert!(state_subaction_owned_by_state(elem, &graph).is_none());
    }

    #[test]
    fn state_subaction_in_package_fails() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new(ElementId::new_v4(), ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let membership = Element::new(ElementId::new_v4(), ElementKind::StateSubactionMembership)
            .with_owner(pkg_id);
        let mem_id = graph.add_element(membership);

        let elem = graph.get_element(&mem_id).unwrap();
        let result = state_subaction_owned_by_state(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S042");
    }

    #[test]
    fn subject_in_requirement_passes() {
        let mut graph = ModelGraph::new();
        let req =
            Element::new(ElementId::new_v4(), ElementKind::RequirementDefinition).with_name("Req");
        let req_id = graph.add_element(req);

        let subject =
            Element::new(ElementId::new_v4(), ElementKind::SubjectMembership).with_owner(req_id);
        let subj_id = graph.add_element(subject);

        let elem = graph.get_element(&subj_id).unwrap();
        assert!(subject_membership_in_req_or_case(elem, &graph).is_none());
    }

    #[test]
    fn subject_in_package_fails() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new(ElementId::new_v4(), ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let subject =
            Element::new(ElementId::new_v4(), ElementKind::SubjectMembership).with_owner(pkg_id);
        let subj_id = graph.add_element(subject);

        let elem = graph.get_element(&subj_id).unwrap();
        let result = subject_membership_in_req_or_case(elem, &graph);
        assert!(result.is_some());
    }
}
