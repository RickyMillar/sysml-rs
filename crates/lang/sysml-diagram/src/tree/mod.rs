//! Hierarchical tree payload types for `BrowserView`.
//!
//! Produced by [`to_tree_model`] and embedded in
//! `DiagramPayload::Tree` for the wire format. Consumers (FE, MCP
//! clients, REST callers) work with a recursive `TreeNode` structure
//! directly — no Sprotty SModel or ELK layout involved. The FE renders
//! these in a native React tree (keyboard nav, virtualization,
//! accessible disclosure semantics) rather than Sprotty's tree layout.

use serde::Serialize;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph};

use crate::visual_kind::{self as classify};

/// A complete tree payload.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeModel {
    /// Optional descriptive title.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Generator tag (e.g. `"containment_tree"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Top-level nodes in display order.
    pub roots: Vec<TreeNode>,
}

/// One node in the tree. `children` may nest arbitrarily deep.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeNode {
    /// Stable node id — usually the underlying `ElementId`.
    pub id: String,
    /// Optional element id for navigation (always set for model-backed nodes).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    /// Display label.
    pub label: String,
    /// SysML element kind name (e.g. `"PartUsage"`) — useful for
    /// per-kind icons / styling on the FE.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind_label: Option<String>,
    /// Optional stereotype text (e.g. `"\u{00ab}part\u{00bb}"`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stereotype: Option<String>,
    /// CSS class hints.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub css_classes: Vec<String>,
    /// Nested children — depth-unbounded.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<TreeNode>,
}

// ── Generator ────────────────────────────────────────────────────────────

/// Build a containment-tree payload from the model graph, scoped by `expose`.
///
/// - `expose = Some(id)`: root the containment tree at the exposed element — the
///   whole subtree beneath it. (SPEC-SILENT, 3.12 — the language spec defines
///   BrowserView but not its expose-scoping; Browser is a containment hierarchy,
///   so the expose target is the containment root.)
/// - `expose = None`: top-level elements with no owner, **standard library
///   excluded** (SPEC-SILENT impl choice; mirrors General/Grid/Sequence so a
///   no-expose Browser doesn't dump the whole stdlib). Unlike
///   `is_effectively_top_level` (which lifts package-nested parts), Browser keeps
///   nested parts under their owning package.
///
/// Children: walk `graph.children_of(element.id)` recursively, applying the same
/// membership / import filter. Cycle-safe via a visited set.
pub fn to_tree_model(graph: &ModelGraph, expose: Option<&ElementId>) -> TreeModel {
    let mut visited = std::collections::HashSet::new();

    let roots = if let Some(eid) = expose {
        // Expose-scoped: the exposed element is the single containment root.
        graph
            .get_element(eid)
            .filter(|e| !classify::is_browser_noise_kind(&e.kind))
            .map(|e| {
                visited.insert(e.id.to_string());
                vec![build_node(graph, e, &mut visited)]
            })
            .unwrap_or_default()
    } else {
        let mut top_level: Vec<&Element> = graph
            .elements
            .values()
            .filter(|e| e.owner.is_none() && !classify::is_browser_noise_kind(&e.kind))
            .filter(|e| !graph.is_library_element(&e.id))
            .collect();
        // C13: roots in source declaration order (spans first; name/id tiebreak).
        sysml_core::element_ordering::sort_elements_by_source_order(&mut top_level);
        top_level
            .into_iter()
            .filter_map(|e| {
                if visited.insert(e.id.to_string()) {
                    Some(build_node(graph, e, &mut visited))
                } else {
                    None
                }
            })
            .collect()
    };

    TreeModel {
        title: Some("Containment Tree".to_owned()),
        kind: Some("containment_tree".to_owned()),
        roots,
    }
}

fn build_node(
    graph: &ModelGraph,
    element: &Element,
    visited: &mut std::collections::HashSet<String>,
) -> TreeNode {
    let id = element.id.to_string();
    // C12: shared display-name synthesis — unnamed transitions read
    // `source → target`, unnamed redefinitions read `:>> name`, instead of
    // leaking "unnamed" rows.
    let label = crate::smodel::builders::element_display_name(element, graph);

    // C13: children in source declaration order, not hash-index order.
    let mut ordered: Vec<&Element> = graph
        .children_of(&element.id)
        .filter(|child| !classify::is_browser_noise_kind(&child.kind))
        .collect();
    sysml_core::element_ordering::sort_elements_by_source_order(&mut ordered);
    let children = ordered
        .into_iter()
        .filter_map(|child| {
            if visited.insert(child.id.to_string()) {
                Some(build_node(graph, child, visited))
            } else {
                None
            }
        })
        .collect();

    TreeNode {
        id: id.clone(),
        element_id: Some(id),
        label,
        kind_label: Some(kind_label(&element.kind)),
        stereotype: None, // FE today renders kind_label; stereotype is optional polish
        css_classes: classify::element_css_classes(&element.kind),
        children,
    }
}

