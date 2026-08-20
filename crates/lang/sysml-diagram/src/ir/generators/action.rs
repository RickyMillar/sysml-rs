//! ActionFlowView IR generator.
//!
//! Produces `DiagramIR` for Activity/Action Flow Diagrams — action nodes
//! connected by control flow edges.
//!
//! Uses `sysml_runtime::actions::compile_action()` to compile the ModelGraph
//! into `ActionGraphIR`, then maps `ActionNodeIR` / `ActionEdgeIR` to
//! `DiagramNode` / `DiagramEdge` IR types.
//!
//! ## Key design decisions vs. the old `smodel::action` generator
//!
//! - **DiagramEdge::control_flow()**: Edges use `DiagramEdgeKind::ControlFlow { guard }`
//!   instead of raw `SEdge` construction. The guard is stringified once here;
//!   `render.rs` wraps it in brackets for display.
//! - **Node sizes**: Set via `DiagramNode.size` for control nodes (initial, final,
//!   decision, merge, fork, join, terminate).
//! - **Ports on Perform nodes**: Expressed as `DiagramPort` with `PortDirection::In`
//!   / `PortDirection::Out`, not raw `SPort` structs.
//! - **HeaderStyle**: Control nodes use `HeaderStyle::None`; action/perform nodes
//!   use `HeaderStyle::Inline` for a single name label.

use sysml_core::{ElementId, ElementKind, ModelGraph};
use sysml_runtime::actions::{ActionEdgeIR, ActionGraphIR, ActionNodeIR};
use tracing::instrument;

use crate::ir::generator::{GeneratorContext, ViewGenerator};
use crate::ir::types::{DiagramIR, DiagramNode, HeaderStyle, DiagramChild, PortSide, DiagramPort, PortDirection, DiagramEdge, DiagramEdgeKind, EndpointMode, NodeTag, PortTag};
use crate::smodel::builders;
use crate::smodel::ViewType;
use crate::visual_kind::VisualKind;

/// Generates Action Flow Diagrams.
pub struct ActionFlowViewGenerator;

impl ViewGenerator for ActionFlowViewGenerator {
    fn view_type(&self) -> ViewType {
        ViewType::ActionFlow
    }

    fn elk_algorithm(&self) -> &str {
        "layered"
    }

    fn elk_direction(&self) -> Option<&str> {
        Some("DOWN")
    }

    #[instrument(skip_all)]
    #[allow(clippy::indexing_slicing)] // Indexes guarded by length checks above
    fn generate(&self, ctx: &GeneratorContext) -> DiagramIR {
        let graph = ctx.graph;

        // Spec ViewFilter (4.5) is applied at top-level ActionDefinition
        // collection. Switching from name-only to element-based iteration
        // lets the filter check `ElementKind` / stereotypes per definition.
        // The expose fence (`in_exposed_scope`) keeps a declared `expose X`
        // view from sweeping in every ActionDefinition in the merged graph
        // (stdlib included — D-B1's 106-node StartFlowView).
        let action_names: Vec<String> = graph
            .elements_by_kind(&ElementKind::ActionDefinition)
            .filter(|e| ctx.passes_filter(e))
            .filter(|e| ctx.in_exposed_scope(e))
            .filter_map(|e| e.name.clone())
            .collect();

        tracing::info!(
            action_count = action_names.len(),
            "ActionFlowView IR generate"
        );

        if action_names.is_empty() {
            return empty_action_ir("No ActionDefinition found");
        }

        // Wrap each ActionDefinition in a labeled container node.
        // For single definitions, this ensures the action name is visible
        // (without the wrapper, only the internal flow nodes are shown).
        let mut ir = DiagramIR::new(ViewType::ActionFlow);
        for name in &action_names {
            let sub_ir = generate_named_ir(graph, name);
            let mut container = DiagramNode::new(
                format!("container-{}", name),
                VisualKind::Action,
                name.clone(),
            )
            .with_header_style(HeaderStyle::Inline)
            .with_expanded(true)
            .with_tag(NodeTag::ActionContainer);
            container.tooltip = Some(format!("action def {}", name));

            // Nest the sub-IR's nodes and edges as children of the container
            // (ELK requires edges at the LCA level of their source/target)
            for sub_node in sub_ir.nodes {
                container.children.push(DiagramChild::Node(sub_node));
            }
            for edge in sub_ir.edges {
                container.children.push(DiagramChild::Edge(edge));
            }

            ir.nodes.push(container);
        }

        ir
    }

    fn generate_for_owner(
        &self,
        ctx: &GeneratorContext,
        owner_id: &str,
    ) -> Option<DiagramIR> {
        let graph = ctx.graph;
        let owner_eid = ElementId::from_string(owner_id);

        tracing::info!(owner_id = %owner_id, "generate_for_owner (action)");

        // If the owner itself is an action definition, compile it directly
        if let Some(owner) = graph.get_element(&owner_eid) {
            if matches!(
                owner.kind,
                ElementKind::ActionDefinition | ElementKind::ActionUsage
            ) {
                if let Some(name) = &owner.name {
                    return Some(generate_named_ir(graph, name));
                }
            }
        }

        // Otherwise, find ActionDefinition/ActionUsage children and compile each
        let mut ir = DiagramIR::new(ViewType::ActionFlow);
        for child in graph.children_of(&owner_eid) {
            if !matches!(
                child.kind,
                ElementKind::ActionDefinition | ElementKind::ActionUsage
            ) {
                continue;
            }
            if let Some(name) = &child.name {
                let sub_ir = generate_named_ir(graph, name);
                ir.nodes.extend(sub_ir.nodes);
                ir.edges.extend(sub_ir.edges);
            }
        }
        Some(ir)
    }
}

