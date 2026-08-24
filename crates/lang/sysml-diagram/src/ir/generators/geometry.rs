//! GeometryView IR generator — **legacy legacy graph emitter**.
//!
//! Like General but with explicit positions/sizes from element properties.
//! Elements with `x`/`y` properties get fixed positions; elements with
//! `width`/`height` get fixed sizes.  When ALL top-level elements have
//! positions, the layout algorithm is set to "fixed"; otherwise, layered
//! layout is used and ELK will position elements that lack coordinates.
//!
//! Supports expand/collapse, compartment text, port rendering, and edges.
//!
//! ## Status
//!
//! The canonical wire path for `view=geometry` now goes through
//! [`crate::gmodel::to_geometry_model`] → `tagged payload::Geometry(GeometryModel)`,
//! served by REST and MCP. This retired graph-renderer generator is retained for LSP
//! push notifications and the retired CLI graph export path,
//! both of which still expect raw `legacy graph`. Delete once those consumers
//! migrate to typed payloads.

use std::collections::HashSet;

use sysml_core::{Element, ElementKind, ModelGraph, RelationshipKind};
use tracing::instrument;

use crate::ir::generator::{GeneratorContext, ViewGenerator};
use crate::ir::types::{DiagramChild, DiagramIR, DiagramEdge, DiagramNode, DiagramPort};
use crate::view_text;
use crate::ViewType;
use crate::visual_kind::{self as classify, VisualKind};

/// Geometry view generator — positions and sizes from element properties.
pub struct GeometryViewGenerator;

impl ViewGenerator for GeometryViewGenerator {
    fn view_type(&self) -> ViewType {
        ViewType::Geometry
    }

    fn elk_algorithm(&self) -> &str {
        "layered"
    }

    fn elk_direction(&self) -> Option<&str> {
        Some("DOWN")
    }

    #[instrument(skip_all)]
    fn generate(&self, ctx: &GeneratorContext) -> DiagramIR {
        tracing::info!("GeometryView IR generate");

        let graph = ctx.graph;

        // Collect top-level elements (no owner, not membership, not port-kind).
        // Port-kind elements are rendered as SPort children to avoid duplicate IDs.
        let top_level: Vec<_> = graph
            .elements
            .values()
            .filter(|e| !classify::is_membership_kind(&e.kind))
            .filter(|e| !classify::is_import_kind(&e.kind))
            .filter(|e| !classify::is_port_kind(&e.kind))
            .filter(|e| ctx.is_canvas_root(e))
            .collect();

        // Layout algorithm is no longer carried on the IR — the retired graph-renderer adapter
        // derives the fixed layout for Geometry views from `view_type`.
        let mut ir = DiagramIR::new(ViewType::Geometry);

        // Generate nodes for top-level elements.
        for element in &top_level {
            let node = generate_geometry_node(graph, element, ctx.expanded_ids);
            ir.nodes.push(node);
        }

        // Collect IDs of all generated top-level nodes for edge filtering.
        let top_level_ids: HashSet<String> =
            top_level.iter().map(|e| e.id.to_string()).collect();

        // Generate edges for non-ownership relationships, but only when both
        // source and target exist as top-level nodes.  Edges referencing child
        // elements rendered as text labels would have dangling endpoints.
        for rel in graph.relationships.values() {
            if rel.kind == RelationshipKind::Owning {
                continue;
            }
            let src = rel.source.to_string();
            let tgt = rel.target.to_string();
            if !top_level_ids.contains(&src) || !top_level_ids.contains(&tgt) {
                continue;
            }

            let label_text = format!("{:?}", rel.kind);
            let edge = DiagramEdge::relationship(
                rel.id.to_string(),
                src,
                tgt,
                rel.kind.clone(),
                label_text,
            );
            ir.edges.push(edge);
        }

        ir
    }
}

// ── Position/size extraction ─────────────────────────────────────────────

/// Extract position from an element's `x`/`y` properties.
///
/// Two source shapes are accepted:
/// - Direct prop on the part — e.g. `Element::with_prop("x", 100.0)`,
///   the form unit-tests use to seed elements without going through the
///   parser.
/// - SysML `attribute x = 100.0;` body — the parser stores those as
///   AttributeUsage children whose name is `x` and whose `value` prop
///   carries the literal. Geometry.sysml-style fixtures (such as a
///   `Structure/Layout.sysml` laying out floor-plan coordinates) live here.
fn extract_position(element: &Element, graph: &ModelGraph) -> Option<(f64, f64)> {
    let x = read_named_coord(element, graph, "x")?;
    let y = read_named_coord(element, graph, "y")?;
    Some((x, y))
}

