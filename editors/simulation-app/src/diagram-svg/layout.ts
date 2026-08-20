/**
 * elkjs layout + orthogonal edge routing for the SvgCanvas spike.
 *
 * This deliberately co-locates BOTH node placement and orthogonal edge routing
 * in one native pass: elkjs (`elk.layered` + `elk.edgeRouting: ORTHOGONAL`) does
 * both. The spike does NOT use the Rust `sysml-layout` WASM router — keeping
 * layout and routing together in one frontend tool is the point (see findings).
 *
 * Input: the promoted `DiagramIR` scene. Output: a flat list of absolutely-
 * positioned nodes (with their compartment text lines for rendering) and a list
 * of routed edges (absolute polyline points + a label anchor). Pure async fn of
 * the scene → unit-testable with a stub ELK.
 */

import ELK from 'elkjs/lib/elk.bundled.js';
import elkWorkerUrl from 'elkjs/lib/elk-worker.min.js?url';
import type { ElkNode, ElkExtendedEdge, ElkLabel, LayoutOptions } from 'elkjs';
import type {
  DiagramChild,
  DiagramEdge,
  DiagramIR,
  DiagramNode,
  DiagramPort,
  EdgeStyleToken,
  PortSide,
} from './viewmodel-types';
import { glyphSizeFor, shapeForVisualKind } from './shapes';
import { edgeDecor } from './edges';
import { chipSize, wrapLabel } from './label-layout';

const FONT = 12;
const CHAR_W = 6.6; // rough advance at 12px
const LINE_H = 16;
const NAME_H = 22;
const STEREO_H = 14;
const PAD_X = 12;
const PAD_Y = 8;
const MIN_W = 120;
const CONTROL_SIZE = 18;
const PORT_SIZE = 10;
const PORT_LABEL_H = 10;
/** Port-name label advance at the 8px size it actually renders (see the `Ports`
 *  renderer) — elk must reserve THIS width, not the 12px `textWidth`, or its
 *  returned label boxes over-reserve and phantom-overlap their neighbours. */
const PORT_LABEL_CHAR_W = 4.4;

/** A port placed on a node boundary (absolute coords, from elkjs). */
export interface PlacedPort {
  port: DiagramPort;
  x: number;
  y: number;
  width: number;
  height: number;
  /** elk-computed port-label box (absolute), or null when elk placed no label
   *  (fixed-layout path / unlabelled port) — the renderer falls back to a
   *  side-aware position (§2). */
  labelRect: { x: number; y: number; width: number; height: number } | null;
}

/** Map the Rust PortSide enum to elkjs's `elk.port.side`. */
function elkPortSide(side: PortSide | null): string | undefined {
  switch (side) {
    case 'North':
      return 'NORTH';
    case 'South':
      return 'SOUTH';
    case 'East':
      return 'EAST';
    case 'West':
      return 'WEST';
    default:
      return undefined;
  }
}

export interface CompartmentLine {
  text: string;
  elementId: string;
  /** 'Owned' | 'Inherited' | 'Derived' — drives the §F-4 prefix glyph. */
  source: string;
}

export interface PlacedNode {
  node: DiagramNode;
  x: number;
  y: number;
  width: number;
  height: number;
  depth: number;
  /** Header height inside this node (for the header/body split). */
  headerHeight: number;
  /** Collapsed/leaf compartment lines to render inside the body. */
  lines: CompartmentLine[];
  /** Whether this node has placed child nodes (a container). */
  hasChildren: boolean;
  /** Ports placed on this node's boundary by elkjs (absolute coords). */
  ports: PlacedPort[];
}

export interface PlacedEdge {
  edge: DiagramIR['edges'][number];
  points: { x: number; y: number }[];
  label: string;
  /** Preferred center-label anchor: elk's inline-label center when elk placed
   *  one, else the routed-path midpoint vertex. Seeds the FE resolver (§1). */
  labelAt: { x: number; y: number } | null;
}

export interface LayoutResult {
  nodes: PlacedNode[];
  edges: PlacedEdge[];
  width: number;
  height: number;
}

function textWidth(s: string): number {
  return s.length * CHAR_W;
}

/** True when a sub-diagram carries its own complete geometry: the generator
 *  computed every top-level position (Sequence / Grid / Geometry), so elk has
 *  nothing to solve and re-solving it DESTROYS the authored layout. Same
 *  contract as the whole-scene check in `layoutScene`. */
function isFixedSubtree(ir: DiagramIR): boolean {
  return ir.nodes.length > 0 && ir.nodes.every((n) => n.position != null);
}

/** Flatten a node's children into (childNodes, compartmentLines, islandNodes,
 *  fixedIslands).
 *  `islandNodes` are the top-level nodes of every EXPANDED elk-laid embedded
 *  sub-diagram (`DiagramChild::Island`) — they lay out as children of this node
 *  so the sub-diagram (state machine / action flow / IBD internals) renders
 *  nested inside its host. Their own nesting + the island's edges are resolved
 *  by `toElkNode` recursion and `gatherEdges`.
 *
 *  `fixedIslands` are the EXPANDED islands whose subtree is already fully
 *  positioned (`isFixedSubtree` — in practice the sequence island). Flattening
 *  those into elk discarded both their positions AND their sizes: a lifeline
 *  head is a container (it nests message proxies), and the container branch of
 *  `toElkNode` ignores `node.size`, so an authored 282×166 head shrank to the
 *  ~56px elk derived from two 6×6 proxies — then interleaved as a sibling of
 *  the host's IBD part boxes, which is the collision on scratch / OverviewView
 *  / RenderedView (D-S1). They lay out as ONE opaque spacer sized to the
 *  sub-diagram's bounding box, and their interior is spliced back in at the
 *  spacer's placed origin (see `layoutScene`). */
function partitionChildren(children: DiagramChild[]): {
  childNodes: DiagramNode[];
  lines: CompartmentLine[];
  islandNodes: DiagramNode[];
  fixedIslands: DiagramIR[];
} {
  const childNodes: DiagramNode[] = [];
  const lines: CompartmentLine[] = [];
  const islandNodes: DiagramNode[] = [];
  const fixedIslands: DiagramIR[] = [];
  const walk = (cs: DiagramChild[]) => {
    for (const c of cs) {
      if ('Node' in c) {
        childNodes.push(c.Node);
      } else if ('Text' in c) {
        lines.push({ text: c.Text.text, elementId: c.Text.element_id, source: c.Text.source });
      } else if ('Compartment' in c) {
        walk(c.Compartment.children);
      } else if ('Island' in c && c.Island.expanded) {
        if (isFixedSubtree(c.Island.subtree)) fixedIslands.push(c.Island.subtree);
        else islandNodes.push(...c.Island.subtree.nodes);
      }
      // Collapsed islands + Edge children contribute no nested layout nodes.
    }
  };
  walk(children);
  return { childNodes, lines, islandNodes, fixedIslands };
}

