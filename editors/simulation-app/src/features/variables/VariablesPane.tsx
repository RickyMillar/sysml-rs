/**
 * VariablesPane — Cameo-parity-plus variable browser (Round 2 R2.2).
 *
 * Data sources (all Round 1 primitives):
 *   - VariableInspection.current()  → latest value per variable
 *   - VariableInspection.series()   → full time-series for sparklines
 *   - useSessionEvents(sid,'tick')  → live refresh on new snapshots
 *   - workspace TreeNode projection → backend-authoritative `unit` /
 *                                     `isq_dimension` per AttributeUsage
 *                                     (R3.1; replaces metricRegistry
 *                                     name-keyed unit lookup)
 *   - MetricRegistry                → fallback unit + domain metadata
 *                                     (registry-only `domain` heuristic
 *                                     stays here pending a follow-up)
 *   - session detail / constraints  → constraint verdicts (P/F/I/E)
 *
 * Feature checklist (from the R2.2 brief):
 *   [x] Search + filter chips with counter badges
 *   [x] Hierarchical tree grouped by qualified name
 *   [x] Pinned virtual group at the top (persisted)
 *   [x] Sparklines (toggle, windowed via IntersectionObserver)
 *   [x] Constraint pill (P / F / I / E)
 *   [x] Flash animation on value change
 *   [x] Right-click context menu (Override / Breakpoint / Pin / Copy)
 *   [x] Click drills into plot selection
 *   [x] Keyboard nav: /, Up, Down, Enter, Space
 *   [x] Cmd+Shift+V toggles pane focus
 *   [x] Empty / loading states
 */

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
} from 'react';
import { useSessionStore } from '@/features/sessions/store';
import { openOverridePopover } from '@/features/sessions/OverridePopover';
import { useSessionDetail, useSessionTopology } from '@/features/sessions/queries';
import {
  useSessionLiveStore,
  type ConstraintView,
} from '@/features/sessions/sessionLiveStore';
import { isStreamV1Enabled } from '@/features/sessions/useSessionStream';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { usePlotSelectionStore } from '@/features/results/usePlotSelectionStore';
import { metricRegistry } from '@/shared/metrics/registry';
import { useSessionEvents } from '@/engine/SessionEvents';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris, useWorkspaceTree } from '@/features/packages/queries';
import {
  flattenAttributePhysics,
  type AttributePhysicsInfo,
} from '@/features/sessions/tree/flattenAttributePhysics';
import type { SessionDetail, TimePoint } from '@/features/sessions/types';
import {
  VariableRow,
} from './VariableRow';
import {
  VariablesPaneContextMenu,
} from './VariablesPaneContextMenu';
import {
  buildTree,
  computeFilterCounts,
  flattenTree,
  partitionPinned,
  type ConstraintVerdict,
  type VariableEntry,
  type VariableFilter,
  type VariableTreeNode,
} from './VariableTree';
import { useVariablesState } from './useVariablesState';

// ── Props ────────────────────────────────────────────────────────────

export interface VariablesPaneProps {
  /** When true, render in a larger layout (not used today but future-proofs R2.1). */
  expanded?: boolean;
  /** Optional override for which session to read (default: the active session). */
  sessionId?: string | null;
  /** Test hook — replace the variable source with a static list. */
  __testEntries?: VariableEntry[];
}

// ── Helpers ──────────────────────────────────────────────────────────

/**
 * Walk each constraint's authoritative `operands` map to associate
 * every referenced identifier with the constraint's verdict. The wire
 * now carries the full four-valued `VerdictKind`, so the fold takes the
 * worst verdict per variable (`error` > `fail` > `inconclusive` > `pass`,
 * matching `VerdictKind::aggregate`) rather than collapsing everything
 * that isn't a pass into red.
 */
function collectConstraintVerdicts(
  rows: ReadonlyArray<ConstraintView> | null | undefined,
): Map<string, ConstraintVerdict> {
  const out = new Map<string, ConstraintVerdict>();
  if (!rows) return out;
  /** Severity order — mirrors `VerdictKind::aggregate` (worst wins). */
  const rank: Record<string, number> = { pass: 0, inconclusive: 1, fail: 2, error: 3 };
  for (const r of rows) {
    // The tree's ConstraintVerdict vocabulary is lowercase; the wire is not.
    // A row with no verdict at all is backend/frontend skew — skip it rather
    // than throw on `undefined.toLowerCase()` deep inside a render.
    if (r.verdict === undefined) continue;
    const verdict = r.verdict.toLowerCase() as ConstraintVerdict;
    const operands = r.operands;
    if (!operands) continue;
    for (const v of Object.keys(operands)) {
      const prev = out.get(v);
      if (prev !== undefined && (rank[prev] ?? 0) >= (rank[verdict] ?? 0)) continue;
      out.set(v, verdict);
    }
  }
  return out;
}

