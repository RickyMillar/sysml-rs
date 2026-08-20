//! DiagramIR → SGraph renderer.
//!
//! All Sprotty-specific construction lives here:
//! - Type strings, CSS classes, ELK layout options
//! - Header compartments, stereotypes, expand buttons
//! - Size estimation, island wrapping
//! - Connection ports for parallel edge separation
//!
//! Generators never touch SModel types — they produce `DiagramIR`.

use std::collections::HashMap;

use crate::smodel::builders;
use crate::smodel::types::{SGraph, SModelElement, add_connection_ports, SNode, SLabel, SCompartment, SButton, Dimension, default_node_layout_options, estimate_node_size, estimate_node_size_ex, DEFAULT_PADDING_TOP, DEFAULT_PADDING_BOTTOM, Point, SPort, SEdge, EdgePlacement};
use crate::smodel::ViewType;
use crate::visual_kind::{self as classify, CompartmentKind};

use super::rendering_hints::RenderingHints;
use super::types::{DiagramIR, DiagramNode, HeaderStyle, DiagramChild, NodeLayout, NodeKind, DiagramPort, PortDirection, PortSide, DiagramEdge, DiagramEdgeKind, EdgeLabelPlacement, EndpointMode, DiagramButton, ButtonType, NodeTag, EdgeTag, EdgeSubLabelKind, SolverStatus, CompartmentItemSource, SequenceNodeLayout, PortTag};

/// Render a DiagramIR into a Sprotty SGraph.
///
/// This is the **Sprotty adapter**: it is the single place that maps the
/// renderer-agnostic, ELK-free [`DiagramIR`] wire format into Sprotty-specific
/// SModel — CSS classes (from the typed `node_kind`/`tags`/…), ELK layout
/// options (derived per `view_type`), type strings, and size estimation.
pub fn render(ir: &DiagramIR) -> SGraph {
    render_with(ir, None)
}

/// Render with caller-supplied ELK layout overrides (the `ViewRequest` hints).
/// The base graph options are derived from `ir.view_type`; `hints` overlay them.
pub fn render_with(ir: &DiagramIR, hints: Option<&RenderingHints>) -> SGraph {
    let mut counter = 0u32;
    render_inner(ir, &mut counter, hints)
}

/// Base graph-level ELK options for a view type. Reproduces what the generators
/// used to bake into `DiagramIR::{layout_algorithm, graph_layout_options}` before
/// the wire format was stripped of ELK strings (Bucket 1.2). The Sprotty adapter
/// owns this mapping; other renderers ignore it.
fn base_graph_layout_options(view_type: ViewType) -> HashMap<String, String> {
    let mut opts = HashMap::new();
    match view_type {
        ViewType::ActionFlow => {
            opts.insert("elk.algorithm".to_owned(), "org.eclipse.elk.layered".to_owned());
            opts.insert("elk.direction".to_owned(), "DOWN".to_owned());
            opts.insert("elk.portConstraints".to_owned(), "FIXED_SIDE".to_owned());
            opts.insert("elk.spacing.nodeNode".to_owned(), "40".to_owned());
            opts.insert("elk.layered.spacing.nodeNodeBetweenLayers".to_owned(), "50".to_owned());
            opts.insert("elk.layered.spacing.edgeNodeBetweenLayers".to_owned(), "30".to_owned());
            opts.insert("elk.layered.considerModelOrder.strategy".to_owned(), "NODES_AND_EDGES".to_owned());
            opts.insert("elk.layered.nodePlacement.strategy".to_owned(), "BRANDES_KOEPF".to_owned());
            opts.insert("elk.layered.nodePlacement.bk.fixedAlignment".to_owned(), "BALANCED".to_owned());
            opts.insert("elk.layered.compactness.connectedComponents".to_owned(), "true".to_owned());
        }
        ViewType::Grid | ViewType::Sequence | ViewType::Geometry => {
            opts.insert("elk.algorithm".to_owned(), "org.eclipse.elk.fixed".to_owned());
        }
        // General, Interconnection, StateTransition, Requirements, Browser,
        // Parametric: no graph-level overrides — the TS ILayoutConfigurator
        // supplies the defaults.
        _ => {}
    }
    opts
}

/// Apply caller hint overrides onto a base ELK option map (mirrors the former
/// `GeneratorContext::apply_hints`, but at the adapter boundary).
fn apply_hint_overrides(opts: &mut HashMap<String, String>, hints: Option<&RenderingHints>) {
    let Some(h) = hints else { return };
    if let Some(algo) = h.algorithm.as_ref() {
        opts.insert("elk.algorithm".to_owned(), algo.clone());
    }
    if let Some(dir) = h.direction.as_ref() {
        opts.insert("elk.direction".to_owned(), dir.clone());
    }
    if let Some(spacing) = h.spacing_node_node.as_ref() {
        opts.insert("elk.spacing.nodeNode".to_owned(), spacing.clone());
    }
    for (k, v) in &h.extra {
        opts.insert(k.clone(), v.clone());
    }
}

// ── Typed semantic field → Sprotty CSS class mappings ─────────────────────
//
// The IR carries renderer-agnostic typed enums; the Sprotty adapter is the
// single place that names CSS classes. Other renderers map the same enums to
// their own styling.

/// CSS classes for a node, derived from its typed semantic fields.
fn node_css_classes(node: &DiagramNode) -> Vec<String> {
    let mut classes = vec![node.visual_kind.css_class().to_owned()];
    let mut push = |c: String, classes: &mut Vec<String>| {
        if !classes.contains(&c) {
            classes.push(c);
        }
    };

    // Element-kind-derived classes (lowercased kind, definition/usage,
    // visual-kind css, reference) — reproduces the former
    // `classify::element_css_classes` that generators pushed into `css_extras`.
    if let Some(kind) = &node.element_kind {
        for c in classify::element_css_classes(kind) {
            push(c, &mut classes);
        }
    }

    // Authoritative definition/usage nature.
    match node.node_kind {
        NodeKind::Definition => push("definition".to_owned(), &mut classes),
        NodeKind::Usage => push("usage".to_owned(), &mut classes),
        NodeKind::Neutral => {}
    }

    // Semantic decoration tags.
    for tag in &node.tags {
        push(node_tag_class(*tag).to_owned(), &mut classes);
    }

    // Solver satisfaction badge (parametric).
    if let Some(status) = node.solver_status {
        push(format!("parametric-{}", solver_status_str(status)), &mut classes);
    }

    // Fallback definition/usage from stereotype text (covers synthetic nodes
    // that carry no element_kind/node_kind but do carry stereotype text).
    if node.stereotype.contains("definition") {
        push("definition".to_owned(), &mut classes);
    } else if node.stereotype.contains("usage") {
        push("usage".to_owned(), &mut classes);
    }

    // Diagnostic severity overlay.
    if let Some(ref severity) = node.diagnostic_severity {
        classes.push(format!("diagnostic-{}", severity));
    }

    classes
}

