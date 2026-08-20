/**
 * sessionLiveStore — streamed normalized-snapshot mirror + selectors.
 *
 * The Run workflow (and any future streaming consumer) feeds this store
 * from `useSessionStream.ts`. Components subscribe to individual keys
 * via `useVar('T_busbar')` / `useSubsystemState('breaker_1')` /
 * `useVerdictRollup()`, so only the components reading a changed key
 * re-render each tick.
 *
 * Wire contract: matches the Rust `NormalizedSnapshot` /
 * `DeltaFrame` / `VerdictRollup` / `SessionFrame` shapes from
 * `crates/lang/sysml-runtime/src/{snapshot_view,snapshot_diff,aggregates,session_events}.rs`.
 */
import { create } from 'zustand';

// ── Wire types (mirror Rust sysml-runtime) ──────────────────────────

export interface SubsystemView {
  current_state: string;
  completed: boolean;
  kind_label: string;
  /** Eligible transitions from `current_state` as `[event_name, target_state]`.
   *  Optional on the TS side because older cached frames may pre-date the
   *  field (the Rust struct is non-optional with `#[serde(default)]`, so
   *  fresh frames always carry at least `[]`). */
  available_transitions?: Array<[string, string]>;
  /** `ElementId` of the subsystem's source element (StateUsage /
   *  StateDefinition / ODE owner). Used by `mergeLiveState` to do an
   *  id-keyed lookup so nested-scope name collisions do not silently
   *  associate the wrong subsystem to a tree node. Optional because
   *  older cached frames lack the field and legacy subsystems with no
   *  recorded element id come through without it. */
  element_id?: string;
}

/** Four-valued verdict from the standard library's `VerdictKind`
 *  (`VerificationCases.sysml`). A constraint the run could not decide —
 *  an unbound parameter, a non-boolean result — is `Inconclusive`, never
 *  `Fail`; `Fail` means the run evaluated it and it did not hold.
 *
 *  PascalCase because snapshot rows carry the serde-derived spelling, the
 *  same one `diagram-svg/viewmodel-types.ts` uses. (`engine/types.ts`
 *  spells the same enum lowercase for the surfaces that go through the
 *  Rust `Display` impl instead — an existing split, not a new one.) */
export type ConstraintVerdictKind = 'Pass' | 'Fail' | 'Inconclusive' | 'Error';

export interface ConstraintView {
  name: string;
  expression: string | null;
  verdict: ConstraintVerdictKind;
  /** Live scalar values of every identifier referenced by the constraint
   *  expression at the tick it was evaluated (GAP-CONSTR-002). Optional
   *  on the TS side — older cached frames and non-numeric operand sets
   *  come through as absent / empty. */
  operands?: Record<string, number>;
  /** `ElementId` of the constraint usage in the model graph. Used by
   *  `mergeLiveState` to do an id-keyed lookup so nested-scope name
   *  collisions do not silently associate the wrong constraint result
   *  to a tree node. Optional because older cached frames lack the
   *  field and constraints whose IR has no `owner_id` come through
   *  without it. */
  element_id?: string;
}

export interface NormalizedSnapshot {
  tick: number;
  time_ms: number;
  completed: boolean;
  subsystems: Record<string, SubsystemView>;
  scalar_vars: Record<string, number>;
  string_vars: Record<string, string>;
  constraint_results: ConstraintView[];
  /** Live port feature scalars keyed by `owner.port` → `feature` →
   *  `f64`. Omitted when the orchestrator has no `PortRegistry`
   *  (GAP-FLOW-001). Optional on the TS side because older cached
   *  frames may pre-date the field. */
  port_values?: Record<string, Record<string, number>>;
  /** Instantaneous `dy/dt` for every ODE state variable at this tick,
   *  keyed by the same name the state value uses in `scalar_vars`
   *  (GAP-ODE-002). Optional because older cached frames lack it. */
  derivatives?: Record<string, number>;
}