fn kind_label(kind: &ElementKind) -> String {
    format!("{:?}", kind)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind, Relationship, RelationshipKind};

    #[test]
    fn empty_graph_has_no_roots() {
        let graph = ModelGraph::new();
        let tree = to_tree_model(&graph, None);
        assert!(tree.roots.is_empty());
    }

    #[test]
    fn top_level_element_renders_as_root() {
        let mut graph = ModelGraph::new();
        graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("Pkg"));

        let tree = to_tree_model(&graph, None);
        assert_eq!(tree.roots.len(), 1);
        assert_eq!(tree.roots[0].label, "Pkg");
        assert_eq!(tree.roots[0].kind_label.as_deref(), Some("Package"));
    }

    #[test]
    fn owned_children_nest_under_parent() {
        let mut graph = ModelGraph::new();
        let pkg_id = graph.add_element(
            Element::new_with_kind(ElementKind::Package).with_name("Pkg"),
        );
        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Engine")
            .with_owner(pkg_id);
        graph.add_element(part);

        let tree = to_tree_model(&graph, None);
        assert_eq!(tree.roots.len(), 1, "package is the only root");
        assert_eq!(tree.roots[0].children.len(), 1);
        assert_eq!(tree.roots[0].children[0].label, "Engine");
    }

    #[test]
    fn relationship_and_annotation_child_elements_are_hidden() {
        // Subclassification (a Relationship) and Documentation (an
        // AnnotatingElement) are not containment members — they rendered as
        // "«unnamed Subclassification»" / "«unnamed Documentation»" noise rows.
        let mut graph = ModelGraph::new();
        let part_id = graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage).with_name("Engine"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage)
                .with_name("power")
                .with_owner(part_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::Subclassification).with_owner(part_id.clone()),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::Documentation).with_owner(part_id.clone()),
        );

        let tree = to_tree_model(&graph, None);
        let engine = &tree.roots[0];
        let labels: Vec<&str> = engine.children.iter().map(|c| c.label.as_str()).collect();
        assert_eq!(labels, vec!["power"], "only the structural child is shown");
    }

    #[test]
    fn two_level_nesting_is_preserved() {
        let mut graph = ModelGraph::new();
        let pkg_id = graph.add_element(
            Element::new_with_kind(ElementKind::Package).with_name("Pkg"),
        );
        let outer_id = graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("Outer")
                .with_owner(pkg_id),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("Inner")
                .with_owner(outer_id),
        );

        let tree = to_tree_model(&graph, None);
        let pkg = &tree.roots[0];
        let outer = &pkg.children[0];
        assert_eq!(outer.label, "Outer");
        assert_eq!(outer.children.len(), 1);
        assert_eq!(outer.children[0].label, "Inner");
    }

    #[test]
    fn relationships_do_not_appear_as_nodes() {
        let mut graph = ModelGraph::new();
        let req_id = graph.add_element(
            Element::new_with_kind(ElementKind::RequirementUsage).with_name("R"),
        );
        let part_id = graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage).with_name("P"),
        );
        graph.add_relationship(Relationship::new(
            RelationshipKind::Satisfy,
            req_id,
            part_id,
        ));

        let tree = to_tree_model(&graph, None);
        // Two elements → two roots; the Satisfy relationship is not a node.
        assert_eq!(tree.roots.len(), 2);
        let labels: Vec<&str> = tree.roots.iter().map(|n| n.label.as_str()).collect();
        assert!(labels.contains(&"R"));
        assert!(labels.contains(&"P"));
    }

    #[test]
    fn css_classes_are_populated() {
        let mut graph = ModelGraph::new();
        graph.add_element(Element::new_with_kind(ElementKind::PartUsage).with_name("P"));
        let tree = to_tree_model(&graph, None);
        assert!(
            !tree.roots[0].css_classes.is_empty(),
            "PartUsage should produce CSS class hints, got {:?}",
            tree.roots[0].css_classes
        );
    }

    #[test]
    fn element_id_is_set_on_every_node() {
        let mut graph = ModelGraph::new();
        let pkg_id = graph.add_element(
            Element::new_with_kind(ElementKind::Package).with_name("Pkg"),
        );
        graph.add_element(
            Element::new_with_kind(ElementKind::PartUsage)
                .with_name("Part")
                .with_owner(pkg_id),
        );
        let tree = to_tree_model(&graph, None);
        let pkg = &tree.roots[0];
        assert!(pkg.element_id.is_some(), "root must carry element_id");
        assert!(
            pkg.children[0].element_id.is_some(),
            "child must carry element_id"
        );
    }

    #[test]
    fn payload_serialises_with_camelcase_keys() {
        let mut graph = ModelGraph::new();
        graph.add_element(Element::new_with_kind(ElementKind::Package).with_name("X"));
        let tree = to_tree_model(&graph, None);
        let value = serde_json::to_value(&tree).unwrap();
        let root = &value.get("roots").unwrap()[0];
        assert!(root.get("elementId").is_some(), "elementId in camelCase");
        assert!(root.get("kindLabel").is_some(), "kindLabel in camelCase");
    }
}
