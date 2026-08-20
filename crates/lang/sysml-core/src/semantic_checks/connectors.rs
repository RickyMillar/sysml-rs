//! Connector and association validation checks.
//!
//! Validates connector end counts and association constraints.

use crate::validation::{SemanticError, SemanticErrorKind};
use crate::{Element, ElementKind, ModelGraph};

/// Helper: count the end features of a connector-like element.
///
/// End features are children with `isEnd = true` or with kind `EndFeatureMembership`.
fn count_end_features(element: &Element, graph: &ModelGraph) -> usize {
    graph
        .children_of(&element.id)
        .filter(|child| {
            child.kind == ElementKind::EndFeatureMembership
                || child
                    .props
                    .get("isEnd")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
        })
        .count()
}

/// Helper: create a connector end count error.
fn end_count_error(
    element: &Element,
    expected: &str,
    actual: usize,
    rule_id: &'static str,
) -> SemanticError {
    SemanticError {
        element_id: element.id.clone(),
        element_name: element.name.clone(),
        kind: SemanticErrorKind::Custom {
            message: format!("expected {} end features, found {}", expected, actual),
        },
        rule_id,
        is_warning: false,
    }
}

/// Rule S100: A binary connector must have exactly two end features.
pub fn binary_connector_two_ends(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::Connector {
        return None;
    }

    let end_count = count_end_features(element, graph);
    // Only check if ends exist (connectors without ends may be abstract)
    if end_count > 0 && end_count != 2 {
        Some(vec![end_count_error(
            element,
            "exactly 2",
            end_count,
            "S100",
        )])
    } else {
        None
    }
}

/// Rule S101: A binding connector must have exactly two end features.
pub fn binding_connector_two_ends(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::BindingConnector {
        return None;
    }

    let end_count = count_end_features(element, graph);
    if end_count > 0 && end_count != 2 {
        Some(vec![end_count_error(
            element,
            "exactly 2",
            end_count,
            "S101",
        )])
    } else {
        None
    }
}

/// Rule S102: A connection must have at least two end features.
pub fn connection_has_ends(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::ConnectionUsage {
        return None;
    }

    let end_count = count_end_features(element, graph);
    if end_count > 0 && end_count < 2 {
        Some(vec![end_count_error(
            element,
            "at least 2",
            end_count,
            "S102",
        )])
    } else {
        None
    }
}

/// Rule S103: An interface must have at least two end features.
pub fn interface_has_ends(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::InterfaceUsage {
        return None;
    }

    let end_count = count_end_features(element, graph);
    if end_count > 0 && end_count < 2 {
        Some(vec![end_count_error(
            element,
            "at least 2",
            end_count,
            "S103",
        )])
    } else {
        None
    }
}

/// Rule S104: An allocation must have at least two end features.
pub fn allocation_has_ends(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::AllocationUsage {
        return None;
    }

    let end_count = count_end_features(element, graph);
    if end_count > 0 && end_count < 2 {
        Some(vec![end_count_error(
            element,
            "at least 2",
            end_count,
            "S104",
        )])
    } else {
        None
    }
}

/// Rule S105: A succession must have exactly two end features.
pub fn succession_two_ends(element: &Element, graph: &ModelGraph) -> Option<Vec<SemanticError>> {
    if element.kind != ElementKind::Succession {
        return None;
    }

    let end_count = count_end_features(element, graph);
    if end_count > 0 && end_count != 2 {
        Some(vec![end_count_error(
            element,
            "exactly 2",
            end_count,
            "S105",
        )])
    } else {
        None
    }
}

