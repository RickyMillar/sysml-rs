//! Diagram Intermediate Representation types — the renderer-agnostic, serializable
//! **wire format** (Bucket 1.2).
//!
//! `DiagramIR` is the scene contract every frontend consumes (the new React-SVG
//! renderer, and any future renderer). It sits between `ModelGraph` and any
//! concrete rendering:
//! - Generators produce `DiagramIR` (what should appear) using **typed** fields.
//! - The retired graph-renderer adapter (`render.rs`) converts `DiagramIR` → `legacy graph` (how it
//!   looks in retired graph-renderer), mapping the typed fields to ELK options and CSS classes.
//!
//! ## Wire-format invariants (steward-ruled, 2026-06-24)
//!
//! 1. **No retired graph-renderer/ELK artifacts here.** This type carries *no* ELK option strings
//!    (`elk.algorithm`, `elk.port.side`, …) and *no* CSS-class strings. Layout
//!    algorithm/options are derived per `view_type` inside `render.rs`; CSS is
//!    derived from the typed semantic fields below. The redundant `"elk.port.side"`
//!    string is gone — `DiagramPort::side` is the single source of truth.
//! 2. **Renderer-agnostic semantics.** Decorations that used to be free-form CSS
//!    classes are now compile-enforced enums (`NodeKind`, `NodeTag`, `EdgeTag`,
//!    `SolverStatus`, `CompartmentItemSource`). Each renderer maps these to its own
//!    styling; the IR never names a CSS class.
//! 3. **Identity, not file strings.** Source location is *not* stored on nodes;
//!    it comes from the `ElementId ↔ Span` text-map (Bucket 1.6), keyed by
//!    `element_id`.
//! 4. **serde is feature-gated** (`#[cfg_attr(feature = "serde", …)]`) — see the
//!    crate `serde` feature.

use sysml_core::{ElementKind, RelationshipKind};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::ViewType;
use crate::visual_kind::{CompartmentKind, VisualKind};

// ── Semantic classification enums (replace free-form CSS strings) ──────────

/// Whether a node depicts a **definition** (sharp corners, §F-3) or a **usage**
/// (rounded corners), or neither (synthetic/control nodes). CONFORMS-REQUIRED.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum NodeKind {
    /// `… def` element — drawn with sharp corners.
    Definition,
    /// `…` usage element — drawn with rounded corners.
    Usage,
    /// Synthetic, control, or projection node with no definition/usage nature.
    #[default]
    Neutral,
}

impl NodeKind {
    /// Classify an `ElementKind` as definition/usage/neutral.
    pub fn from_element_kind(kind: &ElementKind) -> Self {
        if kind.is_definition() {
            NodeKind::Definition
        } else if kind.is_usage() {
            NodeKind::Usage
        } else {
            NodeKind::Neutral
        }
    }
}

/// Renderer-agnostic semantic decorations on a node. Replaces the old
/// `css_extras: Vec<String>`. Each renderer maps a tag to its own styling; the
/// IR never names a CSS class. Exhaustive on purpose — adding a tag forces every
/// adapter's `match` to update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum NodeTag {
    /// Actor element (stick-figure notation).
    Actor,
    /// Container wrapping an action sub-diagram.
    ActionContainer,
    /// Browser/tree view node.
    BrowserNode,
    /// Grid column header.
    GridColumn,
    /// Grid row header.
    GridRow,
    /// Grid cell container.
    GridCell,
    /// N-ary relationship branch dot (central junction, §F-9).
    NaryDot,
    /// State/constraint nesting reached the recursion depth limit.
    MaxDepth,
    /// State is a submachine reference (`ref`).
    SubmachineRef,
    /// Exhibit state (named property state).
    ExhibitState,
    /// State has parallel region children.
    ParallelRegions,
    /// Decision/`if` node in an action diagram.
    IfNode,
    /// `perform` action (calls an existing action).
    Perform,
    /// Assignment action (`:=`).
    Assign,
    /// Loop node in an action diagram.
    LoopNode,
    /// `while` loop variant (paired with `LoopNode`).
    LoopWhile,
    /// `for` loop variant (paired with `LoopNode`).
    LoopFor,
    /// Stream source → target action.
    StreamSource,
    /// Constraint block node in a parametric view.
    ParametricConstraint,
    /// `assume` constraint (`isAssume`).
    AssumeConstraint,
    /// `require` constraint (`isRequire`).
    RequireConstraint,
    /// Concern framed by a requirement (`framedConcern`).
    FrameConcern,
    /// Verification case verifying a requirement (`verifiedRequirement`).
    VerifyRequirement,
    /// Sequence-diagram proxy participant node.
    SequenceProxy,
    /// Sequence-diagram lifeline participant.
    Lifeline,
    /// Sequence-diagram lifeline header/label.
    LifelineHead,
}

