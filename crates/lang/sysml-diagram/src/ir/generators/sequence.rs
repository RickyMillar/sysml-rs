//! SequenceView IR generator.
//!
//! Produces `DiagramIR` for Sequence Diagrams — lifeline nodes for participants
//! and message edges for flows. Builds the classified
//! [`sysml_runtime::LinkGraph`] (`classify_links_from_graph`) and routes every
//! message-class link ([`LinkClass::routes_as_message`]) into a fixed-layout
//! lifeline diagram with precomputed message routes. PowerBond links (continuous
//! physics) are excluded; declared connectors surface (RSC-3.5c.2b).
//!
//! ## Key features
//!
//! - **Lifeline nodes**: Vertical lanes for each participant, fixed x-position.
//!   Heads carry the contract label `«part» name : Type` (D-N5 / R2-9).
//! - **Proxy nodes**: Small circles on lifelines marking message endpoints.
//!   Every proxy is the endpoint of exactly one message edge — never orphaned.
//! - **Message edges**: Horizontal arrows between proxy nodes with precomputed
//!   routes; the route carries an explicit midpoint so the label sits ON the arrow.
//! - **Self-messages**: Loops back to same lifeline with S-bend routing.
//! - **Fixed layout**: All positions/sizes computed by the generator (no ELK).
//! - **Activation intervals**: Carried in the lifeline's typed `sequence_layout`.
//! - **Add-message buttons**: Between messages on each lifeline.
//! - **Add-lifeline button**: Top-level button to add new lifeline.
//! - **Comment nodes**: Fixed positions relative to lifelines.

use std::collections::{HashMap, HashSet};

use sysml_core::{ElementKind, ModelGraph, RelationshipKind};
use sysml_runtime::flows::FlowEndpoint;
use sysml_runtime::{classify_links_from_graph, LinkClass, LinkGraph};
use tracing::instrument;

use crate::ir::generator::{GeneratorContext, ViewGenerator};
use crate::ir::types::{DiagramIR, DiagramNode, HeaderStyle, NodeLayout, DiagramChild, DiagramButton, DiagramEdge, DiagramEdgeKind, EdgeLabelPlacement, ButtonType, NodeTag, EdgeTag, SequenceNodeLayout};
use crate::view_text;
use crate::ViewType;
use crate::visual_kind::{self as classify, VisualKind};

// ── Layout constants ─────────────────────────────────────────────────────

/// Lifeline head height in pixels.
const HEAD_HEIGHT: f64 = 40.0;

/// Vertical spacing between message occurrence points.
const MESSAGE_SPACING: f64 = 42.0;

/// Horizontal gap between lifeline right edge and next lifeline left edge.
const LIFELINE_GAP: f64 = 80.0;

/// Approximate character width for comment-node text sizing.
const CHAR_WIDTH: f64 = 8.0;

/// Top margin for lifeline heads to prevent clipping at viewport edge.
const LIFELINE_Y_OFFSET: f64 = 10.0;

/// Self-message loop offset to the right of the lifeline center.
const SELF_MSG_LOOP_OFFSET: f64 = 40.0;

/// Self-message vertical drop (target Y below source Y).
const SELF_MSG_DROP: f64 = 20.0;

/// Proxy node size (width and height).
const PROXY_SIZE: f64 = 6.0;

/// Size of add-message buttons.
const BUTTON_SIZE: f64 = 16.0;

/// Size of add-lifeline button.
const ADD_LIFELINE_SIZE: f64 = 24.0;

/// Comment node width.
const COMMENT_WIDTH: f64 = 250.0;

/// Base comment node height (grows with text length).
const COMMENT_BASE_HEIGHT: f64 = 30.0;

/// Approximate height per line of text in a comment node.
const COMMENT_LINE_HEIGHT: f64 = 18.0;

/// Vertical padding between comment nodes.
const COMMENT_PAD: f64 = 12.0;

// ── Generator ────────────────────────────────────────────────────────────

pub struct SequenceViewGenerator;

impl ViewGenerator for SequenceViewGenerator {
    fn view_type(&self) -> ViewType {
        ViewType::Sequence
    }

    fn elk_algorithm(&self) -> &str {
        "fixed"
    }

    fn elk_direction(&self) -> Option<&str> {
        None
    }

    #[instrument(skip_all)]
    fn generate(&self, ctx: &GeneratorContext) -> DiagramIR {
        tracing::info!("SequenceView IR generate");

        // A declared view scopes to its `expose` target: show only the message
        // exchanges among that element's participants, not the whole workspace
        // LinkGraph (which would dump every connector incl. the standard
        // library — thousands of lifelines). Delegate to the owner-scoped path.
        if let Some(expose_id) = ctx.expose_ids.first() {
            return self
                .generate_for_owner(ctx, &expose_id.to_string())
                .unwrap_or_else(|| DiagramIR::new_fixed(ViewType::Sequence));
        }

        let graph = ctx.graph;
        // Sequence is message-driven. Build the classified LinkGraph: connectors
        // surface, PowerBonds (continuous physics) drop. Fall back to deriving
        // edges from Flow/SuccessionFlow/Transition relationships when the
        // classified feed is empty (e.g. transition-only models).
        let (lg, _diags) = classify_links_from_graph(graph);
        let edges = seq_edges_from_link_graph(&lg);
        let ir = if !edges.is_empty() {
            generate_from_flows(&edges, Some(graph))
        } else {
            let rel_edges = flows_from_relationships(graph);
            if !rel_edges.is_empty() {
                generate_from_flows(&rel_edges, Some(graph))
            } else {
                DiagramIR::new_fixed(ViewType::Sequence)
            }
        };
        ir
    }

    #[allow(clippy::indexing_slicing)] // Lifeline indices derived from name lookups in known-length vecs
    fn generate_for_owner(
        &self,
        ctx: &GeneratorContext,
        owner_id: &str,
    ) -> Option<DiagramIR> {
        let graph = ctx.graph;
        let owner_eid = sysml_core::ElementId::from_string(owner_id);

        // Get names of children owned by this element (participants in this scope)
        let owned_names: HashSet<String> = graph
            .children_of(&owner_eid)
            .filter_map(|c| c.name.clone())
            .collect();

        if owned_names.is_empty() {
            return Some(DiagramIR::new_fixed(ViewType::Sequence));
        }

        // Build all message edges from the classified LinkGraph, then filter to
        // those involving owned participants.
        let all_edges = {
            let (lg, _diags) = classify_links_from_graph(graph);
            let edges = seq_edges_from_link_graph(&lg);
            if edges.is_empty() {
                flows_from_relationships(graph)
            } else {
                edges
            }
        };

        let scoped_flows: Vec<_> = all_edges
            .into_iter()
            .filter(|f| {
                owned_names.contains(&f.source.participant)
                    || owned_names.contains(&f.target.participant)
            })
            .collect();

        if scoped_flows.is_empty() {
            return Some(DiagramIR::new_fixed(ViewType::Sequence));
        }

        // Prefix IDs to prevent collisions when embedded
        let mut ir = generate_from_flows(&scoped_flows, Some(graph));
        let prefix = format!("seq-{}/", owner_id);
        prefix_ir_ids(&mut ir, &prefix);
        Some(ir)
    }
}

// ── Core generation from flows ───────────────────────────────────────────