/// Extract size from an element's `width`/`height` properties. See
/// [`extract_position`] for the source-shape contract.
fn extract_size(element: &Element, graph: &ModelGraph) -> Option<(f64, f64)> {
    let width = read_named_coord(element, graph, "width")?;
    let height = read_named_coord(element, graph, "height")?;
    Some((width, height))
}

/// Read a coordinate-shaped float for `name` off `element`, trying the
/// direct-prop path first and falling back to a child AttributeUsage
/// of that name.
fn read_named_coord(element: &Element, graph: &ModelGraph, name: &str) -> Option<f64> {
    if let Some(v) = element.get_prop(name).and_then(|v| v.as_float()) {
        return Some(v);
    }
    graph
        .children_of(&element.id)
        .filter(|c| c.kind == ElementKind::AttributeUsage)
        .find(|c| c.name.as_deref() == Some(name))
        .and_then(|attr| attr.get_prop("value").and_then(|v| v.as_float()))
}

// ── Node generation ──────────────────────────────────────────────────────

/// Build a `DiagramNode` for one element with position/size from properties.
fn generate_geometry_node(
    graph: &ModelGraph,
    element: &Element,
    expanded_ids: &HashSet<String>,
) -> DiagramNode {
    let id = element.id.to_string();
    let kind = &element.kind;
    let name = element.name.as_deref().unwrap_or("unnamed").to_owned();
    let visual_kind = VisualKind::from_element_kind(kind);
    let stereotype = view_text::stereotype_text(kind);
    let is_expanded = expanded_ids.contains(&id);

    let mut node = DiagramNode::new(id, visual_kind, &name)
        .with_stereotype(stereotype);

    // Apply position/size from element properties.
    if let Some((x, y)) = extract_position(element, graph) {
        node = node.with_position(x, y);
    }
    if let Some((w, h)) = extract_size(element, graph) {
        node = node.with_size(w, h);
    }

    node.tooltip = view_text::tooltip_text(element, graph);

    // Element kind drives the adapter-derived kind/definition/usage/visual classes;
    // property-based decorations become typed node tags.
    node = node.with_element_kind(kind.clone());
    node.tags.extend(classify::property_tags(element));

    // Collect owned children (skip memberships) in source order (C13).
    let owned: Vec<_> = super::container::ordered_children(graph, &element.id)
        .into_iter()
        .filter(|child| !classify::is_membership_kind(&child.kind))
        .filter(|child| !classify::is_import_kind(&child.kind))
        .collect();

    let mut has_expandable = false;

    if !owned.is_empty() {
        // Ports with geometry-specific position/size handling
        for child in &owned {
            if classify::is_port_kind(&child.kind) {
                let mut port = DiagramPort::new(
                    child.id.to_string(),
                    child.name.as_deref().unwrap_or("port"),
                );
                if let Some((x, y)) = extract_position(child, graph) {
                    port.position = Some((x, y));
                }
                if let Some((w, h)) = extract_size(child, graph) {
                    port.size = Some((w, h));
                }
                node.ports.push(port);
            }
        }

        // Non-port children: check expandability
        has_expandable = owned.iter().any(|c| !classify::is_port_kind(&c.kind));

        if is_expanded {
            super::container::render_expanded_children(
                graph, kind, &owned, expanded_ids, &mut node, generate_geometry_node,
            );
        } else {
            super::container::render_collapsed_children(
                graph, kind, &owned, &mut node,
            );
        }
    }

    // Expand controls (button + expanded state + layout mode)
    super::container::apply_expand_controls(
        &mut node, has_expandable, !owned.is_empty(), is_expanded,
    );

    node
}

// ── Text rendering for collapsed children ────────────────────────────────