/**
 * Look up `unit` for a variable. Prefers the backend-projected
 * `physicsByPath` map (R3.1 — sourced from the AttributeUsage TreeNode's
 * `unit` field). Falls back to the legacy `metricRegistry` so variables
 * the backend can't infer (e.g. expression-sourced metrics, dead-entry
 * heuristics) still surface a unit when one was registered.
 */
function lookupUnit(
  name: string,
  physicsByPath: Map<string, AttributePhysicsInfo>,
): string | undefined {
  return physicsByPath.get(name)?.unit ?? metricRegistry.get(name)?.unit;
}

/**
 * Look up `domain` for a variable. R3.1 chose option 1: domain stays in
 * `metricRegistry` (which keeps the `classifyVariableDomain` heuristics
 * the registry's syncVariableMetrics call wires up). The TreeNode
 * projection only carries `unit` + `isq_dimension`; mapping ISQ → domain
 * here would re-introduce a frontend heuristic, defeating the purpose.
 */
function lookupDomain(name: string): string | undefined {
  return metricRegistry.get(name)?.domain;
}

/** Build the `VariableEntry[]` from flat scalar/string maps. */
function buildEntries(
  scalars: Record<string, number> | undefined,
  strings: Record<string, string> | undefined,
  verdicts: Map<string, ConstraintVerdict>,
  lastChangedTicks: Map<string, number>,
  physicsByPath: Map<string, AttributePhysicsInfo>,
): VariableEntry[] {
  const out: VariableEntry[] = [];
  if (scalars) {
    for (const [name, raw] of Object.entries(scalars)) {
      out.push({
        name,
        value: raw,
        unit: lookupUnit(name, physicsByPath),
        domain: lookupDomain(name),
        constraint: verdicts.get(name),
        lastChangedTick: lastChangedTicks.get(name) ?? null,
      });
    }
  }
  if (strings) {
    for (const [name, raw] of Object.entries(strings)) {
      out.push({
        name,
        value: raw,
        unit: lookupUnit(name, physicsByPath),
        domain: lookupDomain(name),
        constraint: verdicts.get(name),
        lastChangedTick: lastChangedTicks.get(name) ?? null,
      });
    }
  }
  out.sort((a, b) => a.name.localeCompare(b.name));
  return out;
}

function coerceValue(raw: unknown): VariableEntry['value'] {
  if (raw == null) return null;
  if (typeof raw === 'number' || typeof raw === 'boolean' || typeof raw === 'string') return raw;
  return raw as Record<string, unknown>;
}

// ── Keyboard shortcut singleton ─────────────────────────────────────
//
// Cmd/Ctrl+Shift+V focuses the pane's search input. During the ninebar
// migration VariablesPane can mount TWICE at once (right rail + legacy
// workbench panel) — two independent `window.addEventListener('keydown',
// …)` registrations would both fire on every keystroke (F15: "also
// grabs a window-level shortcut — singleton bugs when dual-mounted").
//
// Fix: install the real DOM listener exactly once (module scope, guarded
// by `shortcutHandlerInstalled`) and have it dispatch to whichever pane
// most recently mounted (`activeShortcutFocus`). On unmount a pane hands
// the shortcut back to whichever mount was active before it, so e.g.
// unmounting the workbench pane restores the rail pane's shortcut. This
// is simpler than scoping the listener to the focused DOM element (the
// pane isn't always focused when the user reaches for the shortcut) and
// correct for any number of concurrent mounts, not just two.
let activeShortcutFocus: (() => void) | null = null;
let shortcutHandler: ((e: KeyboardEvent) => void) | null = null;

function installVariablesPaneShortcutOnce(): void {
  if (shortcutHandler || typeof window === 'undefined') return;
  shortcutHandler = (e: KeyboardEvent) => {
    if ((e.metaKey || e.ctrlKey) && e.shiftKey && e.key.toLowerCase() === 'v') {
      e.preventDefault();
      activeShortcutFocus?.();
    }
  };
  window.addEventListener('keydown', shortcutHandler);
}

/** Claim the module-level Cmd/Ctrl+Shift+V shortcut for this mount. */
function useVariablesPaneShortcut(focus: () => void): void {
  useEffect(() => {
    installVariablesPaneShortcutOnce();
    const previous = activeShortcutFocus;
    activeShortcutFocus = focus;
    return () => {
      if (activeShortcutFocus === focus) activeShortcutFocus = previous;
    };
  }, [focus]);
}

/**
 * Test-only — undo the module-level shortcut singleton (including the real
 * `window.addEventListener` registration) so each spec can start clean.
 * Mirrors the `__resetVariablesStoreForTests` convention in
 * useVariablesState.ts. Not called by production code.
 */
