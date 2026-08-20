/**
 * SvgCanvas — React-SVG diagram renderer over the Rust ViewModel (Bucket 2 spike).
 *
 * THE graph renderer for the simulation app. Renders one view's geometry from
 * `sysml.diagram.viewmodel`, lays it out + routes edges with elkjs (one native
 * pass, frontend-side), pans/zooms with d3-zoom, and links bidirectionally to the
 * editor through the single `selectedId` selection store. No Sprotty.
 *
 * This is a go/no-go spike: viability over polish.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import type { MouseEvent as ReactMouseEvent, PointerEvent as ReactPointerEvent, ReactNode } from 'react';
import { useQuery } from '@tanstack/react-query';
import { select as d3select } from 'd3-selection';
import { zoom as d3zoom, zoomIdentity, type ZoomBehavior } from 'd3-zoom';
import { getDiagnosticOverlay, getSimOverlay, getVerdictOverlay, getViewModel } from '@/shared/api/model';
import { useWorkspaceStore } from '@/store/workspace';
import { useSelectionStore } from '@/features/selection/store';
import { useDiagramLinkStore } from '@/features/diagram-link/store';
import { useSessionStore } from '@/features/sessions/store';
import {
  buildParentMap,
  layoutScene,
  orthogonalElbow,
  type LayoutResult,
  type PlacedNode,
} from './layout';
import { useManualLayoutStore, type ManualDelta } from './manual-layout';
import {
  EMPTY_VIEW_CONTENT,
  FRAME_HEADING_H,
  ViewFrameLayer,
  frameExtents,
  headingTabWidth,
} from './frame';
import { colorsForVisualKind, themeableCanvasPalette } from './palette';
import { effectiveLod, lodLabel, type Lod } from './lod';
import { isCardShape, shapeForVisualKind, type ShapeName } from './shapes';
import { edgeDecor } from './edges';
import {
  elideForLod,
  resolveEdgeLabels,
  type ResolvedLabel,
  type Rect as LabelRect,
} from './label-layout';
import {
  diagnosticTooltip,
  diagnosticsForNode,
  formatOverlayValue,
  isActive,
  isCompleted,
  overlayForNode,
  severityGlyph,
  verdictForNode,
  verdictPillStyle,
  worstState,
} from './overlay';
import type {
  DiagnosticOverlay,
  DiagramIR,
  ElementDiagnostics,
  ElementOverlay,
  ElementVerdict,
  EdgeStyleToken,
  SimOverlay,
  VerdictOverlay,
  ViewModel,
} from './viewmodel-types';

export interface SceneTruncation {
  nodes: number;
  edges: number;
  totalNodes: number;
  totalEdges: number;
}

/** Cap a scene for rendering (3.11): if it exceeds `nodeCap`/`edgeCap`, keep the
 *  first `nodeCap` top-level nodes and the edges among them (then `edgeCap`), and
 *  report what was dropped so the renderer can show a "showing N of M" banner.
 *  Returns the scene unchanged + `truncation: null` when within caps. Island
 *  subtree nodes/edges live inside kept nodes, so they're unaffected. */
export function truncateScene(
  scene: DiagramIR,
  nodeCap: number,
  edgeCap: number,
): { scene: DiagramIR; truncation: SceneTruncation | null } {
  const totalNodes = scene.nodes.length;
  const totalEdges = scene.edges.length;
  if (totalNodes <= nodeCap && totalEdges <= edgeCap) return { scene, truncation: null };
  const keptNodes = scene.nodes.slice(0, nodeCap);
  const keptIds = new Set(keptNodes.map((n) => n.element_id));
  const keptEdges = scene.edges
    .filter((e) => keptIds.has(e.source_id) && keptIds.has(e.target_id))
    .slice(0, edgeCap);
  return {
    scene: { ...scene, nodes: keptNodes, edges: keptEdges },
    truncation: { nodes: keptNodes.length, edges: keptEdges.length, totalNodes, totalEdges },
  };
}

const SOURCE_GLYPH: Record<string, string> = { Inherited: '^', Derived: '/', Owned: '' };

/** Axis-aligned rect in scene coordinates (geometry dump / label layout). */
type Rect = { x: number; y: number; width: number; height: number };

/** Port-label font (8px) advance — the fallback box + drawn text agree. */
const PORT_LABEL_CHAR_W = 4.4;
const PORT_LABEL_H = 10;

/** Where a port's name draws (§2): elk's computed label box when it placed one,
 *  else a side-aware fallback OUTSIDE the node border (W→left/right-aligned,
 *  E→right/left-aligned, N/S→above/below centered). Returns null for an
 *  unnamed port. Scene coordinates (the node <g> carries any drag transform). */
function portLabelBox(
  pp: PlacedNode['ports'][number],
): { rect: Rect; textX: number; textY: number; anchor: 'start' | 'middle' | 'end' } | null {
  const name = pp.port.name;
  if (!name) return null;
  if (pp.labelRect) {
    const r = pp.labelRect;
    return { rect: r, textX: r.x, textY: r.y + r.height - 2, anchor: 'start' };
  }
  const w = name.length * PORT_LABEL_CHAR_W;
  const h = PORT_LABEL_H;
  const cx = pp.x + pp.width / 2;
  const cy = pp.y + pp.height / 2;
  const side = pp.port.side;
  if (side === 'West') {
    const rect = { x: pp.x - 4 - w, y: cy - h / 2, width: w, height: h };
    return { rect, textX: pp.x - 4, textY: cy + 3, anchor: 'end' };
  }
  if (side === 'East') {
    const rect = { x: pp.x + pp.width + 4, y: cy - h / 2, width: w, height: h };
    return { rect, textX: pp.x + pp.width + 4, textY: cy + 3, anchor: 'start' };
  }
  if (side === 'South') {
    const rect = { x: cx - w / 2, y: pp.y + pp.height + 2, width: w, height: h };
    return { rect, textX: cx, textY: pp.y + pp.height + 2 + h - 2, anchor: 'middle' };
  }
  // North / unassigned: above the glyph (kept OUTSIDE the node border).
  const rect = { x: cx - w / 2, y: pp.y - 2 - h, width: w, height: h };
  return { rect, textX: cx, textY: pp.y - 4, anchor: 'middle' };
}

/** Port-label obstacle rect shifted by the port's owning-node drag delta. */
function portLabelRect(
  pp: PlacedNode['ports'][number],
  d: { dx: number; dy: number },
): Rect | null {
  const box = portLabelBox(pp);
  if (!box) return null;
  return { x: box.rect.x + d.dx, y: box.rect.y + d.dy, width: box.rect.width, height: box.rect.height };
}

/** Sequence-lifeline head height (mirrors `LIFELINE_HEAD`) — the only solid part
 *  of a lifeline; the lane below it is message routing space. */
const LIFELINE_HEAD_H = 40;

/** Whether a node's lane/body is routing space rather than a solid obstacle: a
 *  container (transition labels belong in its body) or a sequence lifeline
 *  (messages + labels live on its lane). Such nodes are excluded from the G1
 *  edge-label-vs-body gate and seed only their solid head/header band. */
function isRoutingLane(pn: PlacedNode): boolean {
  return pn.hasChildren || pn.node.visual_kind === 'Lifeline';
}

/** Solid-band height of a node (the obstacle a label must avoid): a container's
 *  header, a lifeline's head, else the full node. */
function solidBandHeight(pn: PlacedNode): number {
  if (pn.node.visual_kind === 'Lifeline') return LIFELINE_HEAD_H;
  if (pn.hasChildren) return Math.max(pn.headerHeight, 1);
  return pn.height;
}

/** Frame heading tab + corner info compartments as obstacle rects (G1/§1). */
function frameObstacles(frame: NonNullable<ViewModel['frame']>, layout: LayoutResult): Rect[] {
  const content =
    layout.nodes.length === 0 ? EMPTY_VIEW_CONTENT : { width: layout.width, height: layout.height };
  const ext = frameExtents(content, frame);
  const out: Rect[] = [
    { x: ext.x0, y: ext.y0, width: headingTabWidth(frame), height: FRAME_HEADING_H },
  ];
  if (frame.top_right) {
    const w = frame.top_right.text.length * 6.6 + 16;
    out.push({ x: ext.x1 - w, y: ext.y0, width: w, height: 18 });
  }
  if (frame.bottom_left) {
    const w = frame.bottom_left.text.length * 6.6 + 16;
    out.push({ x: ext.x0, y: ext.y1 - 18, width: w, height: 18 });
  }
  if (frame.bottom_right) {
    const w = frame.bottom_right.text.length * 6.6 + 16;
    out.push({ x: ext.x1 - w, y: ext.y1 - 18, width: w, height: 18 });
  }
  return out;
}

/** Stable empty-override map so views with no manual moves don't re-render. */
const EMPTY_OVERRIDES: Record<string, ManualDelta> = {};

/** Sum a node's own delta with all its container ancestors' deltas (so a
 *  container drag carries its subtree). `deltaOf` returns the raw per-node delta
 *  (or undefined); `parentMap` is the scene-nesting child→parent map. */
function accumulatedOffset(
  id: string,
  deltaOf: (id: string) => { dx: number; dy: number } | undefined,
  parentMap: Record<string, string>,
): { dx: number; dy: number } {
  let dx = 0;
  let dy = 0;
  let cur: string | undefined = id;
  const seen = new Set<string>();
  while (cur && !seen.has(cur)) {
    seen.add(cur);
    const d = deltaOf(cur);
    if (d) {
      dx += d.dx;
      dy += d.dy;
    }
    cur = parentMap[cur];
  }
  return { dx, dy };
}