/// Generate a DiagramIR for a single named action definition.
pub(crate) fn generate_named_ir(graph: &ModelGraph, action_name: &str) -> DiagramIR {
    match sysml_runtime::actions::compile_action(action_name, graph) {
        Ok(action_ir) => generate_from_action_ir(&action_ir, Some(graph)),
        Err(_diags) => empty_action_ir(&format!(
            "Failed to compile action '{}'",
            action_name
        )),
    }
}

/// Flow direction for port placement on action nodes.
/// DOWN = top-to-bottom (ports on NORTH/SOUTH), RIGHT = left-to-right (ports on WEST/EAST).
#[derive(Clone, Copy, PartialEq)]
enum FlowDirection {
    Down,
    Right,
}

impl FlowDirection {
    /// Incoming port side (where edges arrive).
    fn incoming_side(self) -> PortSide {
        match self {
            FlowDirection::Down => PortSide::North,
            FlowDirection::Right => PortSide::West,
        }
    }
    /// Outgoing port side (where edges leave).
    fn outgoing_side(self) -> PortSide {
        match self {
            FlowDirection::Down => PortSide::South,
            FlowDirection::Right => PortSide::East,
        }
    }
    /// Left branch side (for decision else / merge left input).
    fn left_branch_side(self) -> PortSide {
        match self {
            FlowDirection::Down => PortSide::West,
            FlowDirection::Right => PortSide::North,
        }
    }
    /// Right branch side (for decision guard / merge right input).
    fn right_branch_side(self) -> PortSide {
        match self {
            FlowDirection::Down => PortSide::East,
            FlowDirection::Right => PortSide::South,
        }
    }
    /// Fork/join bar dimensions: (width, height).
    fn bar_size(self) -> (f64, f64) {
        match self {
            FlowDirection::Down => (80.0, 6.0),   // horizontal bar
            FlowDirection::Right => (6.0, 80.0),   // vertical bar
        }
    }
}

/// Convert an `ActionGraphIR` into a `DiagramIR`.
fn generate_from_action_ir(action_ir: &ActionGraphIR, graph: Option<&ModelGraph>) -> DiagramIR {
    let mut ir = DiagramIR::new(ViewType::ActionFlow);
    let mut counter = 0u32;
    // ActionFlowView is locked to elk.direction=DOWN via elk-config.ts
    let flow = FlowDirection::Down;

    // Build a map of node ID → node type for edge-to-port wiring
    let node_types: std::collections::HashMap<String, &ActionNodeIR> = action_ir
        .nodes
        .iter()
        .map(|n| (action_node_id(n).to_owned(), n))
        .collect();

    // Generate nodes
    for node in &action_ir.nodes {
        ir.nodes.push(generate_action_node(node, &mut counter, graph, flow));
    }

    // Generate edges with port wiring.
    // Track per-node port usage counters so fan-out/fan-in edges get distinct ports.
    let mut src_port_counter: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut tgt_port_counter: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for edge in &action_ir.edges {
        counter += 1;
        ir.edges.push(generate_flow_edge(
            edge,
            counter,
            &node_types,
            &mut src_port_counter,
            &mut tgt_port_counter,
        ));
    }

    ir
}

/// Extract the ID from an ActionNodeIR variant.
fn action_node_id(node: &ActionNodeIR) -> &str {
    node.id()
}