/// A message edge for the sequence generator, decoupled from any runtime IR.
/// Built either from the classified [`LinkGraph`] (production path — drops
/// PowerBonds, surfaces connectors) or from Flow/SuccessionFlow/Transition
/// relationships (the relationship fallback). Replaces the former direct
/// dependency on `FlowConnectionIR` (RSC-3.5c.2b).
#[derive(Debug, Clone)]
pub(crate) struct SeqEdge {
    /// Stable id for node/edge id generation.
    pub id: String,
    /// Source participant + port.
    pub source: FlowEndpoint,
    /// Target participant + port.
    pub target: FlowEndpoint,
    /// Succession-ordered edge.
    pub is_succession: bool,
    /// Spec `FlowTransfer.isMove` — carried verbatim from the link so the
    /// rendered message edge keeps the same value it had pre-migration.
    pub is_move: bool,
    /// Spec `FlowTransfer.isPush` — carried verbatim from the link.
    pub is_push: bool,
    /// Optional payload type label.
    pub payload_type: Option<String>,
}

/// Adapt the classified [`LinkGraph`] into the message edges a sequence diagram
/// shows: every link whose class [`LinkClass::routes_as_message`] (i.e. all but
/// PowerBond). `LinkIR` has no string id, so the originating `element_id` is
/// used as the stable edge id.
pub(crate) fn seq_edges_from_link_graph(lg: &LinkGraph) -> Vec<SeqEdge> {
    lg.iter()
        .filter(|l| l.class.routes_as_message())
        .map(|l| SeqEdge {
            id: l.element_id.to_string(),
            source: FlowEndpoint::new(&l.source.owner, &l.source.port),
            target: FlowEndpoint::new(&l.target.owner, &l.target.port),
            is_succession: l.is_succession,
            is_move: l.is_move,
            is_push: l.is_push,
            payload_type: l.payload_type.clone(),
        })
        .collect()
}