export function __resetVariablesPaneShortcutForTests(): void {
  if (shortcutHandler && typeof window !== 'undefined') {
    window.removeEventListener('keydown', shortcutHandler);
  }
  shortcutHandler = null;
  activeShortcutFocus = null;
}

// ── Main component ───────────────────────────────────────────────────

export function VariablesPane({
  expanded,
  sessionId: sessionIdProp,
  __testEntries,
}: VariablesPaneProps = {}) {
  // Test-only render-count probe (F15 guardrail regression coverage).
  // Zero runtime cost beyond a data-* attribute — lets tests assert the
  // pane's OWN render count is unaffected by an unrelated live-store
  // value change, as opposed to a per-row re-render, which fires via
  // that row's own useVar/useStringVar subscription (VariableRow.tsx)
  // and never touches this counter.
  const renderCountRef = useRef(0);
  renderCountRef.current += 1;

  const activeSessionId = useSessionStore((s) => s.activeSessionId);
  const phase = useSessionStore((s) => s.phase);
  const sessionId = sessionIdProp ?? activeSessionId;
  const isActiveSession = sessionId !== null && sessionId === activeSessionId;
  const selectedScope = useSessionStore((s) => s.selectedScope);
  const popSelectedScopeTo = useSessionStore((s) => s.popSelectedScopeTo);
  const { data: topology } = useSessionTopology(sessionId);

  // Stage 6b data sources:
  //   - active session + streaming enabled: read from sessionLiveStore.
  //   - active session + streaming disabled (flag unset): fall through to
  //     useSessionDetail with includeVariables=true, matching the
  //     pre-stream behaviour.
  //   - other session (compare / archive): includeVariables=true.
  const streamingOn = isStreamV1Enabled();
  const { data: sessionDetail } = useSessionDetail(sessionId, {
    includeVariables: !isActiveSession || !streamingOn,
  });

  // ── Live-store subscription shape (F15 guardrail) ───────────────────
  // The pane must NEVER subscribe to the whole `s.snapshot` object (nor
  // to `useTimeSeriesStore.revision` — see the sparkline source below):
  // either fires on every WS delta frame regardless of whether anything
  // this pane cares about changed, and per the ninebar plan this pane can
  // mount TWICE at once (right rail + workbench during migration), so
  // that cost would be paid twice. Instead subscribe to two cheap
  // per-key selectors:
  //   - `liveTick`: a plain number, changes once per tick.
  //   - `namesKey`: a joined string of variable names, changes only when
  //     a variable is added/removed. Zustand's default `Object.is`
  //     equality on a returned string primitive means a tick that only
  //     changes existing values (no add/remove) never re-renders the
  //     pane via this selector.
  // Both stay gated on `s.sessionId === sessionId`, mirroring the old
  // `liveSnapshot` selector's guard — `sessionLiveStore` is single-
  // session (F15 finding #2), so a pane showing a non-active session
  // (archive/compare) must not pick up whatever session happens to be
  // streaming live right now.
  const isLive = isActiveSession && streamingOn;
  const liveTick = useSessionLiveStore((s) =>
    isLive && s.sessionId === sessionId ? (s.snapshot?.tick ?? null) : null,
  );
  const namesKey = useSessionLiveStore((s) => {
    if (!isLive || s.sessionId !== sessionId || !s.snapshot) return '';
    const scalarKeys = Object.keys(s.snapshot.scalar_vars);
    const stringKeys = Object.keys(s.snapshot.string_vars);
    if (scalarKeys.length === 0 && stringKeys.length === 0) return '';
    return scalarKeys.join(',') + '|' + stringKeys.join(',');
  });

  // Derive the effective scalar / string / constraint sources. Active-
  // session path reads the live store IMPERATIVELY (getState(), not a
  // hook) so this memo only re-runs on the two cheap triggers above,
  // never on `s.snapshot` identity. Individual rows re-source their own
  // display value straight from the store (see VariableRow.tsx) — the
  // maps embedded here only feed tree structure / search / filter chips
  // / the override-prompt default, not per-row rendering.
  // Fallback path extracts from the (rare) include_variables=true polled
  // detail.
  const { scalarVars, stringVars, constraintRows, currentTick } = useMemo(() => {
    if (isLive) {
      const snap = useSessionLiveStore.getState().snapshot;
      return {
        scalarVars: snap?.scalar_vars,
        stringVars: snap?.string_vars,
        constraintRows: snap?.constraint_results ?? [],
        currentTick: snap?.tick,
      };
    }
    const raw = (sessionDetail?.latest_snapshot as Record<string, unknown> | null | undefined) ?? null;
    if (!raw) {
      return {
        scalarVars: undefined as Record<string, number> | undefined,
        stringVars: undefined as Record<string, string> | undefined,
        constraintRows: [] as ConstraintView[],
        currentTick: sessionDetail?.summary.tick,
      };
    }
    const rawVars = (raw.variables as Record<string, unknown> | undefined) ?? {};
    const scalars: Record<string, number> = {};
    const strs: Record<string, string> = {};
    for (const [name, v] of Object.entries(rawVars)) {
      if (typeof v === 'number' && Number.isFinite(v)) scalars[name] = v;
      else if (typeof v === 'boolean') scalars[name] = v ? 1 : 0;
      else if (typeof v === 'string') strs[name] = v;
    }
    // `latest_snapshot.constraint_results` is projected through the
    // same `ConstraintView` as the stream, so we can consume it
    // directly. The backend owns shape; no client-side remapping.
    const cRows = Array.isArray(raw.constraint_results)
      ? (raw.constraint_results as ConstraintView[])
      : [];
    return {
      scalarVars: scalars,
      stringVars: strs,
      constraintRows: cRows,
      currentTick: sessionDetail?.summary.tick,
    };
  }, [isLive, liveTick, namesKey, sessionDetail]);

  // Sparkline sample source — read IMPERATIVELY (never subscribed to
  // `useTimeSeriesStore.revision`, per the F15 guardrail above) so a new
  // time-series point never triggers a pane re-render on its own.
  // Sparklines simply pick up fresh samples the next time the pane
  // renders for another reason (e.g. `liveTick` advancing).
  const getAllSeries = useCallback(
    () => useTimeSeriesStore.getState().getTimeSeries(),
    [],
  );

  // Track per-variable "last change" ticks so the "Changed recently" chip
  // is accurate. Rebuilt on every tick event from the session bus.
  const lastChangedRef = useRef<Map<string, number>>(new Map());
  const lastValueRef = useRef<Map<string, unknown>>(new Map());

  // Drop accumulated per-variable dedup state when the session changes —
  // tick numbers and variable names reset with a fresh run.
  useEffect(() => {
    lastChangedRef.current.clear();
    lastValueRef.current.clear();
  }, [sessionId]);

  useEffect(() => {
    if (currentTick === undefined) return;
    const visit = (map: Record<string, unknown> | undefined) => {
      if (!map) return;
      for (const [name, raw] of Object.entries(map)) {
        const prev = lastValueRef.current.get(name);
        if (prev !== raw) {
          lastValueRef.current.set(name, raw);
          lastChangedRef.current.set(name, currentTick);
        }
      }
    };
    visit(scalarVars);
    visit(stringVars);
  }, [currentTick, scalarVars, stringVars]);

  // Also re-render pane on verdict flip so pills update promptly.
  const [, setVerdictNonce] = useState(0);
  useSessionEvents(sessionId, 'verdict-flip', () => setVerdictNonce((n) => n + 1));

  // Compose entries.
  const verdicts = useMemo(
    () => collectConstraintVerdicts(constraintRows),
    [constraintRows],
  );

  // Backend-projected physics info for AttributeUsage nodes (R3.1).
  // Replaces the per-name `metricRegistry.get(...)` unit lookup with a
  // dotted-path map sourced from each TreeNode's `unit` / `isq_dimension`
  // fields. The legacy registry stays in `lookupUnit` as a fallback for
  // variables the backend can't infer (expression-sourced metrics,
  // dead-entry heuristics).
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const wsUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);
  const { data: treesByUri } = useWorkspaceTree(wsUris);
  const physicsByPath = useMemo<Map<string, AttributePhysicsInfo>>(() => {
    if (!treesByUri) return new Map();
    const merged = new Map<string, AttributePhysicsInfo>();
    for (const [, tree] of treesByUri) {
      const flat = flattenAttributePhysics(tree);
      for (const [k, v] of flat) merged.set(k, v);
    }
    return merged;
  }, [treesByUri]);

  const entries = useMemo(() => {
    if (__testEntries) return __testEntries;
    return buildEntries(
      scalarVars,
      stringVars,
      verdicts,
      lastChangedRef.current,
      physicsByPath,
    );
  }, [scalarVars, stringVars, verdicts, __testEntries, physicsByPath]);

  // Pane state (pin, expand, filter, search, sparkline toggle).
  const ui = useVariablesState();

  // Derive the "hidden" predicate: hides stdlib (any name whose first
  // path segment isn't a known topology module / subsystem) and
  // anything outside the current scope. The topology roots set stays
  // empty on non-live sessions, in which case everything shows — that
  // matches the archive / compare case where we don't have a topology.
  const hidden = useMemo(() => {
    const userRoots = new Set<string>();
    const topoModules = (topology as { modules?: Array<{ label?: string; id?: string; subsystems?: Array<{ name?: string }> }> } | null | undefined)?.modules ?? [];
    for (const m of topoModules) {
      if (m.label) userRoots.add(m.label);
      if (m.id) userRoots.add(m.id);
      for (const s of m.subsystems ?? []) if (s.name) userRoots.add(s.name);
    }
    const scopePrefix = selectedScope.length ? selectedScope.join('.') : '';
    return (name: string): boolean => {
      // Default-hidden accounting vars.
      if (name.startsWith('__') || name === 't_ms' || name === 'tick' || name === 'clock_time') return true;
      // Stdlib filter — names whose first segment isn't in the topology.
      if (userRoots.size > 0) {
        const firstSeg = name.split('.', 1)[0];
        if (!userRoots.has(firstSeg)) return true;
      }
      // Scope filter — must start with scope prefix (or equal it).
      if (scopePrefix) {
        if (name !== scopePrefix && !name.startsWith(scopePrefix + '.')) return true;
      }
      return false;
    };
  }, [topology, selectedScope]);

  const counts = useMemo(
    () => computeFilterCounts(entries, {
      pinned: ui.pinned,
      currentTick: currentTick,
      hidden,
    }),
    [entries, ui.pinned, currentTick, hidden],
  );

  // Partition pinned into a virtual group; build the normal tree for the rest.
  const { pinned: pinnedList, rest } = useMemo(
    () => partitionPinned(entries, ui.pinned),
    [entries, ui.pinned],
  );
  const restTree = useMemo(
    () => buildTree(rest, {
      pinned: ui.pinned,
      search: ui.search,
      filter: ui.filter === 'pinned' ? 'all' : ui.filter,
      currentTick: currentTick,
      hidden,
    }),
    [rest, ui.pinned, ui.search, ui.filter, currentTick, hidden],
  );
  const pinnedTree = useMemo(
    () => buildTree(pinnedList, {
      // filter still applies inside the pinned group (search + chip)
      pinned: ui.pinned,
      search: ui.search,
      filter: ui.filter === 'pinned' ? 'all' : ui.filter,
      currentTick: currentTick,
      hidden,
    }),
    [pinnedList, ui.pinned, ui.search, ui.filter, currentTick, hidden],
  );

  // Flattened row lists for keyboard nav + windowed rendering.
  const flatPinned = useMemo(
    () => flattenTree(pinnedTree, ui.collapsed),
    [pinnedTree, ui.collapsed],
  );
  const flatRest = useMemo(
    () => flattenTree(restTree, ui.collapsed),
    [restTree, ui.collapsed],
  );
  const combinedFlat = useMemo(() => [...flatPinned, ...flatRest], [flatPinned, flatRest]);

  // Search-input focus ref — '/' keyboard focuses it.
  const searchRef = useRef<HTMLInputElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  // Cmd+Shift+V → focus the pane / its search input. Registered through
  // the module-level singleton above so dual mounts don't double-fire.
  const focusSearch = useCallback(() => {
    searchRef.current?.focus();
  }, []);
  useVariablesPaneShortcut(focusSearch);

  // In-pane keyboard nav.
  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      if (e.key === '/' && document.activeElement !== searchRef.current) {
        e.preventDefault();
        searchRef.current?.focus();
        return;
      }
      if (!combinedFlat.length) return;
      const idx = combinedFlat.findIndex((n) => n.path === ui.selectedPath);
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        const next = idx < 0 ? 0 : Math.min(idx + 1, combinedFlat.length - 1);
        ui.setSelectedPath(combinedFlat[next].path);
      } else if (e.key === 'ArrowUp') {
        e.preventDefault();
        const next = idx < 0 ? combinedFlat.length - 1 : Math.max(idx - 1, 0);
        ui.setSelectedPath(combinedFlat[next].path);
      } else if (e.key === 'Enter' && idx >= 0) {
        e.preventDefault();
        const node = combinedFlat[idx];
        if (node.isLeaf) handleActivate(node.path);
        else ui.toggleCollapsed(node.path);
      } else if (e.key === ' ' && idx >= 0) {
        e.preventDefault();
        const node = combinedFlat[idx];
        if (node.isLeaf) ui.togglePinned(node.path);
      }
    },
    // handleActivate / ui methods are stable refs from the store, we pull in
    // only what varies.
    [combinedFlat, ui.selectedPath, ui.setSelectedPath, ui.toggleCollapsed, ui.togglePinned],
  );

  // ── Context menu state + action wiring ─────────────────────────────
  const [menu, setMenu] = useState<{ name: string; pos: { x: number; y: number } } | null>(null);

  const handleContextMenu = useCallback(
    (path: string, position: { x: number; y: number }) => {
      setMenu({ name: path, pos: position });
    },
    [],
  );

  const plotSelections = usePlotSelectionStore((s) => s.selectionsBySession);
  const setPlotSelection = usePlotSelectionStore((s) => s.setSelected);

  const handleActivate = useCallback(
    (path: string) => {
      if (!sessionId) return;
      // Toggle the variable into the plot selection — drilling in
      // updates the chart. This satisfies the "drill into plot" brief.
      const existing = plotSelections[sessionId] ?? [];
      if (!existing.includes(path)) {
        setPlotSelection(sessionId, [...existing, path]);
      }
      // Also surface as a browser event for diagram highlight wiring
      // (diagram listens on `sysml-variable-select` in Round 2+).
      try {
        window.dispatchEvent(new CustomEvent('sysml-variable-select', { detail: { name: path } }));
      } catch { /* non-browser env */ }
    },
    [sessionId, plotSelections, setPlotSelection],
  );

  const handleOverride = useCallback(
    (name: string) => {
      if (!sessionId) return;
      const current = entries.find((e) => e.name === name)?.value;
      // One consolidated override surface (ninebar Phase 3, F9/F15):
      // anchored popover with apply-&-re-run + explicit fork action.
      openOverridePopover(name, current == null ? null : String(current));
    },
    [sessionId, entries],
  );

  const handleAddBreakpoint = useCallback(
    async (name: string) => {
      if (!sessionId) return;
      // eslint-disable-next-line no-alert
      const raw = window.prompt(`Threshold value for ${name} (breakpoint when exceeded)`, '0');
      if (raw == null) return;
      const threshold = Number(raw);
      if (!Number.isFinite(threshold)) return;
      // Lazy-load SessionControl's breakpoint client so the Variables pane
      // doesn't force engine-side imports into the critical path.
      try {
        const { createBreakpointClient } = await import('@/engine/SessionControl');
        const client = createBreakpointClient();
        await client.set(sessionId, {
          kind: 'threshold-crossing',
          target: name,
          variable: name,
          threshold,
          direction: 'either',
        });
      } catch (err) {
        // eslint-disable-next-line no-console
        console.warn('[VariablesPane] breakpoint set failed:', err);
      }
    },
    [sessionId],
  );

  const handleCopyName = useCallback((name: string) => {
    try {
      if (typeof navigator !== 'undefined' && navigator.clipboard) {
        void navigator.clipboard.writeText(name);
      }
    } catch { /* ignore */ }
  }, []);

  // ── Render ─────────────────────────────────────────────────────────

  const sessionIsIdle = phase === 'idle' || !sessionId;

  return (
    <div
      ref={containerRef}
      data-testid="variables-pane"
      data-render-count={renderCountRef.current}
      tabIndex={0}
      onKeyDown={onKeyDown}
      className="flex flex-col h-full"
      style={{
        background: 'var(--surface-sunken)',
        borderLeft: '1px solid var(--border-default)',
        minWidth: 300,
        maxWidth: expanded ? undefined : 380,
      }}
    >
      <Header
        searchRef={searchRef}
        search={ui.search}
        onSearchChange={ui.setSearch}
        filter={ui.filter}
        onFilterChange={ui.setFilter}
        counts={counts}
        showSparklines={ui.showSparklines}
        onToggleSparklines={ui.setShowSparklines}
      />

      <ScopeBreadcrumb path={selectedScope} onPop={popSelectedScopeTo} />

      <div className="flex-1 overflow-y-auto" data-testid="variables-pane-body">
        {sessionIsIdle && (
          <EmptyState
            icon="play_circle"
            title="No session running"
            body="Start a session to see variables here."
          />
        )}

        {!sessionIsIdle && combinedFlat.length === 0 && entries.length === 0 && (
          <SkeletonRows />
        )}

        {!sessionIsIdle && combinedFlat.length === 0 && entries.length > 0 && (
          <EmptyState
            icon="search_off"
            title="No matches"
            body={ui.search ? `No variables match "${ui.search}".` : 'No variables match the current filter.'}
          />
        )}

        {flatPinned.length > 0 && (
          <PinnedGroupHeader count={flatPinned.filter((n) => n.isLeaf).length} />
        )}
        {renderRows(flatPinned, ui, getAllSeries, handleContextMenu, handleActivate, isLive)}

        {flatPinned.length > 0 && flatRest.length > 0 && (
          <div style={{ height: 8 }} />
        )}

        {renderRows(flatRest, ui, getAllSeries, handleContextMenu, handleActivate, isLive)}
      </div>

      <VariablesPaneContextMenu
        variableName={menu?.name ?? null}
        isPinned={menu ? ui.pinned.has(menu.name) : false}
        isInChart={
          !!menu && !!sessionId && (plotSelections[sessionId] ?? []).includes(menu.name)
        }
        position={menu?.pos ?? { x: 0, y: 0 }}
        onToggleChart={(name) => {
          if (!sessionId) return;
          usePlotSelectionStore.getState().toggleSelected(sessionId, name);
        }}
        onOverride={handleOverride}
        onAddBreakpoint={handleAddBreakpoint}
        onTogglePin={(n) => ui.togglePinned(n)}
        onCopyName={handleCopyName}
        onClose={() => setMenu(null)}
      />
    </div>
  );
}

