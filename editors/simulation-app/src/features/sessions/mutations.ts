/**
 * React-query mutations for session write operations.
 *
 * Each mutation invalidates the relevant query keys on success per ADR-004:
 * - Step: invalidate ['sessions'] (list) and ['session', id] (detail).
 *   Also writes the returned SessionSummary directly into the cache to
 *   avoid an extra round-trip.
 * - Start/stop/reset/fork/inject: invalidate ['sessions'] and ['session', id].
 */

import { useMutation, useQueryClient } from '@tanstack/react-query';
import { httpPost } from '../../shared/api/http';
import { sessionKeys } from './queries';
import type { CreateSessionParams } from './sessionCreate';
import type { SessionSummary } from './types';

// ── API helper ────────────────────────────────────────────────────────

function cmd<T>(command: string, params: Record<string, unknown> = {}): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

// ── Mutations ─────────────────────────────────────────────────────────

/**
 * Create a session via the unified `sysml.sessions.create` entry point.
 *
 * The server infers the `SessionKind` from the model + optional `target` and
 * always returns one `SessionSummary` shape — the client no longer picks a
 * kind-specific `*.start` command or decodes divergent response shapes (see
 * `createParamsForTarget` to derive the params from a run target.
 */
export function useCreateSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: CreateSessionParams) => {
      const args: Record<string, unknown> = { uri: params.uri };
      if (params.target) args.target = params.target;
      if (params.dtMs !== undefined) args.dt_ms = params.dtMs;
      if (params.maxTimeMs !== undefined) args.max_time_ms = params.maxTimeMs;
      // Wire shape is a list of [key, value] pairs (Rust `[(String, String)]`).
      if (params.overrides && params.overrides.length > 0) {
        args.overrides = params.overrides;
      }
      return cmd<SessionSummary>('sysml.sessions.create', args);
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: sessionKeys.lists() });
    },
  });
}

/**
 * Step a session forward by one tick.
 * Writes the returned SessionSummary into the detail cache.
 *
 * Always dispatches the unified `sysml.sessions.step` command. The
 * backend's alias-aware override path translates canonical tree-path
 * keys (e.g. `ProductionCell.station5.thermalModel.flow`) into the
 * runtime-prefix alias (`station5.flow`) the scoped subsystem
 * sync_in actually reads. Overrides are typically drained from the
 * session store's `draftOverrides` map by the controller and this
 * tick consumes them once.
 */
export function useStepSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: {
      sessionId: string;
      event?: string;
      overrides?: [string, string][];
      /**
       * Advance this many ticks server-side in one call (default 1).
       * Capped at MAX_BULK_STEP_TICKS (20_000) — a larger value is an
       * InvalidInput error, never silently clamped. Used by the
       * controller's fast-forward loop to reach far-off events (e.g. the
       * hybrid trip ~132k ticks out) without a per-tick round-trip. Existing
       * single-tick callers pass no `ticks` and are unaffected.
       */
      ticks?: number;
    }) => {
      const args: Record<string, unknown> = {
        session_id: params.sessionId,
        overrides: params.overrides ?? [],
      };
      if (params.event !== undefined) args.event = params.event;
      if (params.ticks !== undefined) args.ticks = params.ticks;
      return cmd<SessionSummary>('sysml.sessions.step', args);
    },
    onSuccess: (data, variables) => {
      // Write returned summary directly into cache to skip a poll round-trip.
      qc.setQueryData(sessionKeys.detail(variables.sessionId), (old: unknown) => {
        if (old && typeof old === 'object' && 'summary' in old) {
          return { ...(old as Record<string, unknown>), summary: data };
        }
        return old;
      });
      void qc.invalidateQueries({ queryKey: sessionKeys.lists() });
    },
  });
}

/**
 * Stop a session.
 */
export function useStopSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (sessionId: string) =>
      cmd<void>('sysml.sessions.stop', { session_id: sessionId }),
    onSuccess: (_data, sessionId) => {
      void qc.invalidateQueries({ queryKey: sessionKeys.lists() });
      void qc.invalidateQueries({ queryKey: sessionKeys.detail(sessionId) });
    },
  });
}

