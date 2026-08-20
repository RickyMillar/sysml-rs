/**
 * SessionTreeV2 — Run page Zone 1 panel, rooted in the model's part
 * hierarchy (Phase B integration).
 *
 * Replaces the flat TopologyView + right-hand VariablesPane with one
 * richer tree: parts nest, attributes show live values, state machines
 * show their current state, constraints show pass/fail dots, ODEs show
 * integrator status.
 *
 * State & persistence:
 *  - focusPath wired through the A2 session store; clicking any row
 *    sets `focusPath` to the root→target chain so the Breadcrumb above
 *    Zone 2 stays in lock-step.
 *  - expandedSet + filterMode persisted to localStorage keyed on
 *    workspaceRoot. First-arrival seeds `expandedSet` via a depth-2
 *    walk so the user sees 3 levels without clicking chevrons; a
 *    persisted state pre-empts that seed so scroll context survives
 *    reloads.
 *  - Pin state is in-memory only, keyed by node id. New keyspace
 *    distinct from the name-keyed useVariablesState (which the
 *    soon-to-be-deleted VariablesPane owned).
 *  - Per-SM inject menu (B4) + inline attribute editor (B5) + hover-
 *    sparklines (B6) are follow-ups — the rows already expose the
 *    callbacks, so adding them is pure wiring.
 *
 * `variant` prop (ninebar screenshot-comparison ruling B, 2026-07-14 —
 * "Rail tree gets a 'calm' treatment"): `variant="rail"` restyles the
 * tree for the ninebar left rail WITHOUT forking — same tree hook, same
 * row components, same selection/pin/filter state and handlers. Only
 * ever passed by `RunWorkflow.tsx`'s ninebar branch; every other call
 * site (including `RunWorkflow`'s own legacy branch) omits it and gets
 * the unchanged `variant="default"` rendering. Four differences, all
 * gated on `variant === 'rail'`:
 *   1. Header: quiet "topology n/n" row instead of the labelled
 *      counts+expand/collapse header.
 *   2. Filters + search collapse behind one icon button that opens a
 *      `Popover` hosting the SAME `FilterBar` instance (reused, not
 *      rebuilt) — no always-visible chip row / search box.
 *   3. [DEFERRED] Per-row pin / "add to Plots" sparkline hover-reveal.
 *      A CSS-only hide-until-row-hover was scoped but NOT shipped:
 *      getting the row-hover selector wrong would permanently hide the
 *      pin control (a real regression — no way to pin), which is worse
 *      than the current always-visible buttons. The rail container
 *      carries a `data-nb-rail-tree` marker so a future, verified CSS
 *      rule (targeting the `*-pin` / `*-spark-btn` testids under a
 *      confirmed hovered-row selector) can add this without touching
 *      `AttributeRow`. Pins remain always-visible in the rail for now.
 *   4. The bottom DETAIL split-pane (`Splitter` + `DetailPanel`) is not
 *      rendered at all. Row selection instead opens the right rail's
 *      `inspector` context (`useRightRailStore.open('inspector')`) —
 *      the rail's home for "what's selected" now — and feeds it via
 *      the same `useSelectionStore.select()` `DiagnosticsPanel` /
 *      `ReadinessChip` already use, so `InspectorRailContext` renders
 *      the newly-selected element without any new plumbing.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { useSessionStore } from '../store';
import { openOverridePopover } from '../OverridePopover';
import { useSessionModelTree } from './useSessionModelTree';
import {
  findPathToNode,
  walkModelTree,
} from './buildModelTree';
import {
  filterTree,
  type DefinitionMode,
  type TreeFilterMode,
} from './filterTree';
import { splitAttributesByActivity } from './splitAttrs';
import { DetailPanel } from './detail/DetailPanel';
import { promoteToPlots } from './detail/promoteToPlots';
import { Splitter, clampSplitPosition } from './Splitter';
import { ModelTreeView } from './ModelTreeView';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { usePlotSelectionStore } from '@/features/results/usePlotSelectionStore';
import { VariablesPaneContextMenu } from '@/features/variables/VariablesPaneContextMenu';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceStore } from '@/store/workspace';
import { WORKSPACE_URI } from '@/shared/api/model';
import { useSelectionStore } from '@/features/selection/store';
import { useRightRailStore } from '@/app/rail/railStore';
import { Popover } from '@/shared/overlays/Popover';
import type { ModelTreeNode, SmTreeNode, ConstraintTreeNode } from './types';
import type { AttributeTreeNode } from './types';
import { SmContextMenu } from './contextmenu/SmContextMenu';
import { ConstraintContextMenu } from './contextmenu/ConstraintContextMenu';
import { findDeclaredViewForElement } from './preferredView';
import { useViewsList } from '@/features/views/queries';

export type SessionTreeVariant = 'default' | 'rail';

// Default auto-expand depth: 0 = roots only, 1 = roots + their direct
// children, 2 = three visible levels. espresso-production-cell has parts 3
// levels deep (ProductionCell → Station1 → GroupHead); the plan's target is
// "3 levels visible by default without chevron clicks".
const DEFAULT_EXPAND_DEPTH = 2;

// Default detail-pane height in pixels (tree + splitter + detail).
// 240 gives ~3 rows of detail content without stealing too much tree.
const DEFAULT_DETAIL_HEIGHT_PX = 240;
const MIN_DETAIL_HEIGHT_PX = 100;
// "Hidden" sentinel so the user can collapse the detail pane to 0.
const COLLAPSE_THRESHOLD_PX = 24;

// localStorage persistence. Keyed per workspace root so different
// projects don't collide. Tree state reset on workspace change.
const STORAGE_PREFIX = 'sysml.tree.v2.';

/**
 * Workspaces whose depth-2 default expansion has already been applied
 * since the page loaded. Module-scoped so HMR remounts / React
 * StrictMode double-mounts don't wipe the "seeded" flag — a remount
 * with an empty `expandedSet` (because the user just clicked
 * "Collapse all") would otherwise trip the one-shot seed effect and
 * re-expand the tree against the user's wishes.
 */
const seededWorkspaces = new Set<string>();

interface PersistedState {
  expanded: string[];
  filterMode: TreeFilterMode;
  definitionMode?: DefinitionMode;
  detailHeightPx?: number;
  /** Group root-level elements by their source package (file). When
   *  omitted / false we merge every file's contents into one flat
   *  sorted list. */
  groupByPackage?: boolean;
}