export function SvgCanvas() {
  const focusedUri = useWorkspaceStore((s) => s.focusedUri);
  const selectedViewId = useWorkspaceStore((s) => s.selectedViewId);
  const select = useSelectionStore((s) => s.select);
  const selectedId = useSelectionStore((s) => s.selectedElementId);
  const selectionOrigin = useSelectionStore((s) => s.selectionOrigin);
  const setTextMap = useDiagramLinkStore((s) => s.setTextMap);

  const svgRef = useRef<SVGSVGElement | null>(null);
  const [transform, setTransform] = useState('translate(0,0) scale(1)');
  // Reactive zoom scale (1 = 100%) — drives the level-of-detail band + the info
  // pill. `zoomK` (ref) stays the source for drag math; this mirrors it for
  // render. Both update together in the zoom/fit/pan handlers.
  const [zoomScale, setZoomScale] = useState(1);
  const zoomK = useRef(1); // current zoom scale, for drag-delta → diagram-coord conversion
  const transformRef = useRef(zoomIdentity); // latest d3 transform (for pan-into-view geometry)
  const [layout, setLayout] = useState<LayoutResult | null>(null);
  const [layoutError, setLayoutError] = useState<string | null>(null);
  // Last one-shot fit (layout-quality brief §6): recorded so the geometry
  // dump can report fit scale / content box for the G5–G7 gates.
  const fitRef = useRef<{
    scale: number;
    tx: number;
    ty: number;
    contentBox: { x: number; y: number; width: number; height: number };
    viewport: { width: number; height: number };
  } | null>(null);
  // Set when the scene exceeds the render cap and was partially rendered
  // (3.11). Drives the "showing N of M" banner instead of a blank refusal.
  const [truncation, setTruncation] = useState<SceneTruncation | null>(null);

  // Drill-down expansion: ids the user has expanded beyond the view's own
  // auto-expansion. The viewmodel command MERGES this with the default
  // expansion, so this set only *adds* expansions (collapsing a node we expanded
  // removes its id; a node expanded by the view's design has no toggle). Reset
  // when the view changes — ids belong to one view's scene.
  const [expandedIds, setExpandedIds] = useState<Set<string>>(new Set());
  useEffect(() => setExpandedIds(new Set()), [selectedViewId]);
  const expandedKey = useMemo(() => [...expandedIds].sort(), [expandedIds]);
  const toggleExpand = (id: string, expand: boolean) =>
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (expand) next.add(id);
      else next.delete(id);
      return next;
    });

  // Hover highlight (pure UI state).
  const [hoverId, setHoverId] = useState<string | null>(null);

  // Drag = the manual-override layer (3-layer model): a node's position is its
  // elkjs base position + an optional manual delta. During the drag we move in
  // client state with a cheap orthogonal-elbow preview (no per-frame elk); on
  // drop the delta is PINNED into the per-view manual-layout store — the node
  // stays exactly where the user left it, across reselects. No elk reflow: elk
  // INTERACTIVE re-derives coordinates (seeds only bias ordering), which is
  // exactly the snap-back this contract forbids (D-I1). Edges re-anchor via
  // `adjustedEdgePoints`.
  const setManualDelta = useManualLayoutStore((s) => s.setDelta);
  const storedDeltas = useManualLayoutStore((s) =>
    selectedViewId ? s.deltasByView[selectedViewId] : undefined,
  );
  const overrides = storedDeltas ?? EMPTY_OVERRIDES;
  const [dragDelta, setDragDelta] = useState<{ id: string; dx: number; dy: number } | null>(null);
  const dragRef = useRef<{ id: string; px: number; py: number; baseDx: number; baseDy: number; moved: boolean } | null>(null);
  const liveDelta = useRef<{ id: string; dx: number; dy: number } | null>(null);
  const movedRef = useRef(false); // suppress the click-select that follows a drag
  const selectedViewIdRef = useRef(selectedViewId); // for the stable drop handler
  selectedViewIdRef.current = selectedViewId;
  useEffect(() => {
    setDragDelta(null); // a mid-drag view switch must not leak its live delta
  }, [selectedViewId]);

  const onWinMove = useCallback((e: PointerEvent) => {
    const d = dragRef.current;
    if (!d) return;
    const k = zoomK.current || 1;
    if (Math.abs(e.clientX - d.px) + Math.abs(e.clientY - d.py) > 3) d.moved = true;
    const nd = { id: d.id, dx: d.baseDx + (e.clientX - d.px) / k, dy: d.baseDy + (e.clientY - d.py) / k };
    liveDelta.current = nd;
    setDragDelta(nd);
  }, []);
  const onWinUp = useCallback(() => {
    window.removeEventListener('pointermove', onWinMove);
    window.removeEventListener('pointerup', onWinUp);
    const d = dragRef.current;
    const viewId = selectedViewIdRef.current;
    if (d?.moved && liveDelta.current && viewId) {
      const ld = liveDelta.current;
      movedRef.current = true;
      setManualDelta(viewId, d.id, { dx: ld.dx, dy: ld.dy });
    }
    liveDelta.current = null;
    setDragDelta(null);
    dragRef.current = null;
  }, [onWinMove, setManualDelta]);
  const onNodePointerDown = useCallback(
    (e: ReactPointerEvent, id: string) => {
      e.stopPropagation(); // keep d3-zoom from treating this as a pan
      const base = overrides[id] ?? { dx: 0, dy: 0 };
      dragRef.current = { id, px: e.clientX, py: e.clientY, baseDx: base.dx, baseDy: base.dy, moved: false };
      window.addEventListener('pointermove', onWinMove);
      window.addEventListener('pointerup', onWinUp);
    },
    [overrides, onWinMove, onWinUp],
  );

  // Select, unless a drag just happened (the trailing click is suppressed).
  const handleSelect = useCallback(
    (id: string | null) => {
      if (movedRef.current) {
        movedRef.current = false;
        return;
      }
      select(focusedUri, id, 'diagram');
    },
    [select, focusedUri],
  );

  const vmQuery = useQuery({
    queryKey: ['viewmodel', selectedViewId, expandedKey],
    queryFn: async () => {
      if (!focusedUri || !selectedViewId) return null;
      return (await getViewModel(focusedUri, selectedViewId, expandedKey)) as unknown as ViewModel;
    },
    enabled: !!focusedUri && !!selectedViewId,
    staleTime: 30_000,
  });

  const vm = vmQuery.data ?? null;

  // Per-tick simulation overlay (session state). Poll while a session is the
  // active one; joins to the scene by ElementId. expandedKey matches the
  // viewmodel call so the scene ids align.
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const overlayQuery = useQuery({
    queryKey: ['sim_overlay', activeSessionId, selectedViewId, expandedKey],
    queryFn: async () => {
      if (!activeSessionId || !selectedViewId) return null;
      return (await getSimOverlay(activeSessionId, selectedViewId, expandedKey)) as unknown as SimOverlay;
    },
    enabled: !!activeSessionId && !!selectedViewId,
    refetchInterval: 300,
    staleTime: 0,
  });
  const overlay = overlayQuery.data ?? null;

  // Per-run verdict overlay (session state, companion verdict sidecar to the
  // sim overlay): constraint solver pass/fail badges + solved values, joined to
  // the scene by ElementId. Same scene-id alignment contract as the sim overlay.
  const verdictQuery = useQuery({
    queryKey: ['verdict_overlay', activeSessionId, selectedViewId, expandedKey],
    queryFn: async () => {
      if (!activeSessionId || !selectedViewId) return null;
      return (await getVerdictOverlay(
        activeSessionId,
        selectedViewId,
        expandedKey,
      )) as unknown as VerdictOverlay;
    },
    enabled: !!activeSessionId && !!selectedViewId,
    refetchInterval: 300,
    staleTime: 0,
  });
  const verdictOverlay = verdictQuery.data ?? null;
  // Stored verification verdicts computed at an earlier tick than the session
  // is now at are STALE — kept and labeled (dimmed pill + tooltip), never
  // silently dropped.
  const verdictStale =
    verdictOverlay?.verified_at_tick != null && verdictOverlay.tick > verdictOverlay.verified_at_tick;

  // Static diagnostics sidecar (validation badges, brief §4 "badge NE"):
  // workspace state, no session needed. Same scene-id alignment contract as the
  // sim/verdict overlays; refreshes with the viewmodel cadence (diagnostics
  // change on edit, not per tick).
  const diagnosticQuery = useQuery({
    queryKey: ['diagnostic_overlay', selectedViewId, expandedKey],
    queryFn: async () => {
      if (!selectedViewId) return null;
      return (await getDiagnosticOverlay(selectedViewId, expandedKey)) as unknown as DiagnosticOverlay;
    },
    enabled: !!selectedViewId,
    staleTime: 30_000,
  });
  const diagnosticOverlay = diagnosticQuery.data ?? null;

  // Publish the text-map for the editor→diagram (cursor) direction.
  useEffect(() => {
    setTextMap(focusedUri, vm?.text_map ?? null);
    return () => setTextMap(null, null);
  }, [focusedUri, vm, setTextMap]);

  // Run elkjs layout+routing whenever the scene changes.
  useEffect(() => {
    let cancelled = false;
    if (!vm?.scene) {
      setLayout(null);
      setTruncation(null);
      return;
    }
    // Render cap: an unscoped/huge scene (thousands of nodes/edges, e.g. the
    // whole standard library) would hang elkjs. Rather than refuse with a blank
    // canvas (3.11), render a partial scene — the first NODE_CAP top-level nodes
    // and the edges among them — and surface a "showing N of M" banner. After
    // WI1/WI2/3.10 almost nothing hits this, but it keeps the tab responsive on
    // a pathological scene instead of locking up.
    const NODE_CAP = 250;
    const EDGE_CAP = 600;
    const { scene, truncation: tr } = truncateScene(vm.scene, NODE_CAP, EDGE_CAP);
    setTruncation(tr);
    setLayoutError(null);
    // Aspect-aware packing (§3): feed elk the live viewport aspect so
    // disconnected sibling sets pack toward the canvas shape instead of a
    // ribbon. edge_styles let layout reserve room for the composed label text.
    const rect = svgRef.current?.getBoundingClientRect();
    const aspect = rect && rect.height > 0 ? rect.width / rect.height : undefined;
    layoutScene(scene, undefined, vm.tokens.shapes, undefined, vm.tokens.edge_styles, aspect)
      .then((res) => {
        if (!cancelled) setLayout(res);
      })
      .catch((e) => {
        if (!cancelled) setLayoutError(e instanceof Error ? e.message : String(e));
      });
    return () => {
      cancelled = true;
    };
  }, [vm]);

  // d3-zoom: pure utility driving an SVG group transform via React state.
  // Attached via callback ref, NOT a mount effect: the first render is almost
  // always an <Empty> branch (viewmodel still loading), so a `[]`-deps effect
  // binds against a null ref and never re-runs when the real <svg> mounts.
  const attachZoom = useCallback((svg: SVGSVGElement | null) => {
    if (svgRef.current && svgRef.current !== svg) {
      d3select(svgRef.current).on('.zoom', null);
    }
    svgRef.current = svg;
    if (!svg) return;
    const behavior: ZoomBehavior<SVGSVGElement, unknown> = d3zoom<SVGSVGElement, unknown>()
      .scaleExtent([0.1, 4])
      // Wheel zooms anywhere; drag-pan and dblclick-zoom start only on empty
      // canvas. d3 listens natively on the svg, so a node's React-side
      // stopPropagation can't shield it — node gestures (drag-move,
      // dblclick go-to-def) must be excluded here.
      .filter(
        (event) =>
          (!event.ctrlKey || event.type === 'wheel') &&
          !event.button &&
          (event.type === 'wheel' ||
            !(event.target as Element | null)?.closest?.('g[data-element-id]')),
      )
      // A pan's trailing click must not reach the clear-selection handler.
      .clickDistance(4)
      .on('zoom', (event) => {
        zoomK.current = event.transform.k;
        transformRef.current = event.transform;
        setTransform(event.transform.toString());
        setZoomScale(event.transform.k);
      });
    d3select(svg).call(behavior);
  }, []);

  // Fit content into view once a layout lands.
  useEffect(() => {
    const svg = svgRef.current;
    if (!svg || !layout) return;
    // An empty DECLARED view still fits its frame (the honest empty state);
    // an empty ad-hoc scene has nothing to fit.
    const emptyScene = layout.nodes.length === 0;
    if (layout.width === 0 && !(emptyScene && vm?.frame)) return;
    const rect = svg.getBoundingClientRect();
    if (rect.width === 0) return; // not laid out in the DOM yet (e.g. jsdom)
    const pad = 40;
    // Fit the content to the viewport. Large diagrams scale down to fit; small
    // ones scale up to MAX_FIT_SCALE (not beyond, so a 1-2 node view doesn't
    // become absurdly large). Center on BOTH axes — the old code pinned to the
    // top (ty = pad/2), which left small diagrams stranded as a tiny box at the
    // top of a tall empty canvas. A framed view fits its FRAME box, not just
    // the content, so the border/heading are never clipped by the fit.
    // Fit clamps (layout-quality brief §4): scale up small scenes to
    // MAX_FIT_SCALE, but never below MIN_FIT_SCALE — a 16% auto-fit reads as
    // broken, whereas a clamped, pannable 25% with the LOD pill reads as "large
    // scene" (the user pans; the glyph band + truncation banner carry the
    // "this is big" signal).
    const MAX_FIT_SCALE = 1.5;
    const MIN_FIT_SCALE = 0.25;
    const content = emptyScene
      ? EMPTY_VIEW_CONTENT
      : { width: layout.width, height: layout.height };
    const box = vm?.frame
      ? frameExtents(content, vm.frame)
      : { x0: 0, y0: 0, x1: content.width, y1: content.height };
    const bw = box.x1 - box.x0;
    const bh = box.y1 - box.y0;
    const scale = Math.max(
      MIN_FIT_SCALE,
      Math.min(MAX_FIT_SCALE, (rect.width - pad) / bw, (rect.height - pad) / bh),
    );
    const tx = (rect.width - bw * scale) / 2 - box.x0 * scale;
    // Both-axis centering (§4): true centers when content fits; for content
    // TALLER than the viewport keep the top-clamp so reading starts at the top
    // (rather than the top half being scrolled off). Content clamped at
    // MIN_FIT_SCALE overflows and pans from a top-anchored origin.
    const ty = Math.max(pad / 2 - box.y0 * scale, (rect.height - bh * scale) / 2 - box.y0 * scale);
    fitRef.current = {
      scale,
      tx,
      ty,
      contentBox: { x: box.x0, y: box.y0, width: bw, height: bh },
      viewport: { width: rect.width, height: rect.height },
    };
    const t = zoomIdentity.translate(tx, ty).scale(scale || 1);
    zoomK.current = t.k;
    transformRef.current = t;
    d3select(svg).call(
      (d3zoom<SVGSVGElement, unknown>().scaleExtent([0.1, 4]) as ZoomBehavior<SVGSVGElement, unknown>)
        .transform,
      t,
    );
    setTransform(t.toString());
    setZoomScale(t.k);
  }, [layout, vm]);

  // Route the Rust-emitted palette through the CSS-variable indirection layer
  // so the canvas follows the app theme (see themeableCanvasPalette). Memoised
  // on the raw palette identity: the ViewModel is salsa-cached and stable, so
  // this rebuilds only when the model does — never on a theme flip, which is
  // resolved by CSS alone.
  const rawPalette = vm?.tokens.palette;
  const palette = useMemo(
    () => (rawPalette ? themeableCanvasPalette(rawPalette) : rawPalette),
    [rawPalette],
  );
  const typography = vm?.tokens.typography;

  // Level of detail (brief §4): the stricter of the node-count and zoom bands.
  // Drives how much of each node the renderer draws (frontend-only — never a
  // re-fetch) and the info pill.
  const nodeCount = layout?.nodes.length ?? 0;
  const lod = effectiveLod(nodeCount, zoomScale);

  // Placed-node lookup for edge endpoint adjustment during drag.
  const placedById = useMemo(() => {
    const m = new Map<string, PlacedNode>();
    for (const pn of layout?.nodes ?? []) m.set(pn.node.element_id, pn);
    return m;
  }, [layout]);

  // Pan the selected node into view when the selection arrives from outside the
  // diagram (editor cursor / tree / inspector) — a diagram click is already on
  // screen, so it never re-pans. Keeps the current zoom; only translates, and
  // only when the node is actually off-screen, to avoid yanking the viewport.
  useEffect(() => {
    if (!selectedId || selectionOrigin === 'diagram') return;
    const svg = svgRef.current;
    const pn = placedById.get(selectedId);
    if (!svg || !pn) return;
    const rect = svg.getBoundingClientRect();
    if (rect.width === 0) return;
    const t = transformRef.current;
    const cx = pn.x + pn.width / 2;
    const cy = pn.y + pn.height / 2;
    const [sx, sy] = t.apply([cx, cy]);
    const margin = 24;
    if (sx >= margin && sx <= rect.width - margin && sy >= margin && sy <= rect.height - margin) return;
    const k = t.k;
    const nt = zoomIdentity.translate(rect.width / 2 - cx * k, rect.height / 2 - cy * k).scale(k);
    zoomK.current = nt.k;
    transformRef.current = nt;
    d3select(svg).call(
      (d3zoom<SVGSVGElement, unknown>().scaleExtent([0.1, 4]) as ZoomBehavior<SVGSVGElement, unknown>).transform,
      nt,
    );
    setTransform(nt.toString());
    setZoomScale(nt.k);
  }, [selectedId, selectionOrigin, placedById]);

  // Scene-nesting parent map: a node's effective drag delta is the sum of its own
  // delta and all its container ancestors' deltas, so dragging a container moves
  // its descendants (they render as sibling <g>s, not DOM descendants, so the
  // delta must be threaded explicitly — no double-counting).
  const parentMap = useMemo(
    () => (vm?.scene ? buildParentMap(vm.scene) : {}),
    [vm],
  );

  const deltaFor = useCallback(
    (id: string): { dx: number; dy: number } =>
      accumulatedOffset(
        id,
        (c) => (dragDelta && dragDelta.id === c ? dragDelta : overrides[c]),
        parentMap,
      ),
    [dragDelta, overrides, parentMap],
  );

  // Committed-only offset (no live drag delta) — the FE label resolver keys on
  // this so labels re-resolve on DROP (overrides change), not on every drag
  // frame (§1: "memoize on the placed-geometry identity"). During an active
  // drag the resolved labels ride their committed positions; the transient
  // preview doesn't reflow the whole occupancy grid each frame.
  const committedDeltaFor = useCallback(
    (id: string): { dx: number; dy: number } =>
      accumulatedOffset(id, (c) => overrides[c], parentMap),
    [overrides, parentMap],
  );

  // ── FE edge-label de-confliction (layout-quality brief §1) ─────────
  // Runs the deterministic slot resolver over the final placed geometry: port
  // labels + node bodies + frame boxes seed the occupancy grid (ports outrank
  // edge labels — §2 step 4), then center/secondary labels claim non-colliding
  // slots ON their routed paths. Feeds BOTH the renderer and the geometry dump,
  // so what the gates measure is exactly what draws.
  const resolvedLabels = useMemo(() => {
    const byEdge = new Map<string, ResolvedLabel[]>();
    if (!layout || !vm) return byEdge;
    const obstacles: LabelRect[] = [];
    let x0 = Infinity;
    let y0 = Infinity;
    let x1 = -Infinity;
    let y1 = -Infinity;
    for (const pn of layout.nodes) {
      const d = committedDeltaFor(pn.node.element_id);
      // A container's BODY is legitimate routing space (a nested transition
      // label belongs inside its machine) and a sequence lifeline's LANE is
      // where messages + their labels live — seed only the solid HEAD/HEADER
      // band as an obstacle, plus every leaf body.
      const solidH = solidBandHeight(pn);
      const rect = { x: pn.x + d.dx, y: pn.y + d.dy, width: pn.width, height: solidH };
      obstacles.push(rect);
      x0 = Math.min(x0, pn.x + d.dx);
      y0 = Math.min(y0, pn.y + d.dy);
      x1 = Math.max(x1, pn.x + d.dx + pn.width);
      y1 = Math.max(y1, pn.y + d.dy + pn.height);
      for (const pp of pn.ports) {
        obstacles.push({ x: pp.x + d.dx, y: pp.y + d.dy, width: pp.width, height: pp.height });
        const lr = portLabelRect(pp, d);
        if (lr) obstacles.push(lr);
      }
    }
    if (vm.frame) {
      for (const r of frameObstacles(vm.frame, layout)) obstacles.push(r);
    }
    if (!Number.isFinite(x0)) {
      x0 = 0;
      y0 = 0;
      x1 = layout.width;
      y1 = layout.height;
    }
    const edges = layout.edges.map((pe) => {
      const points = adjustedEdgePoints(pe, placedById, committedDeltaFor);
      const decor = edgeDecor(pe.edge, vm.tokens.edge_styles);
      return {
        edgeId: pe.edge.id,
        points,
        centerText: decor.label,
        secondaryTexts: (pe.edge.secondary_labels ?? []).map((s) => s.text),
        preferredAt: pe.labelAt,
      };
    });
    const resolved = resolveEdgeLabels({
      obstacles,
      edges,
      bounds: { x: x0, y: y0, width: x1 - x0, height: y1 - y0 },
    });
    for (const r of resolved) {
      const arr = byEdge.get(r.edgeId) ?? [];
      arr.push(r);
      byEdge.set(r.edgeId, arr);
    }
    return byEdge;
  }, [layout, vm, placedById, committedDeltaFor]);

  // ── Geometry dump (layout-quality brief §6) ────────────────────────
  // `window.__diagramGeometryForTests`: published after layout + label
  // placement so `tools/diagram-review/assert-geometry.mjs` can run the
  // G1–G7 gates against the real rendered geometry (same pattern as the
  // `__workspaceStoreForTests` hook — dev-mode only, never in prod builds).
  // Rects are in scene coordinates; `fit` carries the one-shot fit transform.
  useEffect(() => {
    if (!import.meta.env.DEV) return;
    if (!layout || !vm) {
      delete (window as { __diagramGeometryForTests?: unknown }).__diagramGeometryForTests;
      return;
    }
    const nodes: { id: string; rect: Rect; container: boolean }[] = [];
    const ports: { id: string; rect: Rect; labelRect: Rect | null }[] = [];
    for (const pn of layout.nodes) {
      const d = deltaFor(pn.node.element_id);
      nodes.push({
        id: pn.node.element_id,
        // Report the SOLID band for a routing-lane node (container header /
        // lifeline head) so G1 measures the real obstacle, not the lane.
        rect: {
          x: pn.x + d.dx,
          y: pn.y + d.dy,
          width: pn.width,
          height: isRoutingLane(pn) ? solidBandHeight(pn) : pn.height,
        },
        container: isRoutingLane(pn),
      });
      for (const pp of pn.ports) {
        const rect = { x: pp.x + d.dx, y: pp.y + d.dy, width: pp.width, height: pp.height };
        // Use the SAME box the renderer draws (elk-placed or side-aware).
        const labelRect = portLabelRect(pp, d);
        ports.push({ id: pp.port.element_id, rect, labelRect });
      }
    }
    // Frame heading tab + corner info compartments count as node boxes (G1).
    if (vm.frame) {
      const content =
        layout.nodes.length === 0
          ? EMPTY_VIEW_CONTENT
          : { width: layout.width, height: layout.height };
      const ext = frameExtents(content, vm.frame);
      nodes.push({
        id: 'frame:heading',
        rect: { x: ext.x0, y: ext.y0, width: headingTabWidth(vm.frame), height: FRAME_HEADING_H },
        container: false,
      });
      if (vm.frame.top_right) {
        const w = vm.frame.top_right.text.length * 6.6 + 16;
        nodes.push({ id: 'frame:top_right', rect: { x: ext.x1 - w, y: ext.y0, width: w, height: 18 }, container: false });
      }
      if (vm.frame.bottom_left) {
        const w = vm.frame.bottom_left.text.length * 6.6 + 16;
        nodes.push({ id: 'frame:bottom_left', rect: { x: ext.x0, y: ext.y1 - 18, width: w, height: 18 }, container: false });
      }
      if (vm.frame.bottom_right) {
        const w = vm.frame.bottom_right.text.length * 6.6 + 16;
        nodes.push({ id: 'frame:bottom_right', rect: { x: ext.x1 - w, y: ext.y1 - 18, width: w, height: 18 }, container: false });
      }
    }
    const edges: { id: string; points: { x: number; y: number }[]; portAnchored: boolean | null }[] = [];
    const labels: { edgeId: string; rect: Rect; degraded: boolean; kind: string }[] = [];
    const portCenter = (nodeId: string, portId: string | null): { x: number; y: number } | null => {
      if (!portId) return null;
      const pn = placedById.get(nodeId);
      const pp = pn?.ports.find((p) => p.port.element_id === portId);
      if (!pn || !pp) return null;
      const d = deltaFor(nodeId);
      return { x: pp.x + d.dx + pp.width / 2, y: pp.y + d.dy + pp.height / 2 };
    };
    for (const pe of layout.edges) {
      const points = adjustedEdgePoints(pe, placedById, deltaFor);
      // G4: an edge that carries resolved port ids must start/end within 1px
      // of the port glyph center. null = no resolved ports (gate n/a).
      const sc = portCenter(pe.edge.source_id, pe.edge.source_port_id);
      const tc = portCenter(pe.edge.target_id, pe.edge.target_port_id);
      let portAnchored: boolean | null = null;
      if ((sc || tc) && points.length >= 2) {
        const near = (p: { x: number; y: number }, c: { x: number; y: number }) =>
          Math.abs(p.x - c.x) <= 1 && Math.abs(p.y - c.y) <= 1;
        portAnchored =
          (!sc || near(points[0], sc)) && (!tc || near(points[points.length - 1], tc));
      }
      edges.push({ id: pe.edge.id, points, portAnchored });
    }
    // Labels come from the FE resolver (§1) — exactly what the renderer draws.
    for (const [edgeId, arr] of resolvedLabels) {
      for (const r of arr) {
        labels.push({ edgeId, rect: r.rect, degraded: r.degraded, kind: r.kind });
      }
    }
    (window as { __diagramGeometryForTests?: unknown }).__diagramGeometryForTests = {
      viewId: selectedViewId,
      nodes,
      ports,
      edges,
      labels,
      fit: fitRef.current,
    };
  }, [layout, vm, placedById, deltaFor, resolvedLabels, transform, selectedViewId]);

  // Go-to-definition (double-click): resolve a node's typing classifier via the
  // ViewModel's InteractionMap and select it. Origin 'ui' (not 'diagram') so the
  // target — which may be off-canvas — pans into view (3.13) and reveals source.
  const interactions = vm?.interactions ?? null;
  const handleGoToDef = useCallback(
    (id: string) => {
      const target = interactions?.entries?.[id]?.type_definition;
      if (target) select(focusedUri, target, 'ui');
    },
    [interactions, select, focusedUri],
  );

  if (!focusedUri) {
    return <Empty text="Load a file, then pick a view to render the diagram." />;
  }
  if (!selectedViewId) {
    return <Empty text="Pick a declared view (ViewUsage / ViewDefinition) to render." />;
  }
  if (vmQuery.isLoading) return <Empty text="Loading view…" />;
  if (vmQuery.isError) {
    return <Empty text={`Failed to load ViewModel: ${String(vmQuery.error)}`} />;
  }
  if (layoutError) return <Empty text={`Layout failed: ${layoutError}`} />;
  if (!vm || !palette) return <Empty text="No view." />;

  return (
    <svg
      ref={attachZoom}
      data-testid="svg-canvas"
      width="100%"
      height="100%"
      style={{ background: palette.bg, display: 'block', cursor: 'grab' }}
      onClick={(e) => {
        // Click on empty canvas clears selection.
        if (e.target === svgRef.current) select(null, null, 'diagram');
      }}
    >
      <defs>
        {/* Open V arrow (dependency/flow/transition), target end. */}
        <marker id="svgc-arrow" viewBox="0 0 12 12" refX="10" refY="6" markerWidth="9" markerHeight="9" orient="auto-start-reverse">
          <path d="M1,1 L11,6 L1,11" fill="none" stroke={palette.muted} strokeWidth={1.4} />
        </marker>
        {/* Filled triangle (generalization / composition end / message). */}
        <marker id="svgc-tri-filled" viewBox="0 0 12 12" refX="11" refY="6" markerWidth="11" markerHeight="11" orient="auto-start-reverse">
          <path d="M1,1 L11,6 L1,11 z" fill={palette.muted} />
        </marker>
        {/* Hollow triangle (specialization). */}
        <marker id="svgc-tri-hollow" viewBox="0 0 12 12" refX="11" refY="6" markerWidth="11" markerHeight="11" orient="auto-start-reverse">
          <path d="M1,1 L11,6 L1,11 z" fill={palette.bg} stroke={palette.muted} strokeWidth={1.2} />
        </marker>
        {/* Filled diamond — composite aggregation (§F-8), source end. */}
        <marker id="svgc-diamond-filled" viewBox="0 0 16 12" refX="1" refY="6" markerWidth="14" markerHeight="11" orient="auto-start-reverse">
          <path d="M1,6 L8,1 L15,6 L8,11 z" fill={palette.muted} />
        </marker>
        {/* Open diamond — shared aggregation (§F-8), source end. */}
        <marker id="svgc-diamond-open" viewBox="0 0 16 12" refX="1" refY="6" markerWidth="14" markerHeight="11" orient="auto-start-reverse">
          <path d="M1,6 L8,1 L15,6 L8,11 z" fill={palette.bg} stroke={palette.muted} strokeWidth={1.2} />
        </marker>
        {/* Warm graph-paper: a fine (24px) + coarse (96px) grid, both tracking
            pan/zoom via patternTransform so the paper moves with the diagram. */}
        <pattern id="svgc-grid-minor" width={24} height={24} patternUnits="userSpaceOnUse" patternTransform={transform}>
          <path d="M24,0 H0 V24" fill="none" stroke={palette.grid_minor} strokeWidth={0.5} />
        </pattern>
        <pattern id="svgc-grid-major" width={96} height={96} patternUnits="userSpaceOnUse" patternTransform={transform}>
          <path d="M96,0 H0 V96" fill="none" stroke={palette.grid_major} strokeWidth={0.75} />
        </pattern>
        {/* Amber halo blur (crib §3 sim-active glow "0 0 18px"): a wide-stroked
            outline rect run through this becomes the soft outer glow. */}
        <filter id="svgc-halo" x="-40%" y="-40%" width="180%" height="180%">
          <feGaussianBlur stdDeviation={5} />
        </filter>
        {/* Diagonal hatch for the vd-error pill (brief §3.5 "hatched pill" —
            the redundant non-colour encoding for *couldn't evaluate*). */}
        <pattern id="svgc-hatch" width={4} height={4} patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
          <rect width={4} height={4} fill={palette.verdict.error} />
          <line x1={0} y1={0} x2={0} y2={4} stroke={palette.muted} strokeWidth={1.2} />
        </pattern>
      </defs>
      {/* Paper texture: fixed viewport backdrops, grid lines pan with content. */}
      <rect x={0} y={0} width="100%" height="100%" fill="url(#svgc-grid-minor)" style={{ pointerEvents: 'none' }} />
      <rect x={0} y={0} width="100%" height="100%" fill="url(#svgc-grid-major)" style={{ pointerEvents: 'none' }} />
      <g transform={transform}>
        {/* Framed-view (§8.2.3.26 / contract §C): frame border, «view» heading
            tab, corner info compartments — drawn from `vm.frame` (§F-10) around
            the laid-out content bounds (0,0)-(w,h). */}
        {vm.frame && layout && (
          <ViewFrameLayer
            frame={vm.frame}
            width={layout.nodes.length === 0 ? EMPTY_VIEW_CONTENT.width : layout.width}
            height={layout.nodes.length === 0 ? EMPTY_VIEW_CONTENT.height : layout.height}
            palette={palette}
          />
        )}
        {/* Honest empty state (D-B3): a declared view whose expose/filter
            selects nothing is still a framed view — say so, inside the frame,
            instead of falling back to a bare canvas or a picker surface. */}
        {vm.frame && layout && layout.nodes.length === 0 && (
          <text
            data-testid="svgc-empty-view-note"
            x={EMPTY_VIEW_CONTENT.width / 2}
            y={EMPTY_VIEW_CONTENT.height / 2}
            textAnchor="middle"
            fontSize={12}
            fill={palette.muted}
          >
            This view exposes nothing — its expose/filter selected no elements.
          </text>
        )}

        {/* Nodes. */}
        {layout?.nodes.map((pn) => {
          const d = deltaFor(pn.node.element_id);
          return (
            <Node
              key={pn.node.element_id}
              pn={pn}
              palette={palette}
              categories={vm.tokens.categories}
              shapes={vm.tokens.shapes}
              typography={typography!}
              selected={pn.node.element_id === selectedId}
              hovered={pn.node.element_id === hoverId}
              lod={lod}
              expandedHere={expandedIds.has(pn.node.element_id)}
              overlay={overlayForNode(overlay, pn.node.element_id)}
              verdict={verdictForNode(verdictOverlay, pn.node.element_id)}
              verdictStale={verdictStale}
              diag={diagnosticsForNode(diagnosticOverlay, pn.node.element_id)}
              dx={d.dx}
              dy={d.dy}
              onSelect={handleSelect}
              onToggleExpand={toggleExpand}
              onHover={setHoverId}
              onPointerDown={onNodePointerDown}
              onGoToDef={handleGoToDef}
              hasDef={!!interactions?.entries?.[pn.node.element_id]?.type_definition}
            />
          );
        })}

        {/* Edges on top of node fills so short routes aren't occluded;
            non-interactive so they don't intercept node clicks. */}
        <g style={{ pointerEvents: 'none' }}>
          {layout?.edges.map((pe) => (
            <Edge
              key={pe.edge.id}
              pe={pe}
              points={adjustedEdgePoints(pe, placedById, deltaFor)}
              labels={resolvedLabels.get(pe.edge.id) ?? []}
              lod={lod}
              edgeStyles={vm.tokens.edge_styles}
              muted={palette.muted}
              labelBg={palette.edge_label_bg}
              textColor={palette.text}
            />
          ))}
        </g>
      </g>

      {/* Partial-render banner (3.11). Pinned to the viewport (outside the zoom
          group, in root coords) so it stays put while panning/zooming. */}
      {truncation && (
        <g data-testid="svg-canvas-cap-banner" style={{ pointerEvents: 'none' }}>
          <rect x={0} y={0} width="100%" height={22} fill={palette.bg} opacity={0.92} />
          <rect x={0} y={22} width="100%" height={1} fill={palette.muted} opacity={0.5} />
          <text x={8} y={15} fontSize={11} fill={palette.text}>
            {`Showing ${truncation.nodes} of ${truncation.totalNodes} nodes, ${truncation.edges} of ${truncation.totalEdges} edges — scene capped (scope this view with an expose to see all)`}
          </text>
        </g>
      )}

      {/* Canvas info pill (crib §3): zoom % · node count · LOD band, pinned to
          the viewport's top-right (outside the zoom group). Nudged down when the
          truncation banner occupies the top edge. */}
      {layout && (
        <foreignObject x={0} y={truncation ? 24 : 0} width="100%" height={30} style={{ pointerEvents: 'none' }}>
          <div style={{ display: 'flex', justifyContent: 'flex-end', padding: '6px 10px' }}>
            <span
              data-testid="svg-canvas-lod-pill"
              style={{
                fontFamily: 'ui-monospace, monospace',
                fontSize: 11,
                color: palette.muted,
                background: palette.bg,
                border: `1px solid ${palette.grid_major}`,
                borderRadius: 4,
                padding: '3px 8px',
              }}
            >
              {`${Math.round(zoomScale * 100)}% · ${nodeCount} node${nodeCount === 1 ? '' : 's'} · ${lodLabel(lod)}`}
            </span>
          </div>
        </foreignObject>
      )}
    </svg>
  );
}

