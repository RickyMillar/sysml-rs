/**
 * TypeScript mirror of the serialized Rust `ViewModel` (Bucket 1.2–1.10).
 *
 * Source of truth: `crates/lang/sysml-diagram/src/{view_model,ir/types,
 * design_tokens,text_map}.rs`, serialized with serde's default (no `rename_all`),
 * so struct fields are snake_case and enum variants are PascalCase strings.
 * Externally-tagged enums (`DiagramChild`, `DiagramEdgeKind`) serialize as a
 * single-key object `{ "<Variant>": <payload> }`.
 *
 * This is the contract the SvgCanvas spike consumes. It is intentionally a
 * *subset* — the spike only types the fields it reads. Fields the renderer
 * ignores (buttons, ports, sequence_layout, …) are left loosely typed.
 */

import type { GeometryModel, TableModel, TreeModel } from '@/shared/api/model';

// ── Top-level ViewModel ──────────────────────────────────────────────

export interface ViewModel {
  scene: DiagramIR;
  tokens: DesignTokens;
  text_map: TextMap | null;
  interactions: InteractionMap | null;
  frame: ViewFrame | null;
  /** Non-graph structured model for Grid/Browser/Geometry views. Dedicated
   *  renderers consume it instead of the graph scene; it is `null` for graph
   *  views. */
  non_graph: NonGraphModel | null;
}

/** Tagged non-graph payload (Rust `NonGraphModel`). `data` is the structured
 *  TableModel/TreeModel/GeometryModel the dedicated renderer reads. */
export type NonGraphModel =
  | { kind: 'table'; data: TableModel }
  | { kind: 'tree'; data: TreeModel }
  | { kind: 'geometry'; data: GeometryModel };

export interface DiagramIR {
  view_type: string;
  nodes: DiagramNode[];
  edges: DiagramEdge[];
  buttons: unknown[];
}

// ── Nodes ────────────────────────────────────────────────────────────

export type NodeKind = 'Definition' | 'Usage' | 'Neutral';
export type HeaderStyle = 'Normal' | 'Inline' | 'None';
export type NodeLayout = 'VBox' | 'Free';

export interface DiagramNode {
  element_id: string;
  /** PascalCase `VisualKind` variant name (e.g. "Part", "Package", "State"). */
  visual_kind: string;
  element_kind: string | null;
  node_kind: NodeKind;
  name: string;
  stereotype: string;
  header_style: HeaderStyle;
  children: DiagramChild[];
  ports: DiagramPort[];
  buttons: unknown[];
  expanded: boolean | null;
  tags: string[];
  solver_status: string | null;
  sequence_layout: unknown | null;
  // source_uri/source_range removed in 3.15 — source spans live only in the
  // ViewModel text-map (joined by element_id); the FE link consumes that.
  tooltip: string | null;
  /** Fixed position for Geometry/Grid/Sequence views; else null (→ run elkjs). */
  position: [number, number] | null;
  size: [number, number] | null;
  layout: NodeLayout;
  diagnostic_severity: string | null;
}

// ── Ports (§F-6, IBD/Interconnection) ────────────────────────────────

export type PortSide = 'North' | 'South' | 'East' | 'West';
export type PortDirection = 'In' | 'Out' | 'InOut';

/** A port on a node boundary. Mirrors Rust `DiagramPort` (ir/types.rs). */
export interface DiagramPort {
  element_id: string;
  name: string;
  direction: PortDirection | null;
  is_conjugated: boolean;
  /** §F-6 reference (dotted) port vs behavior port. */
  is_reference: boolean;
  tags: string[];
  sub_ports: DiagramPort[];
  /** Proxy port on an IBD context frame. */
  is_proxy: boolean;
  /** Routing-only port (e.g. state-diagram cardinal ports); never drawn. */
  is_hidden: boolean;
  side: PortSide | null;
  position: [number, number] | null;
  size: [number, number] | null;
}

export type CompartmentItemSource = 'Owned' | 'Inherited' | 'Derived';

/** Externally-tagged `DiagramChild` — exactly one key is present. */
export type DiagramChild =
  | { Node: DiagramNode }
  | {
      Text: {
        compartment: string;
        text: string;
        element_id: string;
        source: CompartmentItemSource;
      };
    }
  | { Compartment: { kind: string; children: DiagramChild[] } }
  | { Island: { view_type: string; display_name: string; subtree: DiagramIR; expanded: boolean } }
  | { Edge: DiagramEdge };

// ── Edges ────────────────────────────────────────────────────────────

/**
 * Externally-tagged `DiagramEdgeKind`. We only branch coarsely on the variant
 * key in the spike (arrow vs diamond vs transition); the payload is loosely
 * typed.
 */
export type DiagramEdgeKind =
  | { Relationship: string }
  | { Transition: { trigger: string | null; guard: string | null } }
  | { Message: { payload: string | null; is_succession: boolean; is_move: boolean; is_push: boolean } }
  | { ControlFlow: { guard: string | null } };

