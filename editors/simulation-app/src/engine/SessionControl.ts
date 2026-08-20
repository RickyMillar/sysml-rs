/**
 * SessionControl — Layer 2 implementation of the DAP-shaped engine surface.
 *
 * Wraps the existing `useSessionController` (autoplay loop + step/retry),
 * the existing session mutations, and the (planned) breakpoint REST
 * endpoints behind the single `SessionControl` contract defined in
 * `./types.ts`.
 *
 * No backend logic lives here — this is a thin adapter layer that every
 * future workflow UI (Run, Verify, Sweep, Monte Carlo, Trade Study)
 * will call into. If a method needs backend data the current
 * controller doesn't surface, the gap is documented in the method body
 * rather than silently fixed.
 */

import { useCallback, useMemo } from 'react';
import { httpPost } from '../shared/api/http';
import { useSessionController } from '../features/sessions/useSessionController';
import {
  useCreateSession,
  useStopSession,
  useForkSession,
} from '../features/sessions/mutations';
import { useSessionStore } from '../features/sessions/store';
import {
  createParamsForTarget,
  type CreateSessionParams,
} from '../features/sessions/sessionCreate';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { useRunTargets } from '@/features/run-targets/queries';
import type { RunTargetSummary } from '@/features/run-targets/types';
import type {
  Breakpoint,
  BreakpointId,
  SessionControl,
  SessionStartConfig,
  SnapshotId,
  Overrides,
  Value,
} from './types';

// ── Pure helpers (exported for tests) ────────────────────────────────

/**
 * Translate a workflow-neutral `SessionStartConfig` into the parameters for
 * the unified `sysml.sessions.create` command.
 *
 * The server infers the session kind, so there is no command/capability
 * dispatch here:
 *   1. An explicit `config.uri` runs that URI (with the optional `config.target`
 *      element name — the server decides simulation / action / orchestrator).
 *   2. Otherwise fall back to the user's currently-selected run target.
 *
 * `config.overrides` are still NOT forwarded at creation, but the reason has
 * changed and is now a deliberate hold rather than a limitation.
 * `sessions.create` DOES accept create-time `overrides` as of the J3 scenario
 * work, and `SessionStartConfig.overrides` is documented as "initial parameter
 * overrides to apply at start" — so forwarding them is what the field means.
 *
 * The hold: every caller here is an analyze workflow (sweep / sensitivity),
 * where switching from per-tick to per-build application changes the numbers
 * those runs produce — the first tick of each variant currently executes at
 * the baseline value and would stop doing so. That is very likely a bug fix,
 * but it moves analysis output and belongs in its own change with its own
 * baselines re-blessed, not folded into a session-setup commit.
 *
 * Until then the controller drains staged `draftOverrides` into the next
 * `sysml.sessions.step` — atomic with the next step from the user's
 * perspective. The create-time path is reached through
 * `useSessionStore.scenarioOverrides` instead (Configure → Scenario).
 */
export function buildCreateParams(
  config: SessionStartConfig,
  target: RunTargetSummary | null,
  defaultDtMs: number,
): CreateSessionParams {
  const dtMs = config.dtMs ?? defaultDtMs;
  if (config.uri) {
    return { uri: config.uri, target: config.target, dtMs };
  }
  return createParamsForTarget(target, dtMs);
}

/**
 * Convert an `Overrides` map to the `[name, value]` tuple form the
 * existing fork/inject endpoints expect.
 */
export function overridesToTuples(overrides: Overrides): [string, string][] {
  return Object.entries(overrides).map(([k, v]) => [k, serializeValue(v)]);
}

/** Serialise a `Value` to the string form the backend override APIs expect. */
export function serializeValue(v: Value): string {
  if (v === null) return 'null';
  if (typeof v === 'string') return v;
  if (typeof v === 'number' || typeof v === 'boolean') return String(v);
  return JSON.stringify(v);
}

// ── Breakpoint REST client (pure — takes a poster for testability) ───

type HttpPoster = <T>(path: string, body?: unknown) => Promise<T>;

/**
 * Serialise a frontend `Breakpoint` to the wire shape
 * `sysml-runtime::breakpoint::Breakpoint` expects (see
 * `crates/lang/sysml-runtime/src/breakpoint.rs`).
 *
 * Frontend types are UX-shaped (`target`, `threshold`, `direction`)
 * while the backend variants carry their own field names
 * (`element_id`, `value`, `op`). We translate at the boundary so
 * neither side has to compromise.
 *
 * `direction` → `op` mapping for threshold-crossing:
 *   - `'rising'`  → `Gt`  (fire when variable goes above threshold)
 *   - `'falling'` → `Lt`  (fire when variable goes below threshold)
 *   - `'either'` (or absent) → `Gt` (default alarm-above semantic)
 *
 * The backend's `Gt/Lt/...` are *value comparisons*, not strict
 * crossings — see `Breakpoint::ThresholdCrossing` docstring.
 * `'either'` doesn't have a perfect inverse; we pick `Gt` as the
 * least-surprising default and document it. Workflows that need
 * true crossing semantics should use the conditional variant.
 */