/** Stable elk id for the spacer standing in for a fixed island. Namespaced so
 *  it can never collide with an element id. */
function fixedIslandSpacerId(hostId: string, index: number): string {
  return `__fixed-island__/${hostId}/${index}`;
}

function headerHeightFor(node: DiagramNode): number {
  if (node.header_style === 'None') return 0;
  return NAME_H + (node.stereotype ? STEREO_H : 0) + 4;
}

function nodeWidthFor(node: DiagramNode, lines: CompartmentLine[]): number {
  const header = Math.max(textWidth(node.name), textWidth(node.stereotype));
  const body = lines.reduce((m, l) => Math.max(m, textWidth(l.text) + 8), 0);
  return Math.max(MIN_W, Math.ceil(Math.max(header, body) + 2 * PAD_X));
}

/** Optional layout hints for an interactive re-layout (post-drag-drop). When
 *  present, elk runs in INTERACTIVE mode seeded with the given absolute node
 *  positions, so it re-routes edges orthogonally (obstacle-avoiding) while
 *  keeping nodes near where the user dropped them. */
export interface LayoutHints {
  /** element_id → absolute (root-relative) position to seed the node at. */
  positions: Record<string, { x: number; y: number }>;
  interactive: boolean;
}

/** Build the ELK node tree, recording per-node render metadata in `meta`.
 *  `shapes` is the Rust-emitted `VisualKind → Shape` map; when present, glyph
 *  nodes (diamonds, fork bars, control circles) get a shape-appropriate box
 *  instead of a text-derived one. `hints` seeds interactive node positions
 *  (elk wants child coords relative to the parent, so absolute hints are
 *  converted using `parentAbs`). */
type NodeMeta = {
  node: DiagramNode;
  lines: CompartmentLine[];
  headerHeight: number;
  hasChildren: boolean;
  /** Visible (non-hidden) ports, in scene order — paired with elk positions. */
  ports: DiagramPort[];
};

function toElkNode(
  node: DiagramNode,
  depth: number,
  meta: Map<string, NodeMeta>,
  shapes?: Record<string, string>,
  hints?: LayoutHints,
  parentAbs: { x: number; y: number } = { x: 0, y: 0 },
  fixedIslandLayouts?: Map<string, LayoutResult>,
): ElkNode {
  const { childNodes, lines, islandNodes, fixedIslands } = partitionChildren(node.children);
  const nodeExpanded = node.expanded === true && childNodes.length > 0;
  // A node is a layout container if it nests expanded child nodes and/or any
  // expanded island sub-diagram. Islands nest regardless of the host's own
  // expand state (the island carries its own `expanded` flag).
  const innerNodes = nodeExpanded ? [...childNodes, ...islandNodes] : islandNodes;
  // Fixed islands enter the graph as opaque spacers sized to their own
  // pre-solved bounding box (see `partitionChildren`); elk places the box, and
  // the interior is spliced back in afterwards.
  const fixedSpacers: ElkNode[] = fixedIslands.map((ir, i) => {
    const id = fixedIslandSpacerId(node.element_id, i);
    const solved = layoutFixed(ir, shapes);
    fixedIslandLayouts?.set(id, solved);
    return { id, width: solved.width, height: solved.height };
  });
  const isContainer = innerNodes.length + fixedSpacers.length > 0;
  const headerHeight = headerHeightFor(node);
  const visiblePorts = (node.ports ?? []).filter((p) => !p.is_hidden);
  meta.set(node.element_id, { node, lines, headerHeight, hasChildren: isContainer, ports: visiblePorts });

  // Interactive seed position (absolute → relative to this node's parent).
  const abs = hints?.positions[node.element_id];
  const pos = abs ? { x: abs.x - parentAbs.x, y: abs.y - parentAbs.y } : {};
  const childAbs = abs ?? parentAbs;

  // Ports on the node boundary (IBD / §F-6). When the backend assigns a side
  // (N/S/E/W) we pin it via `elk.port.side` under FIXED_SIDE so ports sit on the
  // authored boundary; otherwise FREE lets elk distribute them to minimise edge
  // crossings. Edges route to these port ids (see collectEdges), so elk attaches
  // them on the boundary.
  const anyPortSide = visiblePorts.some((p) => elkPortSide(p.side) !== undefined);
  const portBits =
    visiblePorts.length > 0
      ? {
          ports: visiblePorts.map((p) => {
            const side = elkPortSide(p.side);
            // Reserve room for the port label in elk (§2): a native label makes
            // elk keep neighbours clear of the label, not just the glyph.
            const labels: ElkLabel[] = p.name
              ? [{ text: p.name, width: p.name.length * PORT_LABEL_CHAR_W, height: PORT_LABEL_H }]
              : [];
            return {
              id: p.element_id,
              width: PORT_SIZE,
              height: PORT_SIZE,
              ...(labels.length ? { labels } : {}),
              ...(side ? { layoutOptions: { 'elk.port.side': side } } : {}),
            };
          }),
        }
      : {};
  const portConstraint: LayoutOptions =
    visiblePorts.length > 0
      ? { 'elk.portConstraints': anyPortSide ? 'FIXED_SIDE' : 'FREE' }
      : {};
  // Port-label reservation (§2): place labels OUTSIDE the node next to their
  // port, widen the port-port gap (a 10px glyph + ~40px label can't sit 12px
  // apart), and pad the container so labels clear its own children.
  // MEASURED (#71) — the standing G3 failure on AllPartsView / EmptyExposeView
  // is NOT reachable through `spacing.portPort`. Probing the live dump:
  //   * The two clashing ports sit on the SAME node and the SAME side (the old
  //     note here blamed ports on ADJACENT nodes — that was wrong).
  //   * Both have `side: null`, so the node is on elk's `portConstraints: FREE`
  //     path. This is downstream of R4, which correctly stopped inventing a
  //     direction from the port NAME; nothing has since supplied a real one.
  //   * Deriving `portPort` from the widest label (emitted as 44 instead of 24,
  //     verified in the elk input) changes the output by exactly nothing —
  //     under FREE, elk does not honour it. That attempt was reverted.
  //   * elk then emits OVERLAPPING label boxes itself, placing each label
  //     inward toward the gap between the two glyphs (35.2px labels at x=596.8
  //     and x=608.2 on 10px glyphs 35px apart).
  // So the fix is either a real declared port side (model/composer side) or a
  // deterministic FE de-overlap pass over one node's own port labels — not an
  // elk spacing option.
  const portLabelOpts: LayoutOptions =
    visiblePorts.some((p) => p.name)
      ? {
          'org.eclipse.elk.portLabels.placement': 'OUTSIDE',
          'org.eclipse.elk.portLabels.nextToPortIfPossible': 'true',
          'org.eclipse.elk.spacing.portPort': '24',
          'org.eclipse.elk.spacing.portsSurrounding': '[top=12,left=12,bottom=12,right=12]',
        }
      : {};

  if (isContainer) {
    // A container that ALSO carries compartment rows (e.g. a part with both
    // attributes and nested parts) renders those rows in a band directly under
    // the header (SvgCanvas §compartment). elk must reserve that band in the top
    // padding — otherwise children overlap the attribute text and tall
    // compartments spill below the box (corePhysics #67). The reserved height
    // mirrors the leaf body formula: lines.length * LINE_H (stride) + PAD_Y.
    const compartmentBand = lines.length > 0 ? lines.length * LINE_H + PAD_Y : 0;
    return {
      id: node.element_id,
      ...pos,
      ...portBits,
      layoutOptions: {
        'elk.padding': `[top=${headerHeight + compartmentBand + PAD_Y},left=${PAD_X},bottom=${PAD_Y + 4},right=${PAD_X}]`,
        ...portConstraint,
        ...portLabelOpts,
      },
      children: [
        ...innerNodes.map((c) =>
          toElkNode(c, depth + 1, meta, shapes, hints, childAbs, fixedIslandLayouts),
        ),
        ...fixedSpacers,
      ],
    };
  }

  const leafOpts =
    visiblePorts.length > 0 ? { layoutOptions: { ...portConstraint, ...portLabelOpts } } : {};

  // Leaf / collapsed node.
  if (node.size) {
    return { id: node.element_id, ...pos, ...portBits, ...leafOpts, width: node.size[0], height: node.size[1] };
  }
  // Glyph (non-card) shapes get a fixed, shape-appropriate box.
  const glyph = shapes ? glyphSizeFor(shapeForVisualKind(shapes, node.visual_kind)) : null;
  if (glyph) {
    return { id: node.element_id, ...pos, ...portBits, ...leafOpts, width: glyph.w, height: glyph.h };
  }
  // Card / text-derived size from header + compartment lines. A leaf with no
  // compartment lines collapses to header-only (D-L6) — no reserved blank
  // body strip pretending content exists.
  const width = nodeWidthFor(node, lines);
  const isControl = node.header_style === 'None' && lines.length === 0 && !node.name;
  const height = isControl
    ? CONTROL_SIZE
    : headerHeight + (lines.length > 0 ? lines.length * LINE_H + PAD_Y : 0);
  return { id: node.element_id, ...pos, ...portBits, ...leafOpts, width, height };
}