/// Map a node semantic tag to its Sprotty CSS class.
fn node_tag_class(tag: NodeTag) -> &'static str {
    match tag {
        NodeTag::Actor => "actor",
        NodeTag::ActionContainer => "action-container",
        NodeTag::BrowserNode => "browser-node",
        NodeTag::GridColumn => "grid-col",
        NodeTag::GridRow => "grid-row",
        NodeTag::GridCell => "grid-cell",
        NodeTag::NaryDot => "nary-dot",
        NodeTag::MaxDepth => "max-depth",
        NodeTag::SubmachineRef => "ref",
        NodeTag::ExhibitState => "exhibit-state",
        NodeTag::ParallelRegions => "has-parallel-regions",
        NodeTag::IfNode => "if-node",
        NodeTag::Perform => "perform",
        NodeTag::Assign => "assign",
        NodeTag::LoopNode => "loop-node",
        NodeTag::LoopWhile => "while",
        NodeTag::LoopFor => "for",
        NodeTag::StreamSource => "stream-source",
        NodeTag::ParametricConstraint => "parametric-constraint",
        NodeTag::AssumeConstraint => "assume-constraint",
        NodeTag::RequireConstraint => "require-constraint",
        NodeTag::FrameConcern => "frame-concern",
        NodeTag::VerifyRequirement => "verify-requirement",
        NodeTag::SequenceProxy => "sq-proxy",
        NodeTag::Lifeline => "lifeline",
        NodeTag::LifelineHead => "lifeline-head",
    }
}

fn solver_status_str(status: SolverStatus) -> &'static str {
    match status {
        SolverStatus::Pass => "pass",
        SolverStatus::Fail => "fail",
        SolverStatus::Unknown => "unknown",
    }
}

/// Map an edge semantic tag to its Sprotty CSS class.
fn edge_tag_class(tag: EdgeTag) -> &'static str {
    match tag {
        EdgeTag::BindingConnector => "binding-connector",
        EdgeTag::SelfMessage => "self-message",
        EdgeTag::Return => "return",
        EdgeTag::Comment => "comment",
        EdgeTag::Message => "message",
        EdgeTag::NarySegment => "nary-segment",
    }
}

fn port_tag_class(tag: PortTag) -> &'static str {
    match tag {
        PortTag::Control => "control-port",
        PortTag::Parametric => "parametric-port",
        PortTag::BadgeSolved => "param-solved",
        PortTag::BadgeUnsolved => "param-unsolved",
        PortTag::BadgeViolated => "param-violated",
    }
}

fn edge_sublabel_class(kind: EdgeSubLabelKind) -> &'static str {
    match kind {
        EdgeSubLabelKind::TriggerSource => "trigger-source-label",
    }
}

/// CSS classes for a compartment text label, including §F-4 provenance.
fn compartment_text_classes(source: CompartmentItemSource) -> Vec<String> {
    let mut classes = vec!["compartment-text".to_owned()];
    match source {
        CompartmentItemSource::Owned => {}
        CompartmentItemSource::Inherited => classes.push("inherited".to_owned()),
        CompartmentItemSource::Derived => classes.push("derived".to_owned()),
    }
    classes
}

/// Internal render with a shared counter to avoid ID collisions across
/// recursive island subtree renders.
fn render_inner(ir: &DiagramIR, counter: &mut u32, hints: Option<&RenderingHints>) -> SGraph {
    let mut children = Vec::new();

    for node in &ir.nodes {
        children.push(SModelElement::Node(render_node(node, counter)));
    }

    for edge in &ir.edges {
        children.push(SModelElement::Edge(render_edge(edge, counter)));
    }

    for button in &ir.buttons {
        children.push(SModelElement::Button(render_button(
            button,
            "root",
            counter,
        )));
    }

    // Post-process: add invisible connection ports for parallel edge separation
    add_connection_ports(&mut children);

    // Graph-level ELK options: derived from view type, then overlaid with the
    // caller's hints (the ELK strings no longer live on the wire IR).
    let layout_options = {
        let mut opts = base_graph_layout_options(ir.view_type);
        apply_hint_overrides(&mut opts, hints);
        if opts.is_empty() {
            None
        } else {
            Some(opts)
        }
    };

    SGraph {
        id: "root".to_owned(),
        type_: "graph".to_owned(),
        children,
        layout_options,
    }
}