export function storageKeyFor(workspaceRoot: string | null): string | null {
  if (!workspaceRoot) return null;
  return `${STORAGE_PREFIX}${workspaceRoot}`;
}

export function loadPersistedState(workspaceRoot: string | null): PersistedState | null {
  const key = storageKeyFor(workspaceRoot);
  if (!key || typeof window === 'undefined') return null;
  try {
    const raw = window.localStorage.getItem(key);
    if (!raw) return null;
    const parsed = JSON.parse(raw) as Partial<PersistedState>;
    if (!parsed || !Array.isArray(parsed.expanded)) return null;
    const filterMode: TreeFilterMode =
      parsed.filterMode === 'all' ||
      parsed.filterMode === 'live' ||
      parsed.filterMode === 'pinned'
        ? parsed.filterMode
        : 'all';
    const definitionMode: DefinitionMode =
      parsed.definitionMode === 'usages' ||
      parsed.definitionMode === 'definitions' ||
      parsed.definitionMode === 'both'
        ? parsed.definitionMode
        : 'usages';
    const detailHeightPx =
      typeof parsed.detailHeightPx === 'number' &&
      Number.isFinite(parsed.detailHeightPx)
        ? parsed.detailHeightPx
        : undefined;
    // Absent (pre-existing state, or never toggled) → default ON, so
    // the package + ownership view is what everyone gets by default; an
    // explicit stored `false` (user chose flat) is respected.
    const groupByPackage = parsed.groupByPackage ?? true;
    return {
      expanded: parsed.expanded,
      filterMode,
      definitionMode,
      detailHeightPx,
      groupByPackage,
    };
  } catch {
    return null;
  }
}

export function savePersistedState(
  workspaceRoot: string | null,
  state: PersistedState,
): void {
  const key = storageKeyFor(workspaceRoot);
  if (!key || typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(key, JSON.stringify(state));
  } catch {
    /* quota exceeded or private mode — silently skip */
  }
}

/** Seed the expanded set with every node up to `maxDepth` (0-indexed).
 *
 * Commit 2 of the model-tree rework: respects the backend's
 * `defaultCollapsed` hint by skipping the auto-expand for any node
 * the backend has flagged. Set server-side for Port / Connection
 * archetypes whose typed-def inlining produces heavy fan-out — the
 * user can still expand them with one click. The recursion still
 * descends into children of a default-collapsed node so deeper
 * expandable mounts get seeded if the user later opens the parent;
 * those ids end up in the set but the parent's ancestor click is
 * what reveals them.
 */
export function seedExpandedToDepth(
  tree: readonly ModelTreeNode[],
  maxDepth: number,
): Set<string> {
  const out = new Set<string>();
  const walk = (nodes: readonly ModelTreeNode[], depth: number) => {
    for (const n of nodes) {
      if (
        n.children.length > 0 &&
        depth <= maxDepth &&
        !n.defaultCollapsed
      ) {
        out.add(n.id);
      }
      if (depth < maxDepth) walk(n.children, depth + 1);
    }
  };
  walk(tree, 0);
  return out;
}

/** Every id in the tree that has children — used by "Expand all". */
export function collectAllExpandableIds(
  tree: readonly ModelTreeNode[],
): Set<string> {
  const out = new Set<string>();
  walkModelTree(tree, (n) => {
    if (n.children.length > 0) out.add(n.id);
  });
  return out;
}