/**
 * Clear a session's breakpoint-pause flag (BP2 `sysml.sessions.resume`).
 *
 * Idempotent server-side — a no-op success when the session isn't
 * currently paused — so callers can fire this unconditionally from a
 * "Resume" action rather than branching on whether the pause in front
 * of them came from a breakpoint or a plain user Pause click. Does NOT
 * itself advance any ticks; a subsequent `sessions.step` (or the
 * autoplay loop) is what actually moves the session forward.
 */
export function useResumeSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (sessionId: string) =>
      cmd<SessionSummary>('sysml.sessions.resume', { session_id: sessionId }),
    onSuccess: (data, sessionId) => {
      qc.setQueryData(sessionKeys.detail(sessionId), (old: unknown) => {
        if (old && typeof old === 'object' && 'summary' in old) {
          return { ...(old as Record<string, unknown>), summary: data };
        }
        return old;
      });
      void qc.invalidateQueries({ queryKey: sessionKeys.lists() });
    },
  });
}

/**
 * Reset a session to initial state.
 */
export function useResetSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (sessionId: string) =>
      cmd<SessionSummary>('sysml.sessions.reset', { session_id: sessionId }),
    onSuccess: (data, sessionId) => {
      qc.setQueryData(sessionKeys.detail(sessionId), (old: unknown) => {
        if (old && typeof old === 'object' && 'summary' in old) {
          return { ...(old as Record<string, unknown>), summary: data };
        }
        return old;
      });
      void qc.invalidateQueries({ queryKey: sessionKeys.lists() });
    },
  });
}

/**
 * Fork a session (deep clone, optionally rewound to a past tick and
 * with parameter overrides applied atomically).
 *
 * `atTick` — when supplied, the child starts at that tick (rewound
 * from the parent's recorded trace). Without it the fork happens at
 * the parent's current tick. Backend `fork_with_overrides` is the
 * only fork command that accepts `at_tick`, so any past-tick fork
 * routes through it even when the override list is empty.
 */
export function useForkSession() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: {
      sessionId: string;
      overrides?: [string, string][];
      atTick?: number;
    }) => {
      const wantsRewind = typeof params.atTick === 'number';
      const wantsOverrides = !!params.overrides && params.overrides.length > 0;
      if (wantsRewind || wantsOverrides) {
        const args: Record<string, unknown> = {
          session_id: params.sessionId,
          overrides: params.overrides ?? [],
        };
        if (wantsRewind) args.at_tick = params.atTick;
        return cmd<SessionSummary>('sysml.sessions.fork_with_overrides', args);
      }
      return cmd<SessionSummary>('sysml.sessions.fork', {
        session_id: params.sessionId,
      });
    },
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: sessionKeys.lists() });
    },
  });
}

/**
 * Drop every expired session across all buckets (`sysml.sessions.reap`).
 *
 * No params — reaps globally, matching the backend's `sessions_reap`
 * (`crates/tooling/sysml-service/src/lib.rs`), which just returns the
 * count removed. Backs the frame `SessionSwitcherChip`'s "Clear stale"
 * row (ninebar Phase 1, audit F2) — invalidates the session list so
 * expired rows disappear immediately instead of waiting for the next
 * 1 Hz poll.
 */
export function useReapSessions() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: () => cmd<number>('sysml.sessions.reap'),
    onSuccess: () => {
      void qc.invalidateQueries({ queryKey: sessionKeys.lists() });
    },
  });
}

/**
 * Inject an event into a specific subsystem.
 */
export function useInjectEvent() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (params: {
      sessionId: string;
      subsystem: string;
      event: string;
    }) =>
      cmd<SessionSummary>('sysml.sessions.inject', {
        session_id: params.sessionId,
        subsystem: params.subsystem,
        event: params.event,
      }),
    onSuccess: (data, variables) => {
      qc.setQueryData(sessionKeys.detail(variables.sessionId), (old: unknown) => {
        if (old && typeof old === 'object' && 'summary' in old) {
          return { ...(old as Record<string, unknown>), summary: data };
        }
        return old;
      });
      void qc.invalidateQueries({ queryKey: sessionKeys.lists() });
    },
  });
}