export interface DiagramEdge {
  id: string;
  source_id: string;
  target_id: string;
  kind: DiagramEdgeKind;
  label: string;
  source_port_id: string | null;
  target_port_id: string | null;
  precomputed_route: [number, number][] | null;
  endpoint_mode: 'AutoSide' | 'StrictPort';
  label_placement: unknown;
  tags: string[];
  secondary_labels: { text: string; kind: string }[];
}

// ── Design tokens (palette only, Bucket 1.3) ─────────────────────────

/** `Color` is serde-transparent → a bare CSS string e.g. "oklch(94% 0.04 155)". */
export type Color = string;

export interface CategoryColors {
  fill: Color;
  stroke: Color;
  header: Color | null;
}

export interface Palette {
  bg: Color;
  text: Color;
  muted: Color;
  /** Fine (24px) canvas grid line. */
  grid_minor: Color;
  /** Coarse (96px) canvas grid line. */
  grid_major: Color;
  package: CategoryColors;
  block: CategoryColors;
  action: CategoryColors;
  state: CategoryColors;
  requirement: CategoryColors;
  constraint: CategoryColors;
  interface: CategoryColors;
  item: CategoryColors;
  attribute: CategoryColors;
  enumeration: CategoryColors;
  usecase: CategoryColors;
  allocation: CategoryColors;
  flow: CategoryColors;
  occurrence: CategoryColors;
  view: CategoryColors;
  metadata: CategoryColors;
  comment: CategoryColors;
  node_fallback: CategoryColors;
  actor_stroke: Color;
  select: Color;
  control_fill: Color;
  control_stroke: Color;
  compartment_text: Color;
  body_fill: Color;
  sim: {
    active: Color;
    active_glow: Color;
    transition: Color;
    completed: Color;
    inactive_opacity: number;
  };
  /** Diagnostic-severity ramp (brief §3.4) — NE badge colors. Keys match the
   *  `Severity` lowercase wire values so `sev[entry.severity]` indexes directly. */
  sev: {
    info: Color;
    warning: Color;
    error: Color;
  };
  /** Verdict pill colors (brief §3.5) — keys are `VerdictKind` lowercased. */
  verdict: {
    pass: Color;
    fail: Color;
    inconclusive: Color;
    error: Color;
  };
  /** Port stroke colors by feature direction (Rust `PortColors`; the `in_`
   *  key is the literal serde field name). */
  port: {
    in_: Color;
    out: Color;
    inout: Color;
  };
  /** Edge-label backdrop (carries its own alpha). */
  edge_label_bg: Color;
  // (link / lifeline fields exist but the renderer doesn't read them yet)
  [extra: string]: unknown;
}

/** Serialized `EdgeStyle` for one RelationshipKind (Bucket 3). */
export interface EdgeStyleToken {
  /** `ArrowHead` variant: 'Filled' | 'Hollow' | 'Open' | 'None'. */
  arrowhead: string;
  /** `LineStyle` variant: 'Solid' | 'Dashed' | 'Dotted'. */
  line_style: string;
  /** Stereotype keyword (e.g. "«satisfy»") or null. */
  label: string | null;
}

export interface TypographyTokens {
  /** Font size (px) for node name / header labels. */
  label_font_size_px: number;
  /** Font size (px) for compartment text lines. */
  compartment_font_size_px: number;
  /** Vertical stride (px) between compartment text baselines. */
  compartment_line_stride_px: number;
}

export interface DesignTokens {
  palette: Palette;
  /** VisualKind (variant name) → palette category key (a Palette field name).
   *  Emitted from Rust (F3) so the renderer never re-implements the mapping. */
  categories: Record<string, string>;
  /** VisualKind (variant name) → `Shape` variant name (e.g. "RoundedRect",
   *  "Diamond"). Emitted from Rust (Bucket 3) so the renderer dispatches node
   *  outlines without re-listing control/shape kinds. */
  shapes: Record<string, string>;
  /** RelationshipKind (variant name) → edge styling. Emitted from Rust. */
  edge_styles: Record<string, EdgeStyleToken>;
  /** Typography constants (font sizes + line stride). */
  typography: TypographyTokens;
}

// ── Text map (Bucket 1.6) ────────────────────────────────────────────

export interface TextSpan {
  file: string;
  /** Byte offset (Rust). For ASCII source this equals the UTF-16 offset. */
  start: number;
  end: number;
  line: number;
  col: number;
}

export interface TextMap {
  spans: Record<string, TextSpan>;
}

// ── View frame (§F-10) ───────────────────────────────────────────────

export interface FrameSlot {
  text: string;
}