export interface DeltaFrame {
  tick: number;
  time_ms: number;
  completed: boolean;
  scalar_changed?: Record<string, number>;
  scalar_removed?: string[];
  string_changed?: Record<string, string>;
  string_removed?: string[];
  subsystem_changed?: Record<string, SubsystemView>;
  subsystem_removed?: string[];
  constraint_results?: ConstraintView[] | null;
  port_values_changed?: Record<string, Record<string, number>>;
  port_values_removed?: string[];
  derivatives_changed?: Record<string, number>;
  derivatives_removed?: string[];
}

export interface VerdictRollup {
  pass: number;
  fail: number;
  inconclusive: number;
  error: number;
}

export type StreamPhase =
  | 'idle'
  | 'connecting'
  | 'open'
  | 'closed'
  | 'error';

// ── Store shape ──────────────────────────────────────────────────────

interface SessionLiveState {
  /** Session id the live mirror is tracking. `null` when no stream is active. */
  sessionId: string | null;
  /** The current reconstructed snapshot (never `null` once Hello arrives). */
  snapshot: NormalizedSnapshot | null;
  /** Most recent verdict rollup, or `null` when no verdict frame has arrived. */
  verdicts: VerdictRollup | null;
  /** WS connection phase for UI feedback. */
  phase: StreamPhase;
  /** Last tick we received (for reconnect `?since=` cursor). */
  lastTick: number | null;
  /** Last error message, or `null` when none. */
  lastError: string | null;

  // Actions (invoked by useSessionStream).
  applyHello: (sessionId: string, base: NormalizedSnapshot) => void;
  applyTick: (delta: DeltaFrame) => void;
  applyVerdict: (tick: number, verdicts: VerdictRollup) => void;
  markCompleted: (tick: number, timeMs: number) => void;
  setPhase: (phase: StreamPhase) => void;
  setError: (message: string | null) => void;
  reset: () => void;
}

function emptySnapshot(): NormalizedSnapshot {
  return {
    tick: 0,
    time_ms: 0,
    completed: false,
    subsystems: {},
    scalar_vars: {},
    string_vars: {},
    constraint_results: [],
  };
}

/**
 * Apply a delta to a base snapshot in place, producing the next one.
 * Exported so tests can exercise the reducer without spinning up a store.
 */
export function applyDelta(
  base: NormalizedSnapshot,
  delta: DeltaFrame,
): NormalizedSnapshot {
  const next: NormalizedSnapshot = {
    tick: delta.tick,
    time_ms: delta.time_ms,
    completed: delta.completed,
    subsystems: { ...base.subsystems },
    scalar_vars: { ...base.scalar_vars },
    string_vars: { ...base.string_vars },
    constraint_results:
      delta.constraint_results === undefined || delta.constraint_results === null
        ? base.constraint_results
        : delta.constraint_results,
    port_values: base.port_values ? { ...base.port_values } : undefined,
    derivatives: base.derivatives ? { ...base.derivatives } : undefined,
  };

  if (delta.scalar_changed) {
    for (const [k, v] of Object.entries(delta.scalar_changed)) {
      next.scalar_vars[k] = v;
    }
  }
  if (delta.scalar_removed) {
    for (const k of delta.scalar_removed) delete next.scalar_vars[k];
  }
  if (delta.string_changed) {
    for (const [k, v] of Object.entries(delta.string_changed)) {
      next.string_vars[k] = v;
    }
  }
  if (delta.string_removed) {
    for (const k of delta.string_removed) delete next.string_vars[k];
  }
  if (delta.subsystem_changed) {
    for (const [k, v] of Object.entries(delta.subsystem_changed)) {
      next.subsystems[k] = v;
    }
  }
  if (delta.subsystem_removed) {
    for (const k of delta.subsystem_removed) delete next.subsystems[k];
  }
  if (delta.port_values_changed || delta.port_values_removed) {
    // Lazy-create the port map on first delta that touches it so the
    // base snapshot's shape is preserved (undefined → stays undefined
    // when there's nothing to apply).
    const pv = next.port_values ?? {};
    if (delta.port_values_changed) {
      for (const [k, v] of Object.entries(delta.port_values_changed)) {
        pv[k] = v;
      }
    }
    if (delta.port_values_removed) {
      for (const k of delta.port_values_removed) delete pv[k];
    }
    next.port_values = pv;
  }
  if (delta.derivatives_changed || delta.derivatives_removed) {
    const d = next.derivatives ?? {};
    if (delta.derivatives_changed) {
      for (const [k, v] of Object.entries(delta.derivatives_changed)) {
        d[k] = v;
      }
    }
    if (delta.derivatives_removed) {
      for (const k of delta.derivatives_removed) delete d[k];
    }
    next.derivatives = d;
  }
  return next;
}

