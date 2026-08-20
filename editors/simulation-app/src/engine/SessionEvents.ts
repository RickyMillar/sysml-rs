/**
 * R1.5 — SessionEvents stream (Layer 2 / E4).
 *
 * Subscribable event stream derived from polled `SessionDetail` snapshots.
 * Every workflow (Run, Verify, Sweep, Compare, …) subscribes here rather
 * than re-implementing change detection over `sysml.sessions.info`.
 *
 * TRANSPORT DECISION: POLLING
 * ───────────────────────────
 * Chosen over SSE for v1 because:
 *   1. The frontend already polls `sysml.sessions.info` at 10 Hz during
 *      playback (see `features/sessions/queries.ts::useSessionDetail`).
 *   2. Deriving events from snapshot diffs requires zero backend changes
 *      and lets us ship the contract while Agent B lands breakpoint
 *      metadata and Agent C lands verdict structure.
 *   3. The `Unsub` / kind-based `on(...)` contract is transport-agnostic,
 *      so swapping in SSE later is an internal refactor — the callsite
 *      shape is stable. SSE is a future optimization (see "Future work").
 *
 * Shape:
 *   class SessionEventBus — the plain (non-React) bus. Holds one poller
 *     per session and refcounts subscribers. When the last subscriber
 *     for a session unsubs, polling stops for that session.
 *
 *   useSessionEvents(sessionId, kind, cb) — React hook wrapper for
 *     subscribing in component trees. Handles cleanup on unmount.
 *
 * Event kinds (derived from snapshot diffs):
 *   - tick           — new snapshot with monotonically-greater tick
 *   - transition     — any subsystem's currentState changed between ticks
 *   - breakpoint-hit — phase changed to 'paused' AND `summary.
 *                      paused_at_breakpoint` (BP1) carries a fresh
 *                      BreakpointId — see `extractBreakpointMarker`
 *   - verdict-flip   — a constraint result's satisfied flag flipped
 *   - completed      — summary.completed transitioned false -> true
 *   - error          — phase transitioned to 'error'
 *
 * Future work:
 *   - SSE endpoint (`/sysml/sessions/{key}/events`) for sub-100ms latency.
 *   - `typed-events` payload: include diff context (from -> to state, which
 *     constraint flipped) so subscribers don't have to re-query detail.
 *   - Replay-from-tick for late subscribers.
 */

import { useEffect, useRef } from 'react';
import type {
  SessionDetail,
  SessionPhase,
} from '../features/sessions/types';
import type { SessionId } from './types';

// ── Public types ─────────────────────────────────────────────────────

/** Event kinds carried by the stream. */
export type SessionEventKind =
  | 'tick'
  | 'transition'
  | 'breakpoint-hit'
  | 'verdict-flip'
  | 'completed'
  | 'error';

/** Payload for a single event delivery. */
export interface SessionEvent {
  session: SessionId;
  kind: SessionEventKind;
  tick: number;
  timeMs: number;
  /**
   * Optional structured context (event-kind specific):
   *   transition:     { subsystem, fromState, toState }
   *   breakpoint-hit: { breakpointId?, target? } (echo of backend metadata)
   *   verdict-flip:   { constraint, fromSatisfied, toSatisfied }
   *   completed/error: undefined
   *   tick:           undefined (use `tick`/`timeMs` fields directly)
   */
  context?: Record<string, unknown>;
}

/** Subscriber callback. */
export type SessionEventHandler = (event: SessionEvent) => void;

/** Unsubscribe thunk returned by `on(...)`. */
export type Unsub = () => void;

/**
 * Public subscribable surface. Named `SessionEvents` to match the
 * extensibility plan § Layer 2 E4 contract exactly.
 */
export interface SessionEvents {
  on(
    session: SessionId,
    kind: SessionEventKind,
    cb: SessionEventHandler,
  ): Unsub;
}

/**
 * Minimal snapshot source — anything that can hand back the latest
 * `SessionDetail` for a session key. Lets us swap in a React Query cache
 * view, a mock, or a direct backend client interchangeably.
 */