/// Map a single `ActionNodeIR` to a `DiagramNode`.
fn generate_action_node(
    node: &ActionNodeIR,
    counter: &mut u32,
    graph: Option<&ModelGraph>,
    flow: FlowDirection,
) -> DiagramNode {
    *counter += 1;

    let (id, visual_kind, label_text, tags): (String, VisualKind, String, Vec<NodeTag>) = match node {
        ActionNodeIR::Initial { id } => (
            id.clone(),
            VisualKind::InitialNode,
            String::new(),
            vec![],
        ),
        ActionNodeIR::Final { id } => (
            id.clone(),
            VisualKind::FinalNode,
            String::new(),
            vec![],
        ),
        ActionNodeIR::Terminate { id } => (
            id.clone(),
            VisualKind::TerminateNode,
            String::new(),
            vec![],
        ),
        ActionNodeIR::Decision { id } => (
            id.clone(),
            VisualKind::DecisionNode,
            "decide".to_owned(),
            vec![],
        ),
        ActionNodeIR::Merge { id } => (
            id.clone(),
            VisualKind::MergeNode,
            "merge".to_owned(),
            vec![],
        ),
        ActionNodeIR::Fork { id } => (
            id.clone(),
            VisualKind::ForkNode,
            String::new(),
            vec![],
        ),
        ActionNodeIR::Join { id } => (
            id.clone(),
            VisualKind::JoinNode,
            String::new(),
            vec![],
        ),
        ActionNodeIR::Perform { id, action_ref, .. } => (
            id.clone(),
            VisualKind::Action,
            action_ref.clone(),
            vec![NodeTag::Perform],
        ),
        ActionNodeIR::Send { id, target, .. } => (
            id.clone(),
            VisualKind::SendAction,
            format!("send \u{2192} {}", target),
            vec![],
        ),
        ActionNodeIR::Accept { id, .. } => (
            id.clone(),
            VisualKind::AcceptAction,
            "accept".to_owned(),
            vec![],
        ),
        ActionNodeIR::Assign { id, target, .. } => (
            id.clone(),
            VisualKind::Action,
            format!("{} :=", target),
            vec![NodeTag::Assign],
        ),
        ActionNodeIR::If { id, .. } => (
            id.clone(),
            VisualKind::Action,
            "\u{00ab}if\u{00bb}".to_owned(),
            vec![NodeTag::IfNode],
        ),
        ActionNodeIR::WhileLoop { id, .. } => (
            id.clone(),
            VisualKind::Action,
            "\u{00ab}loop\u{00bb}".to_owned(),
            vec![NodeTag::LoopNode, NodeTag::LoopWhile],
        ),
        ActionNodeIR::ForLoop { id, .. } => (
            id.clone(),
            VisualKind::Action,
            "\u{00ab}loop\u{00bb}".to_owned(),
            vec![NodeTag::LoopNode, NodeTag::LoopFor],
        ),
        ActionNodeIR::StreamSource { id, target, .. } => (
            id.clone(),
            VisualKind::SendAction,
            format!("stream \u{2192} {}", target),
            vec![NodeTag::StreamSource],
        ),
    };

    // Look up source location and tooltip from the graph if available
    let looked_up_element = graph.and_then(|g| {
        id.parse::<ElementId>()
            .ok()
            .and_then(|eid| g.get_element(&eid))
    });

    let tooltip = looked_up_element.and_then(|elem| {
        graph.and_then(|g| builders::tooltip_text(elem, g))
    });

    // Determine header style and size based on node kind
    let (header_style, size) = match node {
        ActionNodeIR::Initial { .. }
        | ActionNodeIR::Final { .. }
        | ActionNodeIR::Terminate { .. } => (HeaderStyle::None, Some((20.0, 20.0))),

        ActionNodeIR::Decision { .. } | ActionNodeIR::Merge { .. } => {
            (HeaderStyle::Inline, Some((60.0, 60.0)))
        }

        ActionNodeIR::Fork { .. } | ActionNodeIR::Join { .. } => {
            (HeaderStyle::None, Some(flow.bar_size()))
        }

        // Action-like nodes get an inline label
        _ => (HeaderStyle::Inline, None),
    };

    let mut diagram_node = DiagramNode::new(id, visual_kind, label_text)
        .with_header_style(header_style);

    if let Some((w, h)) = size {
        diagram_node = diagram_node.with_size(w, h);
    }

    diagram_node.tooltip = tooltip;
    diagram_node.tags.extend(tags);

    // === Port generation for all action flow nodes (TOP-DOWN layout) ===
    //
    // Every node gets hidden in/out ports so edges pin to specific attachment
    // points via strict-port mode. This shares the port-based routing pattern
    // with interconnection view (IBD).
    let node_id = &diagram_node.element_id;

    let in_side = flow.incoming_side();
    let out_side = flow.outgoing_side();
    let left_side = flow.left_branch_side();
    let right_side = flow.right_branch_side();
    let (bar_w, bar_h) = flow.bar_size();

    match node {
        // Fork/join bars: hidden ports distributed along the bar
        ActionNodeIR::Fork { .. } | ActionNodeIR::Join { .. } => {
            let positions = [0.1, 0.3, 0.5, 0.7, 0.9];
            let is_down = flow == FlowDirection::Down;
            let bar_long = if is_down { bar_w } else { bar_h };
            let in_css = if is_down { "port-n" } else { "port-w" };
            for (i, &frac) in positions.iter().enumerate() {
                let pos = if is_down { (bar_long * frac, 0.0) } else { (0.0, bar_long * frac) };
                let mut port = DiagramPort::new(
                    format!("{}/{}-{}", node_id, in_css, i), ""
                ).hidden().with_side(in_side);
                port.position = Some(pos);
                port.size = Some((1.0, 1.0));
                port.tags.push(PortTag::Control);
                diagram_node.ports.push(port);
            }
            let out_css = if is_down { "port-s" } else { "port-e" };
            for (i, &frac) in positions.iter().enumerate() {
                let pos = if is_down { (bar_long * frac, bar_h) } else { (bar_w, bar_long * frac) };
                let mut port = DiagramPort::new(
                    format!("{}/{}-{}", node_id, out_css, i), ""
                ).hidden().with_side(out_side);
                port.position = Some(pos);
                port.size = Some((1.0, 1.0));
                port.tags.push(PortTag::Control);
                diagram_node.ports.push(port);
            }
        }

        // Decision: incoming, right-branch (guard), left-branch (else)
        ActionNodeIR::Decision { .. } => {
            let w = 60.0_f64;
            let h = 60.0_f64;
            let is_down = flow == FlowDirection::Down;

            // Incoming port — pushed above diamond tip so edge enters from above
            let mut in_port = DiagramPort::new(
                format!("{}/port-in", node_id), ""
            ).hidden().with_side(in_side);
            in_port.position = Some(if is_down { (w / 2.0, -4.0) } else { (-4.0, h / 2.0) });
            in_port.size = Some((1.0, 1.0));
            diagram_node.ports.push(in_port);

            // Guard/true branch port (hidden — edges wire to it via strict-port)
            let mut guard_port = DiagramPort::new(
                format!("{}/port-guard", node_id), ""
            ).hidden().with_side(right_side);
            guard_port.position = Some(if is_down { (w, h / 2.0) } else { (w / 2.0, h) });
            guard_port.size = Some((1.0, 1.0));
            guard_port.tags.push(PortTag::Control);
            diagram_node.ports.push(guard_port);

            // Else/default branch port (hidden)
            let mut else_port = DiagramPort::new(
                format!("{}/port-else", node_id), ""
            ).hidden().with_side(left_side);
            else_port.position = Some(if is_down { (0.0, h / 2.0) } else { (w / 2.0, 0.0) });
            else_port.size = Some((1.0, 1.0));
            else_port.tags.push(PortTag::Control);
            diagram_node.ports.push(else_port);
        }

        // Merge: hidden EAST/WEST input ports, visible SOUTH output port
        ActionNodeIR::Merge { .. } => {
            let w = 60.0_f64;
            let h = 60.0_f64;
            let is_down = flow == FlowDirection::Down;

            // Hidden input ports (edges pin to them but they're invisible)
            let mut in_e = DiagramPort::new(
                format!("{}/port-in-e", node_id), ""
            ).hidden().with_side(right_side);
            in_e.position = Some(if is_down { (w, h / 2.0) } else { (w / 2.0, h) });
            in_e.size = Some((1.0, 1.0));
            in_e.tags.push(PortTag::Control);
            diagram_node.ports.push(in_e);

            let mut in_w = DiagramPort::new(
                format!("{}/port-in-w", node_id), ""
            ).hidden().with_side(left_side);
            in_w.position = Some(if is_down { (0.0, h / 2.0) } else { (w / 2.0, 0.0) });
            in_w.size = Some((1.0, 1.0));
            in_w.tags.push(PortTag::Control);
            diagram_node.ports.push(in_w);

            // Hidden output port — pushed below diamond tip
            let mut out_p = DiagramPort::new(
                format!("{}/port-out", node_id), ""
            ).hidden().with_side(out_side);
            out_p.position = Some(if is_down { (w / 2.0, h + 4.0) } else { (w + 4.0, h / 2.0) });
            out_p.size = Some((1.0, 1.0));
            out_p.tags.push(PortTag::Control);
            diagram_node.ports.push(out_p);
        }

        // Initial/Final/Terminate: no ports
        ActionNodeIR::Initial { .. } | ActionNodeIR::Final { .. } | ActionNodeIR::Terminate { .. } => {}

        // All other action nodes: hidden in + out ports on flow axis
        _ => {
            let mut in_port = DiagramPort::new(
                format!("{}/port-in", node_id), ""
            ).hidden().with_side(in_side);
            in_port.size = Some((1.0, 1.0));
            in_port.tags.push(PortTag::Control);
            diagram_node.ports.push(in_port);

            let mut out_port = DiagramPort::new(
                format!("{}/port-out", node_id), ""
            ).hidden().with_side(out_side);
            out_port.size = Some((1.0, 1.0));
            out_port.tags.push(PortTag::Control);
            diagram_node.ports.push(out_port);
        }
    }

    // Add ports for Perform nodes (input parameters and output binding)
    if let ActionNodeIR::Perform {
        inputs,
        output_binding,
        ..
    } = node
    {
        let node_id = diagram_node.element_id.clone();

        for (idx, (param_name, _)) in inputs.iter().enumerate() {
            let port = DiagramPort::new(
                format!("{}/in-{}", node_id, idx),
                param_name.clone(),
            )
            .with_direction(PortDirection::In)
            .with_size(8.0, 8.0);
            diagram_node.ports.push(port);
        }

        if let Some(out_name) = output_binding {
            let port = DiagramPort::new(
                format!("{}/out", node_id),
                out_name.clone(),
            )
            .with_direction(PortDirection::Out)
            .with_size(8.0, 8.0);
            diagram_node.ports.push(port);
        }
    }

    diagram_node
}