function Empty({ text }: { text: string }) {
  return (
    <div
      data-testid="svg-canvas-empty"
      style={{ padding: 16, fontSize: 12, color: 'var(--outline, #888)' }}
    >
      {text}
    </div>
  );
}

type PaletteT = NonNullable<ViewModel['tokens']>['palette'];

/** A node's expand/collapse affordance state, or null if it has no toggle. */
export interface ExpandToggle {
  expanded: boolean;
  onToggle: (e: ReactMouseEvent) => void;
}

function Node({
  pn,
  palette,
  categories,
  shapes,
  typography,
  selected,
  hovered,
  lod,
  expandedHere,
  overlay,
  verdict,
  verdictStale,
  diag,
  dx,
  dy,
  onSelect,
  onToggleExpand,
  onHover,
  onPointerDown,
  onGoToDef,
  hasDef,
}: {
  pn: PlacedNode;
  palette: PaletteT;
  categories: Record<string, string>;
  shapes: Record<string, string>;
  typography: NonNullable<ViewModel['tokens']>['typography'];
  selected: boolean;
  hovered: boolean;
  /** Effective level of detail (brief §4) — gates how much of the node draws. */
  lod: Lod;
  /** Whether this node is in the client drill-down expand set. */
  expandedHere: boolean;
  /** This tick's simulation overlay for the node, or null. */
  overlay: ElementOverlay | null;
  /** This run's constraint verdict for the node, or null. */
  verdict: ElementVerdict | null;
  /** Whether stored verification verdicts predate the session's current tick. */
  verdictStale: boolean;
  /** This scene's validation diagnostics for the node, or null. */
  diag: ElementDiagnostics | null;
  /** Manual-override (drag) delta from the elkjs base position. */
  dx: number;
  dy: number;
  onSelect: (id: string) => void;
  onToggleExpand: (id: string, expand: boolean) => void;
  onHover: (id: string | null) => void;
  onPointerDown: (e: ReactPointerEvent, id: string) => void;
  /** Go-to-definition (double-click) — navigates to the node's typing classifier. */
  onGoToDef: (id: string) => void;
  /** True when the InteractionMap has a go-to-definition target for this node. */
  hasDef: boolean;
}) {
  const { node, x, y, width, height } = pn;
  const shape = shapeForVisualKind(shapes, node.visual_kind);
  const click = (e: ReactMouseEvent) => {
    e.stopPropagation();
    onSelect(node.element_id);
  };
  const doubleClick = (e: ReactMouseEvent) => {
    e.stopPropagation();
    onGoToDef(node.element_id);
  };
  // §F-9: an n-ary relationship's central junction dot.
  const isNaryDot = (node.tags ?? []).includes('NaryDot');
  // Expand/collapse: '+' on a collapsed container; '−' on one we expanded.
  // A view-default-expanded node (expanded && !expandedHere) gets no toggle —
  // the additive merge contract can't collapse it. `expanded: false` alone is
  // NOT enough for a '+': the composer also sets it on containers collapsed
  // purely for layout — hidden content exists iff it sent the Expand button.
  const expandable = (node.buttons ?? []).some(
    (b) =>
      typeof b === 'object' && b !== null && (b as { button_type?: unknown }).button_type === 'Expand',
  );
  const showToggle =
    (node.expanded === false && expandable) || (node.expanded === true && expandedHere);
  const toggle: ExpandToggle | null = showToggle
    ? {
        expanded: node.expanded === true,
        onToggle: (e) => {
          e.stopPropagation();
          onToggleExpand(node.element_id, node.expanded !== true);
        },
      }
    : null;
  const transform = dx || dy ? `translate(${dx},${dy})` : undefined;
  return (
    <g
      data-element-id={node.element_id}
      data-has-def={hasDef ? 'true' : undefined}
      transform={transform}
      onClick={click}
      onDoubleClick={doubleClick}
      onPointerDown={(e) => onPointerDown(e, node.element_id)}
      onPointerEnter={() => onHover(node.element_id)}
      onPointerLeave={() => onHover(null)}
      style={{ cursor: 'grab' }}
    >
      {node.tooltip && <title>{node.tooltip}</title>}
      {isNaryDot ? (
        <circle
          cx={x + width / 2}
          cy={y + height / 2}
          r={5}
          fill={selected ? palette.text : palette.muted}
        />
      ) : node.visual_kind === 'Lifeline' ? (
        <LifelineNode pn={pn} palette={palette} categories={categories} selected={selected} hovered={hovered} typography={typography} />
      ) : isCardShape(shape) ? (
        <CardNode pn={pn} palette={palette} categories={categories} shape={shape} selected={selected} hovered={hovered} lod={lod} overlay={overlay} failed={verdict?.verdict === 'Fail'} hasBadge={!!diag} toggle={toggle} typography={typography} />
      ) : (
        <GlyphNode pn={pn} palette={palette} categories={categories} shape={shape} selected={selected} hovered={hovered} lod={lod} overlay={overlay} typography={typography} />
      )}
      {pn.ports.length > 0 && <Ports ports={pn.ports} palette={palette} />}
      {/* ── Overlay layers (brief §4) — compose ADDITIVELY on top of the body.
          Z-order: verdict pill (SW), diagnostic badge (NE), selection handles,
          then the glyph-band worst-state dot. Each channel keeps its own corner
          so a selected + live + failing + flagged node stays legible. */}
      {lod !== 'glyph' && verdict?.verdict && (
        <VerdictPill verdict={verdict} palette={palette} x={x} y={y + height} showValue={lod === 'full'} stale={verdictStale} />
      )}
      {lod !== 'glyph' && diag && (
        <DiagnosticBadge diag={diag} palette={palette} cx={x + width} cy={y} full={lod === 'full'} />
      )}
      {selected && <SelectionHandles palette={palette} x={x} y={y} width={width} height={height} />}
      {lod === 'glyph' && (() => {
        const ws = worstState(overlay, verdict, diag);
        if (!ws) return null;
        const color =
          ws === 'fail' ? palette.verdict.fail : ws === 'active' ? palette.sim.active : palette.sev[ws];
        return <circle cx={x + width - 4} cy={y + 4} r={3.5} fill={color} style={{ pointerEvents: 'none' }} />;
      })()}
    </g>
  );
}

