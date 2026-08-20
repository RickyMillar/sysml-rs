use serde::Serialize;
use std::collections::HashMap;

/// Root graph element
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SGraph {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String, // always "graph"
    pub children: Vec<SModelElement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_options: Option<HashMap<String, String>>,
}

/// A node in the diagram
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SNode {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String, // "node:block", "node:state", "node:action", "node:package", "node:requirement"
    pub children: Vec<SModelElement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Point>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<Dimension>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub css_classes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    /// NOTE: Ignored by ELK — ILayoutConfigurator provides options.
    /// Kept for non-ELK consumers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_options: Option<HashMap<String, String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_uri: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_range: Option<[u32; 4]>,
    /// Whether this node is expanded (shows type definition children inline).
    /// `None` means the node is not expandable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expanded: Option<bool>,
    /// Tooltip text shown on hover (e.g. "«part definition» Vehicle").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tooltip: Option<String>,
    /// Diagnostic severity for visual overlay ("error", "warning", "info", "hint").
    /// When set, the corresponding CSS class (e.g. "diagnostic-error") is also added to css_classes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic_severity: Option<String>,
}

/// An edge connecting two elements
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SEdge {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String, // "edge:flow", "edge:satisfy", "edge:specialize", etc.
    pub source_id: String,
    pub target_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_port_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_port_id: Option<String>,
    pub children: Vec<SModelElement>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub css_classes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub router_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint_mode: Option<String>,
    /// Pre-computed routing points for edge path (e.g. horizontal sequence messages).
    /// When set, Sprotty's PolylineEdgeView uses these instead of computing a route.
    /// NOTE: ELK may overwrite this field during layout. Use `precomputed_route` for
    /// points that must survive ELK processing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routing_points: Option<Vec<Point>>,
    /// Pre-computed routing points preserved across ELK layout.
    /// ELK only overwrites `routingPoints`; this field is left untouched.
    /// The TS router reads this to bypass WASM routing for sequence messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precomputed_route: Option<Vec<Point>>,
}

/// A port on a node boundary
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SPort {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String, // "port"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Point>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<Dimension>,
    pub children: Vec<SModelElement>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub css_classes: Vec<String>,
    /// ELK layout options for port placement (e.g. elk.port.side).
    /// NOTE: ELK reads options from ILayoutConfigurator, not model properties.
    /// Kept for non-ELK consumers and as a hint to the TS configurator.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_options: Option<HashMap<String, String>>,
    /// Port display name — rendered by the view outside the port rect.
    /// Uses `portName` (not `name`) to avoid Sprotty auto-creating a label child.
    #[serde(rename = "portName", skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Diagnostic info attached to a label for diagram rendering.
/// Carries enough info for squiggly underlines, dimming, and tooltips.
/// Derived mechanically from `sysml_span::Diagnostic` fields — no per-error-code mapping.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LabelDiagnostic {
    /// "error", "warning", "info"
    pub severity: String,
    /// Human-readable message for tooltip
    pub message: String,
    /// Error code (e.g. "E200", "IM001")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
    /// Tags: "unnecessary" → dim, "deprecated" → strikethrough
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Available quick-fix titles (pre-computed from diagnostic code).
    /// Displayed as buttons in the diagram popup.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quick_fixes: Vec<String>,
}

/// A text label
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SLabel {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String, // "label:name", "label:stereotype", "label:edge"
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Point>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub css_classes: Vec<String>,
    /// Edge label placement (position along edge, side, rotation).
    /// Sprotty uses this for `EdgeLayoutable` feature on labels that are children of edges.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edge_placement: Option<EdgePlacement>,
    /// Semantic element ID for editable labels (used by rename/editLabel in diagram).
    /// Set on "label:name" labels that correspond to a nameable element.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub semantic_element_id: Option<String>,
    /// Diagnostic info for rendering squiggly underlines and tooltips.
    /// Set by overlay_diagnostics when a diagnostic spans this label's element.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<LabelDiagnostic>,
}

/// Edge label placement configuration for Sprotty's EdgeLayoutable feature.
#[derive(Debug, Clone, Default, Serialize)]
pub struct EdgePlacement {
    /// Position along the edge (0.0 = source, 0.5 = midpoint, 1.0 = target)
    pub position: f64,
    /// Which side of the edge: "on", "left", "right", "top", "bottom"
    pub side: String,
    /// Whether to rotate the label to follow the edge direction
    pub rotate: bool,
    /// Perpendicular offset from the edge line (positive = away from edge side)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
}

/// A compartment (container for grouped content)
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SCompartment {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String, // "comp:header", "comp:attributes", "comp:operations"
    pub children: Vec<SModelElement>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub css_classes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_options: Option<HashMap<String, String>>,
}

/// Position
#[derive(Debug, Clone, Default, Serialize)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

/// Size
#[derive(Debug, Clone, Default, Serialize)]
pub struct Dimension {
    pub width: f64,
    pub height: f64,
}