// ── Row rendering ────────────────────────────────────────────────────

function renderRows(
  flat: VariableTreeNode[],
  ui: ReturnType<typeof useVariablesState>,
  getAllSeries: () => Record<string, TimePoint[]>,
  onContextMenu: (path: string, pos: { x: number; y: number }) => void,
  onActivate: (path: string) => void,
  live: boolean,
) {
  const all = getAllSeries();
  return flat.map((node) => {
    const samples = ui.showSparklines && node.isLeaf
      ? extractRecentSamples(all[node.path])
      : EMPTY_SAMPLES;
    return (
      <VariableRow
        key={node.path}
        node={node}
        collapsed={ui.collapsed.has(node.path)}
        onToggleCollapse={ui.toggleCollapsed}
        selected={ui.selectedPath === node.path}
        onSelect={ui.setSelectedPath}
        pinned={ui.pinned.has(node.path)}
        showSparkline={ui.showSparklines && node.isLeaf}
        sparklineSamples={samples}
        onContextMenu={onContextMenu}
        onActivate={onActivate}
        live={live}
      />
    );
  });
}

const EMPTY_SAMPLES: number[] = [];
const SPARKLINE_WINDOW = 30;

function extractRecentSamples(points: TimePoint[] | undefined): number[] {
  if (!points || points.length === 0) return EMPTY_SAMPLES;
  const slice = points.length > SPARKLINE_WINDOW ? points.slice(-SPARKLINE_WINDOW) : points;
  return slice.map((p) => p.v);
}