/// Generate a DiagramIR from sequence message edges.
///
/// Each unique participant becomes a lifeline node, and each edge becomes a
/// message edge between lifelines. Proxy occurrence points are added as children
/// of lifeline nodes, and edges connect to proxies.
#[allow(clippy::indexing_slicing)] // Lifeline/slot indices derived from HashMap lookups over known participants
pub(crate) fn generate_from_flows(flows: &[SeqEdge], graph: Option<&ModelGraph>) -> DiagramIR {
    let mut ir = DiagramIR::new_fixed(ViewType::Sequence);

    if flows.is_empty() {
        return ir;
    }

    // Collect unique participants in insertion order
    let mut participants: Vec<String> = Vec::new();
    for flow in flows {
        if !participants.contains(&flow.source.participant) {
            participants.push(flow.source.participant.clone());
        }
        if !participants.contains(&flow.target.participant) {
            participants.push(flow.target.participant.clone());
        }
    }

    // Lifeline head labels per the graphical-notation contract sequence
    // section (D-N5 / R2-9): `«part» name : Type`. Bare participant name when
    // the participant has no backing graph element (flow-only fixtures).
    let participant_labels: HashMap<String, String> = participants
        .iter()
        .map(|name| (name.clone(), lifeline_head_label(name, graph)))
        .collect();

    // Compute lifeline widths from the head labels. This is a renderer-neutral
    // text measurement contract (bold 13px label estimate plus horizontal
    // padding), so x-strides do not depend on a retired graph renderer.
    let participant_widths: HashMap<String, f64> = participants
        .iter()
        .map(|name| {
            let label = &participant_labels[name];
            let width = ((label.chars().count() as f64) * 10.5 + 30.0).max(100.0);
            (name.clone(), width)
        })
        .collect();

    // Compute slot positions: self-messages consume 2 vertical slots
    let mut slot_for_flow: Vec<usize> = Vec::with_capacity(flows.len());
    let mut current_slot: usize = 0;
    for flow in flows {
        slot_for_flow.push(current_slot);
        if flow.source.participant == flow.target.participant {
            current_slot += 2;
        } else {
            current_slot += 1;
        }
    }
    let total_slots = current_slot;

    // Compute uniform lifeline height based on total slots
    let uniform_height = HEAD_HEIGHT + ((total_slots + 1) as f64) * MESSAGE_SPACING;

    // Compute horizontal positions (gap-based, not fixed-stride)
    let mut participant_x: HashMap<String, f64> = HashMap::new();
    {
        let mut x = 0.0;
        for participant in &participants {
            participant_x.insert(participant.clone(), x);
            x += participant_widths[participant] + LIFELINE_GAP;
        }
    }

    // Track activation events per lifeline: (y, is_receive)
    let mut activation_events: HashMap<String, Vec<(f64, bool)>> = HashMap::new();

    // Track proxy nodes per lifeline for later attachment
    let mut proxies_per_participant: HashMap<String, Vec<DiagramNode>> = HashMap::new();

    for (i, flow) in flows.iter().enumerate() {
        let slot = slot_for_flow[i];
        let proxy_y = HEAD_HEIGHT + ((slot + 1) as f64) * MESSAGE_SPACING;
        let is_self = flow.source.participant == flow.target.participant;
        let src_width = participant_widths[&flow.source.participant];
        let tgt_width = participant_widths[&flow.target.participant];

        // Source proxy — centered on lifeline
        let src_proxy = DiagramNode::new(
            format!("proxy:{}:src", i),
            VisualKind::SqProxy,
            "",
        )
        .with_header_style(HeaderStyle::None)
        .with_position(src_width / 2.0 - PROXY_SIZE / 2.0, proxy_y)
        .with_size(PROXY_SIZE, PROXY_SIZE)
        .with_layout(NodeLayout::Free)
        .with_tag(NodeTag::SequenceProxy);

        proxies_per_participant
            .entry(flow.source.participant.clone())
            .or_default()
            .push(src_proxy);

        // Target proxy (offset down for self-messages)
        let tgt_proxy_y = if is_self {
            proxy_y + SELF_MSG_DROP
        } else {
            proxy_y
        };
        let tgt_proxy = DiagramNode::new(
            format!("proxy:{}:tgt", i),
            VisualKind::SqProxy,
            "",
        )
        .with_header_style(HeaderStyle::None)
        .with_position(tgt_width / 2.0 - PROXY_SIZE / 2.0, tgt_proxy_y)
        .with_size(PROXY_SIZE, PROXY_SIZE)
        .with_layout(NodeLayout::Free)
        .with_tag(NodeTag::SequenceProxy);

        proxies_per_participant
            .entry(flow.target.participant.clone())
            .or_default()
            .push(tgt_proxy);

        // Track activation events
        let src_msg_y = proxy_y + PROXY_SIZE / 2.0;
        activation_events
            .entry(flow.source.participant.clone())
            .or_default()
            .push((src_msg_y, false)); // send
        let tgt_msg_y = tgt_proxy_y + PROXY_SIZE / 2.0;
        activation_events
            .entry(flow.target.participant.clone())
            .or_default()
            .push((tgt_msg_y, true)); // receive
    }

    // Compute activation intervals per lifeline (receive starts, send ends)
    let activations_per_participant = compute_activation_intervals(&activation_events);

    // Compute message Y positions per lifeline for insertion-point buttons
    let msg_positions_per_participant =
        compute_msg_positions(flows, &slot_for_flow);

    // Create lifeline nodes
    for participant in &participants {
        let lifeline_width = participant_widths[participant];

        let mut lifeline = DiagramNode::new(
            format!("lifeline:{}", participant),
            VisualKind::Lifeline,
            participant_labels[participant].as_str(),
        )
        .with_header_style(HeaderStyle::Inline)
        .with_position(participant_x[participant], LIFELINE_Y_OFFSET)
        .with_size(lifeline_width, uniform_height)
        .with_layout(NodeLayout::Free)
        // Fixed-layout scenes render lifeline children (the occurrence
        // proxies) unconditionally, so the node IS expanded. Without this the
        // D-B2 scene-consistency pass treats children of non-expanded hosts
        // as not laid out and drops every message edge — the R2-9 "orphan
        // endpoint dots with no arrows" failure.
        .with_expanded(true)
        .with_tag(NodeTag::Lifeline)
        .with_tag(NodeTag::LifelineHead);

        // Source location from the graph
        if let Some(graph) = graph {
            if let Some(element) = graph
                .elements
                .values()
                .find(|e| e.name.as_deref() == Some(participant.as_str()))
            {
                lifeline.tooltip = view_text::tooltip_text(element, graph);
            }
        }

        // Typed sequence layout payload: lifeline width + activation intervals.
        lifeline.sequence_layout = Some(SequenceNodeLayout {
            lifeline_width,
            activations: activations_per_participant
                .get(participant)
                .cloned()
                .unwrap_or_default(),
        });

        // Add proxy nodes as children
        if let Some(proxies) = proxies_per_participant.remove(participant) {
            for proxy in proxies {
                lifeline.children.push(DiagramChild::Node(proxy));
            }
        }

        // Add insertion-point (+) buttons between messages
        add_insertion_buttons(
            &mut lifeline,
            participant,
            lifeline_width,
            uniform_height,
            msg_positions_per_participant.get(participant),
            flows.len(),
        );

        ir.nodes.push(lifeline);
    }

    // Add "new lifeline" button to the right of the last lifeline
    if let Some(last_participant) = participants.last() {
        let last_x = participant_x[last_participant];
        let last_w = participant_widths[last_participant];
        ir.buttons.push(
            DiagramButton::add_lifeline()
                .with_position_size(
                    last_x + last_w + LIFELINE_GAP / 2.0 - ADD_LIFELINE_SIZE / 2.0,
                    HEAD_HEIGHT / 2.0 - ADD_LIFELINE_SIZE / 2.0,
                    ADD_LIFELINE_SIZE,
                    ADD_LIFELINE_SIZE,
                ),
        );
    }

    // Create message edges with routing points
    for (i, flow) in flows.iter().enumerate() {
        let slot = slot_for_flow[i];
        let proxy_y = HEAD_HEIGHT + ((slot + 1) as f64) * MESSAGE_SPACING;
        let msg_y = proxy_y + PROXY_SIZE / 2.0;
        let src_width = participant_widths[&flow.source.participant];
        let tgt_width = participant_widths[&flow.target.participant];
        let src_x = participant_x
            .get(&flow.source.participant)
            .copied()
            .unwrap_or(0.0)
            + src_width / 2.0;
        let tgt_x = participant_x
            .get(&flow.target.participant)
            .copied()
            .unwrap_or(0.0)
            + tgt_width / 2.0;

        let is_self = flow.source.participant == flow.target.participant;

        // Build label text per the contract §B end-label for flows:
        // `UsageDeclaration` + optional `of <payload>` — i.e. the flow's
        // declared name, then the payload type. On the LinkGraph path the
        // SeqEdge id is the originating element's id; recover the name from
        // the graph. Unnamed links (and the relationship-fallback path, whose
        // id is a relationship id) fall back to the source port/step name.
        let flow_name = graph
            .and_then(|g| g.get_element(&sysml_core::ElementId::from_string(&flow.id)))
            .and_then(|e| e.name.clone());
        let mut label_text = flow_name.unwrap_or_else(|| flow.source.port.clone());
        if let Some(ref payload) = flow.payload_type {
            label_text.push_str(&format!(" of {}", payload));
        }

        // Compute routing points
        let route_points = if is_self {
            vec![
                (src_x, msg_y),
                (src_x + SELF_MSG_LOOP_OFFSET, msg_y),
                (src_x + SELF_MSG_LOOP_OFFSET, msg_y + SELF_MSG_DROP),
                (src_x, msg_y + SELF_MSG_DROP),
            ]
        } else {
            // Explicit midpoint so renderers that place the edge label at the
            // middle route point put it ON the arrow (D-N5), not at the
            // sender endpoint.
            vec![
                (src_x, msg_y),
                ((src_x + tgt_x) / 2.0, msg_y),
                (tgt_x, msg_y),
            ]
        };

        // Edge label placement
        let label_side = if is_self {
            "right".to_owned()
        } else if src_x <= tgt_x {
            "left".to_owned()
        } else {
            "right".to_owned()
        };

        let mut edge = DiagramEdge::message(
            format!("message:{}", i),
            format!("proxy:{}:src", i),
            format!("proxy:{}:tgt", i),
            Some(label_text),
            flow.is_succession,
            flow.is_move,
            flow.is_push,
        )
        .with_route(route_points)
        .with_label_placement(EdgeLabelPlacement {
            position: 0.5,
            side: label_side,
            offset: Some(2.0),
            rotate: false,
        });

        // Tag self-message / return edges
        if is_self {
            edge.tags.push(EdgeTag::SelfMessage);
        } else if src_x > tgt_x {
            edge.tags.push(EdgeTag::Return);
        }

        ir.edges.push(edge);
    }

    // Add comment/documentation nodes from the graph — but ONLY those that
    // annotate a participant in this scene (the element itself or an ancestor
    // is a rendered lifeline). Without this scope the loop dumps every comment
    // in the workspace, including the entire standard library's documentation
    // (thousands of nodes).
    if let Some(graph) = graph {
        let participant_set: HashSet<&str> = participants.iter().map(|s| s.as_str()).collect();
        let annotates_participant = |element: &sysml_core::Element| -> bool {
            let mut owner = element.owner.as_ref();
            let mut depth = 0u32;
            while let Some(oid) = owner {
                if depth > 16 {
                    break;
                }
                match graph.get_element(oid) {
                    Some(oe) => {
                        if oe
                            .name
                            .as_deref()
                            .is_some_and(|n| participant_set.contains(n))
                        {
                            return true;
                        }
                        owner = oe.owner.as_ref();
                        depth += 1;
                    }
                    None => break,
                }
            }
            false
        };
        let mut comment_idx = 0u32;
        let last_msg_y = HEAD_HEIGHT + ((total_slots + 1) as f64) * MESSAGE_SPACING;
        let mut comment_y = last_msg_y;
        for element in graph.elements.values() {
            if !matches!(
                element.kind,
                ElementKind::Comment | ElementKind::Documentation
            ) {
                continue;
            }
            if !annotates_participant(element) {
                continue;
            }
            let body = element
                .get_prop("body")
                .or_else(|| element.get_prop("documentation"))
                .and_then(|v| v.as_str())
                .unwrap_or("").to_owned();
            if body.is_empty() {
                continue;
            }

            // Estimate height from text length (approximate chars per line)
            let chars_per_line = (COMMENT_WIDTH / CHAR_WIDTH) as usize;
            let line_count = if chars_per_line > 0 {
                (body.len() / chars_per_line).max(1)
            } else {
                1
            };
            let node_height = COMMENT_BASE_HEIGHT + (line_count as f64) * COMMENT_LINE_HEIGHT;

            let mut comment_node = DiagramNode::new(
                format!("comment:{}", comment_idx),
                VisualKind::Comment,
                body,
            )
            .with_header_style(HeaderStyle::Inline)
            .with_position(10.0, comment_y)
            .with_size(COMMENT_WIDTH, node_height)
            .with_layout(NodeLayout::Free);

            comment_node.tooltip = view_text::tooltip_text(element, graph);

            ir.nodes.push(comment_node);
            comment_y += node_height + COMMENT_PAD;
            comment_idx += 1;
        }
    }

    ir
}