/** Selection corner handles (crib §3): 6×6 solid-ink squares outset on each
 *  corner. With the ink border they are the selection channel — present at
 *  every LOD band (brief §4: selection survives even the glyph band). */
function SelectionHandles({
  palette,
  x,
  y,
  width,
  height,
}: {
  palette: PaletteT;
  x: number;
  y: number;
  width: number;
  height: number;
}) {
  const s = 6;
  const corners: [number, number][] = [
    [x, y],
    [x + width, y],
    [x, y + height],
    [x + width, y + height],
  ];
  return (
    <g style={{ pointerEvents: 'none' }}>
      {corners.map(([cx, cy], i) => (
        <rect key={i} x={cx - s / 2} y={cy - s / 2} width={s} height={s} fill={palette.text} />
      ))}
    </g>
  );
}

/** The SW verdict pill (brief §3.5 / §4 "pill (SW)"): glyph + optional solved
 *  value on the node's bottom-left corner. Pass/fail are solid hue pills;
 *  inconclusive is a dashed outline; error a hatched neutral-dark fill — the
 *  redundant non-colour encodings. */
function VerdictPill({
  verdict,
  palette,
  x,
  y,
  showValue,
  stale = false,
}: {
  verdict: ElementVerdict;
  palette: PaletteT;
  x: number;
  /** The node's bottom edge — the pill straddles it. */
  y: number;
  /** Whether to append the solved value (full LOD only). */
  showValue: boolean;
  /** Verification ran at an earlier tick than the session is at now —
   *  dim the pill and say so, rather than dropping it. */
  stale?: boolean;
}) {
  const style = verdictPillStyle(verdict);
  if (!style) return null;
  const value =
    showValue && verdict.value != null
      ? Number.isInteger(verdict.value)
        ? String(verdict.value)
        : Number(verdict.value.toPrecision(3)).toString()
      : null;
  const text = value ? `${style.glyph} ${value}` : style.glyph;
  const h = 15;
  const w = Math.max(h, text.length * 6.4 + 9);
  const color = palette.verdict[style.token];
  const fill = style.hatched ? 'url(#svgc-hatch)' : style.solid ? color : palette.bg;
  const ink = style.solid ? palette.text : style.hatched ? palette.muted : color;
  return (
    <g data-testid="svgc-verdict-pill" opacity={stale ? 0.55 : 1} style={{ pointerEvents: 'none' }}>
      {stale && <title>stale — verified at an earlier tick; re-run verification</title>}
      <rect
        x={x - 4}
        y={y - h / 2}
        width={w}
        height={h}
        rx={h / 2}
        fill={fill}
        stroke={style.solid ? 'none' : color}
        strokeWidth={style.solid ? 0 : 1.25}
        strokeDasharray={style.dashed ? '3 2' : undefined}
      />
      <text
        x={x - 4 + w / 2}
        y={y + 3.5}
        fontSize={10}
        fontWeight={700}
        textAnchor="middle"
        fill={ink}
      >
        {text}
      </text>
    </g>
  );
}