/// Sequence-diagram lifeline layout payload. This is rendering **data** (not ELK
/// styling) the sequence view needs — it used to be smuggled through
/// `layout_options` as the `lifelineWidth` / `activations` strings. Carried typed
/// here; the retired graph-renderer adapter serializes it back into the legacy graph model layout options.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct SequenceNodeLayout {
    /// Lifeline column width.
    pub lifeline_width: f64,
    /// Activation-box intervals as `(start_y, end_y)` pairs.
    pub activations: Vec<(f64, f64)>,
}

/// Solver satisfaction status badge on a parametric node. Replaces the dynamic
/// `parametric-{pass|fail|unknown}` CSS class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum SolverStatus {
    /// Constraint satisfied (✓).
    Pass,
    /// Constraint violated (✗).
    Fail,
    /// Not yet solved / indeterminate (?).
    Unknown,
}

// ── Graph-level IR ───────────────────────────────────────────────────────

/// Complete diagram IR for one view.
///
/// Layout algorithm and graph-level ELK options are **not** stored here — the
/// retired graph-renderer adapter (`render.rs`) derives them from `view_type` (e.g. fixed layout
/// for Grid/Sequence/Geometry, layered/DOWN for the rest).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiagramIR {
    pub view_type: ViewType,
    pub nodes: Vec<DiagramNode>,
    pub edges: Vec<DiagramEdge>,
    /// Top-level buttons (e.g. "add lifeline" in sequence diagrams).
    pub buttons: Vec<DiagramButton>,
}

// ── Node IR ──────────────────────────────────────────────────────────────

/// A node in the diagram IR.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiagramNode {
    /// Element ID from the ModelGraph, or a synthetic ID for layout helpers.
    pub element_id: String,
    /// Visual classification for this node.
    pub visual_kind: VisualKind,
    /// Underlying spec element kind, when this node maps to a real element.
    /// `None` for synthetic/layout nodes (n-ary dots, grid cells, control nodes).
    /// Carries the precise kind the renderer styles from; supersedes the old
    /// lowercased-`ElementKind` CSS class.
    pub element_kind: Option<ElementKind>,
    /// Definition vs usage nature (§F-3, CONFORMS-REQUIRED).
    pub node_kind: NodeKind,
    /// Display name (may be empty for control nodes).
    pub name: String,
    /// Stereotype text (e.g. "«part definition»"). Empty = no stereotype.
    pub stereotype: String,
    /// How to render the header (stereotype + name).
    pub header_style: HeaderStyle,
    /// Children: nested nodes, text lines, compartments, or sub-diagram islands.
    pub children: Vec<DiagramChild>,
    /// Ports on this node's boundary.
    pub ports: Vec<DiagramPort>,
    /// Interactive buttons (expand/collapse, add message, etc.).
    pub buttons: Vec<DiagramButton>,
    /// Expansion state for container nodes.
    ///
    /// - `None`: Not expandable (leaf node or no meaningful children).
    /// - `Some(false)`: Collapsed — children as text labels in compartments (VBox layout).
    /// - `Some(true)`: Expanded — children as nested DiagramNodes (Free/ELK layout).
    ///
    /// Use `container::apply_expand_controls()` for the common case.
    pub expanded: Option<bool>,
    /// Renderer-agnostic semantic decorations (replaces free-form CSS classes).
    pub tags: Vec<NodeTag>,
    /// Solver satisfaction status (parametric views only).
    pub solver_status: Option<SolverStatus>,
    /// Sequence-diagram lifeline layout data (sequence view only).
    pub sequence_layout: Option<SequenceNodeLayout>,
    // Source location (`source_uri`/`source_range`) was REMOVED in 3.15: the
    // `ViewModel`'s `ElementId↔Span` text-map (`crate::text_map`, Bucket 1.6) is
    // the single typed home for source spans — look them up by `element_id`. The
    // FE link already consumes the text-map (byte-offset accurate). The legacy
    // legacy graph carried these on `SNode` for VS Code go-to-source / the legacy graph
    // diagnostic overlay; that path is unmaintained (we've moved to the ViewModel
    // renderer), so `SNode` source is now `None` and the legacy graph source overlay is
    // inert. See the ViewModel node contract if the legacy path is ever revived.
    /// Tooltip text.
    pub tooltip: Option<String>,
    /// Fixed position (for Geometry/Grid/Sequence views).
    pub position: Option<(f64, f64)>,
    /// Fixed size (for Geometry/Grid/Sequence views).
    pub size: Option<(f64, f64)>,
    /// Layout mode for children.
    pub layout: NodeLayout,
    /// Diagnostic severity overlay ("error", "warning", "info", "hint").
    pub diagnostic_severity: Option<String>,
}