/// Map an `ActionEdgeIR` to a `DiagramEdge`, wiring to hidden ports on
/// control nodes (fork/join/decision/merge) for proper edge attachment.
///
/// Per-node port counters distribute fan-out/fan-in edges across distinct
/// ports along the fork/join bar (ports 0-4 per side).
fn generate_flow_edge(
    edge: &ActionEdgeIR,
    counter: u32,
    node_types: &std::collections::HashMap<String, &ActionNodeIR>,
    src_port_counter: &mut std::collections::HashMap<String, usize>,
    tgt_port_counter: &mut std::collections::HashMap<String, usize>,
) -> DiagramEdge {
    let id = format!("flow-{}-{}-{}", edge.from, edge.to, counter);
    let guard_str = edge.guard.as_ref().map(|g| format!("{:?}", g));

    let mut diagram_edge =
        DiagramEdge::control_flow(id, edge.from.clone(), edge.to.clone(), guard_str);

    // Wire source port: use the outgoing port of the source node
    if let Some(src_node) = node_types.get(&edge.from) {
        let src_idx = *src_port_counter.entry(edge.from.clone()).or_insert(0);
        let src_id = src_node.id();
        diagram_edge.source_port_id = match src_node {
            // Fork: fan-out across SOUTH ports (0-4)
            ActionNodeIR::Fork { .. } => {
                Some(format!("{}/port-s-{}", src_id, src_idx % 5))
            }
            // Join: single outgoing from SOUTH center
            ActionNodeIR::Join { .. } => Some(format!("{}/port-s-2", src_id)),
            // Decision: pin edges to guard (EAST) or else (WEST) ports.
            // Use guard text to distinguish: guarded → EAST, default/else → WEST.
            // When no guard info, alternate: first=WEST (else), second=EAST (guard).
            ActionNodeIR::Decision { .. } => {
                if edge.guard.is_some() {
                    Some(format!("{}/port-guard", src_id))
                } else if src_idx == 0 {
                    Some(format!("{}/port-else", src_id))
                } else {
                    Some(format!("{}/port-guard", src_id))
                }
            }
            // Merge: outgoing from SOUTH
            ActionNodeIR::Merge { .. } => Some(format!("{}/port-out", src_id)),
            // Initial/Final/Terminate: no ports
            ActionNodeIR::Initial { .. }
            | ActionNodeIR::Final { .. }
            | ActionNodeIR::Terminate { .. } => None,
            // All other action nodes: hidden out port (SOUTH)
            _ => Some(format!("{}/port-out", src_id)),
        };
        *src_port_counter.entry(edge.from.clone()).or_insert(0) += 1;
    }

    // Wire target port: use the incoming port of the target node
    if let Some(tgt_node) = node_types.get(&edge.to) {
        let tgt_idx = *tgt_port_counter.entry(edge.to.clone()).or_insert(0);
        let tgt_id = tgt_node.id();
        diagram_edge.target_port_id = match tgt_node {
            // Fork: incoming from NORTH center
            ActionNodeIR::Fork { .. } => Some(format!("{}/port-n-2", tgt_id)),
            // Join: fan-in across NORTH ports (0-4)
            ActionNodeIR::Join { .. } => {
                Some(format!("{}/port-n-{}", tgt_id, tgt_idx % 5))
            }
            // Decision: incoming from NORTH
            ActionNodeIR::Decision { .. } => Some(format!("{}/port-in", tgt_id)),
            // Merge: pin incoming edges to WEST (first) and EAST (second).
            ActionNodeIR::Merge { .. } => {
                if tgt_idx == 0 {
                    Some(format!("{}/port-in-w", tgt_id))
                } else {
                    Some(format!("{}/port-in-e", tgt_id))
                }
            }
            // Initial/Final/Terminate: no ports
            ActionNodeIR::Initial { .. }
            | ActionNodeIR::Final { .. }
            | ActionNodeIR::Terminate { .. } => None,
            // All other action nodes: hidden in port (NORTH)
            _ => Some(format!("{}/port-in", tgt_id)),
        };
        *tgt_port_counter.entry(edge.to.clone()).or_insert(0) += 1;
    }

    // If either endpoint is wired to a port, use strict-port mode so the
    // WASM router pins the edge to the exact port position.
    if diagram_edge.source_port_id.is_some() || diagram_edge.target_port_id.is_some() {
        diagram_edge.endpoint_mode = EndpointMode::StrictPort;
    }

    diagram_edge
}