export function SessionTreeV2({ variant = 'default' }: { variant?: SessionTreeVariant } = {}) {
  // ninebar rail calm-treatment (ruling B). Everything below is gated on
  // this — the default (legacy) rendering is byte-for-byte unchanged, so
  // no existing test / call site is affected.
  const rail = variant === 'rail';
  // Filter popover (rail only): the filter chips + search collapse behind
  // one icon in the header instead of an always-on bar.
  const [railFilterOpen, setRailFilterOpen] = useState(false);
  const railFilterBtnRef = useRef<HTMLButtonElement>(null);

  // Grouping toggle — initialised from persisted state on workspace
  // hydration below. Read first so the tree hook receives it in the
  // same render as the filter-bar UI.
  // Package + ownership is the default organisation (SysML models are
  // namespace + containment trees); the "Pkgs" chip toggles it off to
  // the flat archetype view.
  const [groupByPackage, setGroupByPackage] = useState<boolean>(true);
  // Read before the tree hook so we can pass it the session we expect
  // to be live (reset-path guard — UX closeout #4 / #17).
  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const { tree, isLoading, currentTick } = useSessionModelTree({
    groupByPackage,
    expectedSessionId: activeSessionId,
  });
  const focusPath = useSessionStore((s) => s.focusPath);
  const setFocusPath = useSessionStore((s) => s.setFocusPath);
  const navigate = useNavigate();
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const setActiveSessionTarget = useWorkspaceUIStore((s) => s.setActiveSessionTarget);

  // Live list of declared views across the merged workspace, used by
  // the click handler below to jump the diagram to a view that
  // exposes the clicked element. Bucket 5.R2 — the legacy
  // kind-based heuristic was deleted; declared views are the only
  // diagram source.
  //
  // Bucket 5-followup-2 (2026-05-05): the per-row "views (N)" chip pulls
  // the workspace-wide view list. Cross-file expose targets are part of
  // the model, so a per-file scope would hide views authored elsewhere.
  const focusedUri = useWorkspaceStore((s) => s.focusedUri);
  const viewsList = useViewsList(WORKSPACE_URI);
  const selectedViewId = useWorkspaceStore((s) => s.selectedViewId);

  // Map ElementId → views that DIRECTLY expose it. Built once per
  // sysml.query view-list refresh so per-row affordance lookups are O(1).
  //
  // We map only the direct `expose` target, NOT its descendants. An
  // earlier revision (Bucket 5-followup-3) propagated each view onto
  // every descendant of its expose target so `expose CoffeeMachine`
  // would also light up `brewer`, `waterTank`, … — but rendering the
  // full inline view row on every descendant means the same view
  // repeats once per expanded descendant (e.g. `coreModelBrowser`
  // showing 3× under StationPhysicsModel + ReadyLatch + a derivative).
  // The view belongs to the element it names; showing it once, on that
  // element, is the honest reading. (If descendant coverage is ever
  // wanted it should be a subtle indicator, not a repeated row.)
  const viewsByElementId = useMemo(() => {
    type ViewSummary = { id: string; name: string | null; kind: string };
    const byElementId = new Map<string, ViewSummary[]>();
    for (const v of viewsList.data ?? []) {
      for (const e of v.exposed) {
        const targetId = e.exposed_element_id;
        if (!targetId) continue;
        const list = byElementId.get(targetId);
        const summary: ViewSummary = { id: v.id, name: v.name, kind: v.kind };
        // A view can expose the same element more than once (multiple
        // Expose memberships) — dedupe by view id so the row is unique.
        if (list) {
          if (!list.some((existing) => existing.id === v.id)) list.push(summary);
        } else {
          byElementId.set(targetId, [summary]);
        }
      }
    }
    return byElementId;
  }, [viewsList.data]);

  const handlePickView = useCallback((viewId: string) => {
    useWorkspaceStore.getState().setSelectedViewId(viewId);
  }, []);

  // Subscribe to time-series updates for the sparkline column. The
  // revision bump is what tells React to re-render — the actual read
  // happens in a memoised lookup below so we don't rebuild the map
  // every render.
  const timeSeriesRevision = useTimeSeriesStore((s) => s.revision);

  // Local expand state — persisted to localStorage keyed on workspaceRoot.
  const [expandedSet, setExpandedSet] = useState<Set<string>>(() => new Set());
  // Local pin state — keyed by node id. New keyspace (see header note).
  const [pinnedIds, setPinnedIds] = useState<Set<string>>(() => new Set());
  // Detail-pane height in px. The parent layout owns clamping —
  // Splitter fires raw proposals, we clamp against the container
  // bounds before committing. Persisted to localStorage alongside
  // the rest of the tree UI state.
  const [detailHeightPx, setDetailHeightPx] = useState<number>(
    DEFAULT_DETAIL_HEIGHT_PX,
  );
  // Container ref drives clamping: we cap detail height at 60% of
  // the Zone-1 container so the tree can't get squished to zero.
  const containerRef = useRef<HTMLDivElement | null>(null);
  // Filter chip + search text. Default LIVE when a session is running
  // so the user is not drowned in attributes that the backend hasn't
  // observed yet; default ALL when idle because LIVE would be empty.
  const [filterMode, setFilterMode] = useState<TreeFilterMode>('all');
  const [definitionMode, setDefinitionMode] =
    useState<DefinitionMode>('usages');
  const [searchQuery, setSearchQuery] = useState('');
  // Flip default mode on session start/stop — the user can still
  // override and the choice sticks for the rest of the session. We
  // also treat a loaded-from-localStorage filterMode as "already
  // chosen" so the auto-switch doesn't stomp a persisted preference.
  const [autoModeSwitched, setAutoModeSwitched] = useState(false);
  useEffect(() => {
    if (!autoModeSwitched && activeSessionId) {
      setFilterMode('live');
      setAutoModeSwitched(true);
    }
    if (!activeSessionId) setAutoModeSwitched(false);
  }, [activeSessionId, autoModeSwitched]);
  // Right-click context menu state — discriminated by node kind so
  // we can render kind-specific menus from the same dispatch path.
  // `null` hides every menu variant.
  type ContextMenuState =
    | {
        kind: 'variable';
        variableName: string;
        isPinned: boolean;
        isInChart: boolean;
        position: { x: number; y: number };
      }
    | {
        kind: 'sm';
        node: SmTreeNode;
        position: { x: number; y: number };
      }
    | {
        kind: 'constraint';
        node: ConstraintTreeNode;
        position: { x: number; y: number };
      };
  const [contextMenu, setContextMenu] = useState<ContextMenuState | null>(null);

  // ── localStorage hydration ─────────────────────────────────────
  // One-shot load per workspaceRoot change. A persisted state
  // pre-empts the depth-2 seed below (so user scroll context
  // survives a reload) and marks autoModeSwitched so the
  // session-start handler doesn't clobber the persisted filter
  // mode.
  const [hydratedRoot, setHydratedRoot] = useState<string | null>(null);
  useEffect(() => {
    if (workspaceRoot === hydratedRoot) return;
    setHydratedRoot(workspaceRoot);
    const persisted = loadPersistedState(workspaceRoot);
    if (persisted) {
      setExpandedSet(new Set(persisted.expanded));
      setFilterMode(persisted.filterMode);
      setDefinitionMode(persisted.definitionMode ?? 'usages');
      setGroupByPackage(persisted.groupByPackage ?? false);
      if (typeof persisted.detailHeightPx === 'number') {
        setDetailHeightPx(persisted.detailHeightPx);
      }
      setAutoModeSwitched(true);
      // Mark seeded even if `persisted.expanded` is the empty array —
      // an empty persisted set means the user explicitly collapsed
      // everything, not "fresh workspace". Re-seeding would undo
      // that choice on reload.
      if (workspaceRoot) seededWorkspaces.add(workspaceRoot);
    } else {
      // Reset to defaults when we hop workspaces (or workspaceRoot
      // clears) so leftover state from the previous workspace
      // doesn't show up on the new tree.
      setExpandedSet(new Set());
      setFilterMode('all');
      setDefinitionMode('usages');
      setGroupByPackage(true);
      setDetailHeightPx(DEFAULT_DETAIL_HEIGHT_PX);
      setAutoModeSwitched(false);
    }
  }, [workspaceRoot, hydratedRoot]);

  // ── localStorage write-back ────────────────────────────────────
  // Eager write on state change. React batches updates; a tight
  // expand/collapse loop results in at most one write per commit.
  useEffect(() => {
    if (!workspaceRoot || workspaceRoot !== hydratedRoot) return;
    savePersistedState(workspaceRoot, {
      expanded: Array.from(expandedSet),
      filterMode,
      definitionMode,
      detailHeightPx,
      groupByPackage,
    });
  }, [
    workspaceRoot,
    hydratedRoot,
    expandedSet,
    filterMode,
    definitionMode,
    detailHeightPx,
    groupByPackage,
  ]);

  const handleToggleExpand = useCallback((id: string) => {
    setExpandedSet((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  // "Expand all" opens every node that has children — structural
  // escape hatch for the user who wants the whole model visible.
  const handleExpandAll = useCallback(() => {
    setExpandedSet(collectAllExpandableIds(tree));
  }, [tree]);

  // "Collapse all" resets to the empty set. The user can then
  // re-expand individually; depth-2 seed won't re-fire (guarded
  // behind `prev.size === 0`, but the seed has already run once so
  // no further seeding happens without a workspace change).
  const handleCollapseAll = useCallback(() => {
    setExpandedSet(new Set());
  }, []);

  // Splitter drag handler. Clamps against the live container height
  // so the tree can't vanish. Values below COLLAPSE_THRESHOLD_PX
  // snap to 0 so users can fully hide the detail pane by dragging
  // down hard.
  const handleSplitChange = useCallback((proposal: number) => {
    const containerHeight = containerRef.current?.clientHeight ?? 0;
    if (proposal < COLLAPSE_THRESHOLD_PX) {
      setDetailHeightPx(0);
      return;
    }
    setDetailHeightPx(
      clampSplitPosition(proposal, containerHeight, {
        minPx: MIN_DETAIL_HEIGHT_PX,
      }),
    );
  }, []);

  /**
   * Phase 3 — when the user clicks the source-preview popover on a
   * tree row (not the row itself), push selection + focused URI and
   * pop the Source utility drawer. The row's primary click action is
   * untouched.
   */
  const handlePromotePreview = useCallback((node: ModelTreeNode) => {
    const ws = useWorkspaceStore.getState();
    if (node.uri && ws.focusedUri !== node.uri) {
      ws.setFocusedUri(node.uri);
    }
    useSelectionStore.getState().select(node.uri ?? null, node.elementId ?? null);
    useWorkspaceUIStore.getState().setActiveUtility('source');
  }, []);

  const handleSelect = useCallback(
    (node: ModelTreeNode) => {
      const chain = findPathToNode(tree, node.id);
      if (!chain) return;
      setFocusPath(chain.map((n) => n.id));
      // Auto-expand ancestors when the user clicks a deep node via
      // search (for keyboard / future-search use). Today this is a
      // no-op for clicks since the user already expanded them.
      setExpandedSet((prev) => {
        const next = new Set(prev);
        for (const ancestor of chain.slice(0, -1)) next.add(ancestor.id);
        return next;
      });

      // Phase 4 — Navigation UX: a click in the model tree should pull
      // the diagram pane to a view that exposes the same subject.
      // Bucket 5.R2: search the live `sysml.query view-list` for a declared view
      // whose `Expose` chain reaches the clicked ElementId. If one
      // exists, select it (the URL sync mirrors via ?view_id=). If
      // none, the diagram pane stays on whatever the user had
      // selected — the empty state plus a "Create view for this
      // element" affordance (Bucket 5.A2) is the right answer; the
      // anti-pattern is silently inventing a default render.
      const ws = useWorkspaceStore.getState();
      if (node.uri && ws.focusedUri !== node.uri) {
        void ws.focusFile(node.uri);
      }
      const viewsData = viewsList.data ?? [];
      // If the clicked node IS a declared view, render it directly — same
      // behaviour as clicking it in the Views drawer (the two surfaces used to
      // diverge: the tree only ever selected-as-element, so a view row showed
      // its detail but never rendered). Otherwise fall back to a view that
      // EXPOSES the clicked element.
      const clickedView = node.elementId
        ? viewsData.find((v) => v.id === node.elementId)
        : undefined;
      if (clickedView) {
        ws.setSelectedViewId(clickedView.id);
      } else {
        const declared = findDeclaredViewForElement(node.elementId ?? null, viewsData);
        if (declared) {
          ws.setSelectedViewId(declared.id);
        }
      }

      // Ruling B (rail): the DETAIL split-pane is gone; selection opens
      // the right rail's `inspector` context instead — its new home for
      // "what's selected". Feed it the same `useSelectionStore.select()`
      // the inspector context already reads (identical to how the source
      // slide-out / diagnostics select), then reveal the rail.
      if (rail) {
        useSelectionStore.getState().select(node.uri ?? null, node.elementId ?? null);
        useRightRailStore.getState().open('inspector');
      }
    },
    [tree, setFocusPath, viewsList.data, rail],
  );

  const handleTogglePin = useCallback((node: ModelTreeNode) => {
    setPinnedIds((prev) => {
      const next = new Set(prev);
      if (next.has(node.id)) {
        next.delete(node.id);
      } else {
        next.add(node.id);
        // Auto-promote to the Plots tab when a variable is pinned.
        if ((node.kind === 'attribute' || node.kind === 'calc') && activeSessionId) {
          promoteToPlots(attributeVarName(node as AttributeTreeNode), activeSessionId);
        }
      }
      return next;
    });
  }, [activeSessionId]);

  // Build a per-attribute sparkline lookup from the time-series ring
  // buffer. Keyed by attribute *name* (bare leaf — how the backend
  // emits values) with a fallback to the fully-qualified ownerPath.name.
  // Re-memoised when the time-series revision changes so samples
  // stay fresh across ticks.
  const getSparklineSamples = useCallback(
    (node: ModelTreeNode): readonly number[] => {
      if (node.kind !== 'attribute') return EMPTY_SAMPLES;
      const map = useTimeSeriesStore.getState().getTimeSeries();
      const fullPath = node.ownerPath
        ? `${node.ownerPath}.${node.name}`
        : node.name;
      const points = map[fullPath] ?? map[node.name];
      if (!points || points.length < 3) return EMPTY_SAMPLES;
      // Last 30 ticks — matches the plan's "30-tick sparkline" hint.
      const start = Math.max(0, points.length - 30);
      const out = new Array<number>(points.length - start);
      for (let i = start; i < points.length; i++) out[i - start] = points[i].v;
      return out;
    },
    // Revision is the subscription signal — the lookup itself is pure
    // so the dep array closes over it by design.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [timeSeriesRevision],
  );

  // ── Context menu wiring (Override / Breakpoint / Pin / Copy) ────

  const handleContextMenu = useCallback(
    (node: ModelTreeNode, position: { x: number; y: number }) => {
      // Value-bearing rows (attribute / calc / ODE state var) — same
      // override / breakpoint / chart / pin actions; reuses the
      // existing variable-shaped menu.
      if (node.kind === 'attribute' || node.kind === 'calc' || node.kind === 'ode') {
        const name = attributeVarName(node as AttributeTreeNode);
        const selected = activeSessionId
          ? usePlotSelectionStore.getState().getSelected(activeSessionId)
          : [];
        setContextMenu({
          kind: 'variable',
          variableName: name,
          isPinned: pinnedIds.has(node.id),
          isInChart: selected.includes(name),
          position,
        });
        return;
      }
      // SM rows — break-on-state-entry submenu + inject-event submenu.
      if (node.kind === 'sm') {
        setContextMenu({ kind: 'sm', node, position });
        return;
      }
      // Constraint rows — break-on-violation.
      if (node.kind === 'constraint') {
        setContextMenu({ kind: 'constraint', node, position });
        return;
      }
      // Parts / sections / other kinds get no menu in this phase.
    },
    [pinnedIds, activeSessionId],
  );

  const closeContextMenu = useCallback(() => setContextMenu(null), []);

  const handleOverride = useCallback(
    (name: string) => {
      if (!activeSessionId) return;
      // The session runtime resolves staged overrides by bare attribute name.
      // Tree rows use an owner-qualified series key, which otherwise reaches
      // the backend as an unknown override target (RS002).
      openOverridePopover(runtimeOverrideName(name));
    },
    [activeSessionId],
  );

  const handleAddBreakpoint = useCallback(
    async (name: string) => {
      if (!activeSessionId) return;
      // eslint-disable-next-line no-alert
      const raw = window.prompt(
        `Threshold value for ${name} (breakpoint when exceeded)`,
        '0',
      );
      if (raw == null) return;
      const threshold = Number(raw);
      if (!Number.isFinite(threshold)) return;
      try {
        const { createBreakpointClient } = await import(
          '@/engine/SessionControl'
        );
        const client = createBreakpointClient();
        await client.set(activeSessionId, {
          kind: 'threshold-crossing',
          target: name,
          variable: name,
          threshold,
          direction: 'either',
        });
      } catch (err) {
        console.warn('[SessionTreeV2] breakpoint set failed:', err);
      }
    },
    [activeSessionId],
  );

  const handleTogglePinByName = useCallback(
    (name: string) => {
      // Find the node by name (ownerPath + leaf match) and toggle.
      const attr = findAttributeByVarName(tree, name);
      if (attr) handleTogglePin(attr);
    },
    [tree, handleTogglePin],
  );

  const handleBreakOnStateEntry = useCallback(
    async (stateElementId: string, _stateName: string) => {
      if (!activeSessionId) return;
      try {
        const { createBreakpointClient } = await import('@/engine/SessionControl');
        const client = createBreakpointClient();
        await client.set(activeSessionId, {
          kind: 'state-entry',
          target: stateElementId,
        });
      } catch (err) {
        console.warn('[SessionTreeV2] state-entry breakpoint failed:', err);
      }
    },
    [activeSessionId],
  );

  const handleBreakOnViolation = useCallback(
    async (constraintElementId: string, _name: string) => {
      if (!activeSessionId) return;
      try {
        const { createBreakpointClient } = await import('@/engine/SessionControl');
        const client = createBreakpointClient();
        await client.set(activeSessionId, {
          kind: 'constraint-violation',
          target: constraintElementId,
        });
      } catch (err) {
        console.warn('[SessionTreeV2] constraint-violation breakpoint failed:', err);
      }
    },
    [activeSessionId],
  );

  const handleInjectEvent = useCallback(
    async (subsystemName: string, eventName: string) => {
      if (!activeSessionId) return;
      try {
        const { httpPost } = await import('@/shared/api/http');
        await httpPost('/api/command', {
          command: 'sysml.sessions.inject',
          params: {
            session_id: activeSessionId,
            subsystem: subsystemName,
            event: eventName,
          },
        });
      } catch (err) {
        console.warn('[SessionTreeV2] inject event failed:', err);
      }
    },
    [activeSessionId],
  );

  const handleToggleChart = useCallback(
    (name: string) => {
      // No-op without an active session — the chart selection store
      // is keyed by session id. Silent because this can't happen
      // through normal UI (the menu was opened from a session-scoped
      // tree row), but be safe in case of stale state.
      if (!activeSessionId) return;
      usePlotSelectionStore.getState().toggleSelected(activeSessionId, name);
    },
    [activeSessionId],
  );

  const handleCopyName = useCallback((name: string) => {
    try {
      if (typeof navigator !== 'undefined' && navigator.clipboard) {
        void navigator.clipboard.writeText(name);
      }
    } catch {
      /* clipboard API unavailable — non-browser / no permission */
    }
  }, []);

  const handleLaunchRunnable = useCallback(
    (node: ModelTreeNode) => {
      setActiveSessionTarget(node.elementId || node.id);
      switch (node.rawKind) {
        case 'AnalysisCaseUsage':
        case 'AnalysisCaseDefinition':
          navigate(`/analyze?case_id=${encodeURIComponent(node.elementId || node.id)}`);
          return;
        case 'VerificationCaseUsage':
        case 'VerificationCaseDefinition':
          navigate('/verify');
          return;
        default:
          navigate('/run');
      }
    },
    [navigate, setActiveSessionTarget],
  );

  // Apply the chip + search filter FIRST (prunes unnamed attributes,
  // narrows to live/pinned, matches search, dedups defs vs usages),
  // then split attributes into Outputs / Parameters sections, then
  // reorder pinned nodes to the top.
  const filteredTree = useMemo(
    () =>
      filterTree(tree, {
        mode: filterMode,
        pinnedIds,
        searchQuery,
        definitionMode,
      }),
    [tree, filterMode, pinnedIds, searchQuery, definitionMode],
  );

  const visibleTree = useMemo(
    () => splitAttributesByActivity(filteredTree, currentTick),
    [filteredTree, currentTick],
  );

  const orderedTree = useMemo(
    () => (pinnedIds.size === 0 ? visibleTree : reorderPinned(visibleTree, pinnedIds)),
    [visibleTree, pinnedIds],
  );

  // ── First-arrival seed (no persisted state) ───────────────────
  // Once the (post-split) tree arrives, open everything to
  // DEFAULT_EXPAND_DEPTH so users see 3 levels of structure without
  // clicking. Also open every `outputs` section, but leave
  // `parameters` sections collapsed by default.
  //
  // Crucially: runs AT MOST ONCE per workspace. `visibleTree` gets a
  // new reference every tick (it recomposes when `currentTick`
  // advances, since the "stale attribute → parameters bucket" split
  // depends on the tick). Re-seeding on every visibleTree reference
  // change would defeat "Collapse all": user empties the set →
  // visibleTree recomposes on the next tick → old guard
  // `prev.size === 0 → seed` re-fires → tree re-expands under the
  // user's hands.
  //
  // State lives in a module-level `Set` rather than `useRef` so
  // HMR-triggered remounts (or a React StrictMode double-mount)
  // don't reset the "already seeded" flag and cause a re-seed on
  // top of the user's collapsed state.
  useEffect(() => {
    if (!workspaceRoot) return;
    if (visibleTree.length === 0) return;
    if (seededWorkspaces.has(workspaceRoot)) return;
    // If a persisted state already populated the set, mark this
    // workspace seeded and skip — the user's last choice wins.
    if (expandedSet.size > 0) {
      seededWorkspaces.add(workspaceRoot);
      return;
    }
    // Package mode adds a container tier at depth 0 (the packages
    // themselves), so the same numeric depth opens one more model
    // level than the flat view and first arrival reads as a wall of
    // rows. Shave one level so both modes open ~the same number of
    // visible tiers: packages expanded, top-level model containers
    // expanded, their members visible but collapsed.
    const seedDepth = groupByPackage
      ? DEFAULT_EXPAND_DEPTH - 1
      : DEFAULT_EXPAND_DEPTH;
    const seeded = seedExpandedToDepth(visibleTree, seedDepth);
    walkModelTree(visibleTree, (n) => {
      if (n.kind === 'section' && n.sectionKind === 'outputs') {
        seeded.add(n.id);
      }
    });
    setExpandedSet(seeded);
    seededWorkspaces.add(workspaceRoot);
    // Deliberately omit expandedSet from deps — we read it once on
    // first arrival to respect hydrated state, not to re-seed on
    // every collapse.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [visibleTree, workspaceRoot]);

  const focusedId = focusPath.length > 0 ? focusPath[focusPath.length - 1] : null;

  const detailVisible = detailHeightPx > 0;

  return (
    <div
      ref={containerRef}
      /* `-panel`, not the bare `session-tree-v2`: this element is the whole
         rail panel (header + filters + tree + detail), while the id
         `session-tree-v2` means the TREE — which is `ModelTreeView`'s
         scroller below, the element the row testids hang off and every
         suite's selectors expect. Both carried the bare id until finding 16;
         the wrapper is the one that had to give it up. */
      data-testid="session-tree-v2-panel"
      data-nb-rail-tree={rail ? '' : undefined}
      className="flex flex-col h-full overflow-hidden"
      style={
        rail
          ? { background: 'transparent' }
          : {
              background: 'var(--surface-container-low)',
              borderRight: '1px solid var(--outline-variant)',
            }
      }
    >
      <Header
        rail={rail}
        pinnedCount={pinnedIds.size}
        expandedCount={expandedSet.size}
        isLoading={isLoading}
        totalNodeCount={countNodes(tree)}
        visibleNodeCount={countNodes(visibleTree)}
        onExpandAll={handleExpandAll}
        onCollapseAll={handleCollapseAll}
        filterBtnRef={railFilterBtnRef}
        filterActive={filterMode !== 'all' || searchQuery.length > 0}
        onToggleFilter={() => setRailFilterOpen((v) => !v)}
      />
      {/* Ruling B (rail): the filter chips + search collapse behind the
          header's filter icon (a Popover), instead of an always-on bar.
          Default variant keeps the inline FilterBar. Same FilterBar
          instance either way — only its host differs. */}
      {rail ? (
        <Popover
          anchorEl={railFilterBtnRef.current}
          open={railFilterOpen}
          onClose={() => setRailFilterOpen(false)}
        >
          <div style={{ width: 260 }}>
            <FilterBar
              mode={filterMode}
              onModeChange={setFilterMode}
              definitionMode={definitionMode}
              onDefinitionModeChange={setDefinitionMode}
              searchQuery={searchQuery}
              onSearchChange={setSearchQuery}
              pinnedCount={pinnedIds.size}
              groupByPackage={groupByPackage}
              onGroupByPackageChange={setGroupByPackage}
            />
          </div>
        </Popover>
      ) : (
        <FilterBar
          mode={filterMode}
          onModeChange={setFilterMode}
          definitionMode={definitionMode}
          onDefinitionModeChange={setDefinitionMode}
          searchQuery={searchQuery}
          onSearchChange={setSearchQuery}
          pinnedCount={pinnedIds.size}
          groupByPackage={groupByPackage}
          onGroupByPackageChange={setGroupByPackage}
        />
      )}
      {/* ModelTreeView owns its own scroll container now (UX closeout
          #4 / #17 — its virtualized big-model path needs to measure +
          scroll its own root). This wrapper just constrains height. */}
      <div className="flex-1 min-h-0">
        <ModelTreeView
          tree={orderedTree}
          expandedSet={expandedSet}
          onToggleExpand={handleToggleExpand}
          focusedId={focusedId}
          onSelectNode={handleSelect}
          pinnedIds={pinnedIds}
          onTogglePin={handleTogglePin}
          onContextMenu={handleContextMenu}
          onLaunchRunnable={handleLaunchRunnable}
          getSparklineSamples={getSparklineSamples}
          onSparklineClick={(name) =>
            promoteToPlots(name, activeSessionId)
          }
          viewsByElementId={viewsByElementId}
          selectedViewId={selectedViewId}
          onPickView={handlePickView}
          previewUri={focusedUri}
          onPromotePreview={handlePromotePreview}
          sectionHeaderScope={groupByPackage ? 'none' : 'root'}
          testIdPrefix="session-tree-v2"
        />
      </div>
      {/* Ruling B (rail): NO bottom DETAIL split-pane — selection opens
          the right-rail `inspector` context (see handleSelect). Default
          variant keeps the in-tree splitter + detail pane. */}
      {!rail && detailVisible && (
        <>
          <Splitter
            detailHeightPx={detailHeightPx}
            onPositionChange={handleSplitChange}
            containerRef={containerRef}
            testId="session-tree-v2-splitter"
          />
          <div
            data-testid="session-tree-v2-detail-shell"
            style={{ height: detailHeightPx, flexShrink: 0 }}
          >
            <DetailPanel testIdPrefix="session-tree-v2-detail" />
          </div>
        </>
      )}
      {!rail && !detailVisible && (
        <button
          type="button"
          data-testid="session-tree-v2-reveal-detail"
          onClick={() => setDetailHeightPx(DEFAULT_DETAIL_HEIGHT_PX)}
          style={{
            border: 'none',
            borderTop: '1px solid var(--outline-variant)',
            background: 'var(--surface-container-lowest)',
            color: 'var(--outline)',
            cursor: 'pointer',
            padding: '4px 8px',
            fontSize: 10,
            fontWeight: 600,
            letterSpacing: '0.08em',
            textTransform: 'uppercase',
            textAlign: 'left',
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 12, marginRight: 4 }}>
            expand_less
          </span>
          Show detail
        </button>
      )}
      {contextMenu?.kind === 'variable' && (
        <VariablesPaneContextMenu
          variableName={contextMenu.variableName}
          isPinned={contextMenu.isPinned}
          isInChart={contextMenu.isInChart}
          position={contextMenu.position}
          onToggleChart={handleToggleChart}
          onOverride={handleOverride}
          onAddBreakpoint={handleAddBreakpoint}
          onTogglePin={handleTogglePinByName}
          onCopyName={handleCopyName}
          onClose={closeContextMenu}
        />
      )}
      {contextMenu?.kind === 'sm' && (
        <SmContextMenu
          node={contextMenu.node}
          position={contextMenu.position}
          onBreakOnStateEntry={handleBreakOnStateEntry}
          onInjectEvent={(event) =>
            handleInjectEvent(contextMenu.node.name, event)
          }
          onCopyName={handleCopyName}
          onClose={closeContextMenu}
        />
      )}
      {contextMenu?.kind === 'constraint' && (
        <ConstraintContextMenu
          node={contextMenu.node}
          position={contextMenu.position}
          onBreakOnViolation={handleBreakOnViolation}
          onCopyName={handleCopyName}
          onClose={closeContextMenu}
        />
      )}
    </div>
  );
}

const EMPTY_SAMPLES: readonly number[] = [];

/** Best-guess variable name for backend lookups (override, breakpoint). */
function attributeVarName(node: AttributeTreeNode): string {
  return node.ownerPath ? `${node.ownerPath}.${node.name}` : node.name;
}

/** Convert a tree/time-series key to the runtime's name-keyed override key. */
export function runtimeOverrideName(variableName: string): string {
  return variableName.split('.').at(-1) || variableName;
}

/** Find a value-bearing leaf (attribute / calc / ode) whose
 *  backend-name matches. The pin toggle / chart toggle handlers
 *  operate on names and need to recover the originating node so
 *  they can pin by id. Now covers calc + ode since the right-click
 *  menu opens on those rows too. */
function findAttributeByVarName(
  tree: readonly ModelTreeNode[],
  name: string,
): AttributeTreeNode | null {
  for (const n of tree) {
    if (
      (n.kind === 'attribute' || n.kind === 'calc' || n.kind === 'ode') &&
      attributeVarName(n as AttributeTreeNode) === name
    ) {
      return n as AttributeTreeNode;
    }
    const inner = findAttributeByVarName(n.children, name);
    if (inner) return inner;
  }
  return null;
}

function Header({
  rail = false,
  pinnedCount,
  expandedCount,
  isLoading,
  totalNodeCount,
  visibleNodeCount,
  onExpandAll,
  onCollapseAll,
  filterBtnRef,
  filterActive = false,
  onToggleFilter,
}: {
  rail?: boolean;
  pinnedCount: number;
  expandedCount: number;
  isLoading: boolean;
  totalNodeCount: number;
  visibleNodeCount: number;
  onExpandAll: () => void;
  onCollapseAll: () => void;
  filterBtnRef?: React.RefObject<HTMLButtonElement | null>;
  filterActive?: boolean;
  onToggleFilter?: () => void;
}) {
  const hiddenCount = totalNodeCount - visibleNodeCount;

  // Ruling B (rail): a quiet "topology n/n" row + one filter icon (the
  // popover trigger) — no "Session tree" label, no expand/collapse-all
  // (rows still expand via their own chevrons), no always-on counts.
  if (rail) {
    return (
      <div
        className="flex items-center gap-2 px-3 shrink-0"
        data-testid="session-tree-v2-rail-header"
        style={{
          height: 'var(--row-compact)',
          borderBottom: '1px solid var(--border-hairline)',
          fontSize: 'var(--text-xs)',
          color: 'var(--text-muted)',
        }}
      >
        <span style={{ textTransform: 'uppercase', letterSpacing: '0.06em' }}>topology</span>
        <span className="mono-text" data-testid="session-tree-v2-counts">
          {isLoading ? 'loading…' : hiddenCount > 0 ? `${visibleNodeCount}/${totalNodeCount}` : `${totalNodeCount}`}
        </span>
        <div style={{ flex: 1 }} />
        <button
          type="button"
          ref={filterBtnRef}
          data-testid="session-tree-v2-rail-filter-toggle"
          data-active={filterActive}
          onClick={onToggleFilter}
          title="Filter & search"
          aria-label="Filter & search"
          style={{
            border: 'none',
            background: 'transparent',
            padding: 0,
            cursor: 'pointer',
            color: filterActive ? 'var(--accent-fg)' : 'var(--text-muted)',
            display: 'inline-flex',
            alignItems: 'center',
            justifyContent: 'center',
            width: 18,
            height: 18,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 14 }} aria-hidden="true">
            {filterActive ? 'filter_alt' : 'filter_list'}
          </span>
        </button>
      </div>
    );
  }

  return (
    <div
      className="flex items-center gap-2 px-3 shrink-0"
      style={{
        height: 28,
        borderBottom: '1px solid var(--outline-variant)',
        fontSize: 9,
        fontWeight: 600,
        letterSpacing: '0.1em',
        textTransform: 'uppercase',
        color: 'var(--outline)',
      }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: 14 }}>
        account_tree
      </span>
      <span>Session tree</span>
      <span
        className="mono-text"
        style={{
          marginLeft: 'auto',
          fontSize: 10,
          textTransform: 'none',
          letterSpacing: 0,
          color: 'var(--outline)',
        }}
        data-testid="session-tree-v2-counts"
      >
        {isLoading
          ? 'loading…'
          : hiddenCount > 0
            ? `${visibleNodeCount}/${totalNodeCount} · ${expandedCount} open · ${pinnedCount} pinned`
            : `${totalNodeCount} · ${expandedCount} open · ${pinnedCount} pinned`}
      </span>
      <HeaderIconButton
        icon="unfold_more"
        title="Expand all"
        onClick={onExpandAll}
        testId="session-tree-v2-expand-all"
      />
      <HeaderIconButton
        icon="unfold_less"
        title="Collapse all"
        onClick={onCollapseAll}
        testId="session-tree-v2-collapse-all"
      />
    </div>
  );
}

function HeaderIconButton({
  icon,
  title,
  onClick,
  testId,
}: {
  icon: string;
  title: string;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      onClick={onClick}
      title={title}
      aria-label={title}
      style={{
        border: 'none',
        background: 'transparent',
        padding: 0,
        cursor: 'pointer',
        color: 'var(--outline)',
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
        width: 18,
        height: 18,
      }}
    >
      <span
        className="material-symbols-outlined"
        style={{ fontSize: 14 }}
        aria-hidden="true"
      >
        {icon}
      </span>
    </button>
  );
}

function FilterBar({
  mode,
  onModeChange,
  definitionMode,
  onDefinitionModeChange,
  searchQuery,
  onSearchChange,
  pinnedCount,
  groupByPackage,
  onGroupByPackageChange,
}: {
  mode: TreeFilterMode;
  onModeChange: (mode: TreeFilterMode) => void;
  definitionMode: DefinitionMode;
  onDefinitionModeChange: (mode: DefinitionMode) => void;
  searchQuery: string;
  onSearchChange: (q: string) => void;
  pinnedCount: number;
  groupByPackage: boolean;
  onGroupByPackageChange: (v: boolean) => void;
}) {
  return (
    <div
      className="flex flex-col gap-1 px-2 py-1 shrink-0"
      style={{
        borderBottom: '1px solid var(--outline-variant)',
        background: 'var(--surface-container-lowest)',
      }}
    >
      <div className="flex gap-1" data-testid="session-tree-v2-filter-chips">
        <FilterChip
          label="All"
          active={mode === 'all'}
          onClick={() => onModeChange('all')}
          testId="session-tree-v2-chip-all"
        />
        <FilterChip
          label="Live"
          active={mode === 'live'}
          onClick={() => onModeChange('live')}
          title="Only attributes with values observed this session"
          testId="session-tree-v2-chip-live"
        />
        <FilterChip
          label={pinnedCount > 0 ? `Pinned (${pinnedCount})` : 'Pinned'}
          active={mode === 'pinned'}
          onClick={() => onModeChange('pinned')}
          testId="session-tree-v2-chip-pinned"
        />
        <div style={{ flex: 1 }} />
        <FilterChip
          label="Pkgs"
          active={groupByPackage}
          onClick={() => onGroupByPackageChange(!groupByPackage)}
          title="Group root-level elements by source file"
          testId="session-tree-v2-chip-group-by-package"
        />
        <DefinitionModeSegmented
          mode={definitionMode}
          onChange={onDefinitionModeChange}
        />
      </div>
      <input
        type="text"
        data-testid="session-tree-v2-search"
        placeholder="Filter by name…"
        value={searchQuery}
        onChange={(e) => onSearchChange(e.target.value)}
        style={{
          width: '100%',
          height: 22,
          padding: '0 6px',
          background: 'var(--surface-container)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 3,
          fontSize: 11,
        }}
      />
    </div>
  );
}

function FilterChip({
  label,
  active,
  onClick,
  title,
  testId,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
  title?: string;
  testId: string;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      data-active={active}
      onClick={onClick}
      title={title}
      style={{
        border: `1px solid ${active ? 'var(--primary)' : 'var(--outline-variant)'}`,
        background: active
          ? 'color-mix(in srgb, var(--primary) 18%, transparent)'
          : 'transparent',
        color: active ? 'var(--primary)' : 'var(--on-surface-variant)',
        padding: '2px 8px',
        borderRadius: 10,
        fontSize: 10,
        fontWeight: active ? 600 : 500,
        cursor: 'pointer',
        lineHeight: 1.4,
      }}
    >
      {label}
    </button>
  );
}

function DefinitionModeSegmented({
  mode,
  onChange,
}: {
  mode: DefinitionMode;
  onChange: (mode: DefinitionMode) => void;
}) {
  const opts: Array<{
    value: DefinitionMode;
    label: string;
    title: string;
  }> = [
    {
      value: 'usages',
      label: 'Uses',
      title: 'Usages — drop *Definition rows with a matching usage',
    },
    {
      value: 'definitions',
      label: 'Defs',
      title: 'Definitions — drop *Usage rows',
    },
    { value: 'both', label: 'Both', title: 'Show both definitions and usages' },
  ];
  return (
    <div
      data-testid="session-tree-v2-defmode"
      className="inline-flex"
      style={{
        border: '1px solid var(--outline-variant)',
        borderRadius: 10,
        overflow: 'hidden',
        fontSize: 10,
      }}
    >
      {opts.map((opt, idx) => {
        const active = opt.value === mode;
        return (
          <button
            key={opt.value}
            type="button"
            data-testid={`session-tree-v2-defmode-${opt.value}`}
            data-active={active}
            onClick={() => onChange(opt.value)}
            title={opt.title}
            style={{
              border: 'none',
              borderLeft: idx > 0 ? '1px solid var(--outline-variant)' : 'none',
              background: active
                ? 'color-mix(in srgb, var(--primary) 18%, transparent)'
                : 'transparent',
              color: active ? 'var(--primary)' : 'var(--on-surface-variant)',
              padding: '2px 8px',
              fontSize: 10,
              fontWeight: active ? 600 : 500,
              cursor: 'pointer',
              lineHeight: 1.4,
            }}
          >
            {opt.label}
          </button>
        );
      })}
    </div>
  );
}

function countNodes(tree: readonly ModelTreeNode[]): number {
  let n = 0;
  walkModelTree(tree, () => {
    n++;
  });
  return n;
}

/**
 * Produce a new tree where each parent's children list has its pinned
 * entries bubbled to the top (stable order within pinned + within
 * unpinned). Structural only — values carry through.
 */
function reorderPinned(
  tree: readonly ModelTreeNode[],
  pinnedIds: ReadonlySet<string>,
): ModelTreeNode[] {
  const reorder = (nodes: readonly ModelTreeNode[]): ModelTreeNode[] => {
    const out = nodes.map((n) => ({ ...n, children: reorder(n.children) }));
    out.sort((a, b) => {
      const ap = pinnedIds.has(a.id) ? 0 : 1;
      const bp = pinnedIds.has(b.id) ? 0 : 1;
      return ap - bp;
    });
    return out as ModelTreeNode[];
  };
  return reorder(tree);
}
