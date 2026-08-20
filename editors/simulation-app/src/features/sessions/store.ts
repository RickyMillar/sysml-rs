/**
 * Session view store — Zustand slice for client-only session state.
 *
 * Per ADR-004 section 2: this store holds ONLY local view preferences
 * and the 6-phase state machine. NO remote data lives here — all
 * backend-owned data flows through react-query.
 */

import { create } from 'zustand';
import { useInvestigationTrail } from '@/features/investigation/useInvestigationTrail';
import type { SessionPhase, SessionViewState } from './types';

interface SessionStoreState {
  // ── Active session identity ───────────────────────────────────────
  activeSessionId: string | null;
  selectedTargetId: string | null;

  // ── 6-phase state machine (ADR-004 section 4) ────────────────────
  phase: SessionPhase;

  // ── Per-session view preferences ──────────────────────────────────
  monitoredVariables: string[];
  compareBaseline: string | null;
  draftOverrides: Record<string, string>;

  // ── Subaction drill-down breadcrumb ───────────────────────────────
  /** Action names forming a breadcrumb trail; empty = root scope. */
  focusedActionPath: string[];

  // ── Model-tree focus (Phase A2 scaffolding) ───────────────────────
  /**
   * ElementIds forming a strict prefix path from the session root to the
   * currently-focused node in the session tree — the single source of
   * truth for "which node is active in the detail region". Empty means
   * the root session is focused. Unlike `focusedActionPath`, this is
   * NOT a stack: click-to-navigate REPLACES it (see `setFocusPath`) so
   * it can never grow beyond the depth of the clicked node. Drives the
   * breadcrumb bar + detail region once Phase B lands; no UI reads it
   * yet.
   *
   * "Breadcrumb — hierarchical, not cyclic".
   */
  focusPath: string[];

  // ── Variables scope ──────────────────────────────────────────────
  /**
   * Dotted path the Variables pane is scoped to. `null` (or empty) means
   * "root" — show only top-level variables. Selecting a module or
   * subsystem in the SessionTree pushes a new segment; the breadcrumb in
   * the pane pops back up the tree. Variables whose name doesn't start
   * with any known topology module become invisible (stdlib filter).
   */
  selectedScope: string[];

  // Drill-from-verdict target (R3.5 receiver) — the single-hop
  // `drilledFrom` field that used to live here has moved to
  // `useInvestigationTrail` (features/investigation/useInvestigationTrail.ts),
  // a multi-hop breadcrumb store (ninebar Phase 1, audit F15). Its trail
  // is cleared from `setActiveSession` / `resetViewState` below so the
  // "cleared when the active session changes" semantic is preserved.

  // ── Step failure (P5) ──────────────────────────────────────────────
  /**
   * The most recent `sessions.step` failure (e.g. RS002 on a mistyped or
   * stale override target), surfaced by `useSessionController`'s catch
   * paths. `null` when there is nothing to show. Drives a dismissable
   * error banner — a failed step (and the draft override it drained,
   * single-shot) must never disappear silently into the console.
   */
  stepError: { message: string } | null;

  // ── Step-loop configuration ───────────────────────────────────────
  stepsPerSecond: number;

  /** Backend simulation step size in milliseconds (passed to orchestrate.workspace.start). */
  dtMs: number;

  /**
   * Scenario overrides staged for the NEXT session's construction, in the
   * order the user entered them. Passed to `sessions.create`, so they hold
   * from that session's first tick.
   *
   * Deliberately NOT `draftOverrides` and deliberately NOT per-session view
   * state. A draft override drains into the next STEP, so ticks already run
   * keep the old value — it edits a run in flight; only a create-time
   * override can be quoted as the scenario a whole trajectory was produced
   * under. And it lives here beside `dtMs` rather than in `SessionViewState`
   * because `resetViewState()` fires on session switch: staging severe,
   * creating the run, and then seeing Configure report "None" would read as
   * if the run were not severe. It is run configuration, and it persists
   * until the user changes it.
   */
  scenarioOverrides: [string, string][];

  // ── Actions ───────────────────────────────────────────────────────
  setActiveSession: (id: string | null) => void;
  setSelectedTarget: (id: string | null) => void;
  setPhase: (phase: SessionPhase) => void;

  /** Transition to a new phase only if the current phase is in `from`. */
  transitionPhase: (from: SessionPhase[], to: SessionPhase) => void;

  setMonitoredVariables: (vars: string[]) => void;
  toggleMonitoredVariable: (name: string) => void;
  setCompareBaseline: (id: string | null) => void;
  setDraftOverride: (name: string, value: string) => void;
  clearDraftOverrides: () => void;
  /** Replace the staged create-time scenario wholesale (Configure edits it). */
  setScenarioOverrides: (pairs: [string, string][]) => void;
  setStepsPerSecond: (hz: number) => void;
  setDtMs: (ms: number) => void;

