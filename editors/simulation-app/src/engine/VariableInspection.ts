/**
 * VariableInspection — Layer 2 implementation of the universal variable
 * read contract (E3).
 *
 * Wraps the existing snapshot (react-query) + time-series buffer stores
 * behind the single `VariableInspection` interface defined in
 * `./types.ts`. Every workflow UI that wants "what's X now / at tick T /
 * across runs" calls through here; none re-implement storage.
 */

import { useMemo } from 'react';
import { useQueryClient, type QueryClient } from '@tanstack/react-query';
import { sessionKeys } from '../features/sessions/queries';
import { useTimeSeriesStore } from '../shared/data/useTimeSeriesStore';
import type { SessionDetail, TimePoint } from '../features/sessions/types';
import type {
  SessionId,
  Value,
  VariableInspection,
  VariableName,
} from './types';

// ── Pure helpers (exported for tests) ────────────────────────────────

/**
 * Extract a single variable's value from a session detail's latest snapshot.
 * Returns null when the snapshot or variable is missing.
 */
export function readCurrent(
  detail: SessionDetail | undefined | null,
  name: VariableName,
): Value | null {
  if (!detail?.latest_snapshot) return null;
  const variables = (detail.latest_snapshot.variables ?? null) as
    | Record<string, Value>
    | null;
  if (!variables || typeof variables !== 'object') return null;
  return variables[name] ?? null;
}

/**
 * Given a series of `TimePoint`s (t = time_ms, v = numeric value),
 * return the value at the Nth logical step (0-indexed), where N is
 * interpreted as the tick offset.
 *
 * GAP: the time-series buffer is indexed by time_ms, not tick. For
 * fixed-dt simulations the two are proportional (tick = round(time_ms/dt)),
 * but for variable-dt or event-driven sessions this mapping breaks.
 * A faithful `atTick` requires the backend to expose a tick->snapshot
 * index (planned for the R1 engine backend work). Until then we treat
 * tick as the series index, which is correct for fixed-step sessions
 * and a monotone approximation otherwise.
 */
export function readAtTick(series: TimePoint[], tick: number): Value | null {
  if (!Number.isFinite(tick) || tick < 0) return null;
  const idx = Math.floor(tick);
  if (idx >= series.length) return null;
  const point = series[idx];
  return point ? point.v : null;
}

/** Select a single variable's time-series from the buffer's full map. */
export function readSeries(
  allSeries: Record<string, TimePoint[]>,
  name: VariableName,
): TimePoint[] {
  return allSeries[name] ?? [];
}

/**
 * Collect a variable's current value across multiple sessions.
 *
 * `getDetail` is injected so callers can either read from react-query
 * cache (production) or pass a test stub.
 */
export function readAcrossSessions(
  sessions: SessionId[],
  name: VariableName,
  getDetail: (id: SessionId) => SessionDetail | undefined | null,
): Map<SessionId, Value> {
  const out = new Map<SessionId, Value>();
  for (const id of sessions) {
    const value = readCurrent(getDetail(id), name);
    if (value !== null) {
      out.set(id, value);
    }
  }
  return out;
}

// ── QueryClient-backed helper (for the hook + tests with a real QC) ─

/**
 * Build a `VariableInspection` using a supplied `QueryClient` and
 * time-series store accessor. Exposed for testing with a real
 * QueryClient but a stub time-series store.
 */
export function createVariableInspection(
  queryClient: QueryClient,
  getAllSeries: () => Record<string, TimePoint[]>,
): VariableInspection {
  const getDetail = (id: SessionId): SessionDetail | undefined =>
    queryClient.getQueryData<SessionDetail>(sessionKeys.detail(id));

  return {
    current: (session, name) => readCurrent(getDetail(session), name),

    atTick: (_session, tick, name) => {
      // Time-series store is global (single active session). For
      // non-active sessions we'd need per-session history; see GAP
      // comment on `readAtTick` above.
      const all = getAllSeries();
      return readAtTick(readSeries(all, name), tick);
    },

    series: (_session, name) => {
      // As with atTick: single-session scope today. `_session` is
      // retained for API compatibility and future expansion.
      const all = getAllSeries();
      return readSeries(all, name);
    },

    acrossSessions: (sessions, name) =>
      readAcrossSessions(sessions, name, getDetail),
  };
}

// ── React hook ───────────────────────────────────────────────────────

/**
 * Return a `VariableInspection` bound to the current react-query client
 * and time-series store. Safe to call inside any component / hook.
 *
 * The returned interface methods read synchronously from cache — they
 * do NOT issue new network requests. Callers that need an authoritative
 * fetch should use the underlying `useSessionDetail` hook directly.
 */
export function useVariableInspection(): VariableInspection {
  const queryClient = useQueryClient();
  return useMemo(
    () =>
      createVariableInspection(queryClient, () =>
        useTimeSeriesStore.getState().getTimeSeries(),
      ),
    [queryClient],
  );
}