/// How to render a node's header (stereotype + name).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum HeaderStyle {
    /// Standard: stereotype label + name label in a header compartment.
    Normal,
    /// Single inline label (for Browser, Grid cells). No compartment wrapper.
    Inline,
    /// No header at all (for control nodes, proxy nodes).
    None,
}

/// Layout mode for a node's children.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum NodeLayout {
    /// Vertical box layout (retired graph-renderer VBoxLayouter).
    VBox,
    /// No layout — children positioned by ELK or fixed coordinates.
    Free,
}

// ── Child IR ─────────────────────────────────────────────────────────────

/// Provenance of a compartment item (§F-4, CONFORMS-REQUIRED). Distinguishes
/// owned features from inherited (`^` redefinition prefix) and derived (`/`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum CompartmentItemSource {
    /// Feature declared directly on this element.
    #[default]
    Owned,
    /// Feature inherited from a supertype (rendered with `^`).
    Inherited,
    /// Derived feature (rendered with `/`).
    Derived,
}

/// Child content within a node.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum DiagramChild {
    /// Nested node (expanded child).
    Node(DiagramNode),

    /// Text line in a compartment (collapsed child).
    ///
    /// Dual-projection invariant (§F-2): an expanded child is a
    /// `DiagramChild::Node` and its collapsed form is a `DiagramChild::Text`
    /// with the **same** `element_id`. Never two distinct IDs for one element.
    Text {
        compartment: CompartmentKind,
        text: String,
        element_id: String,
        /// Owned / inherited / derived provenance (§F-4).
        source: CompartmentItemSource,
    },

    /// Pre-structured compartment with explicit children.
    /// Used when a generator needs to control compartment content directly
    /// (e.g. documentation, relationship references, enumerations).
    Compartment {
        kind: CompartmentKind,
        children: Vec<DiagramChild>,
    },

    /// Sub-diagram island (state, action, IBD, sequence).
    Island {
        view_type: ViewType,
        display_name: String,
        subtree: DiagramIR,
        expanded: bool,
    },

    /// Edge nested inside a container node (ELK requires edges at LCA level).
    Edge(DiagramEdge),
}

// ── Port IR ──────────────────────────────────────────────────────────────

/// A port on a node boundary.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiagramPort {
    pub element_id: String,
    pub name: String,
    pub direction: Option<PortDirection>,
    pub is_conjugated: bool,
    /// Reference port (dotted variant, §F-6 — drawn distinctly from a behavior port).
    pub is_reference: bool,
    /// Renderer-agnostic semantic decorations (replaces free-form port CSS classes).
    pub tags: Vec<PortTag>,
    /// Nested sub-ports (for composite port hierarchies).
    pub sub_ports: Vec<DiagramPort>,
    /// Whether this is a proxy port on a context frame (IBD).
    pub is_proxy: bool,
    /// Whether this port is hidden (used for routing, not displayed).
    /// State diagrams inject hidden cardinal ports for edge routing.
    pub is_hidden: bool,
    /// Fixed port side constraint (§F-6 anchor side). Single source of truth —
    /// the retired graph-renderer adapter derives the `elk.port.side` option from this.
    pub side: Option<PortSide>,
    /// Fixed position (for Sequence proxy nodes, etc.).
    pub position: Option<(f64, f64)>,
    /// Fixed size (scale by depth for composite ports).
    pub size: Option<(f64, f64)>,
}

/// Renderer-agnostic semantic decoration on a port (replaces the old port
/// `css_extras`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PortTag {
    /// Control-flow port on an action node (fork/join/decision/merge).
    Control,
    /// Parameter/attribute port on a parametric constraint block.
    Parametric,
    /// Parametric solver badge — constraint parameter solved.
    BadgeSolved,
    /// Parametric solver badge — parameter not yet solved.
    BadgeUnsolved,
    /// Parametric solver badge — parameter violates its constraint.
    BadgeViolated,
}

