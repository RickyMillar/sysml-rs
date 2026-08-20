//! Browser view IR generator — **legacy SGraph emitter**.
//!
//! Produces a pure ownership-hierarchy tree with no edges.
//! Each element becomes a `DiagramNode` with `HeaderStyle::Inline`
//! and recursive children for expanded nodes.
//!
//! ## Status
//!
//! The canonical wire path for `view=browser` now goes through
//! [`crate::tree::to_tree_model`] → `DiagramPayload::Tree(TreeModel)`,
//! served by REST and MCP. Note that the typed path uses *strict*
//! ownership for roots (`element.owner.is_none()`), whereas this
//! Sprotty generator uses `is_effectively_top_level` (lifts through
//! packages) — a deliberate semantic difference between containment
//! tree and diagram canvas. This generator is retained for LSP push
//! notifications and the CLI `export smodel --view browser` path,
//! both of which still expect raw `SGraph`. Delete once those
//! consumers migrate to typed payloads.

use std::collections::HashSet;

use tracing::instrument;

use crate::ir::generator::{GeneratorContext, ViewGenerator};
use crate::ir::types::{DiagramIR, DiagramNode, DiagramChild, DiagramButton, HeaderStyle, NodeLayout, NodeTag};
use crate::smodel::ViewType;
use crate::visual_kind::{self as classify, VisualKind};

/// Browser view generator — pure ownership tree, no edges.
pub struct BrowserViewGenerator;

impl ViewGenerator for BrowserViewGenerator {
    fn view_type(&self) -> ViewType {
        ViewType::Browser
    }

    fn elk_algorithm(&self) -> &str {
        "layered"
    }

    fn elk_direction(&self) -> Option<&str> {
        Some("DOWN")
    }

    #[instrument(skip_all)]
    fn generate(&self, ctx: &GeneratorContext) -> DiagramIR {
        tracing::info!("BrowserView IR generate");

        let mut ir = DiagramIR::new(ViewType::Browser);

        // No ELK layout needed for tree browser
        // (DiagramIR::new already sets layout_algorithm = None)

        // Top-level elements (no owner, skip memberships), in source order (C13)
        let mut top_level: Vec<_> = ctx
            .graph
            .elements
            .values()
            .filter(|e| ctx.is_canvas_root(e) && !classify::is_browser_noise_kind(&e.kind))
            .collect();
        sysml_core::element_ordering::sort_elements_by_source_order(&mut top_level);

        let mut visited = HashSet::new();
        for element in &top_level {
            if visited.insert(element.id.to_string()) {
                let node = generate_browser_node(ctx, element, &mut visited);
                ir.nodes.push(node);
            }
        }

        ir
    }
}