/// Lifeline head text per the graphical-notation contract sequence section
/// (contract §C `sv`; declaration-text keyword per §A.3/A.4, D-N1):
/// `«part» name : Type`. The `: Type` suffix is omitted when the participant
/// is untyped; participants with no backing graph element (or no graph at
/// all — flow-only fixtures) fall back to the bare participant name.
fn lifeline_head_label(participant: &str, graph: Option<&ModelGraph>) -> String {
    let Some(graph) = graph else {
        return participant.to_owned();
    };
    let Some(element) = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some(participant))
    else {
        return participant.to_owned();
    };
    let keyword = classify::element_keyword(&element.kind);
    let type_name = classify::find_type_definition(graph, element)
        .and_then(|t| t.name.clone())
        .or_else(|| {
            element
                .get_prop("unresolved_type")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        });
    match type_name {
        Some(t) => format!("\u{00ab}{keyword}\u{00bb} {participant} : {t}"),
        None => format!("\u{00ab}{keyword}\u{00bb} {participant}"),
    }
}

// ── Flow extraction from relationships ───────────────────────────────────

/// Build `SeqEdge`s from Flow/SuccessionFlow/Transition relationships.
fn flows_from_relationships(graph: &ModelGraph) -> Vec<SeqEdge> {
    let type_to_participant = build_type_to_participant_map(graph);

    let mut flows = Vec::new();
    for rel in graph.relationships.values() {
        let is_succession = matches!(
            rel.kind,
            RelationshipKind::SuccessionFlow | RelationshipKind::Transition
        );
        if !matches!(
            rel.kind,
            RelationshipKind::Flow
                | RelationshipKind::SuccessionFlow
                | RelationshipKind::Transition
        ) {
            continue;
        }

        let src_element = graph.get_element(&rel.source);
        let tgt_element = graph.get_element(&rel.target);

        let source_step_name = src_element
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| rel.source.to_string());
        let target_step_name = tgt_element
            .and_then(|e| e.name.clone())
            .unwrap_or_else(|| rel.target.to_string());

        // Resolve participant via type → usage mapping
        let source_participant = src_element
            .and_then(|e| resolve_participant(graph, e, &type_to_participant))
            .unwrap_or_else(|| source_step_name.clone());
        let target_participant = tgt_element
            .and_then(|e| resolve_participant(graph, e, &type_to_participant))
            .unwrap_or_else(|| target_step_name.clone());

        let payload_type = rel
            .props
            .get("origin_flow")
            .and_then(|v| v.as_ref())
            .and_then(|id| graph.get_element(id))
            .and_then(|e| {
                e.get_prop("payloadType")
                    .and_then(|v| v.as_str())
                    .map(String::from)
            });

        flows.push(SeqEdge {
            id: rel.id.to_string(),
            source: FlowEndpoint::new(&source_participant, &source_step_name),
            target: FlowEndpoint::new(&target_participant, &target_step_name),
            is_succession,
            is_move: false,
            is_push: false,
            payload_type,
        });
    }

    sort_flows_by_chain(&mut flows);
    flows
}

/// Sort flows by following the succession chain.
#[allow(clippy::indexing_slicing)] // Indices from HashMap over `flows` range; always in bounds
fn sort_flows_by_chain(flows: &mut Vec<SeqEdge>) {
    if flows.len() <= 1 {
        return;
    }

    let all_targets: HashSet<&str> = flows
        .iter()
        .map(|f| f.target.port.as_str())
        .collect();
    let first_idx = flows
        .iter()
        .position(|f| !all_targets.contains(f.source.port.as_str()));

    if let Some(start) = first_idx {
        let mut sorted = Vec::with_capacity(flows.len());
        let mut used = vec![false; flows.len()];
        let mut current = start;

        loop {
            if used[current] {
                break;
            }
            used[current] = true;
            sorted.push(flows[current].clone());

            let next_source = &flows[current].target.port;
            match flows
                .iter()
                .enumerate()
                .find(|(i, f)| !used[*i] && f.source.port == *next_source)
            {
                Some((next_idx, _)) => current = next_idx,
                None => break,
            }
        }

        // Append remaining flows not in the chain
        for (i, f) in flows.iter().enumerate() {
            if !used[i] {
                sorted.push(f.clone());
            }
        }

        *flows = sorted;
    }
}

/// Build a map from type definition name → top-level action usage name.
fn build_type_to_participant_map(graph: &ModelGraph) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for element in graph.elements.values() {
        if !element.kind.is_usage() {
            continue;
        }
        if !classify::is_effectively_top_level(element, graph) {
            continue;
        }
        let Some(usage_name) = element.name.as_deref() else { continue };
        if let Some(type_def) = classify::find_type_definition(graph, element) {
            if let Some(def_name) = type_def.name.as_deref() {
                map.insert(def_name.to_owned(), usage_name.to_owned());
            }
        }
    }
    map
}

/// Resolve a step element to its participant name via type definition.
fn resolve_participant(
    graph: &ModelGraph,
    step: &sysml_core::Element,
    type_to_participant: &HashMap<String, String>,
) -> Option<String> {
    let type_def = classify::find_type_definition(graph, step)?;
    let def_name = type_def.name.as_deref()?;
    type_to_participant.get(def_name).cloned()
}

// ── Helper functions ─────────────────────────────────────────────────────

/// Compute activation intervals per lifeline from activation events.
fn compute_activation_intervals(
    activation_events: &HashMap<String, Vec<(f64, bool)>>,
) -> HashMap<String, Vec<(f64, f64)>> {
    let mut result = HashMap::new();
    for (participant, events) in activation_events {
        let mut intervals = Vec::new();
        let mut active_start: Option<f64> = None;
        for &(y, is_receive) in events {
            if is_receive && active_start.is_none() {
                active_start = Some(y);
            } else if !is_receive {
                if let Some(start) = active_start {
                    intervals.push((start, y));
                    active_start = None;
                }
            }
        }
        if let Some(start) = active_start {
            if let Some(&(last_y, _)) = events.last() {
                if last_y > start {
                    intervals.push((start, last_y));
                }
            }
        }
        if !intervals.is_empty() {
            result.insert(participant.clone(), intervals);
        }
    }
    result
}