/// Port direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PortDirection {
    In,
    Out,
    InOut,
}

/// Fixed port side for ELK placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum PortSide {
    North,
    South,
    East,
    West,
}

impl PortSide {
    pub fn as_elk_str(&self) -> &'static str {
        match self {
            PortSide::North => "NORTH",
            PortSide::South => "SOUTH",
            PortSide::East => "EAST",
            PortSide::West => "WEST",
        }
    }
}

// ── Edge IR ──────────────────────────────────────────────────────────────

/// Renderer-agnostic semantic decoration on an edge. Replaces the old edge
/// `css_extras: Vec<String>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum EdgeTag {
    /// Binding connector (parametric view — equates parameters).
    BindingConnector,
    /// Message looping back to the same lifeline (sequence).
    SelfMessage,
    /// Return message (sequence).
    Return,
    /// Comment/note edge (sequence).
    Comment,
    /// Message edge in an action diagram.
    Message,
    /// N-ary relationship branch segment (radiates from a `NodeTag::NaryDot`, §F-9).
    NarySegment,
}

/// A secondary label rendered below an edge's primary label (e.g. a trigger
/// source annotation). Replaces the old `(text, css_class)` string tuple.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EdgeSubLabel {
    pub text: String,
    pub kind: EdgeSubLabelKind,
}

/// Semantic role of an edge secondary label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum EdgeSubLabelKind {
    /// Transition trigger source annotation (`[via port]`).
    TriggerSource,
}

/// An edge in the diagram IR.
///
/// Composite vs shared aggregation (§F-8, filled vs open diamond) is encoded by
/// the `RelationshipKind` carried in `kind` (`Composition` vs `FeatureMembership`)
/// — there is no separate `is_composite` flag (single source of truth).
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiagramEdge {
    /// Unique edge ID.
    pub id: String,
    /// Source node element ID.
    pub source_id: String,
    /// Target node element ID.
    pub target_id: String,
    /// What kind of edge this is.
    pub kind: DiagramEdgeKind,
    /// Label text (may be empty).
    pub label: String,
    /// Source port ID (for port-to-port routing).
    pub source_port_id: Option<String>,
    /// Target port ID (for port-to-port routing).
    pub target_port_id: Option<String>,
    /// Precomputed route points (for sequence diagrams).
    pub precomputed_route: Option<Vec<(f64, f64)>>,
    /// How endpoints attach to nodes.
    pub endpoint_mode: EndpointMode,
    /// Label placement configuration.
    pub label_placement: EdgeLabelPlacement,
    /// Renderer-agnostic semantic decorations (replaces free-form CSS classes).
    pub tags: Vec<EdgeTag>,
    /// Secondary labels rendered below the primary label
    /// (trigger source annotations, value badges, etc.).
    pub secondary_labels: Vec<EdgeSubLabel>,
}

/// How edge endpoints attach to nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum EndpointMode {
    /// ELK routes to closest side (default for most edges).
    AutoSide,
    /// Endpoints attach to specific ports (IBD connections).
    StrictPort,
}

/// Edge label placement configuration.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct EdgeLabelPlacement {
    /// Position along the edge (0.0 = source, 0.5 = midpoint, 1.0 = target).
    pub position: f64,
    /// Which side of the edge: "on", "left", "right".
    pub side: String,
    /// Perpendicular offset from edge line.
    pub offset: Option<f64>,
    /// Whether to rotate label to follow edge direction.
    pub rotate: bool,
}

impl Default for EdgeLabelPlacement {
    fn default() -> Self {
        Self {
            position: 0.5,
            side: "on".to_owned(),
            offset: None,
            rotate: false,
        }
    }
}

/// The kind of a diagram edge.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum DiagramEdgeKind {
    /// Standard relationship edge. The `RelationshipKind` encodes composite vs
    /// shared aggregation (§F-8): `Composition` → filled diamond,
    /// `FeatureMembership` → open diamond.
    Relationship(RelationshipKind),
    /// State machine transition.
    Transition {
        trigger: Option<String>,
        guard: Option<String>,
    },
    /// Sequence diagram message.
    Message {
        payload: Option<String>,
        is_succession: bool,
        is_move: bool,
        is_push: bool,
    },
    /// Action flow control edge.
    ControlFlow {
        guard: Option<String>,
    },
}

// ── Button IR ────────────────────────────────────────────────────────────