export interface SnapshotSource {
  /** Latest observed detail, or null if none has been polled yet. */
  getDetail(session: SessionId): SessionDetail | null;
  /** Current phase as known to the frontend (drives breakpoint-hit / error). */
  getPhase(session: SessionId): SessionPhase | null;
}

/** Timer abstraction so tests can inject fake timers deterministically. */
export interface Scheduler {
  setInterval: (fn: () => void, ms: number) => unknown;
  clearInterval: (handle: unknown) => void;
}

const DEFAULT_SCHEDULER: Scheduler = {
  setInterval: (fn, ms) => globalThis.setInterval(fn, ms),
  clearInterval: (handle) =>
    globalThis.clearInterval(handle as ReturnType<typeof setInterval>),
};

/** Default poll cadence. 250ms is a compromise between latency and load. */
export const DEFAULT_POLL_INTERVAL_MS = 250;

// ── Internals ────────────────────────────────────────────────────────

/**
 * Per-session subscriber registry + last-seen snapshot used for diffing.
 */
interface SessionWatch {
  subscribers: Map<SessionEventKind, Set<SessionEventHandler>>;
  lastTick: number | null;
  lastTimeMs: number;
  lastSubsystems: Record<string, string>; // subsystem name -> currentState
  lastConstraints: Record<string, boolean>; // constraint name -> satisfied
  lastPhase: SessionPhase | null;
  lastCompleted: boolean;
  lastBreakpointMarker: unknown; // sentinel; new reference = new hit
  timer: unknown | null;
}

function newWatch(): SessionWatch {
  return {
    subscribers: new Map(),
    lastTick: null,
    lastTimeMs: 0,
    lastSubsystems: {},
    lastConstraints: {},
    lastPhase: null,
    lastCompleted: false,
    lastBreakpointMarker: undefined,
    timer: null,
  };
}

// ── SessionEventBus — the plain bus class ────────────────────────────

/**
 * Refcounted polling event bus. Starts polling a session when the first
 * subscriber for any kind appears; stops when the last one leaves.
 *
 * Not a React thing. React wrapper below (`useSessionEvents`) is thin.
 */
export class SessionEventBus implements SessionEvents {
  private readonly watches = new Map<SessionId, SessionWatch>();
  private readonly source: SnapshotSource;
  private readonly scheduler: Scheduler;
  private readonly intervalMs: number;

  constructor(opts: {
    source: SnapshotSource;
    scheduler?: Scheduler;
    intervalMs?: number;
  }) {
    this.source = opts.source;
    this.scheduler = opts.scheduler ?? DEFAULT_SCHEDULER;
    this.intervalMs = opts.intervalMs ?? DEFAULT_POLL_INTERVAL_MS;
  }

  on(
    session: SessionId,
    kind: SessionEventKind,
    cb: SessionEventHandler,
  ): Unsub {
    const watch = this.watches.get(session) ?? newWatch();
    if (!this.watches.has(session)) this.watches.set(session, watch);

    const set = watch.subscribers.get(kind) ?? new Set();
    set.add(cb);
    watch.subscribers.set(kind, set);

    // Seed baseline from whatever's already in the cache so the first
    // real poll doesn't spuriously fire "change" events for ambient state.
    if (watch.timer === null) {
      this.seed(session, watch);
      watch.timer = this.scheduler.setInterval(
        () => this.tick(session),
        this.intervalMs,
      );
    }

    return () => this.off(session, kind, cb);
  }

  /**
   * Manually force a poll cycle. Useful for tests and for callers who
   * already know fresh data just arrived (e.g. after a mutation).
   */
  poll(session: SessionId): void {
    this.tick(session);
  }

  /** Stop every watcher (test helper, also used on shell unmount). */
  disposeAll(): void {
    for (const [session, watch] of this.watches) {
      if (watch.timer !== null) this.scheduler.clearInterval(watch.timer);
      watch.timer = null;
      watch.subscribers.clear();
      this.watches.delete(session);
    }
  }

  // ── internal ────────────────────────────────────────────────────