// ── Subcomponents ────────────────────────────────────────────────────

function Header({
  searchRef,
  search,
  onSearchChange,
  filter,
  onFilterChange,
  counts,
  showSparklines,
  onToggleSparklines,
}: {
  searchRef: React.RefObject<HTMLInputElement | null>;
  search: string;
  onSearchChange: (s: string) => void;
  filter: VariableFilter;
  onFilterChange: (f: VariableFilter) => void;
  counts: ReturnType<typeof computeFilterCounts>;
  showSparklines: boolean;
  onToggleSparklines: (show: boolean) => void;
}) {
  return (
    <div
      className="flex flex-col gap-1.5 px-2 py-2 shrink-0"
      style={{ borderBottom: '1px solid var(--border-default)' }}
    >
      <div className="flex items-center gap-1.5">
        <span
          className="material-symbols-outlined"
          style={{ fontSize: '14px', color: 'var(--text-muted)' }}
        >
          search
        </span>
        <input
          ref={searchRef}
          data-testid="variables-pane-search"
          type="text"
          value={search}
          onChange={(e) => onSearchChange(e.target.value)}
          placeholder="Search (supports glob, e.g. circuit*.temp)"
          className="flex-1 bg-transparent border-none outline-none mono-text"
          style={{ fontSize: 'var(--text-xs)', color: 'var(--text-primary)' }}
        />
        <label
          className="flex items-center gap-1 cursor-pointer"
          title="Show sparklines"
          style={{ color: 'var(--text-muted)', fontSize: '9px' }}
        >
          <input
            data-testid="variables-pane-toggle-sparklines"
            type="checkbox"
            checked={showSparklines}
            onChange={(e) => onToggleSparklines(e.target.checked)}
            style={{ accentColor: 'var(--accent-fg)' }}
          />
          <span className="material-symbols-outlined" style={{ fontSize: '14px' }}>
            show_chart
          </span>
        </label>
      </div>
      <div className="flex items-center gap-1 flex-wrap">
        <FilterChip label="All"         active={filter === 'all'}          count={counts.all}         onClick={() => onFilterChange('all')} />
        <FilterChip label="Passing"     active={filter === 'passing'}      count={counts.passing}     onClick={() => onFilterChange('passing')}      accent="var(--verdict-pass)" />
        <FilterChip label="Failing"     active={filter === 'failing'}      count={counts.failing}     onClick={() => onFilterChange('failing')}      accent="var(--verdict-fail)" />
        <FilterChip label="Inconclusive"active={filter === 'inconclusive'} count={counts.inconclusive}onClick={() => onFilterChange('inconclusive')}accent="var(--verdict-inconclusive)" />
        <FilterChip label="Error"       active={filter === 'error'}        count={counts.error}       onClick={() => onFilterChange('error')}        accent="var(--verdict-error)" />
        <FilterChip label="Changed"     active={filter === 'changed'}      count={counts.changed}     onClick={() => onFilterChange('changed')} />
        <FilterChip label="Pinned"      active={filter === 'pinned'}       count={counts.pinned}      onClick={() => onFilterChange('pinned')}       accent="var(--chart-series-3)" />
      </div>
    </div>
  );
}