/// Create an empty/fallback DiagramIR with a message node.
fn empty_action_ir(message: &str) -> DiagramIR {
    let mut ir = DiagramIR::new(ViewType::ActionFlow);
    let mut node = DiagramNode::new("message", VisualKind::Generic, message.to_owned())
        .with_header_style(HeaderStyle::Inline);
    node.tooltip = None;
    ir.nodes.push(node);
    ir
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    use sysml_core::ModelGraph;
    use sysml_runtime::actions::ActionGraphIR;
    use sysml_runtime::expressions::ExprIR;

    use crate::ir::generator::GeneratorContext;
    use crate::ir::render::render;

    fn make_ctx<'a>(graph: &'a ModelGraph, expanded: &'a HashSet<String>) -> GeneratorContext<'a> {
        GeneratorContext::new(graph, expanded)
    }

    /// Helper: build a simple IR with one node between Initial and Final.
    fn ir_with_node(node: ActionNodeIR) -> ActionGraphIR {
        let node_id = node.id().to_string();
        ActionGraphIR {
            id: "test".to_string(),
            name: "Test".to_string(),
            nodes: vec![
                ActionNodeIR::Initial {
                    id: "init".to_string(),
                },
                node,
                ActionNodeIR::Final {
                    id: "end".to_string(),
                },
            ],
            edges: vec![
                ActionEdgeIR {
                    from: "init".to_string(),
                    to: node_id.clone(),
                    guard: None,
                },
                ActionEdgeIR {
                    from: node_id,
                    to: "end".to_string(),
                    guard: None,
                },
            ],
            initial_node_id: "init".to_string(),
            final_node_ids: vec!["end".to_string()],
            parameters: vec![],
        }
    }

    // ── Basic action graph ──────────────────────────────────────────────

    #[test]
    fn basic_action_graph_from_ir() {
        let action_ir = ActionGraphIR {
            id: "a1".to_string(),
            name: "TestAction".to_string(),
            nodes: vec![
                ActionNodeIR::Initial {
                    id: "init".to_string(),
                },
                ActionNodeIR::Perform {
                    id: "do-stuff".to_string(),
                    action_ref: "DoStuff".to_string(),
                    inputs: vec![],
                    output_binding: None,
                    sub_action: None,
                },
                ActionNodeIR::Final {
                    id: "end".to_string(),
                },
            ],
            edges: vec![
                ActionEdgeIR {
                    from: "init".to_string(),
                    to: "do-stuff".to_string(),
                    guard: None,
                },
                ActionEdgeIR {
                    from: "do-stuff".to_string(),
                    to: "end".to_string(),
                    guard: None,
                },
            ],
            initial_node_id: "init".to_string(),
            final_node_ids: vec!["end".to_string()],
            parameters: vec![],
        };

        let ir = generate_from_action_ir(&action_ir, None);

        // 3 nodes, 2 edges
        assert_eq!(ir.nodes.len(), 3);
        assert_eq!(ir.edges.len(), 2);

        // Check node kinds
        assert_eq!(ir.nodes[0].visual_kind, VisualKind::InitialNode);
        assert_eq!(ir.nodes[1].visual_kind, VisualKind::Action);
        assert_eq!(ir.nodes[2].visual_kind, VisualKind::FinalNode);

        // Check name label on Perform node
        assert_eq!(ir.nodes[1].name, "DoStuff");

        // Verify it renders to SGraph without error
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();
        assert!(json.contains("node:initialNode"));
        assert!(json.contains("node:action"));
        assert!(json.contains("node:finalNode"));
        assert!(json.contains("DoStuff"));
    }

    // ── Control nodes ───────────────────────────────────────────────────

    #[test]
    fn control_node_sizes() {
        // Initial/Final/Terminate → (20, 20)
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::Initial {
                id: "init2".to_string(),
            }),
            None,
        );
        assert_eq!(ir.nodes[0].size, Some((20.0, 20.0)));

        // Decision/Merge → (40, 40)
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::Decision {
                id: "d1".to_string(),
            }),
            None,
        );
        // The decision node is the second node (after init)
        let decision = ir.nodes.iter().find(|n| n.element_id == "d1").unwrap();
        assert_eq!(decision.size, Some((60.0, 60.0)));
        assert_eq!(decision.visual_kind, VisualKind::DecisionNode);

        // Fork/Join → (80, 6)
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::Fork {
                id: "f1".to_string(),
            }),
            None,
        );
        let fork = ir.nodes.iter().find(|n| n.element_id == "f1").unwrap();
        assert_eq!(fork.size, Some((80.0, 6.0)));
        assert_eq!(fork.visual_kind, VisualKind::ForkNode);
    }

    #[test]
    fn control_nodes_have_no_header() {
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::Initial {
                id: "init".to_string(),
            }),
            None,
        );
        let init = ir.nodes.iter().find(|n| n.element_id == "init").unwrap();
        assert_eq!(init.header_style, HeaderStyle::None);
        assert!(init.name.is_empty());
    }

    // ── Send / Accept / Assign ──────────────────────────────────────────

    #[test]
    fn send_node_kind() {
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::Send {
                id: "s1".to_string(),
                payload: ExprIR::LiteralBool(true),
                target: "Receiver".to_string(),
                port_target: None,
            }),
            None,
        );
        let send = ir.nodes.iter().find(|n| n.element_id == "s1").unwrap();
        assert_eq!(send.visual_kind, VisualKind::SendAction);
        assert!(send.name.contains("Receiver"));
    }

    #[test]
    fn accept_node_kind() {
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::Accept {
                id: "a1".to_string(),
                source: None,
                payload_binding: "msg".to_string(),
                port_source: None,
            }),
            None,
        );
        let accept = ir.nodes.iter().find(|n| n.element_id == "a1").unwrap();
        assert_eq!(accept.visual_kind, VisualKind::AcceptAction);
        assert_eq!(accept.name, "accept");
    }

    #[test]
    fn assign_node_kind() {
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::Assign {
                id: "asgn1".to_string(),
                target: "x".to_string(),
                value: ExprIR::LiteralInt(42),
            }),
            None,
        );
        let assign = ir.nodes.iter().find(|n| n.element_id == "asgn1").unwrap();
        assert_eq!(assign.visual_kind, VisualKind::Action);
        assert!(assign.tags.contains(&NodeTag::Assign));
        assert!(assign.name.contains("x"));
    }

    // ── Structured control flow (If / While / For) ──────────────────────

    #[test]
    fn if_node_structure() {
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::If {
                id: "if1".to_string(),
                condition: ExprIR::LiteralBool(true),
                then_branch: "then".to_string(),
                else_branch: None,
            }),
            None,
        );
        let if_node = ir.nodes.iter().find(|n| n.element_id == "if1").unwrap();
        assert_eq!(if_node.visual_kind, VisualKind::Action);
        assert!(if_node.tags.contains(&NodeTag::IfNode));
        assert!(if_node.name.contains("\u{00ab}if\u{00bb}"));
        // Should NOT have fixed size (it's an action-like node)
        assert!(if_node.size.is_none());
    }

    #[test]
    fn while_loop_structure() {
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::WhileLoop {
                id: "w1".to_string(),
                condition: ExprIR::LiteralBool(true),
                body_entry: "body".to_string(),
                exit_node: "exit".to_string(),
            }),
            None,
        );
        let w = ir.nodes.iter().find(|n| n.element_id == "w1").unwrap();
        assert_eq!(w.visual_kind, VisualKind::Action);
        assert!(w.tags.contains(&NodeTag::LoopNode));
        assert!(w.tags.contains(&NodeTag::LoopWhile));
        assert!(w.name.contains("\u{00ab}loop\u{00bb}"));
    }

    #[test]
    fn for_loop_structure() {
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::ForLoop {
                id: "f1".to_string(),
                variable: "item".to_string(),
                sequence: ExprIR::LiteralInt(0),
                body_entry: "body".to_string(),
                exit_node: "exit".to_string(),
            }),
            None,
        );
        let f = ir.nodes.iter().find(|n| n.element_id == "f1").unwrap();
        assert_eq!(f.visual_kind, VisualKind::Action);
        assert!(f.tags.contains(&NodeTag::LoopNode));
        assert!(f.tags.contains(&NodeTag::LoopFor));
    }

    // ── Perform node ports ──────────────────────────────────────────────

    #[test]
    fn perform_node_ports() {
        let ir = generate_from_action_ir(
            &ir_with_node(ActionNodeIR::Perform {
                id: "p1".to_string(),
                action_ref: "Sub".to_string(),
                inputs: vec![
                    ("x".to_string(), ExprIR::LiteralInt(1)),
                    ("y".to_string(), ExprIR::LiteralInt(2)),
                ],
                output_binding: Some("result".to_string()),
                sub_action: None,
            }),
            None,
        );

        let perform = ir.nodes.iter().find(|n| n.element_id == "p1").unwrap();

        // Should have 2 hidden control ports (in/out) + 2 input ports + 1 output port = 5
        assert_eq!(perform.ports.len(), 5);

        // First two ports are hidden control-flow ports
        assert!(perform.ports[0].is_hidden);
        assert!(perform.ports[1].is_hidden);

        // Input ports (after hidden control ports)
        let in0 = &perform.ports[2];
        assert_eq!(in0.element_id, "p1/in-0");
        assert_eq!(in0.name, "x");
        assert_eq!(in0.direction, Some(PortDirection::In));

        let in1 = &perform.ports[3];
        assert_eq!(in1.element_id, "p1/in-1");
        assert_eq!(in1.name, "y");

        // Output port
        let out = &perform.ports[4];
        assert_eq!(out.element_id, "p1/out");
        assert_eq!(out.name, "result");
        assert_eq!(out.direction, Some(PortDirection::Out));
    }

    // ── Guard labels on edges ───────────────────────────────────────────

    #[test]
    fn guarded_edge_has_guard_label() {
        let action_ir = ActionGraphIR {
            id: "test".to_string(),
            name: "Test".to_string(),
            nodes: vec![
                ActionNodeIR::Initial {
                    id: "init".to_string(),
                },
                ActionNodeIR::Decision {
                    id: "d1".to_string(),
                },
                ActionNodeIR::Perform {
                    id: "p1".to_string(),
                    action_ref: "A".to_string(),
                    inputs: vec![],
                    output_binding: None,
                    sub_action: None,
                },
                ActionNodeIR::Final {
                    id: "end".to_string(),
                },
            ],
            edges: vec![
                ActionEdgeIR {
                    from: "init".to_string(),
                    to: "d1".to_string(),
                    guard: None,
                },
                ActionEdgeIR {
                    from: "d1".to_string(),
                    to: "p1".to_string(),
                    guard: Some(ExprIR::LiteralBool(true)),
                },
                ActionEdgeIR {
                    from: "p1".to_string(),
                    to: "end".to_string(),
                    guard: None,
                },
            ],
            initial_node_id: "init".to_string(),
            final_node_ids: vec!["end".to_string()],
            parameters: vec![],
        };

        let ir = generate_from_action_ir(&action_ir, None);

        // The guarded edge should have a ControlFlow kind with guard
        let guarded = ir
            .edges
            .iter()
            .find(|e| e.source_id == "d1" && e.target_id == "p1")
            .unwrap();
        match &guarded.kind {
            DiagramEdgeKind::ControlFlow { guard } => {
                assert!(guard.is_some(), "guarded edge should have guard text");
            }
            other => panic!("expected ControlFlow, got {:?}", other),
        }

        // The unguarded edge should have no guard
        let unguarded = ir
            .edges
            .iter()
            .find(|e| e.source_id == "init" && e.target_id == "d1")
            .unwrap();
        match &unguarded.kind {
            DiagramEdgeKind::ControlFlow { guard } => {
                assert!(guard.is_none(), "unguarded edge should have no guard");
            }
            other => panic!("expected ControlFlow, got {:?}", other),
        }
    }

    // ── Empty / fallback ────────────────────────────────────────────────

    #[test]
    fn empty_graph_produces_message_node() {
        let graph = ModelGraph::new();
        let gen = ActionFlowViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        assert_eq!(ir.nodes.len(), 1);
        assert!(ir.nodes[0].name.contains("No ActionDefinition"));
    }

    // ── Multiple definitions ────────────────────────────────────────────

    #[test]
    fn multiple_definitions_produce_containers() {
        use sysml_core::Element;

        let mut graph = ModelGraph::new();
        let def1 =
            Element::new_with_kind(ElementKind::ActionDefinition).with_name("Action1");
        let def2 =
            Element::new_with_kind(ElementKind::ActionDefinition).with_name("Action2");
        graph.add_element(def1);
        graph.add_element(def2);

        let gen = ActionFlowViewGenerator;
        let ir = gen.generate(&make_ctx(&graph, &HashSet::new()));

        // Should have container nodes for each definition (order is non-deterministic)
        assert_eq!(ir.nodes.len(), 2);
        let names: Vec<&str> = ir.nodes.iter().map(|n| n.name.as_str()).collect();
        assert!(names.contains(&"Action1"));
        assert!(names.contains(&"Action2"));
        assert!(ir.nodes.iter().all(|n| n.tags.contains(&NodeTag::ActionContainer)));
    }

    // ── generate_for_owner ──────────────────────────────────────────────

    #[test]
    fn generate_for_owner_with_action_definition() {
        use sysml_core::Element;

        let mut graph = ModelGraph::new();
        let def =
            Element::new_with_kind(ElementKind::ActionDefinition).with_name("BrewCoffee");
        let def_id = graph.add_element(def);

        let gen = ActionFlowViewGenerator;
        let ir = gen
            .generate_for_owner(
                &make_ctx(&graph, &HashSet::new()),
                &def_id.to_string(),
            )
            .expect("should produce IR");

        // Even if compile_action fails (no action body), we get the fallback
        assert_eq!(ir.view_type, ViewType::ActionFlow);
    }

    #[test]
    fn generate_for_owner_with_structural_parent() {
        use sysml_core::Element;

        let mut graph = ModelGraph::new();

        // Structural parent (PartDefinition)
        let part =
            Element::new_with_kind(ElementKind::PartDefinition).with_name("Vehicle");
        let part_id = graph.add_element(part);

        // Action child under the part
        let action = Element::new_with_kind(ElementKind::ActionDefinition)
            .with_name("Drive")
            .with_owner(part_id.clone());
        graph.add_element(action);

        let gen = ActionFlowViewGenerator;
        let ir = gen
            .generate_for_owner(
                &make_ctx(&graph, &HashSet::new()),
                &part_id.to_string(),
            )
            .expect("should produce IR");

        // Should have attempted to compile the Drive action
        assert_eq!(ir.view_type, ViewType::ActionFlow);
    }

    // ── Render round-trip ───────────────────────────────────────────────

    #[test]
    fn renders_to_valid_sgraph() {
        let action_ir = ActionGraphIR {
            id: "a1".to_string(),
            name: "Test".to_string(),
            nodes: vec![
                ActionNodeIR::Initial {
                    id: "init".to_string(),
                },
                ActionNodeIR::Decision {
                    id: "d1".to_string(),
                },
                ActionNodeIR::Fork {
                    id: "fork1".to_string(),
                },
                ActionNodeIR::Join {
                    id: "join1".to_string(),
                },
                ActionNodeIR::Send {
                    id: "s1".to_string(),
                    payload: ExprIR::LiteralBool(true),
                    target: "Srv".to_string(),
                    port_target: None,
                },
                ActionNodeIR::Accept {
                    id: "acc1".to_string(),
                    source: None,
                    payload_binding: "msg".to_string(),
                    port_source: None,
                },
                ActionNodeIR::Final {
                    id: "end".to_string(),
                },
            ],
            edges: vec![
                ActionEdgeIR {
                    from: "init".to_string(),
                    to: "d1".to_string(),
                    guard: None,
                },
                ActionEdgeIR {
                    from: "d1".to_string(),
                    to: "fork1".to_string(),
                    guard: Some(ExprIR::LiteralBool(true)),
                },
                ActionEdgeIR {
                    from: "fork1".to_string(),
                    to: "s1".to_string(),
                    guard: None,
                },
                ActionEdgeIR {
                    from: "fork1".to_string(),
                    to: "acc1".to_string(),
                    guard: None,
                },
                ActionEdgeIR {
                    from: "s1".to_string(),
                    to: "join1".to_string(),
                    guard: None,
                },
                ActionEdgeIR {
                    from: "acc1".to_string(),
                    to: "join1".to_string(),
                    guard: None,
                },
                ActionEdgeIR {
                    from: "join1".to_string(),
                    to: "end".to_string(),
                    guard: None,
                },
            ],
            initial_node_id: "init".to_string(),
            final_node_ids: vec!["end".to_string()],
            parameters: vec![],
        };

        let ir = generate_from_action_ir(&action_ir, None);
        let sgraph = render(&ir);
        let json = serde_json::to_string(&sgraph).unwrap();

        // Verify all node types rendered
        assert!(json.contains("node:initialNode"));
        assert!(json.contains("node:decisionNode"));
        assert!(json.contains("node:forkNode"));
        assert!(json.contains("node:joinNode"));
        assert!(json.contains("node:sendAction"));
        assert!(json.contains("node:acceptAction"));
        assert!(json.contains("node:finalNode"));

        // Verify edge type
        assert!(json.contains("edge:flow"));
    }
}