/// Produce the textual representation of a collapsed child element.
/// Delegates to the shared `container::compartment_text_for_element`.
fn render_child_text(element: &Element, graph: &ModelGraph) -> String {
    super::container::compartment_text_for_element(
        element,
        graph,
        crate::visual_kind::CompartmentKind::Members,
    )
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use sysml_core::{Element, ElementKind, ModelGraph, Relationship, RelationshipKind};

    use crate::ir::generator::GeneratorContext;

    static EMPTY_SET: std::sync::LazyLock<HashSet<String>> =
        std::sync::LazyLock::new(HashSet::new);

    fn make_ctx(graph: &ModelGraph) -> GeneratorContext {
        GeneratorContext::new(graph, &EMPTY_SET)
    }

    fn make_ctx_with<'a>(
        graph: &'a ModelGraph,
        expanded: &'a HashSet<String>,
    ) -> GeneratorContext<'a> {
        GeneratorContext::new(graph, expanded)
    }

    // ── Empty graph ──────────────────────────────────────────────────



    // ── Position extraction ──────────────────────────────────────────



    // Source-shape coverage: SysML files spell coordinates as
    // `attribute x = 100.0;` inside a part body, which the parser
    // emits as AttributeUsage children rather than as a `prop` on
    // the part itself. The geometry generator must read both shapes
    // — without this, fixtures like a floor-plan
    // `Structure/Layout.sysml` would
    // render at None positions and the canvas would fall back to
    // ELK's auto-layout.
    #[test]
    fn geometry_reads_coords_from_attribute_usage_children() {
        use sysml_core::Value;

        let mut graph = ModelGraph::new();
        let part = Element::new_with_kind(ElementKind::PartUsage).with_name("Box");
        let part_id = graph.add_element(part);

        // Mirror the parser's shape: AttributeUsage child with `value`
        // prop carrying the literal default.
        let mut x_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("x")
            .with_owner(part_id.clone());
        x_attr.set_prop("value", Value::Float(100.0));
        graph.add_element(x_attr);

        let mut y_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("y")
            .with_owner(part_id.clone());
        y_attr.set_prop("value", Value::Float(200.0));
        graph.add_element(y_attr);

        let mut w_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("width")
            .with_owner(part_id.clone());
        w_attr.set_prop("value", Value::Float(60.0));
        graph.add_element(w_attr);

        let mut h_attr = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("height")
            .with_owner(part_id.clone());
        h_attr.set_prop("value", Value::Float(40.0));
        graph.add_element(h_attr);

        let gen = GeometryViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        let box_node = ir.nodes.iter().find(|n| n.name == "Box").unwrap();
        assert_eq!(box_node.position, Some((100.0, 200.0)));
        assert_eq!(box_node.size, Some((60.0, 40.0)));
    }

    // ── Expand/collapse ──────────────────────────────────────────────

    #[test]
    fn geometry_ir_collapsed_children_as_text() {
        let mut graph = ModelGraph::new();

        let parent = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Container")
            .with_prop("x", 100.0)
            .with_prop("y", 100.0);
        let parent_id = graph.add_element(parent);

        let child = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Inner")
            .with_owner(parent_id);
        graph.add_element(child);

        let gen = GeometryViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        // Parent node should exist
        assert_eq!(ir.nodes.len(), 1);
        let parent_node = &ir.nodes[0];
        assert_eq!(parent_node.name, "Container");

        // Child should be collapsed text
        let has_text = parent_node.children.iter().any(|c| {
            matches!(c, DiagramChild::Text { text, .. } if text.contains("Inner"))
        });
        assert!(has_text, "collapsed child should appear as text");

        // Should have expand button
        assert!(
            !parent_node.buttons.is_empty(),
            "node with children should have expand button"
        );
        assert_eq!(parent_node.expanded, Some(false));
    }

    #[test]
    fn geometry_ir_expanded_children_as_nodes() {
        let mut graph = ModelGraph::new();

        let parent = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Container")
            .with_prop("x", 100.0)
            .with_prop("y", 100.0);
        let parent_id = graph.add_element(parent);

        let child = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Inner")
            .with_prop("x", 20.0)
            .with_prop("y", 60.0)
            .with_owner(parent_id.clone());
        graph.add_element(child);

        let mut expanded = HashSet::new();
        expanded.insert(parent_id.to_string());

        let gen = GeometryViewGenerator;
        let ir = gen.generate(&make_ctx_with(&graph, &expanded));

        let parent_node = &ir.nodes[0];
        assert_eq!(parent_node.expanded, Some(true));

        // Child should be a nested DiagramChild::Node
        let has_child_node = parent_node.children.iter().any(|c| {
            matches!(c, DiagramChild::Node(n) if n.name == "Inner")
        });
        assert!(
            has_child_node,
            "expanded child should appear as nested node"
        );

        // The nested node should carry its position
        if let Some(DiagramChild::Node(child_node)) = parent_node
            .children
            .iter()
            .find(|c| matches!(c, DiagramChild::Node(n) if n.name == "Inner"))
        {
            assert_eq!(child_node.position, Some((20.0, 60.0)));
        }
    }

    // ── Port rendering ───────────────────────────────────────────────

    #[test]
    fn geometry_ir_ports_on_boundary() {
        let mut graph = ModelGraph::new();

        let block = Element::new_with_kind(ElementKind::PartDefinition)
            .with_name("Block")
            .with_prop("x", 50.0)
            .with_prop("y", 50.0);
        let block_id = graph.add_element(block);

        let port = Element::new_with_kind(ElementKind::PortUsage)
            .with_name("dataIn")
            .with_prop("x", 0.0)
            .with_prop("y", 20.0)
            .with_prop("width", 10.0)
            .with_prop("height", 10.0)
            .with_owner(block_id);
        graph.add_element(port);

        let gen = GeometryViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        assert_eq!(ir.nodes.len(), 1);
        let block_node = &ir.nodes[0];

        assert_eq!(block_node.ports.len(), 1);
        let p = &block_node.ports[0];
        assert_eq!(p.name, "dataIn");
        assert_eq!(p.position, Some((0.0, 20.0)));
        assert_eq!(p.size, Some((10.0, 10.0)));

        // Ports should NOT appear as expandable children
        assert!(
            block_node.buttons.is_empty(),
            "port-only children should not trigger expand button"
        );
    }

    // ── Edge filtering ───────────────────────────────────────────────

    #[test]
    fn geometry_ir_filters_edges_with_dangling_endpoints() {
        let mut graph = ModelGraph::new();

        let a = Element::new_with_kind(ElementKind::PartDefinition).with_name("A");
        let a_id = graph.add_element(a);

        let b = Element::new_with_kind(ElementKind::PartDefinition).with_name("B");
        let b_id = graph.add_element(b);

        // Child element (will render as text, not top-level node)
        let child = Element::new_with_kind(ElementKind::PartUsage)
            .with_name("Child")
            .with_owner(a_id.clone());
        let child_id = graph.add_element(child);

        // Valid edge: A -> B (both top-level)
        let rel_ok =
            Relationship::new(RelationshipKind::Reference, a_id.clone(), b_id);
        graph.add_relationship(rel_ok);

        // Dangling edge: A -> Child (child is not top-level)
        let rel_bad =
            Relationship::new(RelationshipKind::Specialize, a_id, child_id);
        graph.add_relationship(rel_bad);

        let gen = GeometryViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        assert_eq!(
            ir.edges.len(),
            1,
            "only the A->B edge should be present; dangling edge filtered"
        );
    }

    // ── Integer properties ───────────────────────────────────────────

    #[test]
    fn geometry_ir_handles_integer_props() {
        let mut graph = ModelGraph::new();
        let elem = Element::new_with_kind(ElementKind::Package)
            .with_name("IntPos")
            .with_prop("x", 10i64)
            .with_prop("y", 20i64)
            .with_prop("width", 100i64)
            .with_prop("height", 50i64);
        graph.add_element(elem);

        let gen = GeometryViewGenerator;
        let ir = gen.generate(&make_ctx(&graph));

        assert_eq!(ir.nodes.len(), 1);
        let node = &ir.nodes[0];
        assert!(
            node.position.is_some(),
            "integer props should convert via as_float()"
        );
        assert!(node.size.is_some());
    }

    // ── View type metadata ───────────────────────────────────────────

    #[test]
    fn geometry_ir_view_type_and_algorithm() {
        let gen = GeometryViewGenerator;
        assert_eq!(gen.view_type(), ViewType::Geometry);
        assert_eq!(gen.elk_algorithm(), "layered");
        assert_eq!(gen.elk_direction(), Some("DOWN"));
    }

    // ── Render roundtrip ─────────────────────────────────────────────


}