  private off(
    session: SessionId,
    kind: SessionEventKind,
    cb: SessionEventHandler,
  ): void {
    const watch = this.watches.get(session);
    if (!watch) return;
    const set = watch.subscribers.get(kind);
    if (set) {
      set.delete(cb);
      if (set.size === 0) watch.subscribers.delete(kind);
    }

    // If no subscribers remain for this session, stop polling.
    if (watch.subscribers.size === 0) {
      if (watch.timer !== null) this.scheduler.clearInterval(watch.timer);
      this.watches.delete(session);
    }
  }

  /** Populate baseline from latest known detail without firing events. */
  private seed(session: SessionId, watch: SessionWatch): void {
    const detail = this.source.getDetail(session);
    const phase = this.source.getPhase(session);
    if (detail) {
      watch.lastTick = detail.summary.tick;
      watch.lastCompleted = detail.summary.completed;
      watch.lastSubsystems = Object.fromEntries(
        detail.subsystems.map((s) => [s.name, s.current_state]),
      );
      watch.lastConstraints = collectConstraints(detail.latest_snapshot);
      watch.lastBreakpointMarker = extractBreakpointMarker(detail.summary);
    }
    watch.lastPhase = phase;
  }

  private emit(
    session: SessionId,
    watch: SessionWatch,
    kind: SessionEventKind,
    tick: number,
    timeMs: number,
    context?: Record<string, unknown>,
  ): void {
    const set = watch.subscribers.get(kind);
    if (!set || set.size === 0) return;
    const ev: SessionEvent = { session, kind, tick, timeMs, context };
    // Copy to array so handler unsubs don't corrupt iteration.
    for (const cb of Array.from(set)) {
      try {
        cb(ev);
      } catch (err) {
        // eslint-disable-next-line no-console
        console.error('[SessionEvents] subscriber threw:', err);
      }
    }
  }

  /** One poll cycle — diff last-seen vs current and emit events. */
  private tick(session: SessionId): void {
    const watch = this.watches.get(session);
    if (!watch) return;

    const detail = this.source.getDetail(session);
    const phase = this.source.getPhase(session);
    if (!detail) {
      // No snapshot yet — just track phase so error/pause still fire.
      this.dispatchPhaseOnly(session, watch, phase);
      return;
    }

    const curTick = detail.summary.tick;
    const curTimeMs = detail.summary.time_ms;
    const curCompleted = detail.summary.completed;
    const curSubsystems = Object.fromEntries(
      detail.subsystems.map((s) => [s.name, s.current_state]),
    );
    const curConstraints = collectConstraints(detail.latest_snapshot);
    const curBreakpointMarker = extractBreakpointMarker(detail.summary);

    // 1. tick — tick counter advanced.
    if (watch.lastTick !== null && curTick > watch.lastTick) {
      this.emit(session, watch, 'tick', curTick, curTimeMs);
    }

    // 2. transition — any subsystem's current state changed.
    if (watch.lastTick !== null) {
      for (const [name, toState] of Object.entries(curSubsystems)) {
        const fromState = watch.lastSubsystems[name];
        if (fromState !== undefined && fromState !== toState) {
          this.emit(session, watch, 'transition', curTick, curTimeMs, {
            subsystem: name,
            fromState,
            toState,
          });
        }
      }
    }

    // 3. verdict-flip — any constraint's satisfied flag changed.
    if (watch.lastTick !== null) {
      for (const [name, toSat] of Object.entries(curConstraints)) {
        const fromSat = watch.lastConstraints[name];
        if (fromSat !== undefined && fromSat !== toSat) {
          this.emit(session, watch, 'verdict-flip', curTick, curTimeMs, {
            constraint: name,
            fromSatisfied: fromSat,
            toSatisfied: toSat,
          });
        }
      }
    }

    // 4. completed — summary.completed flipped false -> true.
    if (!watch.lastCompleted && curCompleted) {
      this.emit(session, watch, 'completed', curTick, curTimeMs);
    }

    // 5. error — phase transitioned to 'error'.
    if (phase === 'error' && watch.lastPhase !== 'error') {
      this.emit(session, watch, 'error', curTick, curTimeMs);
    }

    // 6. breakpoint-hit — phase -> 'paused' AND a fresh BreakpointId is
    //    present on `summary.paused_at_breakpoint` (BP1).
    const justPaused = phase === 'paused' && watch.lastPhase !== 'paused';
    const markerChanged =
      curBreakpointMarker !== undefined &&
      curBreakpointMarker !== watch.lastBreakpointMarker;
    if (justPaused && markerChanged) {
      // `breakpointId` is the key `BreakpointsPanel`'s subscriber reads
      // (falls back to `context.id`) — keep this shape stable.
      this.emit(session, watch, 'breakpoint-hit', curTick, curTimeMs, {
        breakpointId: curBreakpointMarker,
      });
    }

    // Commit new baseline.
    watch.lastTick = curTick;
    watch.lastTimeMs = curTimeMs; // advisory; unused in diff
    watch.lastSubsystems = curSubsystems;
    watch.lastConstraints = curConstraints;
    watch.lastCompleted = curCompleted;
    watch.lastPhase = phase;
    watch.lastBreakpointMarker = curBreakpointMarker;
  }