function FilterChip({
  label,
  active,
  count,
  onClick,
  accent,
}: {
  label: string;
  active: boolean;
  count: number;
  onClick: () => void;
  accent?: string;
}) {
  const color = accent ?? 'var(--accent-fg)';
  return (
    <button
      type="button"
      data-testid={`variables-pane-chip-${label.toLowerCase()}`}
      data-active={active ? 'true' : 'false'}
      onClick={onClick}
      className="inline-flex items-center gap-1 px-1.5 py-0.5 rounded transition-colors"
      style={{
        fontSize: '9px',
        letterSpacing: '0.04em',
        textTransform: 'uppercase',
        fontWeight: active ? 700 : 500,
        background: active
          ? `color-mix(in srgb, ${color} 22%, transparent)`
          : 'transparent',
        color: active ? color : 'var(--text-muted)',
        border: `1px solid ${active ? color : 'var(--border-default)'}`,
        cursor: 'pointer',
      }}
    >
      {label}
      <span
        style={{
          fontSize: '9px',
          fontWeight: 600,
          opacity: 0.75,
          minWidth: 12,
          textAlign: 'center',
        }}
      >
        {count}
      </span>
    </button>
  );
}

function PinnedGroupHeader({ count }: { count: number }) {
  return (
    <div
      className="px-2 py-1 flex items-center gap-1.5 uppercase tracking-wider"
      style={{
        fontSize: '9px',
        letterSpacing: '0.1em',
        color: 'var(--chart-series-3)',
        fontWeight: 600,
        background: 'color-mix(in srgb, var(--chart-series-3) 6%, transparent)',
      }}
    >
      <span className="material-symbols-outlined" style={{ fontSize: '12px' }}>push_pin</span>
      Pinned
      <span style={{ opacity: 0.6 }}>({count})</span>
    </div>
  );
}