fn render_node(node: &DiagramNode, counter: &mut u32) -> SNode {
    *counter += 1;
    let id = node.element_id.clone();

    // CSS classes derived from the typed semantic fields (no free-form strings).
    let css_classes = node_css_classes(node);

    // Build children
    let mut schildren = Vec::new();

    // Header based on header_style
    match node.header_style {
        HeaderStyle::Normal => {
            if !node.name.is_empty() || !node.stereotype.is_empty() {
                let header =
                    builders::make_header_compartment(&id, &node.stereotype, &node.name);
                schildren.push(SModelElement::Compartment(header));
            }
        }
        HeaderStyle::Inline => {
            // Single label child — no compartment wrapper
            if !node.name.is_empty() {
                *counter += 1;
                schildren.push(SModelElement::Label(SLabel {
                    id: format!("{}/name", id),
                    type_: "label:name".to_owned(),
                    text: node.name.clone(),
                    position: None,
                    css_classes: vec!["name".to_owned()],
                    edge_placement: None,
                    semantic_element_id: None,
                diagnostic: None,
                }));
            }
        }
        HeaderStyle::None => {
            // No header at all (control nodes, proxy nodes)
        }
    }

    // Buttons from node.buttons
    for button in &node.buttons {
        schildren.push(SModelElement::Button(render_button(button, &id, counter)));
    }

    // Render children by kind
    let mut compartment_map: HashMap<CompartmentKind, Vec<SModelElement>> = HashMap::new();

    for child in &node.children {
        match child {
            DiagramChild::Node(child_node) => {
                schildren.push(SModelElement::Node(render_node(child_node, counter)));
            }
            DiagramChild::Text {
                compartment,
                text,
                element_id,
                source,
            } => {
                *counter += 1;
                let label = SLabel {
                    id: format!("{}/text/{}", id, counter),
                    type_: "label:name".to_owned(),
                    text: text.clone(),
                    position: None,
                    css_classes: compartment_text_classes(*source),
                    edge_placement: None,
                    semantic_element_id: Some(element_id.clone()),
                diagnostic: None,
                };
                compartment_map
                    .entry(*compartment)
                    .or_default()
                    .push(SModelElement::Label(label));
            }
            DiagramChild::Compartment { kind, children } => {
                *counter += 1;
                let comp_children: Vec<SModelElement> = children
                    .iter()
                    .map(|c| render_child_element(c, &id, counter))
                    .collect();
                schildren.push(SModelElement::Compartment(SCompartment {
                    id: format!(
                        "{}/{}",
                        id,
                        kind.type_string().replace(':', "_")
                    ),
                    type_: kind.type_string().to_owned(),
                    children: comp_children,
                    css_classes: vec![],
                    layout: Some("vbox".to_owned()),
                    layout_options: None,
                }));
            }
            DiagramChild::Edge(edge) => {
                schildren.push(SModelElement::Edge(render_edge(edge, counter)));
            }
            DiagramChild::Island {
                view_type,
                display_name,
                subtree,
                expanded,
            } => {
                *counter += 1;
                let island_id = format!("{}/island-{}", id, counter);

                if *expanded {
                    // Render the full subtree inside a container node
                    let sub_sgraph = render_inner(subtree, counter, None);

                    // ELK layout options for the island, derived from its view type.
                    let island_opts = base_graph_layout_options(subtree.view_type);

                    let mut island_children: Vec<SModelElement> = Vec::new();

                    // Header label
                    island_children.push(SModelElement::Label(SLabel {
                        id: format!("{}/header", island_id),
                        type_: "label:name".to_owned(),
                        text: display_name.clone(),
                        position: None,
                        css_classes: vec!["island-header".to_owned()],
                        edge_placement: None,
                        semantic_element_id: None,
                    diagnostic: None,
                    }));

                    // Expand/collapse button
                    island_children.push(SModelElement::Button(SButton {
                        id: format!("{}/expand", island_id),
                        type_: "button:expand".to_owned(),
                        position: None,
                        size: None,
                        enabled: true,
                        layout_options: None,
                    }));

                    // Subtree content
                    island_children.extend(sub_sgraph.children);

                    schildren.push(SModelElement::Node(SNode {
                        id: island_id,
                        type_: "node:island".to_owned(),
                        children: island_children,
                        position: None,
                        size: None,
                        css_classes: vec![
                            "island".to_owned(),
                            format!("island-{:?}", view_type).to_lowercase(),
                        ],
                        layout: Some("vbox".to_owned()),
                        layout_options: if island_opts.is_empty() {
                            None
                        } else {
                            Some(island_opts)
                        },
                        source_uri: None,
                        source_range: None,
                        expanded: Some(true),
                        tooltip: Some(display_name.clone()),
                        diagnostic_severity: None,
                    }));
                } else {
                    // Collapsed island: summary label only
                    let mut island_children: Vec<SModelElement> = Vec::new();
                    island_children.push(SModelElement::Label(SLabel {
                        id: format!("{}/summary", island_id),
                        type_: "label:name".to_owned(),
                        text: display_name.clone(),
                        position: None,
                        css_classes: vec!["island-summary".to_owned()],
                        edge_placement: None,
                        semantic_element_id: None,
                    diagnostic: None,
                    }));
                    island_children.push(SModelElement::Button(SButton {
                        id: format!("{}/expand", island_id),
                        type_: "button:expand".to_owned(),
                        position: None,
                        size: None,
                        enabled: true,
                        layout_options: None,
                    }));

                    schildren.push(SModelElement::Node(SNode {
                        id: island_id,
                        type_: "node:island".to_owned(),
                        children: island_children,
                        position: None,
                        size: Some(Dimension {
                            width: 120.0,
                            height: 40.0,
                        }),
                        css_classes: vec![
                            "island".to_owned(),
                            "collapsed".to_owned(),
                            format!("island-{:?}", view_type).to_lowercase(),
                        ],
                        layout: None,
                        layout_options: None,
                        source_uri: None,
                        source_range: None,
                        expanded: Some(false),
                        tooltip: Some(display_name.clone()),
                        diagnostic_severity: None,
                    }));
                }
            }
        }
    }

    // Emit compartments in spec order
    builders::emit_compartments_in_order(&id, &node.visual_kind, compartment_map, &mut schildren);

    // Ports — include all ports (hidden ones get `hidden-port` CSS class for
    // the TypeScript view to hide visually while the router still uses them as pins)
    for port in &node.ports {
        schildren.push(SModelElement::Port(render_port(port, counter)));
    }

    // Layout mode
    let layout = match node.layout {
        NodeLayout::VBox => Some("vbox".to_owned()),
        NodeLayout::Free => None,
    };

    // Layout options: node defaults, plus the sequence-lifeline layout payload
    // (the only node-level "layout option" that is genuine data, not ELK styling).
    let layout_options = {
        let mut opts = default_node_layout_options();
        if let Some(seq) = &node.sequence_layout {
            opts.insert("elk.algorithm".to_owned(), "fixed".to_owned());
            opts.insert("lifelineWidth".to_owned(), format!("{}", seq.lifeline_width));
            if !seq.activations.is_empty() {
                let json = serde_json::to_string(&seq.activations).unwrap_or_default();
                opts.insert("activations".to_owned(), json);
            }
        }
        Some(opts)
    };

    // Size estimation: use provided size, or compute heuristic.
    // Expanded containers with child nodes: let ELK compute the size (returns None)
    // so the parent grows to fit its children.
    let is_expanded = node.expanded == Some(true);
    let has_child_nodes = node.children.iter().any(|c| matches!(c, DiagramChild::Node(_) | DiagramChild::Island { expanded: true, .. }));
    let size = if is_expanded && has_child_nodes {
        // Expanded containers: provide minimum size based on header text so ELK
        // doesn't shrink the container below what stereotype/name labels need.
        // ELK computes the full size from children, but MINIMUM_SIZE constraint
        // uses this as a floor.
        let min = estimate_node_size(&node.name, &node.stereotype, 0);
        Some(node.size.map(|(w, h)| Dimension { width: w.max(min.width), height: h })
            .unwrap_or(min))
    } else {
        node.size.map(|(w, h)| Dimension { width: w, height: h }).or_else(|| {
        // Count children for size estimation
        let mut label_count = 0usize;
        let mut node_count = 0usize;
        let mut max_content_width = 0.0f64;

        for child in &node.children {
            match child {
                DiagramChild::Node(_) => node_count += 1,
                DiagramChild::Text { text, .. } => {
                    label_count += 1;
                    let text_width = (text.len() as f64) * 7.0;
                    if text_width > max_content_width {
                        max_content_width = text_width;
                    }
                }
                DiagramChild::Compartment { children, .. } => {
                    label_count += children.len();
                    for comp_child in children {
                        if let DiagramChild::Text { text, .. } = comp_child {
                            let text_width = (text.len() as f64) * 7.0;
                            if text_width > max_content_width {
                                max_content_width = text_width;
                            }
                        }
                    }
                }
                DiagramChild::Island { expanded, .. } => {
                    if *expanded {
                        node_count += 1;
                    } else {
                        label_count += 1;
                    }
                }
                DiagramChild::Edge(_) => {} // Edges don't affect size estimation
            }
        }

        // Count visible ports — nodes with ports need extra height so ports
        // have space along the boundary (~20px per port).
        let visible_port_count = node.ports.iter().filter(|p| !p.is_hidden).count();

        let mut dim = estimate_node_size_ex(
            &node.name,
            &node.stereotype,
            label_count,
            node_count,
            max_content_width,
            is_expanded,
        );

        if visible_port_count > 0 {
            let port_height = (visible_port_count as f64) * 20.0;
            dim.height = dim.height.max(DEFAULT_PADDING_TOP + port_height + DEFAULT_PADDING_BOTTOM);
        }

        Some(dim)
    })
    };

    // No special layoutOptions for collapsed containers. The view renders
    // collapsed content directly as SVG text — no VBox or ELK involvement.
    // The Rust `size` field tells ELK how big the node is for positioning.

    SNode {
        id,
        type_: node.visual_kind.node_type().to_owned(),
        children: schildren,
        position: node.position.map(|(x, y)| Point { x, y }),
        size,
        css_classes,
        layout,
        layout_options,
        // Source location dropped from the IR node (3.15) — it lives only in the
        // ViewModel text-map now. The legacy SGraph `SNode` keeps the fields for
        // wire-compat but they are no longer populated (the SGraph go-to-source /
        // diagnostic-overlay path is unmaintained; we've moved to the ViewModel).
        source_uri: None,
        source_range: None,
        expanded: node.expanded,
        tooltip: node.tooltip.clone(),
        diagnostic_severity: node.diagnostic_severity.clone(),
    }
}