export const useSessionLiveStore = create<SessionLiveState>((set, get) => ({
  sessionId: null,
  snapshot: null,
  verdicts: null,
  phase: 'idle',
  lastTick: null,
  lastError: null,

  applyHello: (sessionId, base) => {
    set({
      sessionId,
      snapshot: base,
      lastTick: base.tick,
      phase: 'open',
      lastError: null,
      // Hello replaces everything — reset verdict rollup too so the next
      // Verdict frame (or the base's constraint rows at Tick 1) rebuilds it.
      verdicts: null,
    });
  },

  applyTick: (delta) => {
    const base = get().snapshot ?? emptySnapshot();
    const next = applyDelta(base, delta);
    set({ snapshot: next, lastTick: next.tick });
  },

  applyVerdict: (_tick, verdicts) => set({ verdicts }),

  markCompleted: (tick, timeMs) => {
    const snap = get().snapshot;
    if (snap) {
      set({
        snapshot: { ...snap, tick, time_ms: timeMs, completed: true },
        lastTick: tick,
      });
    }
  },

  setPhase: (phase) => set({ phase }),
  setError: (message) => set({ lastError: message }),

  reset: () => set({
    sessionId: null,
    snapshot: null,
    verdicts: null,
    phase: 'idle',
    lastTick: null,
    lastError: null,
  }),
}));

// ── Key-level selectors ─────────────────────────────────────────────

/** Subscribe to one scalar variable; component re-renders only when it changes. */
export function useVar(name: string): number | undefined {
  return useSessionLiveStore((s) => s.snapshot?.scalar_vars[name]);
}

/** Subscribe to one string variable. */
export function useStringVar(name: string): string | undefined {
  return useSessionLiveStore((s) => s.snapshot?.string_vars[name]);
}

/** Subscribe to one subsystem's view. */
export function useSubsystem(name: string): SubsystemView | undefined {
  return useSessionLiveStore((s) => s.snapshot?.subsystems[name]);
}

/** Subscribe to one subsystem's state name (narrower than useSubsystem). */
export function useSubsystemState(name: string): string | undefined {
  return useSessionLiveStore(
    (s) => s.snapshot?.subsystems[name]?.current_state,
  );
}

/** Subscribe to the current verdict rollup. */
export function useVerdictRollup(): VerdictRollup | null {
  return useSessionLiveStore((s) => s.verdicts);
}

/** Subscribe to the current tick (cheap scalar for timers / status bars). */
export function useTick(): number | null {
  return useSessionLiveStore((s) => s.snapshot?.tick ?? null);
}

/** Subscribe to the current simulation time (ms). */
export function useTimeMs(): number | null {
  return useSessionLiveStore((s) => s.snapshot?.time_ms ?? null);
}

/** Subscribe to connection phase (for status bar / banner). */
export function useStreamPhase(): StreamPhase {
  return useSessionLiveStore((s) => s.phase);
}

/** Subscribe to last error message (for UI feedback). */
export function useStreamError(): string | null {
  return useSessionLiveStore((s) => s.lastError);
}