/** The NE diagnostic badge (brief §4 "badge (NE)", crib §3): a 16px circle
 *  bleeding off the top-right corner in the worst-case severity colour, with the
 *  severity glyph and a tooltip listing every diagnostic. At reduced LOD the
 *  badge collapses to a plain severity dot (brief §4 LOD table "badge → dot"). */
function DiagnosticBadge({
  diag,
  palette,
  cx,
  cy,
  full,
}: {
  diag: ElementDiagnostics;
  palette: PaletteT;
  cx: number;
  cy: number;
  full: boolean;
}) {
  const color = palette.sev[diag.severity];
  if (!full) {
    return <circle cx={cx} cy={cy} r={3.5} fill={color} style={{ pointerEvents: 'none' }} />;
  }
  return (
    <g data-testid="svgc-diagnostic-badge">
      <title>{diagnosticTooltip(diag)}</title>
      <circle cx={cx} cy={cy} r={8} fill={color} stroke={palette.bg} strokeWidth={1.25} />
      <text x={cx} y={cy + 3.5} fontSize={10} fontWeight={600} textAnchor="middle" fill={palette.bg}>
        {severityGlyph(diag.severity)}
      </text>
    </g>
  );
}

/** Ports on a node boundary (IBD / §F-6). elk places them; we draw a small
 *  interface-family pill (crib §3: ports carry the port-family tint), stroked
 *  in the feature-direction colour (in/out/inout — the Rust `palette.port`
 *  tokens), dashed for a reference port. They live inside the node's <g>, so
 *  they translate with the node on drag. */