/**
 * Build a child-node → parent-node id map by walking the scene's `DiagramChild`
 * nesting (skipping `Compartment` wrappers). Used by the renderer so dragging a
 * container carries its descendants: a node's effective drag delta is the sum of
 * its own delta and every scene ancestor's delta. Only `Node` nesting counts —
 * compartment text lines aren't draggable nodes.
 */
export function buildParentMap(scene: DiagramIR): Record<string, string> {
  const parent: Record<string, string> = {};
  const walk = (children: DiagramChild[], parentId: string) => {
    for (const c of children) {
      if ('Node' in c) {
        parent[c.Node.element_id] = parentId;
        walk(c.Node.children, c.Node.element_id);
      } else if ('Compartment' in c) {
        walk(c.Compartment.children, parentId);
      } else if ('Island' in c && c.Island.expanded) {
        // An expanded island's subtree nodes are nested inside the host, so a
        // host drag must carry them too.
        for (const sn of c.Island.subtree.nodes) {
          parent[sn.element_id] = parentId;
          walk(sn.children, sn.element_id);
        }
      }
      // Edge / Text children don't contribute draggable node nesting.
    }
  };
  for (const n of scene.nodes) walk(n.children, n.element_id);
  return parent;
}

/** The composed center-label text elk should reserve room for — exactly what
 *  the renderer draws (`edgeDecor(...).label`), so the reserved box matches the
 *  drawn chip. */
function composedEdgeLabel(e: DiagramEdge, edgeStyles: Record<string, EdgeStyleToken>): string {
  return edgeDecor(e, edgeStyles).label;
}

function collectEdges(
  edges: DiagramEdge[],
  visiblePortIds: Set<string>,
  edgeStyles: Record<string, EdgeStyleToken>,
): ElkExtendedEdge[] {
  // Route port-to-port when the edge carries port ids (IBD StrictPort edges);
  // elk attaches the edge to the port on the node boundary. Falls back to the
  // node id for plain (AutoSide) edges — AND for edges whose port is not in
  // the elk graph: the composer routes behavior edges to HIDDEN
  // (routing-only, never-drawn) ports which this layout filters out, and an
  // edge referencing a missing shape is a hard elk import error (D-B2).
  const endpoint = (portId: string | null, nodeId: string) =>
    portId && visiblePortIds.has(portId) ? portId : nodeId;
  return edges.map((e) => {
    // elk-native inline edge label (§1): reserve the WRAPPED chip size so long
    // guard expressions spread the layers instead of piling up. The FE post-
    // pass (label-layout.ts) then de-conflicts final placement.
    const text = composedEdgeLabel(e, edgeStyles);
    const labels: ElkLabel[] = text
      ? [
          {
            text,
            ...chipSize(wrapLabel(text)),
            layoutOptions: { 'org.eclipse.elk.edgeLabels.inline': 'true' },
          },
        ]
      : [];
    return {
      id: e.id,
      sources: [endpoint(e.source_port_id, e.source_id)],
      targets: [endpoint(e.target_port_id, e.target_id)],
      ...(labels.length ? { labels } : {}),
    };
  });
}

/** Walk the built ELK node tree into an id → ElkNode map (edges attach to
 *  their lowest-common-ancestor container by id). */
function indexElkNodes(children: ElkNode[]): Map<string, ElkNode> {
  const m = new Map<string, ElkNode>();
  const walk = (n: ElkNode) => {
    m.set(n.id, n);
    for (const c of n.children ?? []) walk(c);
  };
  for (const c of children) walk(c);
  return m;
}