/// Render a `DiagramChild` as an `SModelElement` (used inside `Compartment` children).
fn render_child_element(
    child: &DiagramChild,
    parent_id: &str,
    counter: &mut u32,
) -> SModelElement {
    match child {
        DiagramChild::Node(node) => SModelElement::Node(render_node(node, counter)),
        DiagramChild::Text {
            text, element_id, ..
        } => {
            *counter += 1;
            SModelElement::Label(SLabel {
                id: format!("{}/text/{}", parent_id, counter),
                type_: "label:name".to_owned(),
                text: text.clone(),
                position: None,
                css_classes: vec!["compartment-text".to_owned()],
                edge_placement: None,
                semantic_element_id: Some(element_id.clone()),
            diagnostic: None,
            })
        }
        DiagramChild::Compartment { kind, children } => {
            *counter += 1;
            let comp_children: Vec<SModelElement> = children
                .iter()
                .map(|c| render_child_element(c, parent_id, counter))
                .collect();
            SModelElement::Compartment(SCompartment {
                id: format!(
                    "{}/{}/{}",
                    parent_id,
                    kind.type_string().replace(':', "_"),
                    counter
                ),
                type_: kind.type_string().to_owned(),
                children: comp_children,
                css_classes: vec![],
                layout: Some("vbox".to_owned()),
                layout_options: None,
            })
        }
        DiagramChild::Island {
            display_name,
            subtree,
            expanded,
            view_type,
        } => {
            *counter += 1;
            let island_id = format!("{}/island-{}", parent_id, counter);
            if *expanded {
                let sub_sgraph = render_inner(subtree, counter, None);
                let mut island_children = vec![SModelElement::Label(SLabel {
                    id: format!("{}/header", island_id),
                    type_: "label:name".to_owned(),
                    text: display_name.clone(),
                    position: None,
                    css_classes: vec!["island-header".to_owned()],
                    edge_placement: None,
                    semantic_element_id: None,
                diagnostic: None,
                })];
                island_children.extend(sub_sgraph.children);
                SModelElement::Node(SNode {
                    id: island_id,
                    type_: "node:island".to_owned(),
                    children: island_children,
                    position: None,
                    size: None,
                    css_classes: vec![
                        "island".to_owned(),
                        format!("island-{:?}", view_type).to_lowercase(),
                    ],
                    layout: Some("vbox".to_owned()),
                    layout_options: None,
                    source_uri: None,
                    source_range: None,
                    expanded: Some(true),
                    tooltip: Some(display_name.clone()),
                    diagnostic_severity: None,
                })
            } else {
                SModelElement::Node(SNode {
                    id: island_id,
                    type_: "node:island".to_owned(),
                    children: vec![SModelElement::Label(SLabel {
                        id: format!("{}/summary", parent_id),
                        type_: "label:name".to_owned(),
                        text: display_name.clone(),
                        position: None,
                        css_classes: vec!["island-summary".to_owned()],
                        edge_placement: None,
                        semantic_element_id: None,
                    diagnostic: None,
                    })],
                    position: None,
                    size: Some(Dimension {
                        width: 120.0,
                        height: 40.0,
                    }),
                    css_classes: vec![
                        "island".to_owned(),
                        "collapsed".to_owned(),
                        format!("island-{:?}", view_type).to_lowercase(),
                    ],
                    layout: None,
                    layout_options: None,
                    source_uri: None,
                    source_range: None,
                    expanded: Some(false),
                    tooltip: Some(display_name.clone()),
                    diagnostic_severity: None,
                })
            }
        }
        DiagramChild::Edge(edge) => {
            SModelElement::Edge(render_edge(edge, counter))
        }
    }
}

fn render_port(port: &DiagramPort, counter: &mut u32) -> SPort {
    *counter += 1;
    let mut css_classes = Vec::new();

    // Direction CSS class from the typed direction field.
    match port.direction {
        Some(PortDirection::In) => css_classes.push("port-in".to_owned()),
        Some(PortDirection::Out) => css_classes.push("port-out".to_owned()),
        Some(PortDirection::InOut) => css_classes.push("port-inout".to_owned()),
        None => {}
    }

    if port.is_reference {
        css_classes.push("reference".to_owned());
    }

    if port.is_conjugated {
        css_classes.push("conjugated".to_owned());
    }

    if port.is_proxy {
        css_classes.push("proxy".to_owned());
    }

    if port.is_hidden {
        css_classes.push("hidden-port".to_owned());
    }

    for tag in &port.tags {
        css_classes.push(port_tag_class(*tag).to_owned());
    }

    // Recursively render sub-ports (also skip hidden sub-ports)
    let children: Vec<SModelElement> = port
        .sub_ports
        .iter()
        .filter(|sp| !sp.is_hidden)
        .map(|sp| SModelElement::Port(render_port(sp, counter)))
        .collect();

    // Size: from port fields, default 10x10 for unset
    let size = port.size.map(|(w, h)| Dimension { width: w, height: h }).or(Some(Dimension {
        width: 10.0,
        height: 10.0,
    }));

    // Layout options: derive the ELK port-side constraint from the typed `side`.
    let layout_options = port.side.map(|side| {
        let mut opts = HashMap::new();
        opts.insert("elk.port.side".to_owned(), side.as_elk_str().to_owned());
        opts
    });

    SPort {
        id: port.element_id.clone(),
        type_: "port".to_owned(),
        position: port.position.map(|(x, y)| Point { x, y }),
        size,
        children,
        css_classes,
        layout_options,
        name: if port.name.is_empty() {
            None
        } else {
            Some(port.name.clone())
        },
    }
}