function Ports({ ports, palette }: { ports: PlacedNode['ports']; palette: PaletteT }) {
  const dirStroke = (d: PlacedNode['ports'][number]['port']['direction']) =>
    d === 'In' ? palette.port.in_ : d === 'Out' ? palette.port.out : d === 'InOut' ? palette.port.inout : palette.interface.stroke;
  return (
    <>
      {ports.map((pp) => (
        <g key={pp.port.element_id} data-element-id={pp.port.element_id}>
          <rect
            x={pp.x}
            y={pp.y}
            width={pp.width}
            height={pp.height}
            rx={2}
            fill={palette.interface.fill}
            stroke={dirStroke(pp.port.direction)}
            strokeWidth={1.25}
            strokeDasharray={pp.port.is_reference ? '2 2' : undefined}
          />
          {(() => {
            // §2: render the port label at elk's computed position (OUTSIDE,
            // next-to-port), or the side-aware fallback — never the old blind
            // `y − 3` centered strip that poked out of West-side nodes.
            const box = portLabelBox(pp);
            if (!box) return null;
            return (
              <text
                x={box.textX}
                y={box.textY}
                fontSize={8}
                textAnchor={box.anchor}
                fill={palette.muted}
              >
                {pp.port.name}
              </text>
            );
          })()}
        </g>
      ))}
    </>
  );
}

/** Mini kind-glyph for a card header (crib §2 shared glyph vocabulary): a ~7×7
 *  stereotype indicator in the family stroke colour, drawn at the header's
 *  top-left corner. Shape echoes the element kind — square (part), circle
 *  (action/interface), rounded square (state), diamond (constraint),
 *  double-border square (requirement). The long tail gets a plain small square. */
function HeaderKindGlyph({
  visualKind,
  cx,
  cy,
  color,
}: {
  visualKind: string;
  cx: number;
  cy: number;
  color: string;
}) {
  const s = 3.4; // half-size → ~7px box
  const sw = 1.4;
  const common = { fill: 'none', stroke: color, strokeWidth: sw };
  switch (visualKind) {
    case 'Action':
    case 'Calculation':
    case 'SendAction':
    case 'AcceptAction':
    case 'Interface':
    case 'Port':
      return <circle cx={cx} cy={cy} r={s} {...common} />;
    case 'State':
      return <rect x={cx - s} y={cy - s} width={s * 2} height={s * 2} rx={1.5} {...common} />;
    case 'Constraint':
      return (
        <polygon points={`${cx},${cy - s} ${cx + s},${cy} ${cx},${cy + s} ${cx - s},${cy}`} {...common} />
      );
    case 'Requirement':
    case 'Concern':
    case 'VerificationCase':
      // Double-border square.
      return (
        <>
          <rect x={cx - s} y={cy - s} width={s * 2} height={s * 2} {...common} />
          <rect x={cx - s + 1.4} y={cy - s + 1.4} width={s * 2 - 2.8} height={s * 2 - 2.8} {...common} strokeWidth={0.9} />
        </>
      );
    default:
      // part / block / item / attribute / long tail → square.
      return <rect x={cx - s} y={cy - s} width={s * 2} height={s * 2} {...common} />;
  }
}

/** Sequence-diagram lifeline (§ Sequence view): a head box carrying the
 *  participant name, with a dashed vertical lane running down to the node's
 *  full height. Message proxies (occurrence dots) and message edges are placed
 *  on top by the fixed-layout pass. The head height matches the generator's
 *  `HEAD_HEIGHT` so proxies/messages line up below the head. */
const LIFELINE_HEAD = 40;
function LifelineNode({
  pn,
  palette,
  categories,
  selected,
  hovered,
  typography,
}: {
  pn: PlacedNode;
  palette: PaletteT;
  categories: Record<string, string>;
  selected: boolean;
  hovered: boolean;
  typography: NonNullable<ViewModel['tokens']>['typography'];
}) {
  const { node, x, y, width, height } = pn;
  const colors = colorsForVisualKind(palette, categories, node.visual_kind);
  const cx = x + width / 2;
  // Selection = ink border (brief §4: selection always wins the border; amber
  // is reserved for the live channel).
  const stroke = selected ? palette.text : hovered ? colors.stroke : colors.stroke;
  const sw = selected ? 1.75 : hovered ? 2 : 1.25;
  return (
    <>
      <line
        x1={cx}
        y1={y + LIFELINE_HEAD}
        x2={cx}
        y2={y + height}
        stroke={palette.muted}
        strokeWidth={1.25}
        strokeDasharray="4 4"
      />
      <rect x={x} y={y} width={width} height={LIFELINE_HEAD} rx={2} fill={colors.fill} stroke={stroke} strokeWidth={sw} />
      <text x={cx} y={y + LIFELINE_HEAD / 2 + 4} fontSize={typography.label_font_size_px} fontWeight={600} textAnchor="middle" fill={palette.text}>
        {node.name}
      </text>
    </>
  );
}