/// Compute message Y positions per lifeline for insertion-point buttons.
#[allow(clippy::indexing_slicing)] // slot_for_flow indexed in lockstep with flows
fn compute_msg_positions(
    flows: &[SeqEdge],
    slot_for_flow: &[usize],
) -> HashMap<String, Vec<(usize, f64)>> {
    let mut positions: HashMap<String, Vec<(usize, f64)>> = HashMap::new();
    for (i, flow) in flows.iter().enumerate() {
        let slot = slot_for_flow[i];
        let proxy_y = HEAD_HEIGHT + ((slot + 1) as f64) * MESSAGE_SPACING;
        let msg_y = proxy_y + PROXY_SIZE / 2.0;
        let is_self = flow.source.participant == flow.target.participant;
        positions
            .entry(flow.source.participant.clone())
            .or_default()
            .push((i, msg_y));
        if !is_self {
            positions
                .entry(flow.target.participant.clone())
                .or_default()
                .push((i, msg_y));
        } else {
            let tgt_y = msg_y + SELF_MSG_DROP;
            positions
                .entry(flow.target.participant.clone())
                .or_default()
                .push((i, tgt_y));
        }
    }
    // Sort by Y position for correct insertion ordering
    for pos_list in positions.values_mut() {
        pos_list.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
        pos_list.dedup_by_key(|p| p.0);
    }
    positions
}

/// Add insertion-point (+) buttons between messages on a lifeline.
#[allow(clippy::indexing_slicing)] // `.windows(2)` guarantees indices 0 and 1 are valid
fn add_insertion_buttons(
    lifeline: &mut DiagramNode,
    participant: &str,
    lifeline_width: f64,
    uniform_height: f64,
    msg_positions: Option<&Vec<(usize, f64)>>,
    total_flows: usize,
) {
    let btn_x = lifeline_width / 2.0 - BUTTON_SIZE / 2.0;

    if let Some(positions) = msg_positions {
        // Button above the first message
        if let Some(&(_, first_y)) = positions.first() {
            let gap_y = (HEAD_HEIGHT + first_y) / 2.0;
            lifeline.buttons.push(
                DiagramButton::add_message(participant, 0)
                    .with_position_size(btn_x, gap_y - BUTTON_SIZE / 2.0, BUTTON_SIZE, BUTTON_SIZE),
            );
        }
        // Buttons between consecutive messages
        for w in positions.windows(2) {
            let (_, y_above) = w[0];
            let (idx_below, y_below) = w[1];
            let gap_y = (y_above + y_below) / 2.0;
            lifeline.buttons.push(
                DiagramButton::add_message(participant, idx_below)
                    .with_position_size(btn_x, gap_y - BUTTON_SIZE / 2.0, BUTTON_SIZE, BUTTON_SIZE),
            );
        }
        // Button below the last message
        if let Some(&(_, last_y)) = positions.last() {
            let gap_y = (last_y + uniform_height) / 2.0;
            lifeline.buttons.push(
                DiagramButton::add_message(participant, total_flows)
                    .with_position_size(btn_x, gap_y - BUTTON_SIZE / 2.0, BUTTON_SIZE, BUTTON_SIZE),
            );
        }
    } else {
        // No messages on this lifeline — single button in the middle
        let gap_y = (HEAD_HEIGHT + uniform_height) / 2.0;
        lifeline.buttons.push(
            DiagramButton::add_message(participant, 0)
                .with_position_size(btn_x, gap_y - BUTTON_SIZE / 2.0, BUTTON_SIZE, BUTTON_SIZE),
        );
    }
}

/// Prefix all IDs in a DiagramIR for embedding (prevent collisions).
fn prefix_ir_ids(ir: &mut DiagramIR, prefix: &str) {
    for node in &mut ir.nodes {
        prefix_node_ids(node, prefix);
    }
    for edge in &mut ir.edges {
        edge.id = format!("{}{}", prefix, edge.id);
        edge.source_id = format!("{}{}", prefix, edge.source_id);
        edge.target_id = format!("{}{}", prefix, edge.target_id);
        if let Some(ref mut sp) = edge.source_port_id {
            *sp = format!("{}{}", prefix, sp);
        }
        if let Some(ref mut tp) = edge.target_port_id {
            *tp = format!("{}{}", prefix, tp);
        }
    }
    for button in &mut ir.buttons {
        if let ButtonType::AddMessage {
            ref mut lifeline_id,
            ..
        } = button.button_type
        {
            *lifeline_id = format!("{}{}", prefix, lifeline_id);
        }
    }
}