fn render_edge(edge: &DiagramEdge, counter: &mut u32) -> SEdge {
    *counter += 1;

    let (type_, mut css_classes) = match &edge.kind {
        DiagramEdgeKind::Relationship(kind) => (
            classify::smodel_edge_type(kind).to_owned(),
            classify::relationship_css_classes(kind),
        ),
        DiagramEdgeKind::Transition { .. } => {
            ("edge:transition".to_owned(), vec!["transition".to_owned()])
        }
        DiagramEdgeKind::Message { is_succession, is_move, is_push, .. } => {
            // Always include "message" so the router picks up precomputedRoute
            // and CSS targets `.sprotty-edge.message` work for all sequence messages.
            let mut classes = vec!["message".to_owned()];
            if *is_succession { classes.push("succession".to_owned()); }
            if *is_move { classes.push("flow-move".to_owned()); }
            if *is_push { classes.push("flow-push".to_owned()); }
            let type_ = if *is_succession {
                "edge:succession"
            } else if *is_move {
                "edge:flow-move"
            } else {
                "edge:message"
            };
            (type_.to_owned(), classes)
        }
        DiagramEdgeKind::ControlFlow { .. } => {
            ("edge:flow".to_owned(), vec!["control-flow".to_owned()])
        }
    };

    // Semantic decorations from the typed edge tags.
    css_classes.extend(edge.tags.iter().map(|t| edge_tag_class(*t).to_owned()));

    // Build label text from edge kind
    let label_text = match &edge.kind {
        DiagramEdgeKind::Transition { trigger, guard } => {
            let mut parts = Vec::new();
            if let Some(t) = trigger {
                parts.push(t.clone());
            }
            if let Some(g) = guard {
                parts.push(format!("[{}]", g));
            }
            if parts.is_empty() && !edge.label.is_empty() {
                edge.label.clone()
            } else {
                parts.join(" ")
            }
        }
        DiagramEdgeKind::ControlFlow { guard } => {
            if let Some(g) = guard {
                if edge.label.is_empty() {
                    format!("[{}]", g)
                } else {
                    format!("{} [{}]", edge.label, g)
                }
            } else {
                edge.label.clone()
            }
        }
        DiagramEdgeKind::Message { payload, .. } => {
            if let Some(p) = payload {
                if edge.label.is_empty() {
                    p.clone()
                } else {
                    format!("{}: {}", edge.label, p)
                }
            } else {
                edge.label.clone()
            }
        }
        _ => edge.label.clone(),
    };

    // Build label with edge placement from the IR
    let mut children = Vec::new();
    if !label_text.is_empty() {
        let placement = &edge.label_placement;
        children.push(SModelElement::Label(SLabel {
            id: format!("{}/label", edge.id),
            type_: "label:edge".to_owned(),
            text: label_text,
            position: None,
            css_classes: vec!["edge-label".to_owned()],
            edge_placement: Some(EdgePlacement {
                position: placement.position,
                side: placement.side.clone(),
                rotate: placement.rotate,
                offset: placement.offset,
            }),
            semantic_element_id: None,
        diagnostic: None,
        }));
    }

    // Secondary labels (trigger source annotations, value badges, etc.)
    for (i, sub) in edge.secondary_labels.iter().enumerate() {
        let placement = &edge.label_placement;
        children.push(SModelElement::Label(SLabel {
            id: format!("{}/secondary-{}", edge.id, i),
            type_: "label:edge".to_owned(),
            text: sub.text.clone(),
            position: None,
            css_classes: vec!["edge-label".to_owned(), edge_sublabel_class(sub.kind).to_owned()],
            edge_placement: Some(EdgePlacement {
                position: placement.position,
                side: placement.side.clone(),
                rotate: placement.rotate,
                offset: Some(placement.offset.unwrap_or(0.0) + 14.0 * (i as f64 + 1.0)),
            }),
            semantic_element_id: None,
            diagnostic: None,
        }));
    }

    // Endpoint mode
    let endpoint_mode = match edge.endpoint_mode {
        EndpointMode::AutoSide => Some("auto-side".to_owned()),
        EndpointMode::StrictPort => Some("strict-port".to_owned()),
    };

    // Precomputed routes
    let routing_points = edge.precomputed_route.as_ref().map(|pts| {
        pts.iter().map(|(x, y)| Point { x: *x, y: *y }).collect()
    });
    let precomputed_route = edge.precomputed_route.as_ref().map(|pts| {
        pts.iter().map(|(x, y)| Point { x: *x, y: *y }).collect()
    });

    SEdge {
        id: edge.id.clone(),
        type_,
        source_id: edge.source_id.clone(),
        target_id: edge.target_id.clone(),
        source_port_id: edge.source_port_id.clone(),
        target_port_id: edge.target_port_id.clone(),
        children,
        css_classes,
        router_kind: None,
        endpoint_mode,
        routing_points,
        precomputed_route,
    }
}