/// A button (for expand/collapse, sequence add-message, etc.)
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SButton {
    pub id: String,
    #[serde(rename = "type")]
    pub type_: String, // "button:expand", "button:addMessage", "button:addLifeline"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<Point>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<Dimension>,
    pub enabled: bool,
    /// Metadata for button actions (e.g., lifelineId, insertionIndex).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layout_options: Option<HashMap<String, String>>,
}

/// Unified SModel element enum
#[derive(Debug, Clone, Serialize)]
#[serde(untagged)]
pub enum SModelElement {
    Graph(SGraph),
    Node(SNode),
    Edge(SEdge),
    Port(SPort),
    Label(SLabel),
    Compartment(SCompartment),
    Button(SButton),
}

/// Default ELK layout options for nodes.
///
/// Sets `elk.nodeSize.constraints = MINIMUM_SIZE` so ELK respects our computed
/// `size` field as a minimum bound rather than ignoring it.
///
/// Fallback for non-ELK consumers. ELK reads options from
/// SysmlLayoutConfigurator, not model properties.
pub fn default_node_layout_options() -> HashMap<String, String> {
    let mut opts = HashMap::new();
    opts.insert(
        "elk.nodeSize.constraints".to_owned(),
        "MINIMUM_SIZE".to_owned(),
    );
    opts
}

/// Post-process children to add connection ports for parallel edge separation.
///
/// When two nodes have multiple edges between them (e.g. A→B and B→A),
/// Sprotty's ManhattanEdgeRouter routes them along identical paths, causing overlap.
/// By adding invisible ports at different positions on each node and routing
/// edges through those ports, the router naturally separates the paths.
///
/// This function:
/// 1. Scans edges and groups by normalized (source, target) node pair
/// 2. For pairs with 2+ edges, creates tiny invisible ports on each node
/// 3. Updates edge `source_id`/`target_id` to reference port IDs
/// 4. Sets `elk.portConstraints: FIXED_ORDER` on affected nodes
#[allow(clippy::indexing_slicing)] // edge_idx from enumeration of children; always in bounds
pub fn add_connection_ports(children: &mut Vec<SModelElement>) {
    // Phase 1: Collect edge metadata
    struct EdgeInfo {
        idx: usize,
        source_id: String,
        target_id: String,
    }

    let mut edges: Vec<EdgeInfo> = Vec::new();
    for (idx, child) in children.iter().enumerate() {
        if let SModelElement::Edge(edge) = child {
            // Skip edges that already have port assignments (e.g. action flow
            // edges wired to control-node ports) — they don't need auto-generated
            // connection ports.
            if edge.source_port_id.is_some() || edge.target_port_id.is_some() {
                continue;
            }
            edges.push(EdgeInfo {
                idx,
                source_id: edge.source_id.clone(),
                target_id: edge.target_id.clone(),
            });
        }
    }

    // Phase 2: Group by normalized (min, max) node pair
    let mut pair_groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for info in &edges {
        let key = if info.source_id <= info.target_id {
            (info.source_id.clone(), info.target_id.clone())
        } else {
            (info.target_id.clone(), info.source_id.clone())
        };
        pair_groups.entry(key).or_default().push(info.idx);
    }

    // Only care about pairs with 2+ edges (parallel edges)
    let multi_pairs: Vec<_> = pair_groups
        .into_iter()
        .filter(|(_, indices)| indices.len() >= 2)
        .collect();

    if multi_pairs.is_empty() {
        return;
    }

    // Phase 3: Build port-creation map and edge mutations
    //
    // IMPORTANT: We set source_port_id/target_port_id on the edge (ELK layout hints)
    // but keep source_id/target_id pointing at the NODES. This way:
    // - ELK uses ports for initial layout separation of parallel edges
    // - Sprotty's ManhattanEdgeRouter still sees node-to-node edges and dynamically
    //   picks the best anchor side after drag (ports pin the anchor point, nodes don't)
    let mut ports_for_node: HashMap<String, Vec<SPort>> = HashMap::new();
    // (edge_index, source_port_id, target_port_id)
    let mut edge_mutations: Vec<(usize, String, String)> = Vec::new();

    for (_pair, edge_indices) in &multi_pairs {
        for (port_idx, &edge_idx) in edge_indices.iter().enumerate() {
            if let SModelElement::Edge(edge) = &children[edge_idx] {
                let src = &edge.source_id;
                let tgt = &edge.target_id;

                let source_port_id = format!("{}/cp-{}-{}", src, tgt, port_idx);
                let target_port_id = format!("{}/cp-{}-{}", tgt, src, port_idx);

                // Connection ports need elk.port.side or ELK's label/size
                // calculator crashes with "ports that have port sides assigned".
                let connection_port_layout = || {
                    let mut opts = std::collections::HashMap::new();
                    opts.insert("elk.port.side".to_owned(), "EAST".to_owned());
                    Some(opts)
                };

                // Create source port (invisible 1x1)
                ports_for_node.entry(src.clone()).or_default().push(SPort {
                    id: source_port_id.clone(),
                    type_: "port".to_owned(),
                    position: None,
                    size: Some(Dimension {
                        width: 1.0,
                        height: 1.0,
                    }),
                    children: vec![],
                    css_classes: vec!["connection-port".to_owned()],
                    layout_options: connection_port_layout(),
                    name: None,
                });

                // Create target port
                ports_for_node.entry(tgt.clone()).or_default().push(SPort {
                    id: target_port_id.clone(),
                    type_: "port".to_owned(),
                    position: None,
                    size: Some(Dimension {
                        width: 1.0,
                        height: 1.0,
                    }),
                    children: vec![],
                    css_classes: vec!["connection-port".to_owned()],
                    layout_options: connection_port_layout(),
                    name: None,
                });

                edge_mutations.push((edge_idx, source_port_id, target_port_id));
            }
        }
    }

    // Phase 4: Apply edge mutations — set port hints but keep source/target as nodes
    for (edge_idx, src_port, tgt_port) in edge_mutations {
        if let SModelElement::Edge(edge) = &mut children[edge_idx] {
            edge.source_port_id = Some(src_port);
            edge.target_port_id = Some(tgt_port);
        }
    }

    // Phase 5: Add ports to parent nodes and set port constraints
    for child in children.iter_mut() {
        if let SModelElement::Node(node) = child {
            if let Some(ports) = ports_for_node.remove(&node.id) {
                for port in ports {
                    node.children.push(SModelElement::Port(port));
                }
                let opts = node.layout_options.get_or_insert_with(HashMap::new);
                opts.insert("elk.portConstraints".to_owned(), "FREE".to_owned());
            }
        }
    }
}