export function serializeBreakpointForWire(bp: Breakpoint): unknown {
  switch (bp.kind) {
    case 'state-entry':
    case 'transition-fire':
    case 'action-invoke':
    case 'constraint-violation':
      return { kind: bp.kind, element_id: bp.target };

    case 'threshold-crossing': {
      const op =
        bp.direction === 'falling' ? 'lt'
          : bp.direction === 'rising' ? 'gt'
            : 'gt'; // 'either' / unspecified → default alarm-above
      return {
        kind: 'threshold-crossing',
        variable: bp.variable,
        op,
        value: bp.threshold,
        debounce_ticks: bp.debounce_ticks ?? 0,
      };
    }

    case 'conditional':
      return {
        kind: 'conditional',
        target: bp.target,
        variable: bp.variable,
        op: bp.op,
        value: bp.value,
        enabled: bp.enabled ?? true,
        ...(bp.label !== undefined ? { label: bp.label } : {}),
      };
  }
}

/**
 * Factory for the breakpoint REST calls. Accepts a `poster` so tests
 * can substitute a mock. Production callers pass `httpPost`.
 *
 * Backend surface (live, see `sysml-service::lib.rs` `breakpoint_*`):
 *   POST /api/command   { command: 'sysml.breakpoint.set',
 *                         params: { session_id, breakpoint } }
 *     → returns BreakpointId (plain string)
 *   POST /api/command   { command: 'sysml.breakpoint.clear',
 *                         params: { session_id, breakpoint_id } }
 *     → returns ()
 *   POST /api/command   { command: 'sysml.breakpoint.list',
 *                         params: { session_id } }
 *     → returns [BreakpointId, Breakpoint][]
 *
 * Every call is scoped to a session — breakpoints live on the
 * orchestrator, not globally. Callers must pass the active session
 * id (typically from `useSessionStore.getState().activeSessionId`).
 */
export function createBreakpointClient(poster: HttpPoster = httpPost) {
  return {
    async set(sessionId: string, loc: Breakpoint): Promise<BreakpointId> {
      const response = await poster<{ id: BreakpointId } | BreakpointId | string>(
        '/api/command',
        {
          command: 'sysml.breakpoint.set',
          params: {
            session_id: sessionId,
            breakpoint: serializeBreakpointForWire(loc),
          },
        },
      );
      if (typeof response === 'string') return response;
      if (response && typeof response === 'object' && 'id' in response) {
        return response.id;
      }
      throw new Error('sysml.breakpoint.set: unexpected response shape');
    },

    async clear(sessionId: string, breakpointId: BreakpointId): Promise<void> {
      await poster<void>('/api/command', {
        command: 'sysml.breakpoint.clear',
        params: { session_id: sessionId, breakpoint_id: breakpointId },
      });
    },

    async list(sessionId: string): Promise<Array<[BreakpointId, Breakpoint]>> {
      const response = await poster<
        Array<[BreakpointId, unknown]> | { breakpoints: Array<[BreakpointId, unknown]> }
      >('/api/command', {
        command: 'sysml.breakpoint.list',
        params: { session_id: sessionId },
      });
      const pairs: Array<[BreakpointId, unknown]> = Array.isArray(response)
        ? response
        : response && typeof response === 'object' && 'breakpoints' in response
          ? (response.breakpoints ?? [])
          : [];
      return pairs.map(([id, raw]) => [id, deserializeBreakpointFromWire(raw)] as [BreakpointId, Breakpoint]);
    },
  };
}

/**
 * Reverse of `serializeBreakpointForWire`. The backend emits the
 * Rust struct shape (`element_id`, `op`, `value`); we lift it back
 * into the UI's `Breakpoint` discriminated union.
 *
 * `op → direction` for threshold-crossing is lossy: `gt` and `ge`
 * both map to `'rising'`, `lt`/`le` to `'falling'`, and `eq`/`ne`
 * fall through to `'either'`. Round-tripping a UI breakpoint
 * through `set → list` therefore stays semantically equivalent for
 * the common cases but doesn't preserve `≥` vs `>` distinctions.
 */
