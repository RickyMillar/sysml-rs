//! Requirement and case validation checks.
//!
//! Validates requirement constraint composites, subject cardinality,
//! and satisfaction typing.

use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph, Value};

/// Helper: count children of a specific kind.
#[allow(clippy::needless_pass_by_value)] // ElementKind is not Copy due to #[non_exhaustive]
fn count_children_of_kind(element: &Element, graph: &ModelGraph, kind: ElementKind) -> usize {
    graph
        .children_of(&element.id)
        .filter(|child| child.kind == kind || child.kind.is_subtype_of(kind.clone()))
        .count()
}

/// Rules S130-S131: Requirement constraints must be composite.
///
/// Assumed and required constraints in requirements must be compositely owned.
pub fn requirement_constraints_composite(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if !matches!(
        element.kind,
        ElementKind::RequirementDefinition | ElementKind::RequirementUsage
    ) && !element
        .kind
        .is_subtype_of(ElementKind::RequirementDefinition)
        && !element.kind.is_subtype_of(ElementKind::RequirementUsage)
    {
        return None;
    }

    let rule_id = match element.kind {
        ElementKind::RequirementDefinition => "S130",
        _ => "S131",
    };

    let mut errors = Vec::new();

    for child in graph.children_of(&element.id) {
        if child.kind == ElementKind::RequirementConstraintMembership {
            // Check if the constraint member is composite (owned)
            let is_composite = child
                .props
                .get("isComposite")
                .and_then(|v| v.as_bool())
                .unwrap_or(true); // Default to composite (most common case)

            if !is_composite {
                errors.push(SemanticError {
                    element_id: child.id.clone(),
                    element_name: child.name.clone(),
                    kind: SemanticErrorKind::Custom {
                        message: "requirement constraints must be composite".to_owned(),
                    },
                    rule_id,
                    is_warning: false,
                });
            }
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S132: A satisfy requirement must be typed by one requirement definition.
pub fn satisfy_req_one_type(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::SatisfyRequirementUsage {
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
                    "a satisfy requirement must be typed by one requirement definition, found {}",
                    typing_count
                ),
            },
            rule_id: "S132",
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rules S133-S134: A concern can have at most one subject.
pub fn concern_at_most_one_subject(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if !matches!(
        element.kind,
        ElementKind::ConcernDefinition | ElementKind::ConcernUsage
    ) {
        return None;
    }

    let count = count_children_of_kind(element, graph, ElementKind::SubjectMembership);
    if count > 1 {
        let rule_id = match element.kind {
            ElementKind::ConcernDefinition => "S133",
            _ => "S134",
        };
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::CardinalityViolation {
                member_type: "SubjectMembership",
                max: 1,
                actual: count,
            },
            rule_id,
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rules S135-S136: A verification case can have at most one subject.
pub fn verification_case_at_most_one_subject(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if !matches!(
        element.kind,
        ElementKind::VerificationCaseDefinition | ElementKind::VerificationCaseUsage
    ) {
        return None;
    }

    let count = count_children_of_kind(element, graph, ElementKind::SubjectMembership);
    if count > 1 {
        let rule_id = match element.kind {
            ElementKind::VerificationCaseDefinition => "S135",
            _ => "S136",
        };
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::CardinalityViolation {
                member_type: "SubjectMembership",
                max: 1,
                actual: count,
            },
            rule_id,
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rules S137-S138: An analysis case can have at most one subject.
pub fn analysis_case_at_most_one_subject(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if !matches!(
        element.kind,
        ElementKind::AnalysisCaseDefinition | ElementKind::AnalysisCaseUsage
    ) {
        return None;
    }

    let count = count_children_of_kind(element, graph, ElementKind::SubjectMembership);
    if count > 1 {
        let rule_id = match element.kind {
            ElementKind::AnalysisCaseDefinition => "S137",
            _ => "S138",
        };
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::CardinalityViolation {
                member_type: "SubjectMembership",
                max: 1,
                actual: count,
            },
            rule_id,
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rules S139-S140: A use case can have at most one subject.
pub fn use_case_at_most_one_subject(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if !matches!(
        element.kind,
        ElementKind::UseCaseDefinition | ElementKind::UseCaseUsage
    ) {
        return None;
    }

    let count = count_children_of_kind(element, graph, ElementKind::SubjectMembership);
    if count > 1 {
        let rule_id = match element.kind {
            ElementKind::UseCaseDefinition => "S139",
            _ => "S140",
        };
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::CardinalityViolation {
                member_type: "SubjectMembership",
                max: 1,
                actual: count,
            },
            rule_id,
            is_warning: false,
        }])
    } else {
        None
    }
}

/// Rules S141-S144: the subject parameter must be the requirement's / case's
/// first input.
///
/// SysML §8.3.21 `validateRequirementDefinitionSubjectParameterPosition` (and the
/// identical `RequirementUsage` rule): the `subjectParameter` of a requirement
/// "must be its first `input`" — `input->notEmpty() and input->first() =
/// subjectParameter`. The parallel case-subject prose (§8.3.x) states the same:
/// the `subject` keyword "must come before the declaration of any other
/// parameters". KerML derives `/input` as the features whose `direction` is `in`
/// or `inout`; a directionless feature (e.g. a plain `attribute`) is NOT an input.
///
/// The subject parameter is always an `in` parameter (§8.3.21.8) and is identified
/// structurally by its `SubjectMembership`. So, among the requirement's/case's
/// owned input features (the subject plus any `in`/`inout` feature), ordered by
/// source position, the subject must come first. A directed input declared before
/// the subject is a violation. (When no subject is declared the subject is
/// implicitly `Anything` and there is no ordering to check.)
pub fn subject_is_first_parameter(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    // Map the element kind to its rule id; non-requirement/case kinds are N/A.
    // Requirement subtypes (e.g. ConcernDefinition) and case subtypes (use-case /
    // verification-case / analysis-case def & usage) inherit the obligation.
    let rule_id = if element.kind == ElementKind::RequirementDefinition
        || element.kind.is_subtype_of(ElementKind::RequirementDefinition)
    {
        "S141"
    } else if element.kind == ElementKind::RequirementUsage
        || element.kind.is_subtype_of(ElementKind::RequirementUsage)
    {
        "S142"
    } else if element.kind == ElementKind::CaseDefinition
        || element.kind.is_subtype_of(ElementKind::CaseDefinition)
    {
        "S143"
    } else if element.kind == ElementKind::CaseUsage
        || element.kind.is_subtype_of(ElementKind::CaseUsage)
    {
        "S144"
    } else {
        return None;
    };

    // Locate the (single) subject. No explicit subject → rule does not apply.
    let subject = graph.children_of(&element.id).find(|c| {
        c.kind == ElementKind::SubjectMembership
            || c.kind.is_subtype_of(ElementKind::SubjectMembership)
    })?;
    let subject_pos = subject.spans.iter().map(|s| s.start).min()?;

    // Any owned input feature (direction `in`/`inout`) declared before the subject
    // means the subject is not the first input.
    for child in graph.children_of(&element.id) {
        let is_input = matches!(
            child.get_prop("direction").and_then(Value::as_str),
            Some("in") | Some("inout")
        );
        if !is_input {
            continue;
        }
        if let Some(child_pos) = child.spans.iter().map(|s| s.start).min() {
            if child_pos < subject_pos {
                let offender = child.name.as_deref().unwrap_or("<anonymous>");
                return Some(vec![SemanticError {
                    element_id: element.id.clone(),
                    element_name: element.name.clone(),
                    kind: SemanticErrorKind::Custom {
                        message: format!(
                            "the subject must be the first parameter; input '{offender}' is \
                             declared before the subject (§8.3.21)"
                        ),
                    },
                    rule_id,
                    is_warning: false,
                }]);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_id::ElementId;
    use sysml_span::Span;

    #[test]
    fn concern_one_subject_passes() {
        let mut graph = ModelGraph::new();
        let concern = Element::new(ElementId::new_v4(), ElementKind::ConcernDefinition)
            .with_name("MyConcern");
        let concern_id = graph.add_element(concern);

        let subject = Element::new(ElementId::new_v4(), ElementKind::SubjectMembership)
            .with_owner(concern_id.clone());
        graph.add_element(subject);

        let elem = graph.get_element(&concern_id).unwrap();
        assert!(concern_at_most_one_subject(elem, &graph).is_none());
    }

    #[test]
    fn concern_two_subjects_fails() {
        let mut graph = ModelGraph::new();
        let concern = Element::new(ElementId::new_v4(), ElementKind::ConcernDefinition)
            .with_name("MyConcern");
        let concern_id = graph.add_element(concern);

        let subject1 = Element::new(ElementId::new_v4(), ElementKind::SubjectMembership)
            .with_owner(concern_id.clone());
        graph.add_element(subject1);

        let subject2 = Element::new(ElementId::new_v4(), ElementKind::SubjectMembership)
            .with_owner(concern_id.clone());
        graph.add_element(subject2);

        let elem = graph.get_element(&concern_id).unwrap();
        let result = concern_at_most_one_subject(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S133");
    }

    #[test]
    fn verification_case_no_subjects_passes() {
        let mut graph = ModelGraph::new();
        let vc = Element::new(ElementId::new_v4(), ElementKind::VerificationCaseDefinition)
            .with_name("VC");
        let vc_id = graph.add_element(vc);

        let elem = graph.get_element(&vc_id).unwrap();
        assert!(verification_case_at_most_one_subject(elem, &graph).is_none());
    }

    /// Build a requirement def with the given ordered (kind, name, direction,
    /// start) children and run the subject-first-parameter check.
    fn subject_order_graph(
        children: &[(ElementKind, &str, Option<&str>, usize)],
    ) -> (ModelGraph, ElementId) {
        let mut graph = ModelGraph::new();
        let req = Element::new(ElementId::new_v4(), ElementKind::RequirementDefinition)
            .with_name("R");
        let req_id = graph.add_element(req);
        for (kind, name, dir, start) in children {
            let mut e = Element::new(ElementId::new_v4(), kind.clone())
                .with_name(*name)
                .with_owner(req_id.clone());
            if let Some(d) = dir {
                e.set_prop("direction", Value::String((*d).to_owned()));
            }
            e.spans.push(Span::new("f", *start, *start + 1));
            graph.add_element(e);
        }
        (graph, req_id)
    }

    #[test]
    fn directed_input_before_subject_is_flagged() {
        // `requirement def R { in earlier; subject s; }`
        let (graph, req_id) = subject_order_graph(&[
            (ElementKind::ReferenceUsage, "earlier", Some("in"), 10),
            (ElementKind::SubjectMembership, "s", None, 20),
        ]);
        let elem = graph.get_element(&req_id).unwrap();
        let result = subject_is_first_parameter(elem, &graph);
        assert!(result.is_some(), "directed input before subject must flag");
        assert_eq!(result.unwrap()[0].rule_id, "S141");
    }

    #[test]
    fn subject_before_directed_input_is_clean() {
        // `requirement def R { subject s; in later; }`
        let (graph, req_id) = subject_order_graph(&[
            (ElementKind::SubjectMembership, "s", None, 10),
            (ElementKind::ReferenceUsage, "later", Some("in"), 20),
        ]);
        let elem = graph.get_element(&req_id).unwrap();
        assert!(subject_is_first_parameter(elem, &graph).is_none());
    }

    #[test]
    fn directionless_attribute_before_subject_is_clean() {
        // `requirement def R { attribute earlier; subject s; }` — a plain attribute
        // has no direction, so it is NOT an input; the subject is still input->first().
        let (graph, req_id) = subject_order_graph(&[
            (ElementKind::AttributeUsage, "earlier", None, 10),
            (ElementKind::SubjectMembership, "s", None, 20),
        ]);
        let elem = graph.get_element(&req_id).unwrap();
        assert!(
            subject_is_first_parameter(elem, &graph).is_none(),
            "a directionless attribute is not an input; ordering must not flag"
        );
    }

    #[test]
    fn no_subject_is_not_applicable() {
        let (graph, req_id) = subject_order_graph(&[(
            ElementKind::ReferenceUsage,
            "earlier",
            Some("in"),
            10,
        )]);
        let elem = graph.get_element(&req_id).unwrap();
        assert!(subject_is_first_parameter(elem, &graph).is_none());
    }

    #[test]
    fn use_case_two_subjects_fails() {
        let mut graph = ModelGraph::new();
        let uc = Element::new(ElementId::new_v4(), ElementKind::UseCaseUsage).with_name("UC");
        let uc_id = graph.add_element(uc);

        for _ in 0..2 {
            let subject = Element::new(ElementId::new_v4(), ElementKind::SubjectMembership)
                .with_owner(uc_id.clone());
            graph.add_element(subject);
        }

        let elem = graph.get_element(&uc_id).unwrap();
        let result = use_case_at_most_one_subject(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S140");
    }
}
