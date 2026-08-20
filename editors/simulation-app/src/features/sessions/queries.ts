/**
 * React-query hooks for session read operations.
 *
 * All remote session data flows through these hooks — no Zustand storage
 * of backend-owned data. Per ADR-004 section 1:
 * - Session list: staleTime 5s, background refetch on focus
 * - Active session detail: staleTime 0 during playback, 10 Hz poll
 * - Topology: staleTime Infinity (structural, doesn't change per-tick)
 */

import { useQuery } from '@tanstack/react-query';
import { httpPost } from '../../shared/api/http';
import type {
  SessionSummary,
  SessionDetail,
  SubsystemSummary,
  SessionQuota,
} from './types';
import type { SystemTopology } from '../../types/physics';
import { httpGet } from '../../shared/api/http';
import { useSessionStore } from './store';
import { normalizeTopologyPayload } from './normalize';

// ── Query key factories ───────────────────────────────────────────────

export const sessionKeys = {
  all: ['sessions'] as const,
  lists: () => [...sessionKeys.all, 'list'] as const,
  detail: (id: string) => [...sessionKeys.all, 'detail', id] as const,
  topology: (id: string) => [...sessionKeys.all, 'topology', id] as const,
  subsystems: (id: string) => [...sessionKeys.all, 'subsystems', id] as const,
  quota: () => [...sessionKeys.all, 'quota'] as const,
  health: () => ['backend-health'] as const,
  /** Phase 4 — names + decimated-series key factories. */
  timeseriesNames: (id: string) =>
    [...sessionKeys.all, 'timeseries_names', id] as const,
  timeseriesDecimated: (
    id: string,
    name: string,
    targetPoints: number,
    startMs: number | null,
    endMs: number | null,
  ) =>
    [
      ...sessionKeys.all,
      'timeseries_decimated',
      id,
      name,
      targetPoints,
      startMs,
      endMs,
    ] as const,
};

// ── API helpers (thin wrappers around httpPost) ───────────────────────

function cmd<T>(command: string, params: Record<string, unknown> = {}): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

// ── Hooks ─────────────────────────────────────────────────────────────

/**
 * Fetch all active sessions. Polls at 1 Hz.
 */
export function useSessionList() {
  return useQuery({
    queryKey: sessionKeys.lists(),
    queryFn: () => cmd<SessionSummary[]>('sysml.sessions.list'),
    staleTime: 5_000,
    refetchInterval: 1_000,
    refetchOnWindowFocus: true,
  });
}

/**
 * Fetch full session detail (summary + subsystems + latest snapshot).
 *
 * Polls at 2 Hz while running, 1 Hz otherwise. The step mutation
 * already writes a fresh `summary` into this cache on every tick, so
 * we don't need a 10 Hz info poll just to drive scalar counters. The
 * `latest_snapshot` payload for espresso-production-cell is ~920 KB (14k
 * variables, 99% of the response), so a faster poll rate drowns the
 * browser in JSON parsing + React-query subscriber churn without
 * giving the user visibly smoother updates.
 *
 * `includeVariables` gates whether the backend ships the variables
 * map. Callers that actually need live values (Variables pane, diagram
 * live labels) pass `true`; the default is `false` for cheap summary
 * polling. See BUG 27 (perf) in docs/test-checklist-2026-04-20.md.
 */
export function useSessionDetail(
  id: string | null,
  opts: { includeVariables?: boolean } = {},
) {
  const phase = useSessionStore((s) => s.phase);
  // Stage 6b: metadata-only poll is cheap (~5 KB). The WebSocket stream
  // carries every live field (tick, time_ms, subsystems, constraints,
  // vars), so the HTTP poll only needs to refresh summary metadata +
  // subsystem `available_transitions` — 2 s cadence is plenty. Callers
  // that explicitly ask for `includeVariables: true` (compare, archive)
  // get the slower 500 ms cadence since they don't have a stream.
  const includeVariables = opts.includeVariables ?? false;
  const interval = phase === 'running'
    ? (includeVariables ? 500 : 2_000)
    : 1_000;

  return useQuery({
    queryKey: [...sessionKeys.detail(id ?? ''), includeVariables ? 'full' : 'meta'],
    queryFn: () =>
      cmd<SessionDetail>('sysml.sessions.info', {
        session_id: id,
        include_variables: includeVariables,
      }),
    enabled: !!id,
    staleTime: 0,
    refetchInterval: interval,
  });
}

/**
 * Fetch session topology (structural, rarely changes).
 *
 * The backend sends snake_case JSON (root_label, current_state, etc.)
 * which we normalize to the camelCase SystemTopology TS type.
 */