/// Rules S106-S108: Connectors (connections, interfaces, flows) must be owned by
/// a Type (definition or usage), not a Package. Per KerML 8.4.4, a connector's
/// `featuringType` must be a Type. A Package is only a Namespace and cannot serve
/// as a structural context for connections.
pub fn connector_owned_by_type(
    element: &Element,
    graph: &ModelGraph,
) -> Option<Vec<SemanticError>> {
    // Only check connector-like usages
    let rule_id = match element.kind {
        ElementKind::ConnectionUsage => "S106",
        ElementKind::InterfaceUsage => "S107",
        ElementKind::FlowUsage => "S108",
        _ => return None,
    };

    let owner_id = element.owner.as_ref()?;
    let owner = graph.elements.get(owner_id)?;

    // Owner must be a definition or usage (i.e., a Type in KerML terms).
    // Packages and library packages are Namespaces, not Types.
    if owner.kind.is_definition() || owner.kind.is_usage() {
        return None;
    }

    let kind_label = match element.kind {
        ElementKind::ConnectionUsage => "connection",
        ElementKind::InterfaceUsage => "interface",
        ElementKind::FlowUsage => "flow",
        _ => "connector",
    };
    // Prefer a readable label: the declared name, else the endpoints, else a
    // generic phrase — never the bare "(unnamed)".
    let label = if let Some(n) = element.name.as_deref() {
        format!("'{n}'")
    } else {
        match (
            element.get_prop("source").and_then(|v| v.as_str()),
            element.get_prop("target").and_then(|v| v.as_str()),
        ) {
            (Some(s), Some(t)) => format!("({s} → {t})"),
            _ => format!("this {kind_label}"),
        }
    };

    Some(vec![SemanticError {
        element_id: element.id.clone(),
        element_name: element.name.clone(),
        kind: SemanticErrorKind::Custom {
            message: format!(
                "{kind_label} {label} is owned by a package; a {kind_label} must be owned by a \
                 part or other type that supplies its featuring context — move it inside the \
                 definition/part that owns the connected features (KerML §8.4.4)"
            ),
        },
        rule_id,
        is_warning: true,
    }])
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_id::ElementId;

    #[test]
    fn connector_with_two_ends_passes() {
        let mut graph = ModelGraph::new();
        let conn = Element::new(ElementId::new_v4(), ElementKind::Connector).with_name("MyConn");
        let conn_id = graph.add_element(conn);

        let end1 = Element::new(ElementId::new_v4(), ElementKind::EndFeatureMembership)
            .with_owner(conn_id.clone());
        graph.add_element(end1);

        let end2 = Element::new(ElementId::new_v4(), ElementKind::EndFeatureMembership)
            .with_owner(conn_id.clone());
        graph.add_element(end2);

        let elem = graph.get_element(&conn_id).unwrap();
        assert!(binary_connector_two_ends(elem, &graph).is_none());
    }

    #[test]
    fn connector_with_three_ends_fails() {
        let mut graph = ModelGraph::new();
        let conn = Element::new(ElementId::new_v4(), ElementKind::Connector).with_name("MyConn");
        let conn_id = graph.add_element(conn);

        for _ in 0..3 {
            let end = Element::new(ElementId::new_v4(), ElementKind::EndFeatureMembership)
                .with_owner(conn_id.clone());
            graph.add_element(end);
        }

        let elem = graph.get_element(&conn_id).unwrap();
        let result = binary_connector_two_ends(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S100");
    }

    #[test]
    fn connector_with_no_ends_passes() {
        let mut graph = ModelGraph::new();
        let conn = Element::new(ElementId::new_v4(), ElementKind::Connector).with_name("MyConn");
        let conn_id = graph.add_element(conn);

        let elem = graph.get_element(&conn_id).unwrap();
        assert!(binary_connector_two_ends(elem, &graph).is_none());
    }

    #[test]
    fn connection_in_part_def_passes() {
        let mut graph = ModelGraph::new();
        let part_def =
            Element::new(ElementId::new_v4(), ElementKind::PartDefinition).with_name("MyPart");
        let part_id = graph.add_element(part_def);

        let conn = Element::new(ElementId::new_v4(), ElementKind::ConnectionUsage)
            .with_name("c1")
            .with_owner(part_id);
        let conn_id = graph.add_element(conn);

        let elem = graph.get_element(&conn_id).unwrap();
        assert!(connector_owned_by_type(elem, &graph).is_none());
    }

    #[test]
    fn connection_in_package_warns() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new(ElementId::new_v4(), ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let conn = Element::new(ElementId::new_v4(), ElementKind::ConnectionUsage)
            .with_name("c1")
            .with_owner(pkg_id);
        let conn_id = graph.add_element(conn);

        let elem = graph.get_element(&conn_id).unwrap();
        let result = connector_owned_by_type(elem, &graph);
        assert!(result.is_some());
        let errs = result.unwrap();
        assert_eq!(errs[0].rule_id, "S106");
        assert!(errs[0].is_warning);
    }

    #[test]
    fn flow_in_package_warns() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new(ElementId::new_v4(), ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let flow = Element::new(ElementId::new_v4(), ElementKind::FlowUsage)
            .with_name("f1")
            .with_owner(pkg_id);
        let flow_id = graph.add_element(flow);

        let elem = graph.get_element(&flow_id).unwrap();
        let result = connector_owned_by_type(elem, &graph);
        assert!(result.is_some());
        assert_eq!(result.unwrap()[0].rule_id, "S108");
    }

    #[test]
    fn interface_in_part_usage_passes() {
        let mut graph = ModelGraph::new();
        let part = Element::new(ElementId::new_v4(), ElementKind::PartUsage).with_name("myPart");
        let part_id = graph.add_element(part);

        let iface = Element::new(ElementId::new_v4(), ElementKind::InterfaceUsage)
            .with_name("i1")
            .with_owner(part_id);
        let iface_id = graph.add_element(iface);

        let elem = graph.get_element(&iface_id).unwrap();
        assert!(connector_owned_by_type(elem, &graph).is_none());
    }

    #[test]
    fn succession_with_two_ends_passes() {
        let mut graph = ModelGraph::new();
        let succ = Element::new(ElementId::new_v4(), ElementKind::Succession).with_name("MySucc");
        let succ_id = graph.add_element(succ);

        let end1 = Element::new(ElementId::new_v4(), ElementKind::EndFeatureMembership)
            .with_owner(succ_id.clone());
        graph.add_element(end1);

        let end2 = Element::new(ElementId::new_v4(), ElementKind::EndFeatureMembership)
            .with_owner(succ_id.clone());
        graph.add_element(end2);

        let elem = graph.get_element(&succ_id).unwrap();
        assert!(succession_two_ends(elem, &graph).is_none());
    }
}