fn render_button(button: &DiagramButton, parent_id: &str, counter: &mut u32) -> SButton {
    *counter += 1;
    let btn_suffix = match &button.button_type {
        ButtonType::Expand => "expand",
        ButtonType::AddMessage { .. } => "addMsg",
        ButtonType::AddLifeline => "addLL",
    };
    let btn_id = format!("{}/{}-{}", parent_id, btn_suffix, counter);

    let (type_, layout_options) = match &button.button_type {
        ButtonType::Expand => ("button:expand".to_owned(), None),
        ButtonType::AddMessage {
            lifeline_id,
            insertion_index,
        } => {
            let mut opts = HashMap::new();
            opts.insert("lifelineId".to_owned(), lifeline_id.clone());
            opts.insert("insertionIndex".to_owned(), insertion_index.to_string());
            ("button:addMessage".to_owned(), Some(opts))
        }
        ButtonType::AddLifeline => ("button:addLifeline".to_owned(), None),
    };

    SButton {
        id: btn_id,
        type_,
        position: button.position.map(|(x, y)| Point { x, y }),
        size: button.size.map(|(w, h)| Dimension { width: w, height: h }),
        enabled: true,
        layout_options,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::smodel::ViewType;
    use crate::visual_kind::VisualKind;

    #[test]
    fn empty_ir_produces_empty_sgraph() {
        let ir = DiagramIR::new(ViewType::General);
        let sgraph = render(&ir);
        assert_eq!(sgraph.id, "root");
        assert_eq!(sgraph.type_, "graph");
        assert!(sgraph.children.is_empty());
    }

    #[test]
    fn single_node_renders() {
        let mut ir = DiagramIR::new(ViewType::General);
        ir.nodes
            .push(DiagramNode::new("e1", VisualKind::Part, "Engine"));
        let sgraph = render(&ir);
        assert_eq!(sgraph.children.len(), 1);
        match &sgraph.children[0] {
            SModelElement::Node(n) => {
                assert_eq!(n.id, "e1");
                assert_eq!(n.type_, "node:block");
            }
            _ => panic!("expected Node"),
        }
    }

    #[test]
    fn edge_renders_with_label() {
        let mut ir = DiagramIR::new(ViewType::General);
        ir.edges.push(DiagramEdge::relationship(
            "rel1",
            "src",
            "tgt",
            sysml_core::RelationshipKind::Satisfy,
            "\u{00ab}satisfy\u{00bb}",
        ));
        let sgraph = render(&ir);
        assert_eq!(sgraph.children.len(), 1);
        match &sgraph.children[0] {
            SModelElement::Edge(e) => {
                assert_eq!(e.source_id, "src");
                assert_eq!(e.target_id, "tgt");
                assert!(!e.children.is_empty());
            }
            _ => panic!("expected Edge"),
        }
    }

    #[test]
    fn port_renders_with_direction() {
        let mut ir = DiagramIR::new(ViewType::Interconnection);
        let mut node = DiagramNode::new("n1", VisualKind::Part, "Block");
        let port = DiagramPort::new("p1", "dataIn").with_direction(PortDirection::In);
        node.ports.push(port);
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let node = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };
        let has_port = node.children.iter().any(|c| {
            matches!(c, SModelElement::Port(p) if p.css_classes.contains(&"port-in".to_string()))
        });
        assert!(has_port, "port should have port-in CSS class");
    }

    // ── New tests ──────────────────────────────────────────────────────

    #[test]
    fn header_style_inline_produces_single_label() {
        let mut ir = DiagramIR::new(ViewType::Browser);
        let node = DiagramNode::new("n1", VisualKind::Part, "MyPart")
            .with_header_style(HeaderStyle::Inline);
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        // Should have a label:name child, NOT a comp:header compartment
        let has_header_comp = snode
            .children
            .iter()
            .any(|c| matches!(c, SModelElement::Compartment(comp) if comp.type_ == "comp:header"));
        assert!(
            !has_header_comp,
            "Inline header should not produce comp:header compartment"
        );

        let has_name_label = snode
            .children
            .iter()
            .any(|c| matches!(c, SModelElement::Label(l) if l.type_ == "label:name" && l.text == "MyPart"));
        assert!(
            has_name_label,
            "Inline header should produce a label:name with the node name"
        );
    }

    #[test]
    fn header_style_none_produces_no_header() {
        let mut ir = DiagramIR::new(ViewType::StateTransition);
        let node = DiagramNode::new("ctrl1", VisualKind::InitialNode, "")
            .with_header_style(HeaderStyle::None);
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        let has_header = snode.children.iter().any(|c| {
            matches!(c, SModelElement::Compartment(comp) if comp.type_ == "comp:header")
                || matches!(c, SModelElement::Label(l) if l.type_ == "label:name")
        });
        assert!(
            !has_header,
            "HeaderStyle::None should produce no header elements"
        );
    }

    #[test]
    fn expand_button_renders_correctly() {
        let mut ir = DiagramIR::new(ViewType::General);
        let node = DiagramNode::new("n1", VisualKind::Part, "Foo").with_expand_button();
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        let has_expand = snode.children.iter().any(|c| {
            matches!(c, SModelElement::Button(b) if b.type_ == "button:expand" && b.enabled)
        });
        assert!(has_expand, "Node with expand_button should have button:expand child");
    }

    #[test]
    fn diagram_child_compartment_renders_as_scompartment() {
        let mut ir = DiagramIR::new(ViewType::General);
        let mut node = DiagramNode::new("n1", VisualKind::Part, "Box");
        node.children.push(DiagramChild::Compartment {
            kind: CompartmentKind::Documentation,
            children: vec![DiagramChild::Text {
                compartment: CompartmentKind::Documentation,
                text: "A documented part".to_string(),
                element_id: "doc1".to_string(),
                source: CompartmentItemSource::Owned,
            }],
        });
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        let has_doc_comp = snode.children.iter().any(|c| {
            matches!(c, SModelElement::Compartment(comp) if comp.type_ == "comp:documentation")
        });
        assert!(
            has_doc_comp,
            "DiagramChild::Compartment should render as SCompartment"
        );
    }

    #[test]
    fn endpoint_mode_strict_port_produces_strict_port() {
        let mut ir = DiagramIR::new(ViewType::Interconnection);
        let edge = DiagramEdge::relationship(
            "e1",
            "src",
            "tgt",
            sysml_core::RelationshipKind::Flow,
            "data",
        )
        .with_ports("src/p1", "tgt/p2");
        ir.edges.push(edge);

        let sgraph = render(&ir);
        let sedge = match &sgraph.children[0] {
            SModelElement::Edge(e) => e,
            _ => panic!("expected Edge"),
        };

        assert_eq!(
            sedge.endpoint_mode,
            Some("strict-port".to_string()),
            "StrictPort endpoint mode should produce 'strict-port' for ELK port routing"
        );
        assert_eq!(sedge.source_port_id, Some("src/p1".to_string()));
        assert_eq!(sedge.target_port_id, Some("tgt/p2".to_string()));
    }

    #[test]
    fn fixed_layout_views_derive_fixed_algorithm() {
        // ELK options are no longer stored on the IR — the adapter derives them
        // from the view type. Grid/Sequence/Geometry → fixed.
        let ir = DiagramIR::new(ViewType::Grid);
        let sgraph = render(&ir);
        let opts = sgraph.layout_options.as_ref().expect("Grid should derive layout_options");
        assert_eq!(opts["elk.algorithm"], "org.eclipse.elk.fixed");
    }

    #[test]
    fn caller_hints_override_base_layout_options() {
        let ir = DiagramIR::new(ViewType::General);
        let hints = RenderingHints::new()
            .with_spacing_node_node("30")
            .with_direction("RIGHT");
        let sgraph = render_with(&ir, Some(&hints));
        let opts = sgraph.layout_options.as_ref().expect("hints should produce layout_options");
        assert_eq!(opts["elk.spacing.nodeNode"], "30");
        assert_eq!(opts["elk.direction"], "RIGHT");
    }

    #[test]
    fn hidden_ports_are_skipped() {
        let mut ir = DiagramIR::new(ViewType::StateTransition);
        let mut node = DiagramNode::new("s1", VisualKind::State, "Running");
        node.ports.push(DiagramPort::new("p-visible", "visPort"));
        node.ports.push(DiagramPort::new("p-hidden", "hiddenPort").hidden());
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        let ports: Vec<&SPort> = snode
            .children
            .iter()
            .filter_map(|c| match c {
                SModelElement::Port(p) => Some(p),
                _ => None,
            })
            .collect();

        let port_ids: Vec<&str> = ports.iter().map(|p| p.id.as_str()).collect();

        assert!(port_ids.contains(&"p-visible"), "visible port should be present");
        assert!(
            port_ids.contains(&"p-hidden"),
            "hidden port should still be rendered (with hidden-port CSS class)"
        );

        // Hidden port gets the "hidden-port" CSS class so the TS view hides it visually
        let hidden = ports.iter().find(|p| p.id == "p-hidden").unwrap();
        assert!(
            hidden.css_classes.contains(&"hidden-port".to_string()),
            "hidden port should have hidden-port CSS class"
        );
    }

    #[test]
    fn transition_edge_composes_trigger_and_guard() {
        let mut ir = DiagramIR::new(ViewType::StateTransition);
        ir.edges.push(DiagramEdge::transition(
            "t1",
            "s1",
            "s2",
            Some("start".to_string()),
            Some("ready".to_string()),
        ));

        let sgraph = render(&ir);
        let sedge = match &sgraph.children[0] {
            SModelElement::Edge(e) => e,
            _ => panic!("expected Edge"),
        };

        let label = match &sedge.children[0] {
            SModelElement::Label(l) => l,
            _ => panic!("expected Label"),
        };
        assert_eq!(label.text, "start [ready]");
    }

    #[test]
    fn control_flow_edge_with_guard() {
        let mut ir = DiagramIR::new(ViewType::ActionFlow);
        ir.edges.push(DiagramEdge::control_flow(
            "cf1",
            "a1",
            "a2",
            Some("x > 0".to_string()),
        ));

        let sgraph = render(&ir);
        let sedge = match &sgraph.children[0] {
            SModelElement::Edge(e) => e,
            _ => panic!("expected Edge"),
        };

        let label = match &sedge.children[0] {
            SModelElement::Label(l) => l,
            _ => panic!("expected Label"),
        };
        assert_eq!(label.text, "[x > 0]");
    }

    #[test]
    fn message_edge_with_payload() {
        let mut ir = DiagramIR::new(ViewType::Sequence);
        ir.edges.push(DiagramEdge::message(
            "m1",
            "l1",
            "l2",
            Some("getData()".to_string()),
            false,
            false,
            false,
        ));

        let sgraph = render(&ir);
        let sedge = match &sgraph.children[0] {
            SModelElement::Edge(e) => e,
            _ => panic!("expected Edge"),
        };

        let label = match &sedge.children[0] {
            SModelElement::Label(l) => l,
            _ => panic!("expected Label"),
        };
        assert_eq!(label.text, "getData()");
    }

    #[test]
    fn precomputed_route_propagates() {
        let mut ir = DiagramIR::new(ViewType::Sequence);
        ir.edges.push(
            DiagramEdge::message("m1", "l1", "l2", None, false, false, false)
                .with_route(vec![(10.0, 20.0), (100.0, 20.0)]),
        );

        let sgraph = render(&ir);
        let sedge = match &sgraph.children[0] {
            SModelElement::Edge(e) => e,
            _ => panic!("expected Edge"),
        };

        assert!(sedge.routing_points.is_some());
        assert!(sedge.precomputed_route.is_some());
        let pts = sedge.precomputed_route.as_ref().unwrap();
        assert_eq!(pts.len(), 2);
        assert_eq!(pts[0].x, 10.0);
        assert_eq!(pts[1].x, 100.0);
    }

    #[test]
    fn top_level_buttons_render() {
        let mut ir = DiagramIR::new(ViewType::Sequence);
        ir.buttons.push(DiagramButton::add_lifeline());

        let sgraph = render(&ir);
        let has_btn = sgraph
            .children
            .iter()
            .any(|c| matches!(c, SModelElement::Button(b) if b.type_ == "button:addLifeline"));
        assert!(has_btn, "top-level addLifeline button should render");
    }

    #[test]
    fn add_message_button_has_metadata() {
        let mut ir = DiagramIR::new(ViewType::Sequence);
        let mut node = DiagramNode::new("ll1", VisualKind::Lifeline, "Server");
        node.buttons.push(DiagramButton::add_message("ll1", 3));
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        let btn = snode.children.iter().find_map(|c| match c {
            SModelElement::Button(b) if b.type_ == "button:addMessage" => Some(b),
            _ => None,
        });
        let btn = btn.expect("should have addMessage button");
        let opts = btn.layout_options.as_ref().expect("should have metadata");
        assert_eq!(opts["lifelineId"], "ll1");
        assert_eq!(opts["insertionIndex"], "3");
    }

    #[test]
    fn node_layout_vbox_and_free() {
        let mut ir = DiagramIR::new(ViewType::General);

        let vbox_node = DiagramNode::new("n1", VisualKind::Part, "VBox");
        let free_node =
            DiagramNode::new("n2", VisualKind::Part, "Free").with_layout(NodeLayout::Free);

        ir.nodes.push(vbox_node);
        ir.nodes.push(free_node);

        let sgraph = render(&ir);

        let n1 = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };
        let n2 = match &sgraph.children[1] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        assert_eq!(n1.layout, Some("vbox".to_string()));
        assert_eq!(n2.layout, None);
    }

    #[test]
    fn node_layout_options_include_defaults() {
        // Node-level ELK options are no longer carried on the IR; the adapter
        // always emits the default node layout options.
        let mut ir = DiagramIR::new(ViewType::General);
        ir.nodes.push(DiagramNode::new("n1", VisualKind::Part, "Plain"));

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        let opts = snode
            .layout_options
            .as_ref()
            .expect("should have layout_options");
        assert!(
            opts.contains_key("elk.nodeSize.constraints"),
            "should include default node layout options"
        );
    }

    #[test]
    fn sequence_layout_serializes_into_node_options() {
        let mut ir = DiagramIR::new(ViewType::Sequence);
        let mut node = DiagramNode::new("ll1", VisualKind::Part, "Lifeline");
        node.sequence_layout = Some(SequenceNodeLayout {
            lifeline_width: 160.0,
            activations: vec![(10.0, 50.0)],
        });
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };
        let opts = snode.layout_options.as_ref().expect("should have layout_options");
        assert_eq!(opts["lifelineWidth"], "160");
        assert_eq!(opts["activations"], "[[10.0,50.0]]");
    }

    #[test]
    fn diagnostic_severity_produces_css_class() {
        let mut ir = DiagramIR::new(ViewType::General);
        let mut node = DiagramNode::new("n1", VisualKind::Part, "Broken");
        node.diagnostic_severity = Some("error".to_string());
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        assert!(
            snode.css_classes.contains(&"diagnostic-error".to_string()),
            "diagnostic severity should add CSS class"
        );
        assert_eq!(snode.diagnostic_severity, Some("error".to_string()));
    }

    #[test]
    fn edge_tags_propagate_to_css() {
        let mut ir = DiagramIR::new(ViewType::General);
        let mut edge = DiagramEdge::relationship(
            "e1",
            "src",
            "tgt",
            sysml_core::RelationshipKind::Specialize,
            "spec",
        );
        edge.tags.push(EdgeTag::NarySegment);
        ir.edges.push(edge);

        let sgraph = render(&ir);
        let sedge = match &sgraph.children[0] {
            SModelElement::Edge(e) => e,
            _ => panic!("expected Edge"),
        };

        assert!(
            sedge.css_classes.contains(&"nary-segment".to_string()),
            "edge tags should propagate to CSS"
        );
    }

    #[test]
    fn port_side_constraint_propagates() {
        let mut ir = DiagramIR::new(ViewType::Interconnection);
        let mut node = DiagramNode::new("n1", VisualKind::Part, "Block");
        let port = DiagramPort::new("p1", "west").with_side(PortSide::West);
        node.ports.push(port);
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };
        let sport = snode.children.iter().find_map(|c| match c {
            SModelElement::Port(p) if p.id == "p1" => Some(p),
            _ => None,
        });
        let sport = sport.expect("should have port p1");
        let opts = sport.layout_options.as_ref().expect("port should have layout_options");
        assert_eq!(opts["elk.port.side"], "WEST");
    }

    #[test]
    fn island_expanded_renders_container_node() {
        let mut ir = DiagramIR::new(ViewType::General);
        let mut subtree = DiagramIR::new(ViewType::StateTransition);
        subtree
            .nodes
            .push(DiagramNode::new("s1", VisualKind::State, "Running"));

        let mut node = DiagramNode::new("p1", VisualKind::Part, "Vehicle");
        node.children.push(DiagramChild::Island {
            view_type: ViewType::StateTransition,
            display_name: "State Machine".to_string(),
            subtree,
            expanded: true,
        });
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        // Find the island container
        let island = snode.children.iter().find_map(|c| match c {
            SModelElement::Node(n) if n.type_ == "node:island" => Some(n),
            _ => None,
        });
        let island = island.expect("should have island node");
        assert!(island.css_classes.contains(&"island".to_string()));
        assert_eq!(island.expanded, Some(true));

        // Island should contain the sub-node
        let has_state = island.children.iter().any(|c| {
            matches!(c, SModelElement::Node(n) if n.type_ == "node:state")
        });
        assert!(has_state, "expanded island should contain subtree nodes");
    }

    #[test]
    fn island_collapsed_renders_summary() {
        let mut ir = DiagramIR::new(ViewType::General);
        let subtree = DiagramIR::new(ViewType::ActionFlow);

        let mut node = DiagramNode::new("p1", VisualKind::Part, "Vehicle");
        node.children.push(DiagramChild::Island {
            view_type: ViewType::ActionFlow,
            display_name: "Actions".to_string(),
            subtree,
            expanded: false,
        });
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        let island = snode.children.iter().find_map(|c| match c {
            SModelElement::Node(n) if n.type_ == "node:island" => Some(n),
            _ => None,
        });
        let island = island.expect("should have island node");
        assert!(island.css_classes.contains(&"collapsed".to_string()));
        assert_eq!(island.expanded, Some(false));
        assert!(island.size.is_some(), "collapsed island should have a fixed size");
    }

    #[test]
    fn size_estimation_when_no_explicit_size() {
        let mut ir = DiagramIR::new(ViewType::General);
        let node = DiagramNode::new("n1", VisualKind::Part, "LongNodeName");
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        assert!(snode.size.is_some(), "node without explicit size should get estimated size");
        let size = snode.size.as_ref().unwrap();
        assert!(size.width >= 100.0, "estimated width should be at least minimum");
        assert!(size.height >= 44.0, "estimated height should be at least minimum");
    }

    #[test]
    fn explicit_size_overrides_estimation() {
        let mut ir = DiagramIR::new(ViewType::General);
        let node =
            DiagramNode::new("n1", VisualKind::Part, "Small").with_size(50.0, 30.0);
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        let size = snode.size.as_ref().unwrap();
        assert_eq!(size.width, 50.0);
        assert_eq!(size.height, 30.0);
    }

    #[test]
    fn edge_label_placement_propagates() {
        let mut ir = DiagramIR::new(ViewType::General);
        let edge = DiagramEdge::relationship(
            "e1",
            "src",
            "tgt",
            sysml_core::RelationshipKind::Flow,
            "data",
        )
        .with_label_placement(EdgeLabelPlacement {
            position: 0.8,
            side: "left".to_string(),
            offset: Some(5.0),
            rotate: true,
        });
        ir.edges.push(edge);

        let sgraph = render(&ir);
        let sedge = match &sgraph.children[0] {
            SModelElement::Edge(e) => e,
            _ => panic!("expected Edge"),
        };

        let label = match &sedge.children[0] {
            SModelElement::Label(l) => l,
            _ => panic!("expected Label"),
        };
        let ep = label.edge_placement.as_ref().expect("should have edge_placement");
        assert_eq!(ep.position, 0.8);
        assert_eq!(ep.side, "left");
        assert_eq!(ep.offset, Some(5.0));
        assert!(ep.rotate);
    }

    #[test]
    fn proxy_port_gets_css_class() {
        let mut ir = DiagramIR::new(ViewType::Interconnection);
        let mut node = DiagramNode::new("n1", VisualKind::Part, "Context");
        let port = DiagramPort::new("p1", "external").proxy();
        node.ports.push(port);
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };
        let sport = snode.children.iter().find_map(|c| match c {
            SModelElement::Port(p) if p.id == "p1" => Some(p),
            _ => None,
        });
        let sport = sport.expect("should have port p1");
        assert!(
            sport.css_classes.contains(&"proxy".to_string()),
            "proxy port should have 'proxy' CSS class"
        );
    }

    #[test]
    fn text_children_grouped_into_compartments() {
        let mut ir = DiagramIR::new(ViewType::General);
        let mut node = DiagramNode::new("n1", VisualKind::Part, "Box");
        node.children.push(DiagramChild::Text {
            compartment: CompartmentKind::Attributes,
            text: "attr x : Int".to_string(),
            element_id: "a1".to_string(),
            source: CompartmentItemSource::Owned,
        });
        node.children.push(DiagramChild::Text {
            compartment: CompartmentKind::Attributes,
            text: "attr y : Int".to_string(),
            element_id: "a2".to_string(),
            source: CompartmentItemSource::Owned,
        });
        ir.nodes.push(node);

        let sgraph = render(&ir);
        let snode = match &sgraph.children[0] {
            SModelElement::Node(n) => n,
            _ => panic!("expected Node"),
        };

        let attr_comp = snode.children.iter().find_map(|c| match c {
            SModelElement::Compartment(comp) if comp.type_ == "comp:attributes" => Some(comp),
            _ => None,
        });
        let comp = attr_comp.expect("should have attributes compartment");
        assert_eq!(comp.children.len(), 2, "should have 2 text labels");
    }
}