export function useSessionTopology(id: string | null) {
  return useQuery({
    queryKey: sessionKeys.topology(id ?? ''),
    queryFn: async () => {
      const raw = await cmd<Record<string, unknown>>('sysml.sessions.topology', { session_id: id });
      return normalizeTopologyPayload(raw);
    },
    enabled: !!id,
    staleTime: Infinity,
  });
}

/**
 * Fetch subsystem list for a session.
 */
export function useSessionSubsystems(id: string | null) {
  return useQuery({
    queryKey: sessionKeys.subsystems(id ?? ''),
    queryFn: () => cmd<SubsystemSummary[]>('sysml.sessions.subsystems', { session_id: id }),
    enabled: !!id,
    staleTime: 5_000,
  });
}

/**
 * Fetch session quota (per-kind bucket usage). Polls at 0.5 Hz per contract.
 */
export function useSessionQuota() {
  return useQuery({
    queryKey: sessionKeys.quota(),
    queryFn: () => cmd<SessionQuota>('sysml.sessions.quota'),
    staleTime: 2_000,
    refetchInterval: 2_000,
  });
}

// ── Phase 4 — Session inspector hooks ────────────────────────────────

/**
 * Wire shape returned by `sysml.sessions.timeseries_names` (Phase 4).
 *
 * Sorted variable names + the size of the underlying ring buffer so a
 * picker can decide whether to truncate / paginate. Matches
 * `TimeSeriesNamesResult` in `crates/tooling/sysml-service/src/types.rs`.
 */
export interface TimeseriesNamesResult {
  names: string[];
  len: number;
  capacity: number;
}

/**
 * Wire shape returned by `sysml.sessions.timeseries` and
 * `sysml.sessions.timeseries_decimated` (Phase 4 — decimated path is
 * the only FE consumer).
 *
 * `points` is oldest→newest, each `time_ms` is the tick's recorded
 * time. For decimated payloads the points are **real input samples**
 * picked by LTTB — `time_ms` round-trips back to the raw series'
 * sample at the same tick, so the FE can use `time_ms` as the
 * cross-representation identifier without a separate `sample_index`.
 */
export interface TimeseriesResult {
  var: string;
  points: Array<{ time_ms: number; value: number }>;
}

/**
 * Enumerate the variable names captured in a session's time-series
 * ring buffer (`sysml.sessions.timeseries_names`). Used by the Session
 * Inspector's variable picker. Cheap; polled at 2s.
 */
export function useSessionTimeseriesNames(id: string | null) {
  return useQuery({
    queryKey: sessionKeys.timeseriesNames(id ?? ''),
    queryFn: () =>
      cmd<TimeseriesNamesResult>('sysml.sessions.timeseries_names', {
        session_id: id,
      }),
    enabled: !!id,
    staleTime: 2_000,
    refetchInterval: 2_000,
  });
}

/**
 * Fetch an LTTB-decimated time series for one variable, scoped to an
 * inclusive `[startMs, endMs]` viewport (`sysml.sessions.timeseries_decimated`).
 *
 * Pass `null`/`null` for the unbounded buffer range. `targetPoints` is
 * the on-screen pixel width; the backend caps the response at the
 * underlying buffer length. Polls at 1s during live playback so the
 * chart trails the running simulation; this is much cheaper than
 * polling the raw `sessions.timeseries` command.
 */
export function useSessionTimeseriesDecimated(
  id: string | null,
  name: string | null,
  targetPoints: number,
  viewport: { startMs: number | null; endMs: number | null } = {
    startMs: null,
    endMs: null,
  },
) {
  return useQuery({
    queryKey: sessionKeys.timeseriesDecimated(
      id ?? '',
      name ?? '',
      targetPoints,
      viewport.startMs,
      viewport.endMs,
    ),
    queryFn: () =>
      cmd<TimeseriesResult>('sysml.sessions.timeseries_decimated', {
        session_id: id,
        var: name,
        target_points: targetPoints,
        start_ms: viewport.startMs,
        end_ms: viewport.endMs,
      }),
    enabled: !!id && !!name,
    staleTime: 0,
    refetchInterval: 1_000,
  });
}

/**
 * Backend API health check. Polls every 5s.
 * Returns true when the backend responds to GET /health, false otherwise.
 */
export function useBackendHealth() {
  return useQuery({
    queryKey: sessionKeys.health(),
    queryFn: async () => {
      try {
        await httpGet<unknown>('/health');
        return true;
      } catch {
        return false;
      }
    },
    staleTime: 5_000,
    refetchInterval: 5_000,
    // Don't throw on failure — we catch inside queryFn
    retry: false,
  });
}
