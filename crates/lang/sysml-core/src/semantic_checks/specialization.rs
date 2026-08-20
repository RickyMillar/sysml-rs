//! Specialization boundary validation checks.
//!
//! Validates that types don't cross specialization boundaries
//! (e.g., DataType cannot specialize Class).

use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph};

/// Helper: find the general types (supertypes via Specialization elements).
fn find_general_kinds(
    element: &Element,
    graph: &ModelGraph,
) -> Vec<(ElementKind, sysml_id::ElementId)> {
    graph
        .specific_to_specializations
        .get(&element.id)
        .into_iter()
        .flat_map(|spec_ids| spec_ids.iter())
        .filter_map(|spec_id| {
            let spec_elem = graph.elements.get(spec_id)?;
            let general_id = spec_elem.props.get("general")?.as_ref()?;
            let general_elem = graph.elements.get(general_id)?;
            Some((general_elem.kind.clone(), general_id.clone()))
        })
        .collect()
}

/// Helper: create a specialization error.
fn spec_error(element: &Element, super_kind: ElementKind, rule_id: &'static str) -> SemanticError {
    SemanticError {
        element_id: element.id.clone(),
        element_name: element.name.clone(),
        kind: SemanticErrorKind::SpecializationViolation {
            sub: element.kind.clone(),
            super_: super_kind,
        },
        rule_id,
        is_warning: false,
    }
}

/// Rule S030: DataType cannot specialize Class or Association.
pub fn datatype_not_specialize_class(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::DataType && !element.kind.is_subtype_of(ElementKind::DataType) {
        return None;
    }

    let generals = find_general_kinds(element, graph);
    let mut errors = Vec::new();

    for (kind, _id) in generals {
        if kind == ElementKind::Class
            || kind.is_subtype_of(ElementKind::Class)
            || kind == ElementKind::Association
            || kind.is_subtype_of(ElementKind::Association)
        {
            errors.push(spec_error(element, kind, "S030"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S031: Class cannot specialize DataType or Association.
pub fn class_not_specialize_datatype(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::Class && !element.kind.is_subtype_of(ElementKind::Class) {
        return None;
    }

    let generals = find_general_kinds(element, graph);
    let mut errors = Vec::new();

    for (kind, _id) in generals {
        if kind == ElementKind::DataType
            || kind.is_subtype_of(ElementKind::DataType)
            || kind == ElementKind::Association
            || kind.is_subtype_of(ElementKind::Association)
        {
            errors.push(spec_error(element, kind, "S031"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S032: Structure cannot specialize Behavior.
pub fn structure_not_specialize_behavior(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::Structure && !element.kind.is_subtype_of(ElementKind::Structure)
    {
        return None;
    }

    let generals = find_general_kinds(element, graph);
    let mut errors = Vec::new();

    for (kind, _id) in generals {
        if kind == ElementKind::Behavior || kind.is_subtype_of(ElementKind::Behavior) {
            errors.push(spec_error(element, kind, "S032"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S033: Behavior cannot specialize Structure.
pub fn behavior_not_specialize_structure(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::Behavior && !element.kind.is_subtype_of(ElementKind::Behavior) {
        return None;
    }

    let generals = find_general_kinds(element, graph);
    let mut errors = Vec::new();

    for (kind, _id) in generals {
        if kind == ElementKind::Structure || kind.is_subtype_of(ElementKind::Structure) {
            errors.push(spec_error(element, kind, "S033"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S034: AttributeDefinition cannot specialize ItemDefinition.
pub fn attribute_def_not_specialize_item_def(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::AttributeDefinition {
        return None;
    }

    let generals = find_general_kinds(element, graph);
    let mut errors = Vec::new();

    for (kind, _id) in generals {
        if kind == ElementKind::ItemDefinition || kind.is_subtype_of(ElementKind::ItemDefinition) {
            errors.push(spec_error(element, kind, "S034"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

/// Rule S035: ItemDefinition cannot specialize AttributeDefinition.
pub fn item_def_not_specialize_attribute_def(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ItemDefinition {
        return None;
    }

    let generals = find_general_kinds(element, graph);
    let mut errors = Vec::new();

    for (kind, _id) in generals {
        if kind == ElementKind::AttributeDefinition
            || kind.is_subtype_of(ElementKind::AttributeDefinition)
        {
            errors.push(spec_error(element, kind, "S035"));
        }
    }

    if errors.is_empty() {
        None
    } else {
        Some(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::Value;
    use sysml_id::ElementId;

    fn setup_specialization(
        graph: &mut ModelGraph,
        specific_kind: ElementKind,
        general_kind: ElementKind,
    ) -> ElementId {
        let specific = Element::new(ElementId::new_v4(), specific_kind).with_name("Sub");
        let specific_id = graph.add_element(specific);

        let general = Element::new(ElementId::new_v4(), general_kind).with_name("Super");
        let general_id = graph.add_element(general);

        let spec = Element::new(ElementId::new_v4(), ElementKind::Specialization)
            .with_prop("specific", Value::Ref(specific_id.clone()))
            .with_prop("general", Value::Ref(general_id));
        graph.add_element(spec);

        specific_id
    }

    #[test]
    fn datatype_specializing_datatype_passes() {
        let mut graph = ModelGraph::new();
        let id = setup_specialization(&mut graph, ElementKind::DataType, ElementKind::DataType);

        let elem = graph.get_element(&id).unwrap();
        assert!(datatype_not_specialize_class(elem, &graph).is_none());
    }

    #[test]
    fn datatype_specializing_class_fails() {
        let mut graph = ModelGraph::new();
        let id = setup_specialization(&mut graph, ElementKind::DataType, ElementKind::Class);

        let elem = graph.get_element(&id).unwrap();
        let result = datatype_not_specialize_class(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S030");
    }

    #[test]
    fn class_specializing_datatype_fails() {
        let mut graph = ModelGraph::new();
        let id = setup_specialization(&mut graph, ElementKind::Class, ElementKind::DataType);

        let elem = graph.get_element(&id).unwrap();
        let result = class_not_specialize_datatype(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S031");
    }

    #[test]
    fn behavior_specializing_structure_fails() {
        let mut graph = ModelGraph::new();
        let id = setup_specialization(&mut graph, ElementKind::Behavior, ElementKind::Structure);

        let elem = graph.get_element(&id).unwrap();
        let result = behavior_not_specialize_structure(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S033");
    }

    #[test]
    fn structure_specializing_behavior_fails() {
        let mut graph = ModelGraph::new();
        let id = setup_specialization(&mut graph, ElementKind::Structure, ElementKind::Behavior);

        let elem = graph.get_element(&id).unwrap();
        let result = structure_not_specialize_behavior(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S032");
    }
}