/// Build a browser DiagramNode for a single element, recursing into children.
fn generate_browser_node(
    ctx: &GeneratorContext,
    element: &sysml_core::Element,
    visited: &mut HashSet<String>,
) -> DiagramNode {
    let id = element.id.to_string();
    let kind = &element.kind;
    // C12: shared display-name synthesis (transitions → `source → target`,
    // redefinitions → `:>> name`) instead of "unnamed".
    let name = crate::smodel::builders::element_display_name(element, ctx.graph);

    let visual_kind = VisualKind::from_element_kind(kind);

    // Recurse into children (skip memberships), in source order (C13)
    let owned: Vec<_> = super::container::ordered_children(ctx.graph, &element.id)
        .into_iter()
        .filter(|child| !classify::is_browser_noise_kind(&child.kind))
        .collect();

    let child_count = owned.len();
    let is_expanded = ctx.expanded_ids.contains(&id);
    let has_children = child_count > 0;

    // Build label text: Browser uses HeaderStyle::Inline with composed text
    let kind_label = kind.display_name();
    let label_text = if !is_expanded && has_children {
        // Collapsed: show child count
        format!("\u{00ab}{}\u{00bb} {} ({})", kind_label, name, child_count)
    } else {
        format!("\u{00ab}{}\u{00bb} {}", kind_label, name)
    };

    // Build children
    let mut children = Vec::new();
    if is_expanded || !has_children {
        // Expanded or leaf: recurse into children
        for child in owned {
            if visited.insert(child.id.to_string()) {
                let child_node = generate_browser_node(ctx, child, visited);
                children.push(DiagramChild::Node(child_node));
            }
        }
    }

    // Build buttons
    let mut buttons = Vec::new();
    let expanded = if has_children {
        buttons.push(DiagramButton::expand());
        Some(is_expanded)
    } else {
        None
    };

    let mut node = DiagramNode::new(id, visual_kind, label_text)
        .with_element_kind(kind.clone());
    node.header_style = HeaderStyle::Inline;
    node.children = children;
    node.buttons = buttons;
    node.expanded = expanded;
    super::container::apply_source_metadata(&mut node, element, ctx.graph);
    node.tags.push(NodeTag::BrowserNode);
    node.layout = NodeLayout::VBox;

    node
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use sysml_core::{Element, ElementKind, ModelGraph};

    use crate::ir::generator::GeneratorContext;
    use crate::ir::render::render;

    fn make_ctx<'a>(graph: &'a ModelGraph, expanded_ids: &'a HashSet<String>) -> GeneratorContext<'a> {
        GeneratorContext::new(graph, expanded_ids)
    }

    #[test]
    fn browser_ir_three_level_nesting() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("MyPackage");
        let pkg_id = graph.add_element(pkg);

        let part = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("MyPart")
            .with_owner(pkg_id.clone());
        let part_id = graph.add_element(part);

        let port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("MyPort")
            .with_owner(part_id.clone());
        graph.add_element(port);

        let mut expanded = HashSet::new();
        expanded.insert(pkg_id.to_string());
        expanded.insert(part_id.to_string());

        let gen = BrowserViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        // Render to SGraph and check JSON
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();

        assert!(json.contains("MyPackage"));
        assert!(json.contains("MyPart"));
        assert!(json.contains("MyPort"));
        assert!(!json.contains("edge:"));
        assert!(json.contains("browser-node"));
    }

    #[test]
    fn browser_ir_empty_graph() {
        let graph = ModelGraph::new();
        let expanded = HashSet::new();
        let gen = BrowserViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        assert_eq!(ir.view_type, ViewType::Browser);
        assert!(ir.nodes.is_empty());
        assert!(ir.edges.is_empty());

        let sgraph = render(&ir);
        assert_eq!(sgraph.type_, "graph");
        assert!(sgraph.children.is_empty());
    }

    #[test]
    fn browser_ir_collapsed_shows_child_count() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        let pkg_id = graph.add_element(pkg);

        let part_a = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("A")
            .with_owner(pkg_id.clone());
        graph.add_element(part_a);

        let part_b = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("B")
            .with_owner(pkg_id);
        graph.add_element(part_b);

        // Collapsed (empty expanded_ids)
        let expanded = HashSet::new();
        let gen = BrowserViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();

        // Should show child count in label
        assert!(
            json.contains("Pkg (2)"),
            "collapsed label should show child count, got: {}",
            json
        );
        // Expand button present
        assert!(json.contains("button:expand"));
        assert!(json.contains(r#""expanded":false"#));
    }

    #[test]
    fn browser_ir_expand_button_on_parent() {
        let mut graph = ModelGraph::new();

        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Root");
        let pkg_id = graph.add_element(pkg);

        let child = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Child")
            .with_owner(pkg_id.clone());
        graph.add_element(child);

        let leaf = Element::new_with_kind(ElementKind::AttributeUsage).with_name("Leaf");
        graph.add_element(leaf);

        let expanded = HashSet::new();
        let gen = BrowserViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();

        // Parent node should have expand button
        assert!(json.contains("button:expand"));
    }

    #[test]
    fn browser_ir_node_has_inline_header() {
        let mut graph = ModelGraph::new();
        let pkg = Element::new_with_kind(ElementKind::Package).with_name("Pkg");
        graph.add_element(pkg);

        let expanded = HashSet::new();
        let gen = BrowserViewGenerator;
        let ctx = make_ctx(&graph, &expanded);
        let ir = gen.generate(&ctx);

        assert_eq!(ir.nodes.len(), 1);
        assert_eq!(ir.nodes[0].header_style, HeaderStyle::Inline);

        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        // Should have label:name (Inline style), NOT comp:header
        assert!(json.contains("label:name"), "should have inline label");
        assert!(
            !json.contains("comp:header"),
            "should NOT have header compartment"
        );
    }

    #[test]
    fn browser_ir_view_type() {
        let gen = BrowserViewGenerator;
        assert_eq!(gen.view_type(), ViewType::Browser);
        assert_eq!(gen.elk_algorithm(), "layered");
        assert_eq!(gen.elk_direction(), Some("DOWN"));
    }
}