function EmptyState({
  icon,
  title,
  body,
}: {
  icon: string;
  title: string;
  body: string;
}) {
  return (
    <div
      className="flex flex-col items-center justify-center h-full px-6 py-10 text-center"
      style={{ color: 'var(--text-muted)' }}
    >
      <span
        className="material-symbols-outlined"
        style={{ fontSize: '28px', marginBottom: 8, color: 'var(--border-default)' }}
      >
        {icon}
      </span>
      <div style={{ fontSize: 'var(--text-sm)', color: 'var(--text-secondary)', fontWeight: 600, marginBottom: 4 }}>
        {title}
      </div>
      <div style={{ fontSize: 'var(--text-xs)' }}>{body}</div>
    </div>
  );
}

function SkeletonRows() {
  return (
    <div
      className="flex flex-col gap-1 p-2"
      aria-busy="true"
      data-testid="variables-pane-skeleton"
    >
      {Array.from({ length: 6 }).map((_, i) => (
        <div
          key={i}
          className="animate-pulse rounded"
          style={{
            height: 18,
            background: 'color-mix(in srgb, var(--text-primary) 6%, transparent)',
          }}
        />
      ))}
    </div>
  );
}

// ── Scope breadcrumb ──────────────────────────────────────────────────

function ScopeBreadcrumb({
  path,
  onPop,
}: {
  path: string[];
  onPop: (depth: number) => void;
}) {
  // "All" is a click target that clears the scope (depth 0). Each
  // segment pops to that depth when clicked.
  const crumbStyle: CSSProperties = {
    fontSize: '10px',
    color: 'var(--text-secondary)',
    cursor: 'pointer',
    padding: '0 2px',
    borderRadius: 2,
  };
  const activeStyle: CSSProperties = {
    ...crumbStyle,
    color: 'var(--text-primary)',
    fontWeight: 600,
    cursor: 'default',
  };
  const sepStyle: CSSProperties = {
    fontSize: '10px',
    color: 'var(--border-default)',
    margin: '0 2px',
  };
  return (
    <div
      data-testid="variables-scope-breadcrumb"
      className="flex items-center shrink-0"
      style={{
        padding: '4px 10px',
        borderBottom: '1px solid var(--border-default)',
        background: 'var(--surface-sunken)',
      }}
    >
      <span
        style={path.length === 0 ? activeStyle : crumbStyle}
        onClick={path.length === 0 ? undefined : () => onPop(0)}
      >
        All
      </span>
      {path.map((seg, i) => {
        const isLast = i === path.length - 1;
        return (
          <span key={i} className="flex items-center">
            <span style={sepStyle}>/</span>
            <span
              style={isLast ? activeStyle : crumbStyle}
              onClick={isLast ? undefined : () => onPop(i + 1)}
              title={isLast ? undefined : `Pop to ${seg}`}
            >
              {seg}
            </span>
          </span>
        );
      })}
    </div>
  );
}