/** The simulation live/completed decor, shared by card and glyph nodes.
 *  Live = the **amber halo** channel (brief §4): a blurred outer glow plus a
 *  crisp thin ring — deliberately soft-edged so it can't be confused with the
 *  hard ink selection border it composes with. Completed keeps a quiet green
 *  ring. Rendered on top of the node body. */
function SimDecor({
  overlay,
  palette,
  x,
  y,
  width,
  height,
  showValue,
}: {
  overlay: ElementOverlay | null;
  palette: PaletteT;
  x: number;
  y: number;
  width: number;
  height: number;
  /** Whether to draw the live scalar value strip (full LOD only, brief §4). */
  showValue: boolean;
}) {
  if (!overlay) return null;
  const active = isActive(overlay);
  const completed = isCompleted(overlay);
  return (
    <>
      {active && (
        <>
          {/* Soft outer glow (crib §3 "0 0 18px"): a wide amber outline run
              through the shared blur filter. */}
          <rect
            x={x - 3}
            y={y - 3}
            width={width + 6}
            height={height + 6}
            rx={9}
            fill="none"
            stroke={palette.sim.active_glow}
            strokeWidth={7}
            filter="url(#svgc-halo)"
            style={{ pointerEvents: 'none' }}
          />
          <rect
            x={x - 2.5}
            y={y - 2.5}
            width={width + 5}
            height={height + 5}
            rx={8}
            fill="none"
            stroke={palette.sim.active}
            strokeWidth={1.5}
            opacity={0.9}
            style={{ pointerEvents: 'none' }}
          />
        </>
      )}
      {!active && completed && (
        <rect
          x={x - 2.5}
          y={y - 2.5}
          width={width + 5}
          height={height + 5}
          rx={8}
          fill="none"
          stroke={palette.sim.completed}
          strokeWidth={2.5}
          opacity={0.6}
          style={{ pointerEvents: 'none' }}
        />
      )}
      {overlay.value && showValue && (
        <g>
          {(() => {
            const text = formatOverlayValue(overlay.value);
            const w = text.length * 6.2 + 8;
            return (
              <>
                <rect x={x + width - w} y={y - 9} width={w} height={16} rx={3} fill={palette.sim.active} opacity={0.9} />
                <text x={x + width - w / 2} y={y + 2.5} fontSize={10} fontWeight={600} textAnchor="middle" fill={palette.bg}>
                  {text}
                </text>
              </>
            );
          })()}
        </g>
      )}
    </>
  );
}

/** Card-like node: Rect / RoundedRect (corner from def/usage), NoteRect (folded
 *  corner), DashedRect (dashed border). Renders header tint + compartment text. */
function CardNode({
  pn,
  palette,
  categories,
  shape,
  selected,
  hovered,
  lod,
  overlay,
  failed,
  hasBadge,
  toggle,
  typography,
}: {
  pn: PlacedNode;
  palette: PaletteT;
  categories: Record<string, string>;
  shape: ShapeName;
  selected: boolean;
  hovered: boolean;
  lod: Lod;
  overlay: ElementOverlay | null;
  /** Failing verdict → translucent fail fill tint (brief §4 "fill tint: fail only"). */
  failed: boolean;
  /** A NE diagnostic badge is present — shifts the live dot out of its corner. */
  hasBadge: boolean;
  toggle: ExpandToggle | null;
  typography: NonNullable<ViewModel['tokens']>['typography'];
}) {
  const { node, x, y, width, height, headerHeight, lines, hasChildren } = pn;
  const colors = colorsForVisualKind(palette, categories, node.visual_kind);
  // LOD gating (brief §4): `chrome` = header glyph + stereotype + name + toggle
  // (shown at full/reduced, dropped at glyph); `detail` = compartment rows +
  // live value strip (full only). At glyph a card is just its shape + family +
  // selection + a live dot.
  const chrome = lod !== 'glyph';
  const detail = lod === 'full';
  // §F-3 + R1 (OMG formal/26-03-02 Tables 4/15/17/19): a DEFINITION is always
  // drawn with SHARP corners — universally, including the action/state/
  // constraint RoundedRect families (`«state def»` etc. are sharp, only the
  // `«state»` usage is rounded). So a Definition never rounds; usages and the
  // neutral RoundedRect family do. Per-kind radius within "rounded" (crib §2):
  // state reads as a pill, action softer, everything else a gentle round.
  const rounded =
    node.node_kind !== 'Definition' && (shape === 'RoundedRect' || node.node_kind === 'Usage');
  const rx = !rounded
    ? 0
    : node.visual_kind === 'State'
      ? 14
      : node.visual_kind === 'Action'
        ? 8
        : 6;
  // Selection = ink border + handles (brief §4 rule 2: selection always wins
  // the border; nothing else may take border weight). Amber never marks
  // selection — it's the live channel. Hover is a quiet family-stroke bump.
  const stroke = selected ? palette.text : colors.stroke;
  const strokeWidth = selected ? 1.75 : hovered ? 2 : 1.25;
  const showHeaderSplit = headerHeight > 0 && (lines.length > 0 || hasChildren);

  return (
    <>
      {shape === 'NoteRect' ? (
        <path
          d={noteRectPath(x, y, width, height)}
          fill={colors.fill}
          stroke={stroke}
          strokeWidth={strokeWidth}
        />
      ) : (
        <rect
          x={x}
          y={y}
          width={width}
          height={height}
          rx={rx}
          ry={rx}
          fill={hasChildren ? palette.body_fill : colors.fill}
          stroke={stroke}
          strokeWidth={strokeWidth}
          strokeDasharray={shape === 'DashedRect' ? '5 3' : undefined}
        />
      )}
      {failed && shape !== 'NoteRect' && (
        // Fail fill tint (brief §4: the ONLY state that may tint the fill) —
        // translucent so the family header/glyph still read underneath.
        <rect x={x} y={y} width={width} height={height} rx={rx} ry={rx} fill={palette.verdict.fail} opacity={0.14} style={{ pointerEvents: 'none' }} />
      )}
      {chrome && showHeaderSplit && colors.header && (
        <path d={headerBandPath(x, y, width, headerHeight, rx)} fill={colors.header} stroke="none" />
      )}
      {/* Header row: a mini kind-glyph at the top-left corner (crib §2 shared
          glyph vocabulary), the «stereotype» keyword, and — when the element is
          live this tick — a pulsing accent dot at the top-right (crib §3). The
          glyph + stereotype are chrome (dropped at glyph LOD); the live dot is
          the "one worst-state dot" that survives every band. */}
      {chrome && (
        <HeaderKindGlyph visualKind={node.visual_kind} cx={x + 9} cy={y + 9} color={colors.stroke} />
      )}
      {chrome && node.stereotype && (
        <text x={x + width / 2} y={y + 13} fontSize={9} textAnchor="middle" fill={palette.muted}>
          {node.stereotype}
        </text>
      )}
      {chrome && overlay && isActive(overlay) && (
        // Live pulse dot (crib §3). Shifts left when the NE diagnostic badge
        // occupies its corner (highest severity wins the badge slot).
        <circle cx={x + width - (hasBadge ? 21 : 9)} cy={y + 9} r={3} fill={palette.sim.active}>
          <animate attributeName="opacity" values="1;0.35;1" dur="1.2s" repeatCount="indefinite" />
        </circle>
      )}
      {chrome && node.name && (
        <text
          x={x + width / 2}
          y={y + (node.stereotype ? 26 : 16)}
          fontSize={typography.label_font_size_px}
          fontWeight={600}
          textAnchor="middle"
          fill={palette.text}
        >
          {node.name}
        </text>
      )}
      {/* Compartment rows are chrome, not detail (brief §4: reduced drops only
          the value strip + meta) — the layout reserves their space, so hiding
          them at reduced rendered every box as a tall EMPTY body (D-L6). */}
      {chrome && lines.map((l, i) => {
        const glyph = SOURCE_GLYPH[l.source] ?? '';
        return (
          <text
            key={`${l.elementId}-${i}`}
            data-element-id={l.elementId}
            x={x + 8}
            y={y + headerHeight + typography.compartment_font_size_px + 1 + i * typography.compartment_line_stride_px}
            fontSize={typography.compartment_font_size_px}
            fill={palette.compartment_text}
          >
            {glyph}
            {l.text}
          </text>
        );
      })}
      {chrome && <SimDecor overlay={overlay} palette={palette} x={x} y={y} width={width} height={height} showValue={detail} />}
      {chrome && toggle && (
        <g onClick={toggle.onToggle} onPointerDown={(e) => e.stopPropagation()} style={{ cursor: 'pointer' }}>
          <rect x={x + width - 16} y={y + 4} width={12} height={12} rx={2} fill={palette.body_fill} stroke={colors.stroke} strokeWidth={1} />
          <text x={x + width - 10} y={y + 13.5} fontSize={12} fontWeight={700} textAnchor="middle" fill={palette.text}>
            {toggle.expanded ? '−' : '+'}
          </text>
        </g>
      )}
    </>
  );
}

/** Non-card node: a standalone glyph (ellipse, diamond, fork bar, control
 *  circles, send/accept pentagons) sized to its layout box. The name renders
 *  inside soft shapes (ellipse/pentagon) and below hard control glyphs. */
