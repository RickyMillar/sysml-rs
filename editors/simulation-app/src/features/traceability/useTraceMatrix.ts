/**
 * useTraceMatrix — react-query hook that fetches the trace matrix (R6.2).
 *
 * Thin wrapper around `sysml.trace_matrix`. The backend returns a flat
 * `Vec<TraceMatrixRow>` (edges only — no pre-grouped rows / columns).
 * This hook builds the grouped `TraceMatrix` shape via
 * `buildTraceMatrix` so the viewer stays pure and can re-filter without
 * touching the wire.
 *
 * Stale-while-revalidate is on by default (react-query's standard
 * behaviour) — `staleTime` is left at the default so the matrix
 * refreshes in the background when the user refocuses the tab. Tests
 * override to `Infinity` to keep query state deterministic.
 */

import { useMemo } from 'react';
import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { httpGet } from '@/shared/api/http';
import { buildTraceMatrix } from './filterTraceMatrix';
import {
  DEFAULT_TRACE_SELECTORS,
  type TraceMatrix,
  type TraceMatrixRow,
  type TraceSelectors,
} from './types';

// ── Query key factory ────────────────────────────────────────────────

/**
 * Query keys for the trace-matrix surface. Exported so unrelated
 * mutations (model reload, etc.) can invalidate the query without
 * stringly-coupling to the internal key shape.
 */
export const traceMatrixKeys = {
  all: ['trace-matrix'] as const,
  lists: () => [...traceMatrixKeys.all, 'list'] as const,
  /** Keyed by workspace uri + the selectors forwarded to the backend. */
  list: (opts: TraceMatrixQueryKey) =>
    [...traceMatrixKeys.lists(), opts] as const,
};

interface TraceMatrixQueryKey extends TraceSelectors {
  workspace_uri: string;
}

// ── Wire shape ───────────────────────────────────────────────────────

/**
 * `sysml.trace_matrix` returns a JSON array of row records, not an
 * envelope. The shape mirrors `sysml_core::query::TraceMatrixRow` ⤵
 *
 *   { source, source_name, target, target_name, relationship }
 *
 * The REST trace endpoint is available in both browser and Tauri-sidecar
 * transports. The generic dispatcher does not expose `sysml.trace_matrix`
 * in the local REST API, so using it made the Browse trace view fail with 404.
 *
 * `workspace_uri` gates readiness and scopes the query cache, but the model
 * API exposes the merged workspace graph under the `__workspace__` sentinel;
 * the UI store otherwise contains a filesystem root, which is not a model URI.
 */
async function fetchTraceMatrixRows(
  _workspaceUri: string,
  selectors: TraceSelectors,
): Promise<TraceMatrixRow[]> {
  const query = new URLSearchParams({
    source_kind: selectors.source_kind,
    relation_kind: selectors.relation_kind,
    target_kind: selectors.target_kind,
  });
  const res = await httpGet<TraceMatrixRow[] | { rows?: TraceMatrixRow[] }>(
    `/models/__workspace__/trace?${query}`,
  );
  // The REST facade returns a bare array. Tolerate a `{ rows: … }` envelope
  // so callers stay compatible with future service wrappers.
  if (Array.isArray(res)) return res;
  return res.rows ?? [];
}

// ── Hook ─────────────────────────────────────────────────────────────

export interface UseTraceMatrixOpts {
  /** Workspace root used to gate and cache the workspace-wide matrix query. */
  workspace_uri: string | null | undefined;
  /**
   * Which element-kind / relationship-kind trio to forward. Defaults
   * to `PartUsage + Satisfy + RequirementUsage` (source=satisfier →
   * target=requirement) — the "which parts satisfy which requirements"
   * lens.
   */
  selectors?: TraceSelectors;
  /** When `false`, park the query. Default `true`. */
  enabled?: boolean;
}

export interface UseTraceMatrixResult {
  /** The grouped matrix, derived from the wire edges. */
  matrix: TraceMatrix;
  /** Loading / error state from react-query. */
  query: UseQueryResult<TraceMatrixRow[], Error>;
}

/**
 * Fetch the trace matrix for a workspace URI. Returns the grouped
 * `TraceMatrix` (derived) alongside the raw react-query result so
 * callers can render loading / error / empty states with the
 * canonical flags.
 *
 * The query is gated on `workspace_uri` — if the caller passes
 * `null` or `undefined` (e.g. no workspace loaded yet) the hook
 * parks and returns an empty matrix, never issuing a request.
 */
export function useTraceMatrix(
  opts: UseTraceMatrixOpts,
): UseTraceMatrixResult {
  const workspaceUri = opts.workspace_uri ?? null;
  const selectors = opts.selectors ?? DEFAULT_TRACE_SELECTORS;
  const enabled = (opts.enabled ?? true) && workspaceUri !== null;

  const query = useQuery<TraceMatrixRow[], Error>({
    queryKey: traceMatrixKeys.list({
      workspace_uri: workspaceUri ?? '',
      ...selectors,
    }),
    queryFn: () => fetchTraceMatrixRows(workspaceUri ?? '', selectors),
    enabled,
  });

  const matrix = useMemo<TraceMatrix>(() => {
    const rows = query.data ?? [];
    return buildTraceMatrix(rows);
  }, [query.data]);

  return { matrix, query };
}