/** All edges that participate in the layout: the scene's top-level edges,
 *  every EXPANDED island's internal edges, and every EXPANDED container's
 *  `DiagramChild::Edge` children (recursively) — the composer nests behavior
 *  edges (transitions / control flows) inside their container node, and
 *  dropping them rendered state/action views edge-less (D-N2). elk routes
 *  cross-hierarchy edges declared at root under `INCLUDE_CHILDREN`, so all
 *  sub-edges live in the same flat list as the scene edges.
 *
 *  Edge children are collected ONLY under a host whose child nodes actually
 *  lay out (`expanded === true`, mirroring `toElkNode`) — an edge whose
 *  endpoint shape isn't in the elk graph is a hard elk import error. */
function gatherEdges(scene: DiagramIR): DiagramEdge[] {
  const out: DiagramEdge[] = [...scene.edges];
  const fromChildren = (children: DiagramChild[], hostExpanded: boolean) => {
    for (const c of children) {
      if ('Island' in c && c.Island.expanded) {
        // A fixed island routes its own edges from `precomputed_route`; handing
        // them to elk would both fail (its shapes aren't in the elk graph) and
        // discard the authored ladder (D-S1).
        if (isFixedSubtree(c.Island.subtree)) continue;
        out.push(...c.Island.subtree.edges);
        for (const sn of c.Island.subtree.nodes)
          fromChildren(sn.children, sn.expanded === true);
      } else if ('Node' in c) {
        fromChildren(c.Node.children, c.Node.expanded === true);
      } else if ('Compartment' in c) {
        fromChildren(c.Compartment.children, hostExpanded);
      } else if ('Edge' in c && hostExpanded) {
        out.push(c.Edge);
      }
    }
  };
  for (const n of scene.nodes) fromChildren(n.children, n.expanded === true);
  return out;
}

/** Shared layered spacings + edge-label options (used by both hierarchy modes). */
const BASE_LAYERED: LayoutOptions = {
  'elk.algorithm': 'layered',
  'elk.direction': 'DOWN',
  'elk.edgeRouting': 'ORTHOGONAL',
  // Generous layer gaps so routed edges have room to be visible between boxes
  // (tightly-packed layers collapse typing/containment edges into hidden stubs).
  'elk.layered.spacing.nodeNodeBetweenLayers': '70',
  'elk.spacing.nodeNode': '45',
  'elk.spacing.edgeNode': '24',
  'elk.layered.spacing.edgeEdgeBetweenLayers': '12',
  // elk-native edge labels (§1): inline labels become dummy nodes in a layer,
  // so the layout physically spreads layers to make room for guard text.
  'org.eclipse.elk.edgeLabels.inline': 'true',
  'org.eclipse.elk.spacing.edgeLabel': '4',
  'org.eclipse.elk.layered.edgeLabels.sideSelection': 'SMART_DOWN',
};

/** Default viewport aspect (w/h) when the caller doesn't pass the live canvas
 *  aspect — the harness runs at 1600×1000. */
const DEFAULT_ASPECT = 1.6;

/** Root options for a scene WITHOUT true cross-hierarchy edges (the large
 *  majority, incl. every failure tile): SEPARATE_CHILDREN unlocks connected-
 *  component separation + aspect-driven packing (§3), so disconnected sibling
 *  sets pack into a grid instead of one horizontal ribbon layer.
 *
 *  NOTE: deliberately NO `layered.wrapping.strategy` here — wrapping belongs on
 *  edge-bearing CONTAINERS, not on the root. At root the components are already
 *  separated and packed independently, so asking elk to wrap layers only makes
 *  it manufacture back-routes BETWEEN components: measured on MixedExposeView,
 *  the two Engine→Gearbox edges (70px apart) routed 1126px at a 4.17× detour
 *  ratio, diving below everything then climbing over the whole canvas. Dropping
 *  it here took them to 154px / 1.00× (dead straight) with ZERO change to the
 *  G1–G7 gate results across all 19 graph tiles. Deep chains still wrap — that
 *  is the per-container option applied below, which is what keeps StartFlowView
 *  banded. */
function separateRootOptions(aspect: number): LayoutOptions {
  return {
    ...BASE_LAYERED,
    'elk.hierarchyHandling': 'SEPARATE_CHILDREN',
    'org.eclipse.elk.separateConnectedComponents': 'true',
    'org.eclipse.elk.aspectRatio': String(aspect),
  };
}

/** Root options for a scene WITH a true cross-hierarchy edge (endpoint nested
 *  below the edge's LCA): INCLUDE_CHILDREN is required to route it, which
 *  disables component separation — so we keep the historical flat behavior plus
 *  the aspect hint + wrapping. */
function includeRootOptions(aspect: number): LayoutOptions {
  return {
    ...BASE_LAYERED,
    'elk.hierarchyHandling': 'INCLUDE_CHILDREN',
    'org.eclipse.elk.aspectRatio': String(aspect),
    'org.eclipse.elk.layered.wrapping.strategy': 'MULTI_EDGE',
  };
}

/** Re-layout options for a post-drop reflow: same routing, but elk derives
 *  layering / ordering from the seeded node positions (INTERACTIVE) so the
 *  diagram settles near where the user left it while edges get fresh orthogonal
 *  routes. */
const INTERACTIVE_OPTIONS: LayoutOptions = {
  ...BASE_LAYERED,
  // Interactive reflow keeps INCLUDE_CHILDREN (its historical behavior) — the
  // packing/component-separation levers are FRESH-layout only (§3: manual
  // deltas are pinned FE-side and never trigger reflow, D-I1).
  'elk.hierarchyHandling': 'INCLUDE_CHILDREN',
  'org.eclipse.elk.interactive': 'true',
  'elk.layered.layering.strategy': 'INTERACTIVE',
  'elk.layered.cycleBreaking.strategy': 'INTERACTIVE',
  // A full INTERACTIVE crossing-min strategy is incompatible with the
  // hierarchy-aware LAYER_SWEEP that INCLUDE_CHILDREN requires; `semiInteractive`
  // keeps LAYER_SWEEP but biases ordering toward the seeded node positions.
  'elk.layered.crossingMinimization.semiInteractive': 'true',
};

export interface ElkLike {
  layout(graph: ElkNode): Promise<ElkNode>;
  terminateWorker?(): void;
}

/** Hard wall-clock cap on one elk pass. A dense scene (the render cap bounds
 *  node COUNT, not elk's routing time — e.g. a General view over a whole model
 *  with dense constraint/binding edges) can otherwise run for minutes; past
 *  this we kill the layout and surface the error banner instead. */
const LAYOUT_TIMEOUT_MS = 20_000;

/** The shared layout engine. In the browser elk runs in a REAL Web Worker
 *  (`elk-worker.min.js` served as a Vite asset), so a pathological scene can
 *  never freeze the tab — and a timed-out pass is actually killable via
 *  `terminateWorker()`. Where `Worker` doesn't exist (vitest / jsdom / node)
 *  the bundled build's in-thread fake worker is used, as before. */