function GlyphNode({
  pn,
  palette,
  categories,
  shape,
  selected,
  hovered,
  lod,
  overlay,
  typography,
}: {
  pn: PlacedNode;
  palette: PaletteT;
  categories: Record<string, string>;
  shape: ShapeName;
  selected: boolean;
  hovered: boolean;
  lod: Lod;
  overlay: ElementOverlay | null;
  typography: NonNullable<ViewModel['tokens']>['typography'];
}) {
  const { node, x, y, width, height } = pn;
  const colors = colorsForVisualKind(palette, categories, node.visual_kind);
  const cx = x + width / 2;
  const cy = y + height / 2;
  const r = Math.min(width, height) / 2;
  // Selection = ink (brief §4 rule 2); hover = a family-stroke weight bump.
  const sel = selected ? palette.text : null;
  const stroke = sel ?? colors.stroke;
  const sw = selected ? 1.75 : hovered ? 2 : 1.25;
  // Soft shapes carry their name inside; hard control glyphs label below.
  const nameInside = shape === 'Ellipse' || shape === 'Pentagon' || shape === 'HourglassPentagon';

  let body: ReactNode;
  switch (shape) {
    case 'Ellipse':
      body = <ellipse cx={cx} cy={cy} rx={width / 2} ry={height / 2} fill={colors.fill} stroke={stroke} strokeWidth={sw} />;
      break;
    case 'Diamond':
      body = (
        <polygon
          points={`${cx},${y} ${x + width},${cy} ${cx},${y + height} ${x},${cy}`}
          fill={colors.fill}
          stroke={stroke}
          strokeWidth={sw}
        />
      );
      break;
    case 'HBar':
      body = <rect x={x} y={cy - height / 2} width={width} height={height} rx={2} fill={sel ?? palette.control_fill} stroke={stroke} strokeWidth={selected ? 2 : 0} />;
      break;
    case 'FilledCircle':
      body = <circle cx={cx} cy={cy} r={r} fill={sel ?? palette.control_fill} stroke={stroke} strokeWidth={selected ? 2 : 0} />;
      break;
    case 'BullseyeCircle':
      body = (
        <>
          <circle cx={cx} cy={cy} r={r} fill={palette.body_fill} stroke={sel ?? palette.control_stroke} strokeWidth={1.5} />
          <circle cx={cx} cy={cy} r={r * 0.5} fill={sel ?? palette.control_fill} />
        </>
      );
      break;
    case 'CrossCircle':
      body = (
        <>
          <circle cx={cx} cy={cy} r={r} fill={palette.body_fill} stroke={stroke} strokeWidth={1.5} />
          <path d={`M${cx - r * 0.5},${cy - r * 0.5} L${cx + r * 0.5},${cy + r * 0.5} M${cx + r * 0.5},${cy - r * 0.5} L${cx - r * 0.5},${cy + r * 0.5}`} stroke={sel ?? palette.control_stroke} strokeWidth={1.5} />
        </>
      );
      break;
    case 'Pentagon': // send action — points right
      body = (
        <polygon
          points={`${x},${y} ${x + width - height / 2},${y} ${x + width},${cy} ${x + width - height / 2},${y + height} ${x},${y + height}`}
          fill={colors.fill}
          stroke={stroke}
          strokeWidth={sw}
        />
      );
      break;
    case 'HourglassPentagon': // accept action — notched left
      body = (
        <polygon
          points={`${x},${y} ${x + width},${y} ${x + width},${y + height} ${x},${y + height} ${x + height / 2},${cy}`}
          fill={colors.fill}
          stroke={stroke}
          strokeWidth={sw}
        />
      );
      break;
    default:
      body = <rect x={x} y={y} width={width} height={height} fill={colors.fill} stroke={stroke} strokeWidth={sw} />;
  }

  return (
    <>
      {body}
      {lod !== 'glyph' && node.name && nameInside && (
        <text x={cx} y={cy + 4} fontSize={typography.compartment_font_size_px} textAnchor="middle" fill={palette.text}>
          {node.name}
        </text>
      )}
      {lod !== 'glyph' && node.name && !nameInside && (
        <text x={cx} y={y + height + 11} fontSize={10} textAnchor="middle" fill={palette.muted}>
          {node.name}
        </text>
      )}
      {lod !== 'glyph' && (
        <SimDecor overlay={overlay} palette={palette} x={x} y={y} width={width} height={height} showValue={lod === 'full'} />
      )}
    </>
  );
}

/** Note (comment) outline: rectangle with a folded top-right corner. */
function noteRectPath(x: number, y: number, w: number, h: number): string {
  const fold = Math.min(12, w / 4, h / 2);
  return `M${x},${y} H${x + w - fold} L${x + w},${y + fold} V${y + h} H${x} Z M${x + w - fold},${y} V${y + fold} H${x + w}`;
}

/** Header band: top of the card, with the top corners rounded to match `rx`. */
function headerBandPath(x: number, y: number, w: number, h: number, rx: number): string {
  if (rx <= 0) {
    return `M${x},${y} h${w} v${h} h${-w} z`;
  }
  return `M${x},${y + rx} a${rx},${rx} 0 0 1 ${rx},${-rx} h${w - 2 * rx} a${rx},${rx} 0 0 1 ${rx},${rx} v${h - rx} h${-w} z`;
}

/** Absolute glyph center of a resolved port (with its owning node's drag delta),
 *  or null when the edge's endpoint isn't a placed port. */
function portCenterFor(
  placedById: Map<string, PlacedNode>,
  deltaFor: (id: string) => { dx: number; dy: number },
  nodeId: string,
  portId: string | null,
): { x: number; y: number } | null {
  if (!portId) return null;
  const pn = placedById.get(nodeId);
  const pp = pn?.ports.find((p) => p.port.element_id === portId);
  if (!pp) return null;
  const d = deltaFor(nodeId);
  return { x: pp.x + d.dx + pp.width / 2, y: pp.y + d.dy + pp.height / 2 };
}

/** Re-route an edge whose endpoint node(s) have a drag delta, keeping it
 *  orthogonal. Untouched edges keep their elkjs route by reference (layout
 *  already snapped their ends to port centers). When both endpoints move by the
 *  SAME delta (a container drag shifting both ends), the elkjs route is rigidly
 *  translated — only a relative move re-elbows. After ANY re-route the endpoints
 *  are re-snapped onto their port glyph centers (§2), since the elbow attaches
 *  to node faces, not ports. */
export function adjustedEdgePoints(
  pe: LayoutResult['edges'][number],
  placedById: Map<string, PlacedNode>,
  deltaFor: (id: string) => { dx: number; dy: number },
): { x: number; y: number }[] {
  const sd = deltaFor(pe.edge.source_id);
  const td = deltaFor(pe.edge.target_id);
  if (!sd.dx && !sd.dy && !td.dx && !td.dy) return pe.points;
  let pts: { x: number; y: number }[];
  if (sd.dx === td.dx && sd.dy === td.dy) {
    // Rigid translation: keep the orthogonal route, shift it (ports move with it
    // by the same delta, so the layout-time snap still holds — no re-snap).
    return pe.points.map((p) => ({ x: p.x + sd.dx, y: p.y + sd.dy }));
  }
  const s = placedById.get(pe.edge.source_id);
  const t = placedById.get(pe.edge.target_id);
  if (!s || !t) return pe.points;
  pts = orthogonalElbow(
    { x: s.x + sd.dx, y: s.y + sd.dy, width: s.width, height: s.height },
    { x: t.x + td.dx, y: t.y + td.dy, width: t.width, height: t.height },
  );
  // Re-anchor ported endpoints onto the port glyph centers (§2), re-squaring
  // the adjacent bend so the route into the port stays orthogonal.
  const sc = portCenterFor(placedById, deltaFor, pe.edge.source_id, pe.edge.source_port_id);
  const tc = portCenterFor(placedById, deltaFor, pe.edge.target_id, pe.edge.target_port_id);
  if (sc && pts.length >= 2) {
    if (Math.abs(pts[1].x - pts[0].x) <= Math.abs(pts[1].y - pts[0].y)) pts[1] = { ...pts[1], x: sc.x };
    else pts[1] = { ...pts[1], y: sc.y };
    pts[0] = sc;
  }
  if (tc && pts.length >= 2) {
    const n = pts.length - 1;
    if (Math.abs(pts[n - 1].x - pts[n].x) <= Math.abs(pts[n - 1].y - pts[n].y)) pts[n - 1] = { ...pts[n - 1], x: tc.x };
    else pts[n - 1] = { ...pts[n - 1], y: tc.y };
    pts[n] = tc;
  }
  return pts;
}

function Edge({
  pe,
  points,
  labels,
  lod,
  edgeStyles,
  muted,
  labelBg,
  textColor,
}: {
  pe: LayoutResult['edges'][number];
  points: { x: number; y: number }[];
  /** FE-resolved, de-conflicted label placements (brief §1). */
  labels: ResolvedLabel[];
  lod: Lod;
  edgeStyles: Record<string, EdgeStyleToken>;
  muted: string;
  labelBg: string;
  textColor: string;
}) {
  if (points.length < 2) return null;
  const d = points.map((p, i) => `${i === 0 ? 'M' : 'L'}${p.x},${p.y}`).join(' ');
  // Markers/dash come from the edge kind + emitted styles; label TEXT +
  // PLACEMENT come from the resolver (seeded from elk's inline-label geometry).
  const decor = edgeDecor(pe.edge, edgeStyles);
  const full = lod === 'full';
  return (
    <g>
      <path
        d={d}
        fill="none"
        stroke={muted}
        strokeWidth={1.25}
        strokeDasharray={decor.dash}
        markerStart={decor.markerStart}
        markerEnd={decor.markerEnd}
      />
      {labels.map((l, i) => (
        <EdgeLabel
          key={i}
          label={l}
          full={full}
          bg={labelBg}
          color={l.kind === 'secondary' ? muted : textColor}
        />
      ))}
    </g>
  );
}

/** One resolved edge-label chip: a `edge_label_bg`-backed box (the token carries
 *  its own alpha) with the wrapped text, elided (≤12ch + …) at reduced/glyph LOD
 *  while the reserved box keeps full size so toggling LOD never reflows (§1). */
function EdgeLabel({
  label,
  full,
  bg,
  color,
}: {
  label: ResolvedLabel;
  full: boolean;
  bg: string;
  color: string;
}) {
  const { rect, lines, text } = label;
  const display = full ? lines : [elideForLod(text, false)];
  // Full text in a tooltip whenever the drawn text isn't the whole label
  // (elided at reduced LOD, or a degraded placement kept its bg — §1 step 4).
  const showTitle = !full || label.degraded;
  return (
    <g>
      {showTitle && <title>{text}</title>}
      <rect x={rect.x} y={rect.y} width={rect.width} height={rect.height} fill={bg} />
      {display.map((line, i) => (
        <text
          key={i}
          x={rect.x + rect.width / 2}
          y={rect.y + (i + 1) * 12 - 2}
          fontSize={10}
          textAnchor="middle"
          fill={color}
        >
          {line}
        </text>
      ))}
    </g>
  );
}