fn prefix_node_ids(node: &mut DiagramNode, prefix: &str) {
    node.element_id = format!("{}{}", prefix, node.element_id);
    for child in &mut node.children {
        if let DiagramChild::Node(ref mut child_node) = child {
            prefix_node_ids(child_node, prefix);
        }
    }
    for port in &mut node.ports {
        port.element_id = format!("{}{}", prefix, port.element_id);
    }
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind, ModelGraph, Relationship};
    use sysml_runtime::flows::FlowEndpoint;
    use std::collections::HashSet;

    static EMPTY_SET: std::sync::LazyLock<HashSet<String>> =
        std::sync::LazyLock::new(HashSet::new);

    fn make_ctx(graph: &ModelGraph) -> GeneratorContext {
        GeneratorContext::new(graph, &EMPTY_SET)
    }

    fn two_flow_fixture() -> Vec<SeqEdge> {
        vec![
            SeqEdge {
                id: "f1".into(),
                source: FlowEndpoint::new("A", "request"),
                target: FlowEndpoint::new("B", "handle"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
            },
            SeqEdge {
                id: "f2".into(),
                source: FlowEndpoint::new("B", "response"),
                target: FlowEndpoint::new("A", "receive"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: Some("Data".into()),
            },
        ]
    }

    /// Helper: find lifeline nodes in IR.
    fn lifeline_nodes(ir: &DiagramIR) -> Vec<&DiagramNode> {
        ir.nodes
            .iter()
            .filter(|n| n.element_id.starts_with("lifeline:"))
            .collect()
    }

    /// Helper: find proxy children inside a lifeline node.
    fn proxy_children(node: &DiagramNode) -> Vec<&DiagramNode> {
        node.children
            .iter()
            .filter_map(|c| match c {
                DiagramChild::Node(n)
                    if n.tags.contains(&NodeTag::SequenceProxy) =>
                {
                    Some(n)
                }
                _ => None,
            })
            .collect()
    }

    // ── Basic generation ────────────────────────────────────────────────

    #[test]
    fn sequence_ir_empty_produces_fixed_layout() {
        let graph = ModelGraph::new();
        let gen = SequenceViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate(&ctx);

        assert_eq!(ir.view_type, ViewType::Sequence);
        assert!(ir.nodes.is_empty());
        assert!(ir.edges.is_empty());
    }

    #[test]
    fn sequence_ir_view_type_and_algorithm() {
        let gen = SequenceViewGenerator;
        assert_eq!(gen.view_type(), ViewType::Sequence);
        assert_eq!(gen.elk_algorithm(), "fixed");
        assert_eq!(gen.elk_direction(), None);
    }

    #[test]
    fn sequence_ir_from_two_flows() {
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);

        assert_eq!(ir.view_type, ViewType::Sequence);

        let lifelines = lifeline_nodes(&ir);
        assert_eq!(lifelines.len(), 2, "expected 2 lifelines");

        assert_eq!(ir.edges.len(), 2, "expected 2 message edges");

        // Verify lifeline IDs
        assert!(lifelines.iter().any(|n| n.element_id == "lifeline:A"));
        assert!(lifelines.iter().any(|n| n.element_id == "lifeline:B"));
    }

    #[test]
    fn sequence_ir_empty_flows() {
        let flows: Vec<SeqEdge> = vec![];
        let ir = generate_from_flows(&flows, None);
        assert!(ir.nodes.is_empty());
        assert!(ir.edges.is_empty());
    }

    // ── Lifeline properties ─────────────────────────────────────────────

    #[test]
    fn sequence_ir_lifeline_type_and_css() {
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);

        for ll in &lifelines {
            assert_eq!(ll.visual_kind, VisualKind::Lifeline);
            assert!(
                ll.tags.contains(&NodeTag::LifelineHead),
                "lifeline should have LifelineHead tag"
            );
        }
    }

    #[test]
    fn sequence_ir_lifeline_head_label_with_graph() {
        // Participant backed by a typed PartUsage → head label carries the
        // declaration-text keyword and the type: `«part» engine : Engine`
        // (contract sequence section, D-N5 / R2-9).
        let mut graph = ModelGraph::new();
        graph.add_element(
            Element::new_with_kind(ElementKind::PartDefinition).with_name("Engine"),
        );
        let mut engine = Element::new_with_kind(ElementKind::PartUsage).with_name("engine");
        engine.set_prop(
            "unresolved_type",
            sysml_core::Value::String("Engine".into()),
        );
        graph.add_element(engine);

        let flows = vec![SeqEdge {
            id: "f1".into(),
            source: FlowEndpoint::new("engine", "torqueOut"),
            target: FlowEndpoint::new("gearbox", "torqueIn"),
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: Some("Torque".into()),
        }];
        let ir = generate_from_flows(&flows, Some(&graph));
        let lifelines = lifeline_nodes(&ir);

        let e = lifelines
            .iter()
            .find(|n| n.element_id == "lifeline:engine")
            .unwrap();
        assert_eq!(e.name, "\u{00ab}part\u{00bb} engine : Engine");

        // No backing element → bare-name fallback, never a fabricated type.
        let g = lifelines
            .iter()
            .find(|n| n.element_id == "lifeline:gearbox")
            .unwrap();
        assert_eq!(g.name, "gearbox");
    }

    #[test]
    fn sequence_ir_lifelines_expanded() {
        // Lifeline children (occurrence proxies) are always rendered in the
        // fixed layout, so the node must declare `expanded` — otherwise the
        // D-B2 scene-consistency pass drops every message edge (R2-9).
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);
        for ll in lifeline_nodes(&ir) {
            assert_eq!(ll.expanded, Some(true));
        }
    }

    #[test]
    fn sequence_ir_lifeline_height() {
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);

        // Uniform height = HEAD_HEIGHT + (total_messages+1)*MESSAGE_SPACING
        // = 40 + (2+1)*42 = 166. Width = estimate_node_size floor (100).
        let a = lifelines.iter().find(|n| n.element_id == "lifeline:A").unwrap();
        assert_eq!(a.size, Some((100.0, 166.0)));

        let b = lifelines.iter().find(|n| n.element_id == "lifeline:B").unwrap();
        assert_eq!(b.size, Some((100.0, 166.0)));
    }

    #[test]
    fn sequence_ir_lifeline_positioning() {
        let flows = vec![
            SeqEdge {
                id: "f1".into(),
                source: FlowEndpoint::new("Alpha", "out"),
                target: FlowEndpoint::new("Beta", "in"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
            },
            SeqEdge {
                id: "f2".into(),
                source: FlowEndpoint::new("Beta", "out"),
                target: FlowEndpoint::new("Gamma", "in"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
            },
        ];

        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);

        assert_eq!(lifelines.len(), 3);
        assert_eq!(lifelines[0].position, Some((0.0, LIFELINE_Y_OFFSET)));
        // "Alpha" (5 chars) → 100px floor, gap 80px → second at 180
        assert_eq!(lifelines[1].position, Some((180.0, LIFELINE_Y_OFFSET)));
        // "Beta" (4 chars) → 100px floor, gap 80px → third at 360
        assert_eq!(lifelines[2].position, Some((360.0, LIFELINE_Y_OFFSET)));
    }

    #[test]
    fn sequence_ir_auto_width_lifelines() {
        let flows = vec![SeqEdge {
            id: "f1".into(),
            source: FlowEndpoint::new("ShortName", "out"),
            target: FlowEndpoint::new("AVeryLongParticipantName", "in"),
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
        }];

        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);

        let short = lifelines
            .iter()
            .find(|n| n.element_id == "lifeline:ShortName")
            .unwrap();
        let long = lifelines
            .iter()
            .find(|n| n.element_id == "lifeline:AVeryLongParticipantName")
            .unwrap();

        // "ShortName" (9 chars): max(9*10.5+30, 100) = 124.5
        assert_eq!(short.size.unwrap().0, 124.5);

        // "AVeryLongParticipantName" (24 chars): max(24*10.5+30, 100) = 282
        assert_eq!(long.size.unwrap().0, 282.0);

        // Positions: short at 0, long at 124.5 + 80 (gap) = 204.5
        assert_eq!(short.position.unwrap().0, 0.0);
        assert_eq!(long.position.unwrap().0, 204.5);
    }

    // ── Proxy nodes ─────────────────────────────────────────────────────

    #[test]
    fn sequence_ir_proxy_points() {
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);

        let a = lifelines.iter().find(|n| n.element_id == "lifeline:A").unwrap();
        let a_proxies = proxy_children(a);
        assert_eq!(
            a_proxies.len(),
            2,
            "A should have 2 proxies (src of f1, tgt of f2)"
        );

        // All proxies should be 6x6
        for p in &a_proxies {
            assert_eq!(p.size, Some((PROXY_SIZE, PROXY_SIZE)));
        }
    }

    #[test]
    fn sequence_ir_proxy_vertical_ordering() {
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);

        let a = lifelines.iter().find(|n| n.element_id == "lifeline:A").unwrap();
        let a_proxies = proxy_children(a);

        // Message 0 proxy at y = 40 + 1*42 = 82
        let proxy_0_src = a_proxies
            .iter()
            .find(|p| p.element_id == "proxy:0:src")
            .unwrap();
        assert_eq!(proxy_0_src.position.unwrap().1, 82.0);

        // Message 1 proxy at y = 40 + 2*42 = 124
        let proxy_1_tgt = a_proxies
            .iter()
            .find(|p| p.element_id == "proxy:1:tgt")
            .unwrap();
        assert_eq!(proxy_1_tgt.position.unwrap().1, 124.0);
    }

    // ── Message edges ───────────────────────────────────────────────────

    #[test]
    fn sequence_ir_message_edges_connect_proxies() {
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);

        assert_eq!(ir.edges[0].source_id, "proxy:0:src");
        assert_eq!(ir.edges[0].target_id, "proxy:0:tgt");
        assert_eq!(ir.edges[1].source_id, "proxy:1:src");
        assert_eq!(ir.edges[1].target_id, "proxy:1:tgt");
    }

    #[test]
    fn sequence_ir_succession_edge() {
        let flows = vec![SeqEdge {
            id: "sf1".into(),
            source: FlowEndpoint::new("Step1", "out"),
            target: FlowEndpoint::new("Step2", "in"),
            is_succession: true,
            is_move: false,
            is_push: false,
            payload_type: None,
        }];

        let ir = generate_from_flows(&flows, None);
        assert_eq!(ir.edges.len(), 1);
        match &ir.edges[0].kind {
            DiagramEdgeKind::Message { is_succession, .. } => {
                assert!(is_succession, "should be succession edge");
            }
            _ => panic!("expected Message edge kind"),
        }
    }

    #[test]
    fn sequence_ir_message_label_format() {
        let flows = vec![
            SeqEdge {
                id: "f1".into(),
                source: FlowEndpoint::new("A", "request"),
                target: FlowEndpoint::new("B", "handle"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
            },
            SeqEdge {
                id: "f2".into(),
                source: FlowEndpoint::new("B", "response"),
                target: FlowEndpoint::new("A", "receive"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: Some("Data".into()),
            },
        ];

        let ir = generate_from_flows(&flows, None);

        // Edge 0: payload is "request"
        match &ir.edges[0].kind {
            DiagramEdgeKind::Message { payload, .. } => {
                assert_eq!(payload.as_deref(), Some("request"));
            }
            _ => panic!("expected Message edge"),
        }

        // Edge 1: payload is "response of Data"
        match &ir.edges[1].kind {
            DiagramEdgeKind::Message { payload, .. } => {
                assert_eq!(payload.as_deref(), Some("response of Data"));
            }
            _ => panic!("expected Message edge"),
        }
    }

    #[test]
    fn sequence_ir_precomputed_routes() {
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);

        // Normal message: 3 routing points (src, midpoint for the label, tgt)
        assert_eq!(ir.edges[0].precomputed_route.as_ref().unwrap().len(), 3);
        // Return message: 3 routing points
        assert_eq!(ir.edges[1].precomputed_route.as_ref().unwrap().len(), 3);
        // The middle point sits halfway between the endpoints (label anchor).
        let route = ir.edges[0].precomputed_route.as_ref().unwrap();
        assert_eq!(route[1].0, (route[0].0 + route[2].0) / 2.0);
        assert_eq!(route[1].1, route[0].1);
    }

    // ── Self-messages ───────────────────────────────────────────────────

    #[test]
    fn sequence_ir_self_message_routing() {
        let flows = vec![
            SeqEdge {
                id: "f1".into(),
                source: FlowEndpoint::new("A", "request"),
                target: FlowEndpoint::new("B", "handle"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
            },
            SeqEdge {
                id: "f2".into(),
                source: FlowEndpoint::new("B", "process"),
                target: FlowEndpoint::new("B", "done"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
            },
        ];

        let ir = generate_from_flows(&flows, None);

        // First edge: normal horizontal message (3 routing points incl. midpoint)
        assert_eq!(ir.edges[0].precomputed_route.as_ref().unwrap().len(), 3);
        assert!(!ir.edges[0].tags.contains(&EdgeTag::SelfMessage));

        // Second edge: self-message loopback (4 routing points)
        assert_eq!(ir.edges[1].precomputed_route.as_ref().unwrap().len(), 4);
        assert!(ir.edges[1].tags.contains(&EdgeTag::SelfMessage));
    }

    #[test]
    fn sequence_ir_self_message_target_proxy_offset() {
        let flows = vec![SeqEdge {
            id: "f1".into(),
            source: FlowEndpoint::new("X", "call"),
            target: FlowEndpoint::new("X", "done"),
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
        }];

        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);
        let x = lifelines
            .iter()
            .find(|n| n.element_id == "lifeline:X")
            .unwrap();
        let proxies = proxy_children(x);
        assert_eq!(proxies.len(), 2, "self-message creates 2 proxies");

        let src = proxies.iter().find(|p| p.element_id == "proxy:0:src").unwrap();
        let tgt = proxies.iter().find(|p| p.element_id == "proxy:0:tgt").unwrap();
        let src_y = src.position.unwrap().1;
        let tgt_y = tgt.position.unwrap().1;
        assert!(
            (tgt_y - src_y - SELF_MSG_DROP).abs() < 0.001,
            "target proxy should be {} below source, got delta={}",
            SELF_MSG_DROP,
            tgt_y - src_y
        );
    }

    #[test]
    fn sequence_ir_self_message_consumes_two_slots() {
        let flows = vec![
            SeqEdge {
                id: "f1".into(),
                source: FlowEndpoint::new("A", "request"),
                target: FlowEndpoint::new("B", "handle"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
            },
            SeqEdge {
                id: "f2".into(),
                source: FlowEndpoint::new("B", "process"),
                target: FlowEndpoint::new("B", "done"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
            },
        ];

        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);

        // 1 normal + 1 self = 3 slots total
        let expected_height = HEAD_HEIGHT + 4.0 * MESSAGE_SPACING;
        let b = lifelines
            .iter()
            .find(|n| n.element_id == "lifeline:B")
            .unwrap();
        assert_eq!(b.size.unwrap().1, expected_height);
    }

    // ── Activation intervals ────────────────────────────────────────────

    #[test]
    fn sequence_ir_activation_boxes() {
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);

        // B receives then sends → should have activation interval
        let b = lifelines
            .iter()
            .find(|n| n.element_id == "lifeline:B")
            .unwrap();
        let b_intervals = &b
            .sequence_layout
            .as_ref()
            .expect("B should have sequence layout")
            .activations;
        assert_eq!(b_intervals.len(), 1);
        assert!(b_intervals[0].1 > b_intervals[0].0);

        // A sends then receives with no subsequent send → no activations
        let a = lifelines
            .iter()
            .find(|n| n.element_id == "lifeline:A")
            .unwrap();
        assert!(
            a.sequence_layout
                .as_ref()
                .map(|sl| sl.activations.is_empty())
                .unwrap_or(true),
            "A should have no activations"
        );
    }

    // ── Buttons ─────────────────────────────────────────────────────────

    #[test]
    fn sequence_ir_insertion_buttons_per_lifeline() {
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);

        // Each lifeline participates in 2 messages → 3 insertion gaps
        let a = lifelines
            .iter()
            .find(|n| n.element_id == "lifeline:A")
            .unwrap();
        let a_buttons: Vec<_> = a.buttons.iter().collect();
        assert_eq!(
            a_buttons.len(),
            3,
            "A has 2 messages → 3 insertion buttons"
        );
        for btn in &a_buttons {
            match &btn.button_type {
                ButtonType::AddMessage { lifeline_id, .. } => {
                    assert_eq!(lifeline_id, "A");
                }
                _ => panic!("expected AddMessage button"),
            }
        }
    }

    #[test]
    fn sequence_ir_add_lifeline_button() {
        let flows = two_flow_fixture();
        let ir = generate_from_flows(&flows, None);

        let add_ll = ir
            .buttons
            .iter()
            .find(|b| matches!(b.button_type, ButtonType::AddLifeline));
        assert!(add_ll.is_some(), "should have an add-lifeline button");

        let btn = add_ll.unwrap();
        assert!(btn.position.is_some());
        assert!(btn.position.unwrap().0 > 0.0, "should be to the right");
    }

    // ── Deduplication ───────────────────────────────────────────────────

    #[test]
    fn sequence_ir_dedup_participants() {
        let flows = vec![
            SeqEdge {
                id: "f1".into(),
                source: FlowEndpoint::new("X", "p1"),
                target: FlowEndpoint::new("Y", "p2"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
            },
            SeqEdge {
                id: "f2".into(),
                source: FlowEndpoint::new("X", "p3"),
                target: FlowEndpoint::new("Y", "p4"),
                is_succession: false,
                is_move: false,
                is_push: false,
                payload_type: None,
            },
        ];

        let ir = generate_from_flows(&flows, None);
        let lifelines = lifeline_nodes(&ir);
        assert_eq!(lifelines.len(), 2, "participants should be deduplicated");
    }

    // (removed `sequence_ir_source_locations` — node source was dropped in 3.15;
    //  source spans now live only in the ViewModel text-map.)

    // ── Render round-trip ───────────────────────────────────────────────



    // ── generate_for_owner ──────────────────────────────────────────────

    #[test]
    fn generate_for_owner_empty_when_no_children() {
        let mut graph = ModelGraph::new();
        let owner = Element::new_with_kind(ElementKind::PartDefinition).with_name("Owner");
        let owner_id = graph.add_element(owner);

        let gen = SequenceViewGenerator;
        let ctx = make_ctx(&graph);
        let ir = gen.generate_for_owner(&ctx, &owner_id.to_string());
        assert!(ir.is_some());
        let ir = ir.unwrap();
        // No flows → empty (except layout algorithm)
        assert!(ir.nodes.is_empty() || ir.edges.is_empty());
    }

    #[test]
    fn generate_delegates_to_owner_scope_when_exposed() {
        // With `expose` set, `generate()` must scope to that owner's
        // participants — NOT classify the whole graph. An owner with no
        // children yields an empty scene, proving the whole-graph path was
        // not taken.
        let mut graph = ModelGraph::new();
        let owner = Element::new_with_kind(ElementKind::PartDefinition).with_name("Owner");
        let owner_id = graph.add_element(owner);

        let ctx = GeneratorContext::new(&graph, &EMPTY_SET).with_expose(&owner_id);
        let ir = SequenceViewGenerator.generate(&ctx);
        assert!(ir.nodes.is_empty(), "exposed empty owner → no lifelines (scoped, not whole-graph)");
        assert_eq!(ir.view_type, ViewType::Sequence);
    }

    // ── ID prefixing ────────────────────────────────────────────────────

    #[test]
    fn prefix_ir_ids_scopes_all_ids() {
        let flows = two_flow_fixture();
        let mut ir = generate_from_flows(&flows, None);
        let prefix = "seq-owner123/";
        prefix_ir_ids(&mut ir, prefix);

        // All node IDs should be prefixed
        for node in &ir.nodes {
            assert!(
                node.element_id.starts_with(prefix),
                "Node ID '{}' should start with '{}'",
                node.element_id,
                prefix
            );
        }
        // All edge IDs + source/target should be prefixed
        for edge in &ir.edges {
            assert!(
                edge.id.starts_with(prefix),
                "Edge ID '{}' should start with '{}'",
                edge.id,
                prefix
            );
            assert!(
                edge.source_id.starts_with(prefix),
                "Edge source_id '{}' should start with '{}'",
                edge.source_id,
                prefix
            );
            assert!(
                edge.target_id.starts_with(prefix),
                "Edge target_id '{}' should start with '{}'",
                edge.target_id,
                prefix
            );
        }
    }

    // ── Comment nodes ───────────────────────────────────────────────────

    #[test]
    fn sequence_ir_includes_comments() {
        // A comment renders only when it annotates a participant in the scene
        // (the element itself or an ancestor is a rendered lifeline). A comment
        // owned by participant "A" renders; a free-floating comment that
        // annotates nothing does NOT (otherwise the whole workspace's — and the
        // standard library's — comments flood every sequence diagram).
        let mut graph = ModelGraph::new();
        let part_a = Element::new_with_kind(ElementKind::PartUsage).with_name("A");
        let a_id = part_a.id.clone();
        graph.add_element(part_a);

        let mut owned = Element::new_with_kind(ElementKind::Comment);
        owned.set_prop("body", sysml_core::Value::String("A test comment".into()));
        owned.owner = Some(a_id);
        graph.add_element(owned);

        let mut orphan = Element::new_with_kind(ElementKind::Comment);
        orphan.set_prop("body", sysml_core::Value::String("unscoped".into()));
        graph.add_element(orphan);

        let flows = vec![SeqEdge {
            id: "f1".into(),
            source: FlowEndpoint::new("A", "out"),
            target: FlowEndpoint::new("B", "in"),
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
        }];

        let ir = generate_from_flows(&flows, Some(&graph));

        let comment_nodes: Vec<_> = ir
            .nodes
            .iter()
            .filter(|n| n.element_id.starts_with("comment:"))
            .collect();
        assert_eq!(comment_nodes.len(), 1, "only the participant-owned comment renders");
        assert_eq!(comment_nodes[0].visual_kind, VisualKind::Comment);
        assert_eq!(comment_nodes[0].name, "A test comment");
    }

    // ── RSC-3.5c.2b: classified LinkGraph → SeqEdge routing ─────────────

    /// One link of each class. The sequence view shows discrete messages, so
    /// SignalLink + MessageChannel + Unknown route as edges and PowerBond
    /// (continuous physics) is dropped. Connectors (ConnectionUsage) surface
    /// whenever their class routes as a message.
    #[test]
    fn seq_edges_drops_power_bond_keeps_messages_and_connectors() {
        use sysml_core::physics::classify::ClassificationConfidence;
        use sysml_core::ElementId;
        use sysml_runtime::{LinkEndpoint, LinkIR, LinkSourceKind};

        let mk = |id: &str, kind: LinkSourceKind, class: LinkClass| LinkIR {
            element_id: ElementId::from_string(id),
            kind,
            source: LinkEndpoint {
                element_id: None,
                owner: format!("{id}_src"),
                port: "p".into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: format!("{id}_tgt"),
                port: "q".into(),
                resolved_registry_key: None,
            },
            class,
            class_confidence: ClassificationConfidence::Declared,
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        };

        let mut lg = LinkGraph::new();
        // A message-flow (FlowUsage → SignalLink).
        lg.intern(mk("flowmsg", LinkSourceKind::FlowUsage, LinkClass::SignalLink));
        // A declared connector (ConnectionUsage → MessageChannel) — surfaces.
        lg.intern(mk("conn", LinkSourceKind::ConnectionUsage, LinkClass::MessageChannel));
        // A power bond (continuous physics) — must be dropped.
        lg.intern(mk("power", LinkSourceKind::ConnectionUsage, LinkClass::PowerBond));

        let edges = seq_edges_from_link_graph(&lg);
        // Owners round-trip as plain strings (unlike the hashed element-id);
        // identify each edge by its source owner.
        let owners: HashSet<&str> = edges.iter().map(|e| e.source.participant.as_str()).collect();

        assert_eq!(edges.len(), 2, "PowerBond must be dropped from sequence edges");
        assert!(owners.contains("flowmsg_src"), "message flow edge must be present");
        assert!(owners.contains("conn_src"), "declared connector edge must surface");
        assert!(!owners.contains("power_src"), "power-bond edge must be absent");
    }
}