let sharedElk: ElkLike | null = null;
function acquireElk(): ElkLike {
  if (!sharedElk) {
    sharedElk =
      typeof Worker !== 'undefined' ? new ELK({ workerUrl: elkWorkerUrl }) : new ELK();
  }
  return sharedElk;
}

/** Race `layout()` against the wall-clock cap. On timeout the worker is
 *  terminated (dropping the runaway pass) and the shared instance reset so the
 *  next layout starts a fresh worker. Exported for tests. */
export async function layoutWithTimeout(
  elk: ElkLike,
  graph: ElkNode,
  timeoutMs: number,
  onTimeout?: () => void,
): Promise<ElkNode> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    return await Promise.race([
      elk.layout(graph),
      new Promise<never>((_, reject) => {
        timer = setTimeout(() => {
          elk.terminateWorker?.();
          onTimeout?.();
          reject(
            new Error(
              `layout timed out after ${Math.round(timeoutMs / 1000)}s — scene too dense to route ` +
                `(${graph.children?.length ?? 0} top-level nodes, ${graph.edges?.length ?? 0} edges); ` +
                `scope this view with an expose`,
            ),
          );
        }, timeoutMs);
      }),
    ]);
  } finally {
    clearTimeout(timer);
  }
}

/**
 * Run elkjs layout+routing over a scene. `elk` is injectable for tests; defaults
 * to the shared worker-backed instance (time-boxed — see `LAYOUT_TIMEOUT_MS`).
 * `shapes` is the Rust-emitted `VisualKind → Shape` map used to size glyph nodes.
 */