export interface ViewFrame {
  view_kind: string;
  name: string;
  /** The view's literally-declared immediate type/supertype (R7, §8.2.3.26) —
   *  the heading suffix `«view» Name : type_name`. `null` when the view declares
   *  no type (heading is then just `«view» Name`). */
  type_name: string | null;
  top_right: FrameSlot | null;
  bottom_left: FrameSlot | null;
  bottom_right: FrameSlot | null;
}

// ── Simulation overlay (Bucket 1.8) ──────────────────────────────────

/** Per-tick element activity. Absent (no entry) = inactive. */
// Rust `Activity` serializes `rename_all = "lowercase"` (sim_overlay.rs), so the
// wire values are lowercase — must match exactly for the overlay join.
export type Activity = 'active' | 'completed';

export interface OverlayValue {
  value: number;
  unit: string | null;
}

export interface ElementOverlay {
  activity: Activity | null;
  value: OverlayValue | null;
}

export interface OverlayChannel {
  channel: string;
  /** Scene node id this channel badges onto, if uniquely resolved. */
  element_id: string | null;
  latest: number | null;
  unit: string | null;
}

/** A simulation tick's overlay for a scene — session state, fetched separately
 *  from the (salsa-cached) ViewModel via `sysml.diagram.sim_overlay`. */
export interface SimOverlay {
  tick: number;
  time_ms: number;
  /** ElementId-string → per-element delta (joins to `DiagramNode.element_id`). */
  elements: Record<string, ElementOverlay>;
  channels: OverlayChannel[];
}

// ── Verdict overlay (requirements/parametric Phase 2) ────────────────

/** Constraint solver verdict. Rust `VerdictKind` serializes with its default
 *  derive (PascalCase variant names), so the wire values are exactly these —
 *  must match for the overlay join. (`engine/types.ts` spells the same enum
 *  lowercase because those surfaces go through `Display` instead.) All four
 *  are emitted: a constraint the run could not decide arrives as
 *  'Inconclusive', distinct from a violation. */
export type VerdictKind = 'Pass' | 'Fail' | 'Inconclusive' | 'Error';

export interface ElementVerdict {
  /** Solver verdict for this constraint usage (Pass when satisfied). */
  verdict: VerdictKind | null;
  /** Solved scalar value behind the verdict, when unambiguous (single operand). */
  value: number | null;
}

/** A run's constraint verdicts for a scene — session state, fetched separately
 *  from the (salsa-cached) ViewModel via `sysml.diagram.verdict_overlay`. The
 *  correct home for solver pass/fail state (the retired Parametric generator
 *  wrongly baked this into the graph-pure scene). Merges two producers: the
 *  per-tick solver `constraint_results` and the session's latest
 *  `sysml.sessions.verify` outcome (verification wins on collision). */
export interface VerdictOverlay {
  tick: number;
  time_ms: number;
  /** Tick the stored verification ran at, when its verdicts contributed.
   *  `tick > verified_at_tick` means those verdicts are STALE (the session
   *  advanced since) — kept and labeled, never silently dropped. */
  verified_at_tick?: number | null;
  /** ElementId-string → per-element verdict (joins to `DiagramNode.element_id`). */
  elements: Record<string, ElementVerdict>;
}

// ── Diagnostic overlay (validation badges, ninebar Phase 2 W3) ───────

/** Diagnostic severity — Rust `sysml_span::Severity`, serialized
 *  `rename_all = "lowercase"`, so the wire values are exactly these. Ordered
 *  info < warning < error. */
export type DiagnosticSeverity = 'info' | 'warning' | 'error';

/** A single diagnostic attached to an element (badge tooltip detail). */
export interface DiagnosticItem {
  severity: DiagnosticSeverity;
  message: string;
  code?: string | null;
}

/** One element's diagnostics: worst-case severity (drives the NE badge colour)
 *  plus every message for the tooltip. */
export interface ElementDiagnostics {
  severity: DiagnosticSeverity;
  items: DiagnosticItem[];
}

/** The static diagnostics sidecar for a scene — fetched separately from the
 *  (salsa-cached) ViewModel via `sysml.diagram.diagnostic_overlay` (needs no
 *  session). Sparse: only elements carrying at least one joinable diagnostic. */
export interface DiagnosticOverlay {
  /** ElementId-string → per-element diagnostics (joins to `DiagramNode.element_id`). */
  elements: Record<string, ElementDiagnostics>;
}

// ── Interaction map (Bucket 1.5) ─────────────────────────────────────

/** Semantic affordances for one scene node (Rust `InteractionEntry`). Sparse —
 *  an element only appears in `InteractionMap.entries` when it has an affordance. */
export interface InteractionEntry {
  /** Resolved typing classifier — the go-to-definition target (an ElementId
   *  string), or null. */
  type_definition: string | null;
}

/** Rust `InteractionMap` — `ElementId`-string → affordance entry, joined to the
 *  scene by `DiagramNode.element_id` (sidecar, like `TextMap`). */
export interface InteractionMap {
  entries: Record<string, InteractionEntry>;
}