export function deserializeBreakpointFromWire(raw: unknown): Breakpoint {
  const r = raw as Record<string, unknown>;
  const kind = r.kind as string;
  switch (kind) {
    case 'state-entry':
    case 'transition-fire':
    case 'action-invoke':
    case 'constraint-violation':
      return { kind, target: String(r.element_id ?? '') } as Breakpoint;

    case 'threshold-crossing': {
      const op = String(r.op ?? 'gt');
      const direction =
        op === 'lt' || op === 'le' ? 'falling'
          : op === 'gt' || op === 'ge' ? 'rising'
            : 'either';
      return {
        kind: 'threshold-crossing',
        target: String(r.variable ?? ''),
        variable: String(r.variable ?? ''),
        threshold: Number(r.value ?? 0),
        direction,
        debounce_ticks: typeof r.debounce_ticks === 'number' ? r.debounce_ticks : 0,
      };
    }

    case 'conditional':
      return {
        kind: 'conditional',
        target: String(r.target ?? ''),
        variable: String(r.variable ?? ''),
        op: (r.op as Breakpoint extends { kind: 'conditional'; op: infer O } ? O : never) ?? 'eq',
        value: Number(r.value ?? 0),
        enabled: typeof r.enabled === 'boolean' ? r.enabled : true,
        ...(typeof r.label === 'string' ? { label: r.label } : {}),
      };
  }
  throw new Error(`unknown breakpoint kind: ${kind}`);
}

// ── React hook ───────────────────────────────────────────────────────

/**
 * Return a `SessionControl` adapter backed by the existing session
 * infrastructure. Call inside a React component / hook.
 *
 * The returned object is stable across renders (memoised on the
 * underlying hooks). Workflow UIs should treat it as a freeze-point:
 * if you need workflow-specific state, keep it locally and only call
 * into this interface for engine operations.
 */
