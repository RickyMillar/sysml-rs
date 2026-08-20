//! Usage typing validation checks.
//!
//! Validates that usages are typed by appropriate definitions.

use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph};

/// Helper: find the types of a feature/usage via FeatureTyping relationships.
///
/// Returns the ElementKind of each type that this element is typed by.
fn find_typing_kinds(element: &Element, graph: &ModelGraph) -> Vec<ElementKind> {
    // Look for FeatureTyping elements that reference this element
    if let Some(typing_ids) = graph.typed_feature_to_typings.get(&element.id) {
        typing_ids
            .iter()
            .filter_map(|typing_id| {
                let typing_elem = graph.elements.get(typing_id)?;
                // Get the "type" property which points to the type
                let type_id = typing_elem.props.get("type")?.as_ref()?;
                let type_elem = graph.elements.get(type_id)?;
                Some(type_elem.kind.clone())
            })
            .collect()
    } else {
        Vec::new()
    }
}

/// Helper: check if element has FeatureTyping relationships at all.
fn has_any_typing(element: &Element, graph: &ModelGraph) -> bool {
    graph
        .typed_feature_to_typings
        .get(&element.id)
        .is_some_and(|ids| !ids.is_empty())
}

/// Helper: create a typing error for a given element.
fn typing_error(
    element: &Element,
    expected: &'static str,
    got: ElementKind,
    rule_id: &'static str,
) -> SemanticError {
    SemanticError {
        element_id: element.id.clone(),
        element_name: element.name.clone(),
        kind: SemanticErrorKind::InvalidTyping { expected, got },
        rule_id,
        is_warning: false,
    }
}