export async function layoutScene(
  scene: DiagramIR,
  elk?: ElkLike,
  shapes?: Record<string, string>,
  hints?: LayoutHints,
  edgeStyles: Record<string, EdgeStyleToken> = {},
  viewportAspect: number = DEFAULT_ASPECT,
): Promise<LayoutResult> {
  // Fixed-layout views (Sequence / Grid / Geometry) carry every position from the
  // generator — there's nothing for elk to solve. Detect them by the contract in
  // viewmodel-types (`position` is set on every top-level node iff the view is
  // fixed) and honor the precomputed geometry directly. (Interactive drag hints
  // don't apply to a fixed scene, so they're ignored.)
  const isFixed = scene.nodes.length > 0 && scene.nodes.every((n) => n.position != null);
  if (isFixed) return layoutFixed(scene, shapes);

  const meta = new Map<string, NodeMeta>();
  // Scene edges + expanded-island sub-edges, in one flat list (see gatherEdges).
  const allEdges = gatherEdges(scene);

  // spacer elk-id → the island's own pre-solved geometry, spliced back in at
  // the spacer's placed origin once elk has positioned the box (D-S1).
  const fixedIslandLayouts = new Map<string, LayoutResult>();
  const children = scene.nodes.map((n) =>
    toElkNode(n, 0, meta, shapes, hints, { x: 0, y: 0 }, fixedIslandLayouts),
  );
  // Port ids actually present in the elk graph (hidden ports are filtered
  // out in toElkNode) — collectEdges falls back to node endpoints for the rest.
  const visiblePortIds = new Set<string>();
  for (const m of meta.values()) for (const p of m.ports) visiblePortIds.add(p.element_id);
  const elkEdges = collectEdges(allEdges, visiblePortIds, edgeStyles);

  // ── Edge ownership by container (§3.2) ─────────────────────────────
  // An edge whose endpoints share a DIRECT parent is placed in that parent's
  // `edges[]` (root for top-level) so SEPARATE_CHILDREN + component separation
  // can act on the rest of the graph. An edge that spans hierarchy levels
  // (endpoints under different parents — e.g. a sequence-island message between
  // proxies in two lifelines) is a TRUE cross-hierarchy edge: it stays at root
  // and forces INCLUDE_CHILDREN, the only mode that can route it. (Nesting such
  // an edge in its LCA container fails — the container's layered layout can't
  // see grandchild endpoints. That was the original flat-at-root behavior.)
  const parentMap = buildParentMap(scene);
  const elkById = indexElkNodes(children);
  const edgeById = new Map(allEdges.map((e) => [e.id, e]));
  // An edge endpoint may be a PORT id rather than a node id — the composer
  // emits port-to-port connectors whose `source_id`/`target_id` ARE the ports
  // (not just `source_port_id`). `parentMap` only knows nodes, so a bare
  // `parentMap[portId]` misses and defaults to 'root'. That silently declared
  // such an edge at root while its real endpoints sat a level down inside a
  // package container — a shape elk cannot route under SEPARATE_CHILDREN, which
  // is what produced the diagonal cross-canvas line and the giant detour box on
  // AllPartsView / EmptyExposeView (D-E2). Resolve a port to its OWNING NODE
  // first: for ownership purposes an edge touching a port belongs at that
  // node's level.
  const ownerNodeOfPort = new Map<string, string>();
  for (const m of meta.values()) {
    for (const p of m.ports) ownerNodeOfPort.set(p.element_id, m.node.element_id);
  }
  const shapeNodeOf = (id: string) => ownerNodeOfPort.get(id) ?? id;
  const parentOf = (id: string) => parentMap[shapeNodeOf(id)] ?? 'root';
  // The top-level ancestor of an endpoint (the direct child of root that
  // contains it) — the coarsening anchor for a cross-package edge (§3 addendum).
  const topAncestor = (id: string): string => {
    let cur = shapeNodeOf(id);
    for (let guard = 0; guard < 64; guard++) {
      const p = parentMap[cur] ?? 'root';
      if (p === 'root') return cur;
      cur = p;
    }
    return cur;
  };
  const rootEdges: ElkExtendedEdge[] = [];
  // Cross-package edges whose LAYOUT endpoints were coarsened to the package
  // boxes (§3 addendum). The route is drawn package→package, but the DiagramEdge
  // keeps its real source_id/target_id — so post-layout route repair must
  // validate against THESE coarsened anchors, not the precise nested endpoints.
  const coarsenedLayoutEndpoints = new Map<string, { source: string; target: string }>();
  let anyCrossHierarchy = false;
  for (const ee of elkEdges) {
    const src = edgeById.get(ee.id);
    if (!src) continue;
    const ps = parentOf(src.source_id);
    const pt = parentOf(src.target_id);
    if (ps === pt) {
      const host = ps === 'root' ? null : elkById.get(ps);
      if (host) (host.edges ??= []).push(ee);
      else rootEdges.push(ee);
    } else {
      // A cross-hierarchy edge. If its endpoints resolve to DIFFERENT top-level
      // ancestors (LCA = root — a cross-PACKAGE edge), coarsen its layout to
      // those package boxes so the scene stays on SEPARATE_CHILDREN + packing
      // instead of forcing INCLUDE_CHILDREN globally and flattening unrelated
      // siblings (#76, steward ruling 2026-07-23; brief §3 addendum). Structural
      // condition only — never gated on view identity. Same-package and
      // same-top-level (e.g. sequence-island) cross-hierarchy edges keep the
      // precise INCLUDE_CHILDREN path. Visual coarsening: `src` keeps its real
      // endpoint ElementIds for selection/hover.
      const sa = topAncestor(src.source_id);
      const ta = topAncestor(src.target_id);
      if (sa !== ta && elkById.has(sa) && elkById.has(ta)) {
        ee.sources = [sa];
        ee.targets = [ta];
        coarsenedLayoutEndpoints.set(ee.id, { source: sa, target: ta });
        rootEdges.push(ee);
      } else {
        anyCrossHierarchy = true;
        rootEdges.push(ee);
      }
    }
  }

  const interactive = hints?.interactive === true;
  const rootOptions = interactive
    ? INTERACTIVE_OPTIONS
    : anyCrossHierarchy
      ? includeRootOptions(viewportAspect)
      : separateRootOptions(viewportAspect);

  // Per-container packing (§3.5): a container whose direct level has NO edges
  // and whose children are all plain leaves (no ports, no nesting) is a pure
  // node-set — rectpacking packs it to the viewport aspect (exactly this job).
  // Mixed / edge-bearing / ported containers stay layered. Skipped entirely
  // when the scene has a cross-hierarchy edge (INCLUDE_CHILDREN + a child using
  // a different algorithm is unsupported by elk). Fresh layouts only.
  //
  // MEASURED (#76): relaxing this per-scene skip to a per-container one does
  // NOT help EmptyExposeView's 3.23× aspect, and the attempt was reverted.
  // Two facts came out of it, both worth not rediscovering:
  //   1. That tile's 18-child container is NOT an edge-free leaf set — it owns
  //      1 internal edge, so it was never a rectpacking candidate anyway.
  //   2. Under INCLUDE_CHILDREN elk flattens the hierarchy into ONE layered
  //      pass and ignores child-container `layoutOptions` outright, so setting
  //      `aspectRatio` on the container changes nothing. Root already sets it
  //      (`includeRootOptions`) and the tile is still 3.23×.
  // The residual is therefore a ROOT-level shape problem — three large sibling
  // containers laid side by side, 5089×1010 — not a per-container one.
  if (!interactive && !anyCrossHierarchy) {
    for (const en of elkById.values()) {
      const kids = en.children ?? [];
      if (kids.length < 2) continue;
      if ((en.edges?.length ?? 0) > 0) {
        // Edge-bearing (layered) container — e.g. a state machine's transition
        // chain. Give it the viewport aspect so it stays viewport-shaped rather
        // than one wide ribbon row (§3.6). elk options aren't inherited into a
        // child node, so they must be set on the container itself.
        //
        // NO `layered.wrapping.strategy: MULTI_EDGE` here (D-L9). D-L8 removed
        // it at root for manufacturing a 4.17× detour; it does the same inside
        // a container, and wrapping a *small* cycle is strictly worse — the
        // wrap cuts a back edge and orbits it around the whole box. Measured on
        // the two edge-bearing tiles, both improved without it:
        //   MixedExposeView/DriveStateMachine — the four transitions to/from
        //     `idle` went 512/578/578/528px → 46/184/158/46px.
        //   runtimeStateMachines — total routed length 1917 → 1287px, worst
        //     detour 889px@1.96× → 405px, and the bottom→top cycle return
        //     straightened to an exact 1.00×.
        // Aspect cost: none — every G5 gate still passes (this tile at 1.21×).
        // (`includeRootOptions` still sets its own copy at root; left alone
        // rather than changed on inference — measure before touching it.)
        en.layoutOptions = {
          ...en.layoutOptions,
          'elk.algorithm': 'layered',
          'elk.direction': 'DOWN',
          'org.eclipse.elk.aspectRatio': String(viewportAspect),
        };
        continue;
      }
      const pureLeaves = kids.every(
        (k) => (k.children?.length ?? 0) === 0 && (k.ports?.length ?? 0) === 0,
      );
      if (!pureLeaves) continue;
      en.layoutOptions = {
        ...en.layoutOptions,
        'elk.algorithm': 'org.eclipse.elk.rectpacking',
        'org.eclipse.elk.aspectRatio': String(viewportAspect),
      };
    }
  }

  const graph: ElkNode = {
    id: 'root',
    layoutOptions: rootOptions,
    children,
    edges: rootEdges,
  };

  // Injected engines (tests) run bare; the shared engine is time-boxed and its
  // worker torn down on timeout so the next layout starts clean.
  const laid = elk
    ? await elk.layout(graph)
    : await layoutWithTimeout(acquireElk(), graph, LAYOUT_TIMEOUT_MS, () => {
        sharedElk = null;
      });

  // Resolve absolute positions (ELK reports child coords relative to parent).
  const placed: PlacedNode[] = [];
  const edges: PlacedEdge[] = [];
  /** Absolute port-glyph centers, for edge-endpoint snapping (§2). */
  const portCenters = new Map<string, { x: number; y: number }>();

  const collectTreeEdges = (n: ElkNode, ax: number, ay: number) => {
    // An edge's section coordinates are relative to its containing node's
    // origin — offset by that container's absolute position (root = 0,0).
    for (const e of n.edges ?? []) {
      const src = edgeById.get(e.id);
      if (!src) continue;
      const points: { x: number; y: number }[] = [];
      for (const sec of e.sections ?? []) {
        points.push({ x: ax + sec.startPoint.x, y: ay + sec.startPoint.y });
        for (const bp of sec.bendPoints ?? []) points.push({ x: ax + bp.x, y: ay + bp.y });
        points.push({ x: ax + sec.endPoint.x, y: ay + sec.endPoint.y });
      }
      // Prefer elk's inline-label center as the resolver seed; else the path
      // midpoint vertex (both in absolute coords).
      let labelAt: { x: number; y: number } | null =
        points.length > 0 ? points[Math.floor((points.length - 1) / 2)] : null;
      const lbl = (e.labels ?? [])[0] as ElkLabel | undefined;
      if (lbl && lbl.x != null && lbl.y != null) {
        labelAt = { x: ax + lbl.x + (lbl.width ?? 0) / 2, y: ay + lbl.y + (lbl.height ?? 0) / 2 };
      }
      edges.push({ edge: src, points, label: src.label, labelAt });
    }
  };

  const walk = (n: ElkNode, ox: number, oy: number, depth: number) => {
    const ax = ox + (n.x ?? 0);
    const ay = oy + (n.y ?? 0);
    // Fixed-island spacer: elk placed the box, so translate the island's own
    // pre-solved nodes + routes onto it. The spacer itself never renders — it
    // has no `meta` entry and therefore no PlacedNode (D-S1).
    const island = fixedIslandLayouts.get(n.id);
    if (island) {
      for (const p of island.nodes) {
        placed.push({ ...p, x: p.x + ax, y: p.y + ay, depth: depth + p.depth });
      }
      for (const pe of island.edges) {
        edges.push({
          ...pe,
          points: pe.points.map((pt) => ({ x: pt.x + ax, y: pt.y + ay })),
          labelAt: pe.labelAt ? { x: pe.labelAt.x + ax, y: pe.labelAt.y + ay } : null,
        });
      }
      return;
    }
    const m = meta.get(n.id);
    if (m) {
      // elk reports port x/y relative to the node; lift to absolute and pair
      // each laid-out port back to its DiagramPort by id.
      const portById = new Map(m.ports.map((p) => [p.element_id, p]));
      const placedPorts: PlacedPort[] = [];
      for (const ep of n.ports ?? []) {
        const dp = portById.get(ep.id);
        if (!dp) continue;
        const px = ax + (ep.x ?? 0);
        const py = ay + (ep.y ?? 0);
        const pw = ep.width ?? PORT_SIZE;
        const ph = ep.height ?? PORT_SIZE;
        // elk-computed port-label box (relative to the port) → absolute (§2).
        const el = (ep.labels ?? [])[0] as ElkLabel | undefined;
        const labelRect =
          el && el.x != null && el.y != null
            ? { x: px + el.x, y: py + el.y, width: el.width ?? 0, height: el.height ?? 0 }
            : null;
        placedPorts.push({ port: dp, x: px, y: py, width: pw, height: ph, labelRect });
        portCenters.set(dp.element_id, { x: px + pw / 2, y: py + ph / 2 });
      }
      placed.push({
        node: m.node,
        x: ax,
        y: ay,
        width: n.width ?? 0,
        height: n.height ?? 0,
        depth,
        headerHeight: m.headerHeight,
        lines: m.lines,
        hasChildren: m.hasChildren,
        ports: placedPorts,
      });
    }
    collectTreeEdges(n, ax, ay);
    for (const c of n.children ?? []) walk(c, ax, ay, depth + 1);
  };
  // Root-level edges are in root coordinates (root at 0,0).
  collectTreeEdges({ id: 'root', edges: laid.edges ?? [] } as ElkNode, 0, 0);
  for (const c of laid.children ?? []) walk(c, 0, 0, 0);

  // Repair routes elk emitted with an endpoint that misses its own node BEFORE
  // port snapping — a malformed section otherwise renders as a diagonal slash
  // to a coordinate no node occupies (AllPartsView Vehicle→Gearbox typing edge).
  const repaired = repairMalformedRoutes(
    edges,
    new Map(placed.map((p) => [p.node.element_id, p])),
    coarsenedLayoutEndpoints,
  );
  if (repaired > 0 && import.meta.env.DEV) {
    console.warn(
      `[diagram-svg] repaired ${repaired} malformed edge route(s): elk returned an ` +
        `endpoint far from its endpoint node; rebuilt as an orthogonal elbow.`,
    );
  }

  // Snap ported edge endpoints onto the port glyph center (§2 anchoring):
  // rounding / node-id fallback can drift the drawn polyline off the drawn
  // glyph — pin the first/last point and re-square the adjacent bend so the
  // route stays orthogonal.
  for (const pe of edges) snapEdgeToPorts(pe, portCenters);

  return {
    nodes: placed,
    edges,
    width: laid.width ?? 0,
    height: laid.height ?? 0,
  };
}