/// Estimate node size from name length and child count.
///
/// Heuristic fallback. In browser, TS TextMeasurer computes pixel-accurate
/// sizes via canvas. These constants are kept in sync with the source of truth:
/// `editors/diagram/src/shapes/shape-catalog.json`.
///
/// Uses approximate character widths at 12px font:
///   - Name (bold 12px trebuchet): ~9px/char
///   - Stereotype (italic 10px trebuchet): ~7.5px/char
///   - Compartment (12px trebuchet): ~7px/char
pub fn estimate_node_size(name: &str, stereotype: &str, child_count: usize) -> Dimension {
    estimate_node_size_ex(name, stereotype, child_count, 0, 0.0, false)
}

// Constants synced with shape-catalog.json and registry-node-view.ts
// These MUST match the view's direct SVG rendering constants.
const DEFAULT_PADDING_LEFT: f64 = 15.0;
const DEFAULT_PADDING_RIGHT: f64 = 15.0;
pub(crate) const DEFAULT_PADDING_TOP: f64 = 36.0; // Header area (stereotype + name + gap)
pub(crate) const DEFAULT_PADDING_BOTTOM: f64 = 4.0; // Minimal bottom margin
const DEFAULT_LINE_HEIGHT: f64 = 14.0; // Matches view LINE_H (compact text)
const DEFAULT_MIN_WIDTH: f64 = 100.0;
const DEFAULT_MIN_HEIGHT: f64 = 44.0;

/// Estimate node size with separate label/node counts and content width.
///
/// - `label_count`: Number of text label children (compartment text lines)
/// - `node_count`: Number of nested SNode children (~80px each when expanded)
/// - `max_content_width`: Widest compartment text line in approximate pixels
/// - `is_expanded`: Whether the node is expanded (children as nodes vs text)
pub fn estimate_node_size_ex(
    name: &str,
    stereotype: &str,
    label_count: usize,
    node_count: usize,
    max_content_width: f64,
    is_expanded: bool,
) -> Dimension {
    // Char width estimates for Manrope bold 13px/700 (~10.5px/char),
    // stereotype is Inter 600 8px uppercase (~6.5px/char). Generous to prevent clipping.
    let name_width = (name.len() as f64) * 10.5 + DEFAULT_PADDING_LEFT + DEFAULT_PADDING_RIGHT;
    let stereo_width =
        (stereotype.len() as f64) * 6.5 + DEFAULT_PADDING_LEFT + DEFAULT_PADDING_RIGHT;
    let header_width = name_width.max(stereo_width);
    // Content width must also be considered for minimum
    let content_width = max_content_width + DEFAULT_PADDING_LEFT + DEFAULT_PADDING_RIGHT;
    let min_width = header_width.max(content_width).max(DEFAULT_MIN_WIDTH);

    // Header height = padding.top (which includes header area)
    let header_height = DEFAULT_PADDING_TOP;

    let children_height = if is_expanded && node_count > 0 {
        // Expanded: nested nodes need more vertical space
        // ELK will compute the actual layout, but we need a reasonable minimum
        DEFAULT_PADDING_BOTTOM + (node_count as f64) * 80.0
    } else if label_count > 0 {
        // Collapsed: text labels at line_height each + bottom padding
        (label_count as f64) * DEFAULT_LINE_HEIGHT + DEFAULT_PADDING_BOTTOM
    } else {
        DEFAULT_PADDING_BOTTOM
    };

    Dimension {
        width: min_width.max(DEFAULT_MIN_WIDTH),
        height: (header_height + children_height).max(DEFAULT_MIN_HEIGHT),
    }
}