  /** Push a subaction name onto the breadcrumb trail. */
  pushFocusedAction: (name: string) => void;

  /**
   * Replace the model-tree focus path. Pass `[]` to focus the root.
   * Consumers always pass a complete path (e.g. the full prefix to the
   * clicked node), never a single segment to append.
   */
  setFocusPath: (path: readonly string[]) => void;

  /**
   * Truncate the focus path to the given depth (0 = root). Used by the
   * breadcrumb to navigate up without the caller having to read the
   * current path.
   */
  navigateFocusToDepth: (depth: number) => void;

  /** Convenience for `setFocusPath([])`. */
  clearFocusPath: () => void;

  /** Replace the Variables-pane scope path (empty = root). */
  setSelectedScope: (path: string[]) => void;
  /** Truncate scope to the given depth (0 = root). */
  popSelectedScopeTo: (depth: number) => void;

  /** Navigate to a specific depth in the breadcrumb trail (0 = root). */
  navigateToActionDepth: (depth: number) => void;

  /** Reset all view state (e.g., when switching sessions). */
  resetViewState: () => void;

  /** Record a `sessions.step` failure for the error banner. */
  setStepError: (message: string) => void;
  /** Dismiss the step-error banner. */
  clearStepError: () => void;
}

const DEFAULT_VIEW_STATE: SessionViewState = {
  monitoredVariables: [],
  compareBaseline: null,
  draftOverrides: {},
};

export const useSessionStore = create<SessionStoreState>((set) => ({
  activeSessionId: null,
  selectedTargetId: null,
  phase: 'idle',
  focusedActionPath: [],
  focusPath: [],
  selectedScope: [],
  stepsPerSecond: 10,
  dtMs: 1.0,
  scenarioOverrides: [],
  stepError: null,
  ...DEFAULT_VIEW_STATE,

  setActiveSession: (id) =>
    // Reset scope + action breadcrumb when switching sessions so a
    // "circuit1.breaker" scope from the prior session doesn't carry
    // into a model where those names don't exist. Also clears the
    // investigation trail (formerly `drilledFrom`) for the same reason —
    // a drill breadcrumb from the previous session's evidence is
    // meaningless once the active session changes.
    set((s) => {
      const changed = id !== s.activeSessionId;
      if (changed) useInvestigationTrail.getState().clear();
      return {
        activeSessionId: id,
        selectedScope: changed ? [] : s.selectedScope,
        focusedActionPath: changed ? [] : s.focusedActionPath,
        focusPath: changed ? [] : s.focusPath,
      };
    }),
  setSelectedTarget: (id) => set({ selectedTargetId: id }),
  setPhase: (phase) => set({ phase }),

  transitionPhase: (from, to) =>
    set((s) => (from.includes(s.phase) ? { phase: to } : s)),

  setMonitoredVariables: (vars) => set({ monitoredVariables: vars }),
  toggleMonitoredVariable: (name) =>
    set((s) => ({
      monitoredVariables: s.monitoredVariables.includes(name)
        ? s.monitoredVariables.filter((v) => v !== name)
        : [...s.monitoredVariables, name],
    })),
  setCompareBaseline: (id) => set({ compareBaseline: id }),
  setDraftOverride: (name, value) =>
    set((s) => ({ draftOverrides: { ...s.draftOverrides, [name]: value } })),
  clearDraftOverrides: () => set({ draftOverrides: {} }),
  setScenarioOverrides: (pairs) => set({ scenarioOverrides: pairs }),
  setStepsPerSecond: (hz) => set({ stepsPerSecond: hz }),
  setDtMs: (ms) => {
    if (!Number.isFinite(ms)) return;
    const clamped = Math.min(1000, Math.max(0.001, ms));
    set({ dtMs: clamped });
  },

  pushFocusedAction: (name) =>
    set((s) => ({ focusedActionPath: [...s.focusedActionPath, name] })),

  setFocusPath: (path) => set({ focusPath: [...path] }),

  navigateFocusToDepth: (depth) =>
    set((s) => ({
      focusPath: s.focusPath.slice(0, Math.max(0, depth)),
    })),

  clearFocusPath: () => set({ focusPath: [] }),

  navigateToActionDepth: (depth) =>
    set((s) => ({ focusedActionPath: s.focusedActionPath.slice(0, depth) })),

  setSelectedScope: (path) => set({ selectedScope: path }),
  popSelectedScopeTo: (depth) =>
    set((s) => ({ selectedScope: s.selectedScope.slice(0, depth) })),

  resetViewState: () => {
    useInvestigationTrail.getState().clear();
    set({
      ...DEFAULT_VIEW_STATE,
      focusedActionPath: [],
      focusPath: [],
      selectedScope: [],
      phase: 'idle',
      stepError: null,
    });
  },

  setStepError: (message) => set({ stepError: { message } }),
  clearStepError: () => set({ stepError: null }),
}));