/// An interactive button in the diagram.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct DiagramButton {
    /// Button type determines rendering and behavior.
    pub button_type: ButtonType,
    /// Fixed position (if any).
    pub position: Option<(f64, f64)>,
    /// Fixed size (if any).
    pub size: Option<(f64, f64)>,
}

/// Button type and associated metadata.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum ButtonType {
    /// Expand/collapse toggle.
    Expand,
    /// Add a message at a specific position in a sequence diagram.
    AddMessage {
        lifeline_id: String,
        insertion_index: usize,
    },
    /// Add a new lifeline to a sequence diagram.
    AddLifeline,
}

// ── Convenience constructors ─────────────────────────────────────────────

impl DiagramIR {
    pub fn new(view_type: ViewType) -> Self {
        Self {
            view_type,
            nodes: Vec::new(),
            edges: Vec::new(),
            buttons: Vec::new(),
        }
    }

    /// Create for a fixed-layout view (Grid, Sequence, Geometry).
    ///
    /// Layout selection now lives in the retired graph-renderer adapter keyed on `view_type`,
    /// so this is a thin alias of [`DiagramIR::new`] retained for call-site
    /// readability.
    pub fn new_fixed(view_type: ViewType) -> Self {
        Self::new(view_type)
    }
}

impl DiagramNode {
    /// Create a minimal node with standard header style.
    pub fn new(element_id: impl Into<String>, visual_kind: VisualKind, name: impl Into<String>) -> Self {
        Self {
            element_id: element_id.into(),
            visual_kind,
            element_kind: None,
            node_kind: NodeKind::Neutral,
            name: name.into(),
            stereotype: String::new(),
            header_style: HeaderStyle::Normal,
            children: Vec::new(),
            ports: Vec::new(),
            buttons: Vec::new(),
            expanded: None,
            tags: Vec::new(),
            solver_status: None,
            sequence_layout: None,
            tooltip: None,
            position: None,
            size: None,
            layout: NodeLayout::VBox,
            diagnostic_severity: None,
        }
    }

    pub fn with_stereotype(mut self, stereotype: impl Into<String>) -> Self {
        self.stereotype = stereotype.into();
        self
    }

    pub fn with_header_style(mut self, style: HeaderStyle) -> Self {
        self.header_style = style;
        self
    }

    pub fn with_expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// Set the underlying spec element kind, deriving `node_kind` from it.
    pub fn with_element_kind(mut self, kind: ElementKind) -> Self {
        self.node_kind = NodeKind::from_element_kind(&kind);
        self.element_kind = Some(kind);
        self
    }

    pub fn with_node_kind(mut self, node_kind: NodeKind) -> Self {
        self.node_kind = node_kind;
        self
    }

    pub fn with_position(mut self, x: f64, y: f64) -> Self {
        self.position = Some((x, y));
        self
    }

    pub fn with_size(mut self, w: f64, h: f64) -> Self {
        self.size = Some((w, h));
        self
    }

    pub fn with_layout(mut self, layout: NodeLayout) -> Self {
        self.layout = layout;
        self
    }

    /// Add a renderer-agnostic semantic tag.
    pub fn with_tag(mut self, tag: NodeTag) -> Self {
        self.tags.push(tag);
        self
    }

    /// Add the given semantic tags.
    pub fn with_tags(mut self, tags: impl IntoIterator<Item = NodeTag>) -> Self {
        self.tags.extend(tags);
        self
    }

    /// Add an expand button to this node.
    pub fn with_expand_button(mut self) -> Self {
        self.buttons.push(DiagramButton {
            button_type: ButtonType::Expand,
            position: None,
            size: None,
        });
        self
    }
}