/// Check that all usages are typed by classifiers (definitions).
///
/// Rule S010: A usage must be typed by definitions.
pub fn usage_typed_by_definitions(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if !has_any_typing(element, graph) {
        return None; // No typing = no violation (types may be implicit)
    }

    let type_kinds = find_typing_kinds(element, graph);
    // If typing relationships exist but all types are unresolved (e.g. library not loaded),
    // skip the check rather than producing false positives.
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        if !kind.is_classifier() && kind != ElementKind::Classifier && !kind.is_definition() {
            errors.push(typing_error(element, "definitions", kind, "S010"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Check that AttributeUsage is typed by DataType/AttributeDefinition.
///
/// Rule S011: An attribute must be typed by attribute definitions.
pub fn attribute_typed_by_datatypes(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::AttributeUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::DataType
            || kind == ElementKind::AttributeDefinition
            || kind.is_subtype_of(ElementKind::DataType)
            || kind.is_subtype_of(ElementKind::AttributeDefinition);
        if !is_valid {
            errors.push(typing_error(element, "attribute definitions", kind, "S011"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S012: EnumerationUsage must be typed by one EnumerationDefinition.
pub fn enumeration_typed_by_one_enum_def(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::EnumerationUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in &type_kinds {
        if *kind != ElementKind::EnumerationDefinition {
            errors.push(typing_error(
                element,
                "enumeration definitions",
                kind.clone(),
                "S012",
            ));
        }
    }
    if type_kinds.len() > 1 {
        errors.push(SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::Custom {
                message: "an enumeration attribute cannot have more than one type".to_owned(),
            },
            rule_id: "S012",
            is_warning: false,
        });
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S013: OccurrenceUsage must be typed by occurrence definitions.
pub fn occurrence_typed_by_occurrence_defs(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::OccurrenceUsage {
        return None;
    }
    check_typed_by_class_subtypes(element, graph, "occurrence definitions", "S013")
}

/// Rule S014: ItemUsage must be typed by item definitions.
pub fn item_typed_by_item_defs(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ItemUsage {
        return None;
    }
    check_typed_by_class_subtypes(element, graph, "item definitions", "S014")
}

/// Rule S015: PartUsage must be typed by at least one PartDefinition.
pub fn part_typed_by_part_defs(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::PartUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    // Check that all types are item definitions (PartDefinition is subtype of ItemDefinition)
    for kind in &type_kinds {
        let is_valid = kind.is_subtype_of(ElementKind::ItemDefinition)
            || *kind == ElementKind::ItemDefinition
            || *kind == ElementKind::PartDefinition;
        if !is_valid {
            errors.push(typing_error(
                element,
                "item definitions",
                kind.clone(),
                "S015",
            ));
        }
    }

    // PartUsage must have at least one PartDefinition specifically
    let has_part_def = type_kinds
        .iter()
        .any(|k| *k == ElementKind::PartDefinition || k.is_subtype_of(ElementKind::PartDefinition));
    if !has_part_def && errors.is_empty() {
        errors.push(SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::Custom {
                message: "a part must be typed by at least one part definition".to_owned(),
            },
            rule_id: "S015",
            is_warning: false,
        });
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S016: PortUsage must be typed by PortDefinitions.
pub fn port_typed_by_port_defs(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::PortUsage {
        return None;
    }
    // Skip port-typing check for `in`/`out` parameters inside behavioral definitions.
    // In calc/constraint/requirement/action contexts, `in` means input parameter,
    // not port direction — these params are correctly typed by data types, not ports.
    if let Some(owner_id) = &element.owner {
        if let Some(owner) = graph.get_element(owner_id) {
            if matches!(
                owner.kind,
                ElementKind::CalculationDefinition
                    | ElementKind::ConstraintDefinition
                    | ElementKind::RequirementDefinition
                    | ElementKind::ActionDefinition
                    | ElementKind::CalculationUsage
                    | ElementKind::ConstraintUsage
                    | ElementKind::RequirementUsage
                    | ElementKind::ActionUsage
                    | ElementKind::FlowDefinition
                    | ElementKind::CaseDefinition
                    | ElementKind::AnalysisCaseDefinition
                    | ElementKind::VerificationCaseDefinition
                    | ElementKind::UseCaseDefinition
            ) {
                return None;
            }
        }
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid =
            kind == ElementKind::PortDefinition || kind.is_subtype_of(ElementKind::PortDefinition);
        if !is_valid {
            errors.push(typing_error(element, "port definitions", kind, "S016"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S017: ActionUsage must be typed by Behavior/ActionDefinition.
pub fn action_typed_by_behavior(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ActionUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::Behavior
            || kind == ElementKind::ActionDefinition
            || kind.is_subtype_of(ElementKind::Behavior)
            || kind.is_subtype_of(ElementKind::ActionDefinition);
        if !is_valid {
            errors.push(typing_error(element, "action definitions", kind, "S017"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S018: ConnectionUsage must be typed by Associations.
pub fn connection_typed_by_association(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ConnectionUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::ConnectionDefinition
            || kind == ElementKind::Association
            || kind.is_subtype_of(ElementKind::Association)
            || kind.is_subtype_of(ElementKind::ConnectionDefinition);
        if !is_valid {
            errors.push(typing_error(
                element,
                "connection definitions",
                kind,
                "S018",
            ));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S019: StateUsage must be typed by state definitions.
pub fn state_typed_by_state_defs(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::StateUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::Behavior
            || kind == ElementKind::StateDefinition
            || kind.is_subtype_of(ElementKind::Behavior)
            || kind.is_subtype_of(ElementKind::StateDefinition);
        if !is_valid {
            errors.push(typing_error(element, "state definitions", kind, "S019"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S020: ConstraintUsage must be typed by one constraint definition.
pub fn constraint_typed_by_predicate(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ConstraintUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::ConstraintDefinition
            || kind == ElementKind::Predicate
            || kind.is_subtype_of(ElementKind::Predicate)
            || kind.is_subtype_of(ElementKind::ConstraintDefinition);
        if !is_valid {
            errors.push(typing_error(
                element,
                "constraint definitions",
                kind,
                "S020",
            ));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S021: RequirementUsage must be typed by one requirement definition.
pub fn requirement_typed_by_one_req_def(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::RequirementUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::RequirementDefinition
            || kind.is_subtype_of(ElementKind::RequirementDefinition)
            || kind.is_subtype_of(ElementKind::ConstraintDefinition);
        if !is_valid {
            errors.push(typing_error(
                element,
                "requirement definitions",
                kind,
                "S021",
            ));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S022: InterfaceUsage must be typed by interface definitions.
pub fn interface_typed_by_interface_defs(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::InterfaceUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::InterfaceDefinition
            || kind.is_subtype_of(ElementKind::InterfaceDefinition);
        if !is_valid {
            errors.push(typing_error(element, "interface definitions", kind, "S022"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S023: FlowUsage must be typed by flow connection definitions.
pub fn flow_typed_by_interaction(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::FlowUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::FlowDefinition
            || kind == ElementKind::Interaction
            || kind.is_subtype_of(ElementKind::Interaction)
            || kind.is_subtype_of(ElementKind::FlowDefinition);
        if !is_valid {
            errors.push(typing_error(
                element,
                "flow connection definitions",
                kind,
                "S023",
            ));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S024: CalculationUsage must be typed by one calculation definition.
pub fn calculation_typed_by_one_calc_def(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::CalculationUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::CalculationDefinition
            || kind.is_subtype_of(ElementKind::CalculationDefinition)
            || kind == ElementKind::Function
            || kind.is_subtype_of(ElementKind::Function);
        if !is_valid {
            errors.push(typing_error(
                element,
                "calculation definitions",
                kind,
                "S024",
            ));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S025: CaseUsage must be typed by one case definition.
pub fn case_typed_by_one_case_def(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::CaseUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid =
            kind == ElementKind::CaseDefinition || kind.is_subtype_of(ElementKind::CaseDefinition);
        if !is_valid {
            errors.push(typing_error(element, "case definitions", kind, "S025"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S026: AllocationUsage must be typed by allocation definitions.
pub fn allocation_typed_by_allocation_defs(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::AllocationUsage {
        return None;
    }
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::AllocationDefinition
            || kind.is_subtype_of(ElementKind::AllocationDefinition);
        if !is_valid {
            errors.push(typing_error(
                element,
                "allocation definitions",
                kind,
                "S026",
            ));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Helper: check that element is typed by Class subtypes (occurrence definitions).
fn check_typed_by_class_subtypes(
    element: &Element,
    graph: &ModelGraph,
    expected: &'static str,
    rule_id: &'static str,
) -> Option<Vec<SemanticError>> {
    if !has_any_typing(element, graph) {
        return None;
    }

    let type_kinds = find_typing_kinds(element, graph);
    if type_kinds.is_empty() {
        return None;
    }
    let mut errors = Vec::new();

    for kind in type_kinds {
        let is_valid = kind == ElementKind::Class
            || kind.is_subtype_of(ElementKind::Class)
            || kind.is_definition();
        if !is_valid {
            errors.push(typing_error(element, expected, kind, rule_id));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S090/S091: AttributeUsage and AttributeDefinition must not be composite.
///
/// Per spec, attributes are always value types (non-composite by nature).
/// Setting isComposite=true on an attribute is a modeling error.
pub fn attribute_must_not_be_composite(
    element: &Element,
    _graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if !matches!(
        element.kind,
        ElementKind::AttributeUsage | ElementKind::AttributeDefinition
    ) {
        return None;
    }

    let is_composite = element
        .get_prop("isComposite")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    if is_composite {
        let rule_id = if element.kind == ElementKind::AttributeUsage {
            "S090"
        } else {
            "S091"
        };
        Some(vec![SemanticError {
            element_id: element.id.clone(),
            element_name: element.name.clone(),
            kind: SemanticErrorKind::Custom {
                message: "attributes are value types and must not be composite".to_owned(),
            },
            rule_id,
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

    fn setup_typed_element(
        graph: &mut ModelGraph,
        usage_kind: ElementKind,
        type_kind: ElementKind,
    ) -> ElementId {
        let usage = Element::new(ElementId::new_v4(), usage_kind).with_name("test");
        let usage_id = graph.add_element(usage);

        let type_elem = Element::new(ElementId::new_v4(), type_kind).with_name("TestType");
        let type_id = graph.add_element(type_elem);

        // Create FeatureTyping element
        let typing = Element::new(ElementId::new_v4(), ElementKind::FeatureTyping)
            .with_prop("typedFeature", crate::meta::Value::Ref(usage_id.clone()))
            .with_prop("type", crate::meta::Value::Ref(type_id));
        graph.add_element(typing);

        usage_id
    }

    #[test]
    fn part_typed_by_part_def_passes() {
        let mut graph = ModelGraph::new();
        let usage_id = setup_typed_element(
            &mut graph,
            ElementKind::PartUsage,
            ElementKind::PartDefinition,
        );

        let elem = graph.get_element(&usage_id).unwrap();
        let result = part_typed_by_part_defs(elem, &graph);
        assert!(result.is_none());
    }

    #[test]
    fn part_typed_by_package_fails() {
        let mut graph = ModelGraph::new();
        let usage_id =
            setup_typed_element(&mut graph, ElementKind::PartUsage, ElementKind::Package);

        let elem = graph.get_element(&usage_id).unwrap();
        let result = part_typed_by_part_defs(elem, &graph);
        assert!(result.is_some());
    }

    #[test]
    fn action_typed_by_action_def_passes() {
        let mut graph = ModelGraph::new();
        let usage_id = setup_typed_element(
            &mut graph,
            ElementKind::ActionUsage,
            ElementKind::ActionDefinition,
        );

        let elem = graph.get_element(&usage_id).unwrap();
        let result = action_typed_by_behavior(elem, &graph);
        assert!(result.is_none());
    }

    #[test]
    fn port_typed_by_part_def_fails() {
        let mut graph = ModelGraph::new();
        let usage_id = setup_typed_element(
            &mut graph,
            ElementKind::PortUsage,
            ElementKind::PartDefinition,
        );

        let elem = graph.get_element(&usage_id).unwrap();
        let result = port_typed_by_port_defs(elem, &graph);
        assert!(result.is_some());
    }

    #[test]
    fn untyped_usage_passes() {
        let mut graph = ModelGraph::new();
        let usage = Element::new(ElementId::new_v4(), ElementKind::PartUsage).with_name("test");
        let usage_id = graph.add_element(usage);

        let elem = graph.get_element(&usage_id).unwrap();
        assert!(usage_typed_by_definitions(elem, &graph).is_none());
        assert!(part_typed_by_part_defs(elem, &graph).is_none());
    }

    #[test]
    fn unresolved_typing_skipped() {
        // Create a graph where PartUsage has a FeatureTyping but the type target is missing
        let mut graph = ModelGraph::new();
        let usage = Element::new(ElementId::new_v4(), ElementKind::PartUsage).with_name("test");
        let usage_id = graph.add_element(usage);

        // Create FeatureTyping pointing to a nonexistent type
        let fake_type_id = ElementId::new_v4();
        let typing = Element::new(ElementId::new_v4(), ElementKind::FeatureTyping)
            .with_prop("typedFeature", crate::meta::Value::Ref(usage_id.clone()))
            .with_prop("type", crate::meta::Value::Ref(fake_type_id));
        graph.add_element(typing);

        let elem = graph.get_element(&usage_id).unwrap();
        // Should return None (skip) rather than producing false positive
        assert!(part_typed_by_part_defs(elem, &graph).is_none());
        assert!(usage_typed_by_definitions(elem, &graph).is_none());
    }
}