  /**
   * Handle phase-only transitions when no detail snapshot is available
   * yet (first polls after session start).
   */
  private dispatchPhaseOnly(
    session: SessionId,
    watch: SessionWatch,
    phase: SessionPhase | null,
  ): void {
    if (phase === 'error' && watch.lastPhase !== 'error') {
      this.emit(session, watch, 'error', watch.lastTick ?? 0, 0);
    }
    watch.lastPhase = phase;
  }
}

// ── Helpers ──────────────────────────────────────────────────────────

function collectConstraints(
  snapshot: Record<string, unknown> | null,
): Record<string, boolean> {
  if (!snapshot) return {};
  const raw = snapshot.constraint_results as Array<Record<string, unknown>> | undefined;
  if (!Array.isArray(raw)) return {};
  const out: Record<string, boolean> = {};
  for (const c of raw) {
    const name = typeof c.name === 'string' ? c.name : undefined;
    if (!name) continue;
    out[name] = !!c.satisfied;
  }
  return out;
}

/**
 * Peek at the `paused_at_breakpoint` field on the session SUMMARY
 * (BP1) — NOT the snapshot. An earlier version of this function
 * speculatively read `latest_snapshot.paused_at_breakpoint` /
 * `.breakpoint_hit` before the backend field landed; the real field is
 * `SessionSummary.paused_at_breakpoint` (see `execution.rs`), so read
 * it from there. Treat `null`/absent identically (the backend
 * `skip_serializing_if`-omits the key when `None`) and normalize both
 * to `undefined` so `markerChanged` in `tick()` above doesn't fire on
 * "still not paused at anything".
 */
function extractBreakpointMarker(
  summary: Pick<SessionDetail['summary'], 'paused_at_breakpoint'> | null | undefined,
): string | undefined {
  return summary?.paused_at_breakpoint ?? undefined;
}

// ── Default singleton + React hook ───────────────────────────────────

let defaultBus: SessionEventBus | null = null;

/**
 * Install the process-wide default bus. The React hook reads from this
 * unless a caller passes a local bus explicitly. Safe to call multiple
 * times — previous bus is disposed.
 */
export function installSessionEventBus(bus: SessionEventBus): void {
  if (defaultBus) defaultBus.disposeAll();
  defaultBus = bus;
}

/** Read the currently-installed default bus (or null if uninstalled). */
export function getSessionEventBus(): SessionEventBus | null {
  return defaultBus;
}

/**
 * React hook — subscribe to a single (session, kind) pair for the
 * lifetime of the component. Uses the installed default bus; falls back
 * to a no-op if no bus has been installed (e.g. in tests that render
 * components without the engine provider).
 */
export function useSessionEvents(
  session: SessionId | null,
  kind: SessionEventKind,
  cb: SessionEventHandler,
): void {
  // Keep a latest-ref of `cb` so subscribers don't churn on render.
  const cbRef = useRef(cb);
  cbRef.current = cb;

  useEffect(() => {
    if (!session) return;
    const bus = getSessionEventBus();
    if (!bus) return;
    const unsub = bus.on(session, kind, (ev) => cbRef.current(ev));
    return unsub;
  }, [session, kind]);
}