impl DiagramEdge {
    /// Create a relationship edge with default placement.
    pub fn relationship(
        id: impl Into<String>,
        source_id: impl Into<String>,
        target_id: impl Into<String>,
        kind: RelationshipKind,
        label: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            source_id: source_id.into(),
            target_id: target_id.into(),
            kind: DiagramEdgeKind::Relationship(kind),
            label: label.into(),
            source_port_id: None,
            target_port_id: None,
            precomputed_route: None,
            endpoint_mode: EndpointMode::AutoSide,
            label_placement: EdgeLabelPlacement::default(),
            tags: Vec::new(),
            secondary_labels: Vec::new(),
        }
    }

    /// Create a transition edge.
    pub fn transition(
        id: impl Into<String>,
        source_id: impl Into<String>,
        target_id: impl Into<String>,
        trigger: Option<String>,
        guard: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            source_id: source_id.into(),
            target_id: target_id.into(),
            kind: DiagramEdgeKind::Transition { trigger, guard },
            label: String::new(),
            source_port_id: None,
            target_port_id: None,
            precomputed_route: None,
            endpoint_mode: EndpointMode::AutoSide,
            label_placement: EdgeLabelPlacement::default(),
            tags: Vec::new(),
            secondary_labels: Vec::new(),
        }
    }

    /// Create a message edge (sequence diagram).
    pub fn message(
        id: impl Into<String>,
        source_id: impl Into<String>,
        target_id: impl Into<String>,
        payload: Option<String>,
        is_succession: bool,
        is_move: bool,
        is_push: bool,
    ) -> Self {
        Self {
            id: id.into(),
            source_id: source_id.into(),
            target_id: target_id.into(),
            kind: DiagramEdgeKind::Message {
                payload,
                is_succession,
                is_move,
                is_push,
            },
            label: String::new(),
            source_port_id: None,
            target_port_id: None,
            precomputed_route: None,
            endpoint_mode: EndpointMode::AutoSide,
            label_placement: EdgeLabelPlacement::default(),
            tags: Vec::new(),
            secondary_labels: Vec::new(),
        }
    }

    /// Create a control flow edge (action diagram).
    pub fn control_flow(
        id: impl Into<String>,
        source_id: impl Into<String>,
        target_id: impl Into<String>,
        guard: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            source_id: source_id.into(),
            target_id: target_id.into(),
            kind: DiagramEdgeKind::ControlFlow { guard },
            label: String::new(),
            source_port_id: None,
            target_port_id: None,
            precomputed_route: None,
            endpoint_mode: EndpointMode::AutoSide,
            label_placement: EdgeLabelPlacement::default(),
            tags: Vec::new(),
            secondary_labels: Vec::new(),
        }
    }

    pub fn with_ports(mut self, source_port: impl Into<String>, target_port: impl Into<String>) -> Self {
        self.source_port_id = Some(source_port.into());
        self.target_port_id = Some(target_port.into());
        self.endpoint_mode = EndpointMode::StrictPort;
        self
    }

    pub fn with_route(mut self, points: Vec<(f64, f64)>) -> Self {
        self.precomputed_route = Some(points);
        self
    }

    pub fn with_label_placement(mut self, placement: EdgeLabelPlacement) -> Self {
        self.label_placement = placement;
        self
    }

    /// Add a renderer-agnostic semantic tag.
    pub fn with_tag(mut self, tag: EdgeTag) -> Self {
        self.tags.push(tag);
        self
    }
}

impl DiagramPort {
    pub fn new(element_id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            element_id: element_id.into(),
            name: name.into(),
            direction: None,
            is_conjugated: false,
            is_reference: false,
            tags: Vec::new(),
            sub_ports: Vec::new(),
            is_proxy: false,
            is_hidden: false,
            side: None,
            position: None,
            size: None,
        }
    }

    pub fn with_direction(mut self, dir: PortDirection) -> Self {
        self.direction = Some(dir);
        self
    }

    pub fn with_side(mut self, side: PortSide) -> Self {
        self.side = Some(side);
        self
    }

    pub fn with_size(mut self, w: f64, h: f64) -> Self {
        self.size = Some((w, h));
        self
    }

    /// Add a renderer-agnostic semantic port tag.
    pub fn with_tag(mut self, tag: PortTag) -> Self {
        self.tags.push(tag);
        self
    }

    pub fn hidden(mut self) -> Self {
        self.is_hidden = true;
        self
    }

    pub fn proxy(mut self) -> Self {
        self.is_proxy = true;
        self
    }
}

impl DiagramButton {
    pub fn expand() -> Self {
        Self {
            button_type: ButtonType::Expand,
            position: None,
            size: None,
        }
    }

    pub fn add_message(lifeline_id: impl Into<String>, insertion_index: usize) -> Self {
        Self {
            button_type: ButtonType::AddMessage {
                lifeline_id: lifeline_id.into(),
                insertion_index,
            },
            position: None,
            size: None,
        }
    }

    pub fn add_lifeline() -> Self {
        Self {
            button_type: ButtonType::AddLifeline,
            position: None,
            size: None,
        }
    }

    /// Set fixed position and size (for sequence diagram buttons).
    pub fn with_position_size(mut self, x: f64, y: f64, w: f64, h: f64) -> Self {
        self.position = Some((x, y));
        self.size = Some((w, h));
        self
    }
}