export function useSessionControl(): SessionControl {
  const controller = useSessionController();
  const createMutation = useCreateSession();
  const stopMutation = useStopSession();
  const forkMutation = useForkSession();

  // Run-target input for the `start` default path (the server infers the kind).
  const activeSessionTarget = useWorkspaceUIStore((s) => s.activeSessionTarget);
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const { data: groups } = useRunTargets(workspaceRoot, wsData?.uris ?? []);

  const target = useMemo<RunTargetSummary | null>(() => {
    if (!activeSessionTarget || !groups) return null;
    for (const g of groups) {
      const t = g.targets.find((tt) => tt.id === activeSessionTarget);
      if (t) return t;
    }
    return null;
  }, [activeSessionTarget, groups]);

  const setActiveSession = useSessionStore((s) => s.setActiveSession);

  const breakpointClient = useMemo(() => createBreakpointClient(), []);

  // ── start ─────────────────────────────────────────────────────────

  const start = useCallback<SessionControl['start']>(
    async (config) => {
      const dtMs = useSessionStore.getState().dtMs;
      const params = buildCreateParams(config, target, dtMs);
      const summary = await createMutation.mutateAsync(params);
      if (!summary.id) {
        throw new Error('SessionControl.start: sessions.create returned no id');
      }
      setActiveSession(summary.id);
      useSessionStore.getState().setPhase('paused');
      return summary.id;
    },
    [target, createMutation, setActiveSession],
  );

  // ── pause / resume ────────────────────────────────────────────────

  // The underlying controller is active-session-scoped (only one
  // session at a time today). We accept the id for API compatibility
  // with the multi-session future but warn if it doesn't match.
  const pause = useCallback<SessionControl['pause']>(
    (id) => {
      const active = useSessionStore.getState().activeSessionId;
      if (active !== id) {
        console.warn(
          `[SessionControl.pause] id=${id} is not the active session (${active}); no-op`,
        );
        return;
      }
      controller.pause();
    },
    [controller],
  );

  // BP5: `controller.resume()` already clears the backend breakpoint-pause
  // flag (`sysml.sessions.resume`) before continuing the step loop — see
  // its doc comment in `useSessionController.ts`. No separate wiring
  // needed here; this adapter just forwards to it after the id check.
  const resume = useCallback<SessionControl['resume']>(
    (id) => {
      const active = useSessionStore.getState().activeSessionId;
      if (active !== id) {
        console.warn(
          `[SessionControl.resume] id=${id} is not the active session (${active}); no-op`,
        );
        return;
      }
      controller.resume();
    },
    [controller],
  );

  // ── step ──────────────────────────────────────────────────────────

  const step = useCallback<SessionControl['step']>(
    async (id, opts) => {
      const active = useSessionStore.getState().activeSessionId;
      if (active !== id) {
        console.warn(
          `[SessionControl.step] id=${id} is not the active session (${active}); no-op`,
        );
        return;
      }
      await controller.stepOnce(opts?.event);
    },
    [controller],
  );

  // ── stop ──────────────────────────────────────────────────────────

  const stop = useCallback<SessionControl['stop']>(
    (id) => {
      const active = useSessionStore.getState().activeSessionId;
      // The UI-level controller.stop() is idempotent and clears local
      // state regardless of id match.
      controller.stop();
      // Also tear down the backend session if it is the active one.
      if (active === id) {
        stopMutation.mutate(id);
      } else {
        // Fire-and-forget teardown for a non-active session.
        stopMutation.mutate(id);
      }
    },
    [controller, stopMutation],
  );

  // ── breakpoints ──────────────────────────────────────────────────
  //
  // Backend `sysml.breakpoint.{set,clear,list}` are session-scoped —
  // breakpoints live on the orchestrator. The engine surface
  // pre-dates multi-session support so its method signatures don't
  // take an explicit id; we resolve the active session out of the
  // store on each call. Callers without an active session get a
  // clear error instead of a silently-dropped 400.
  const requireActiveSession = useCallback((op: string): string => {
    const id = useSessionStore.getState().activeSessionId;
    if (!id) {
      throw new Error(
        `SessionControl.${op}: no active session (start one first)`,
      );
    }
    return id;
  }, []);

  const setBreakpoint = useCallback<SessionControl['setBreakpoint']>(
    (loc) => breakpointClient.set(requireActiveSession('setBreakpoint'), loc),
    [breakpointClient, requireActiveSession],
  );
  const clearBreakpoint = useCallback<SessionControl['clearBreakpoint']>(
    (id) => breakpointClient.clear(requireActiveSession('clearBreakpoint'), id),
    [breakpointClient, requireActiveSession],
  );
  const listBreakpoints = useCallback<SessionControl['listBreakpoints']>(
    async () => {
      const pairs = await breakpointClient.list(
        requireActiveSession('listBreakpoints'),
      );
      // Surface drops the id pairing — callers asking for a list of
      // breakpoints just want the breakpoint specs. If a future
      // workflow needs the ids, expose a paired-listing method
      // instead of changing this contract.
      return pairs.map(([, bp]) => bp);
    },
    [breakpointClient, requireActiveSession],
  );

  // ── inspect / snapshot / fork ────────────────────────────────────

  const inspect = useCallback<SessionControl['inspect']>(
    async (id, target) => {
      // Current backend reports snapshot data via `sysml.sessions.info`
      // which returns the full `SessionDetail`. The engine surface
      // treats inspection as a targeted query over that snapshot.
      //
      // GAP: there's no dedicated `sysml.inspect` endpoint today;
      // workflow UIs that want richer element-level inspection
      // (e.g. hover popups showing a PartUsage's full attribute set)
      // need a new backend command. For now, we route through
      // sessions.info and dig out the requested name.
      const info = await httpPost<{
        latest_snapshot: Record<string, unknown> | null;
        summary?: { tick?: number };
      }>('/api/command', {
        command: 'sysml.sessions.info',
        params: { session_id: id },
      });
      const snapshot = info.latest_snapshot ?? {};
      const variables = (snapshot.variables ?? {}) as Record<string, Value>;
      const value = variables[target] ?? null;
      return {
        target,
        value,
        tick: info.summary?.tick ?? null,
        kind: value === null ? undefined : 'variable',
      };
    },
    [],
  );

  const snapshot = useCallback<SessionControl['snapshot']>(
    async (id) => {
      // GAP: there is no first-class snapshot id today. Snapshots are
      // implicit (every tick gets archived). We return a synthetic id
      // of `<sessionId>@<tick>` so workflow UIs have a stable handle
      // to pass around; the future snapshot API can replace this.
      const info = await httpPost<{ summary?: { tick?: number } }>(
        '/api/command',
        { command: 'sysml.sessions.info', params: { session_id: id } },
      );
      const tick = info.summary?.tick ?? 0;
      return `${id}@${tick}` satisfies SnapshotId;
    },
    [],
  );

  const fork = useCallback<SessionControl['fork']>(
    async (id, atTick, overrides) => {
      // Backend `fork_with_overrides` accepts an optional `at_tick`
      // since R7 — when supplied, the child rewinds to that tick from
      // the parent's recorded trace. The mutation hook routes
      // through `fork_with_overrides` whenever `atTick` is set (even
      // with empty overrides) and through the bare `fork` otherwise.
      const summary = await forkMutation.mutateAsync({
        sessionId: id,
        atTick,
        overrides: overrides ? overridesToTuples(overrides) : undefined,
      });
      if (!summary.id) {
        throw new Error('SessionControl.fork: fork returned no session id');
      }
      return summary.id;
    },
    [forkMutation],
  );

  return useMemo<SessionControl>(
    () => ({
      start,
      pause,
      resume,
      step,
      stop,
      setBreakpoint,
      clearBreakpoint,
      listBreakpoints,
      inspect,
      snapshot,
      fork,
    }),
    [
      start,
      pause,
      resume,
      step,
      stop,
      setBreakpoint,
      clearBreakpoint,
      listBreakpoints,
      inspect,
      snapshot,
      fork,
    ],
  );
}