/** Pin an edge's endpoints to their resolved port glyph centers and re-square
 *  the adjacent bend, keeping the route orthogonal (§2). No-op for edges whose
 *  endpoints aren't resolved ports. Mutates `pe.points` in place. */
interface EdgeRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Manhattan elbow between two node boxes: attach on the faces that point at
 *  each other and route the middle orthogonally (a single mid-axis jog). Keeps
 *  re-routed edges orthogonal instead of collapsing to a diagonal straight line.
 *  No obstacle avoidance — that's the elkjs job; this is the live-drag preview /
 *  persisted route for moved endpoints, and the repair route for a malformed
 *  elk section (see `repairMalformedRoutes`).
 *
 *  Lives HERE rather than in SvgCanvas because layout owns geometry and
 *  SvgCanvas imports from layout — the reverse import would be a cycle, and a
 *  second copy would be a duplicate implementation. */
export function orthogonalElbow(s: EdgeRect, t: EdgeRect): { x: number; y: number }[] {
  const cs = { x: s.x + s.width / 2, y: s.y + s.height / 2 };
  const ct = { x: t.x + t.width / 2, y: t.y + t.height / 2 };
  const dx = ct.x - cs.x;
  const dy = ct.y - cs.y;
  if (Math.abs(dx) >= Math.abs(dy)) {
    // Horizontal-dominant: attach on left/right faces, jog at the mid-x.
    const sx = dx >= 0 ? s.x + s.width : s.x;
    const tx = dx >= 0 ? t.x : t.x + t.width;
    const midX = (sx + tx) / 2;
    return [
      { x: sx, y: cs.y },
      { x: midX, y: cs.y },
      { x: midX, y: ct.y },
      { x: tx, y: ct.y },
    ];
  }
  // Vertical-dominant: attach on top/bottom faces, jog at the mid-y.
  const sy = dy >= 0 ? s.y + s.height : s.y;
  const ty = dy >= 0 ? t.y : t.y + t.height;
  const midY = (sy + ty) / 2;
  return [
    { x: cs.x, y: sy },
    { x: cs.x, y: midY },
    { x: ct.x, y: midY },
    { x: ct.x, y: ty },
  ];
}

/** Distance from a point to the nearest edge of a rect (0 when inside). */
function distanceToRect(p: { x: number; y: number }, r: EdgeRect): number {
  const dx = Math.max(r.x - p.x, 0, p.x - (r.x + r.width));
  const dy = Math.max(r.y - p.y, 0, p.y - (r.y + r.height));
  return Math.hypot(dx, dy);
}

/** Replace any edge route whose drawn endpoint does not land on its endpoint
 *  NODE with a clean orthogonal elbow between the two node boxes.
 *
 *  elk occasionally emits a section whose endPoint is nowhere near the node it
 *  connects — on AllPartsView the Vehicle→Gearbox typing edge ended at relative
 *  y=-236 (absolute y=-224), producing a DIAGONAL slash across the whole canvas
 *  to a coordinate no node occupies. Ownership and offsets were both correct;
 *  the section itself was malformed. The G8 detour gate cannot catch this
 *  because the bogus route is *short* (1.14× — an efficient path to the wrong
 *  place).
 *
 *  This is a REPAIR, not a root-cause fix for elk's behaviour: we detect that
 *  the route contradicts the scene (an endpoint far off its own node) and
 *  rebuild it from the node boxes, which is information we trust. It is
 *  deliberately conservative — only routes that are provably wrong are
 *  replaced, so a legitimate route (including one that attaches at a port
 *  slightly outside the node box) is left untouched. */
function repairMalformedRoutes(
  edges: PlacedEdge[],
  placed: Map<string, PlacedNode>,
  coarsened?: Map<string, { source: string; target: string }>,
): number {
  // Generous: ports sit ON the boundary and elk rounds, so only an endpoint
  // that misses its node by more than a node-ish distance counts as broken.
  const TOLERANCE = 40;
  let repaired = 0;
  for (const pe of edges) {
    const pts = pe.points;
    if (pts.length < 2) continue;
    // A cross-package edge (§3 addendum) was laid out to its coarsened package
    // anchors, not its real nested endpoints — validate the elk route against
    // those anchors, else this would "repair" a perfectly good package→package
    // route into a naive elbow aimed at the precise (nested) endpoints.
    const ends = coarsened?.get(pe.edge.id);
    const s = placed.get(ends?.source ?? pe.edge.source_id);
    const t = placed.get(ends?.target ?? pe.edge.target_id);
    if (!s || !t) continue;
    const sRect = { x: s.x, y: s.y, width: s.width, height: s.height };
    const tRect = { x: t.x, y: t.y, width: t.width, height: t.height };
    const startOff = distanceToRect(pts[0], sRect) > TOLERANCE;
    const endOff = distanceToRect(pts[pts.length - 1], tRect) > TOLERANCE;
    if (!startOff && !endOff) continue;
    pe.points = orthogonalElbow(sRect, tRect);
    repaired++;
  }
  return repaired;
}

export function snapEdgeToPorts(
  pe: PlacedEdge,
  portCenters: Map<string, { x: number; y: number }>,
): void {
  const pts = pe.points;
  if (pts.length < 2) return;
  const snap = (idx: number, adj: number, portId: string | null) => {
    if (!portId) return;
    const c = portCenters.get(portId);
    if (!c) return;
    const end = pts[idx];
    const bend = pts[adj];
    // Re-square the bend so the segment into the port stays axis-aligned: if
    // the endpoint moved mostly horizontally, keep the bend's x aligned to the
    // port center; otherwise its y.
    if (Math.abs(bend.x - end.x) <= Math.abs(bend.y - end.y)) bend.x = c.x;
    else bend.y = c.y;
    pts[idx] = { x: c.x, y: c.y };
  };
  snap(0, 1, pe.edge.source_port_id);
  snap(pts.length - 1, pts.length - 2, pe.edge.target_port_id);
}

/**
 * Lay out a fixed-position scene (Sequence / Grid / Geometry) without elk: the
 * generator already computed every node position/size and every edge route, so
 * the renderer just resolves child positions to absolute and re-anchors routes.
 *
 * Child node positions are parent-relative (Sprotty convention), so a node's
 * absolute origin is `parentAbs + node.position`. Edge routes are re-anchored to
 * the resolved source-node center: the sequence generator's `precomputed_route`
 * omits the lifeline y-offset, so shifting every route point by
 * `sourceCenter − route[0]` lands the route on the true node geometry without
 * duplicating the generator's layout math here.
 */
export function layoutFixed(scene: DiagramIR, shapes?: Record<string, string>): LayoutResult {
  const placed: PlacedNode[] = [];
  const centerById = new Map<string, { x: number; y: number }>();
  let maxX = 0;
  let maxY = 0;

  const sizeFor = (node: DiagramNode, lines: CompartmentLine[]): { w: number; h: number } => {
    if (node.size) return { w: node.size[0], h: node.size[1] };
    const glyph = shapes ? glyphSizeFor(shapeForVisualKind(shapes, node.visual_kind)) : null;
    if (glyph) return glyph;
    const headerHeight = headerHeightFor(node);
    const isControl = node.header_style === 'None' && lines.length === 0 && !node.name;
    return {
      w: nodeWidthFor(node, lines),
      // Header-only when no compartment lines (D-L6), matching toElkNode.
      h: isControl ? CONTROL_SIZE : headerHeight + (lines.length > 0 ? lines.length * LINE_H + PAD_Y : 0),
    };
  };

  const walk = (node: DiagramNode, ox: number, oy: number, depth: number) => {
    const [px, py] = node.position ?? [0, 0];
    const ax = ox + px;
    const ay = oy + py;
    const { childNodes, lines } = partitionChildren(node.children);
    const { w, h } = sizeFor(node, lines);
    placed.push({
      node,
      x: ax,
      y: ay,
      width: w,
      height: h,
      depth,
      headerHeight: headerHeightFor(node),
      lines,
      // Fixed-layout children render as their own absolutely-placed nodes, not as
      // a nested container body, so this node isn't a "container" for rendering.
      hasChildren: false,
      ports: [],
    });
    centerById.set(node.element_id, { x: ax + w / 2, y: ay + h / 2 });
    maxX = Math.max(maxX, ax + w);
    maxY = Math.max(maxY, ay + h);
    for (const c of childNodes) walk(c, ax, ay, depth + 1);
  };
  for (const n of scene.nodes) walk(n, 0, 0, 0);

  const edges: PlacedEdge[] = scene.edges.map((e) => {
    const route = e.precomputed_route;
    let points: { x: number; y: number }[];
    if (route && route.length >= 2) {
      const src = centerById.get(e.source_id);
      const dx = src ? src.x - route[0][0] : 0;
      const dy = src ? src.y - route[0][1] : 0;
      points = route.map(([x, y]) => ({ x: x + dx, y: y + dy }));
    } else {
      const s = centerById.get(e.source_id);
      const t = centerById.get(e.target_id);
      points = s && t ? [s, t] : [];
    }
    const labelAt = points.length > 0 ? points[Math.floor((points.length - 1) / 2)] : null;
    return { edge: e, points, label: e.label, labelAt };
  });

  return { nodes: placed, edges, width: maxX, height: maxY };
}

export const __test = { partitionChildren, nodeWidthFor, headerHeightFor, toElkNode };
export { FONT, LINE_H };
