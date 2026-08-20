/**
 * useSweepSlice — react-query hook for the post-hoc batch slice (R5.4).
 *
 * Thin wrapper around the backend `sysml.batch.slice` command. Given a
 * `batch_id` and a `SliceFilter`, returns the filtered `ChildDescriptor[]`.
 * The hook keys the query by `(batch_id, filter)` so React Query caches
 * identical filters, and refetches whenever either changes.
 *
 * Used by the results shell to narrow the table / tornado / heatmap view
 * without re-running the sweep. The pre-run filter lives in
 * `preRunFilter.ts` (client-side, never reaches the wire).
 *
 * Contract boundaries:
 *   - The hook is disabled when `batch_id` is empty / null, so an
 *     un-run sweep does not spam the backend.
 *   - A `null` filter clears the slice — we short-circuit to an
 *     "undefined query" (enabled=false) in that case, so callers can
 *     toggle the filter on and off without tearing down the query.
 *   - The hook accepts a react-query `refetchInterval` so callers can
 *     poll while the batch is still streaming; the default is
 *     disabled (the batch runner already pushes child completions).
 */

import { useMemo } from 'react';
import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import type { ChildDescriptor, SliceFilter } from '@/engine/types';

// ── Query key factory ────────────────────────────────────────────────

/**
 * Query keys for the sweep slice surface. Exported so other code paths
 * (e.g. a mutation that re-triggers a child run) can invalidate the
 * slice without stringly-coupling.
 */
export const sliceKeys = {
  all: ['sweep-slice'] as const,
  forBatch: (batchId: string) => [...sliceKeys.all, batchId] as const,
  with: (batchId: string, filter: SliceFilter) =>
    [...sliceKeys.forBatch(batchId), filter] as const,
};

// ── Wire shapes ──────────────────────────────────────────────────────

/** Response envelope from `sysml.batch.slice`. */
interface BatchSliceResponse {
  children: ChildDescriptor[];
}

// ── Helper ──────────────────────────────────────────────────────────

function cmd<T>(
  command: string,
  params: Record<string, unknown> = {},
): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

/**
 * Invoke `sysml.batch.slice` directly. Exported so non-React callers
 * (tests, imperative scripts) can reuse the transport without pulling in
 * react-query.
 */
export async function fetchSweepSlice(
  batchId: string,
  filter: SliceFilter,
): Promise<ChildDescriptor[]> {
  const res = await cmd<BatchSliceResponse>('sysml.batch.slice', {
    batch_id: batchId,
    filter,
  });
  return res.children ?? [];
}

// ── Hook ────────────────────────────────────────────────────────────

export interface UseSweepSliceOpts {
  /**
   * Poll interval in milliseconds — useful while the batch is still
   * running and child rows are landing one at a time. Defaults to
   * disabled (backend push events drive the UI in the steady state).
   */
  refetchIntervalMs?: number | false;
  /**
   * Gate the query — handy for panels that mount before a batch id is
   * known. Defaults to "enabled iff batch_id and filter are present".
   */
  enabled?: boolean;
}

/**
 * Fetch the sliced children for a batch. When `filter` is `null`, the
 * query is parked and the caller falls back to its unsliced view.
 */
export function useSweepSlice(
  batchId: string | null,
  filter: SliceFilter | null,
  opts: UseSweepSliceOpts = {},
): UseQueryResult<ChildDescriptor[], Error> {
  // Memoise the filter reference so callers that recreate it every render
  // do not churn the query cache. Filters are tiny objects; JSON-stringify
  // is cheap and side-steps `useMemo` deps that are themselves unstable.
  const stableFilter = useMemo(
    () => filter,
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [filter ? JSON.stringify(filter) : null],
  );

  const isEnabled =
    opts.enabled ??
    (typeof batchId === 'string' && batchId.length > 0 && stableFilter !== null);

  return useQuery<ChildDescriptor[], Error>({
    queryKey:
      stableFilter !== null && batchId
        ? sliceKeys.with(batchId, stableFilter)
        : sliceKeys.all,
    queryFn: async () => {
      if (!batchId || !stableFilter) return [];
      return fetchSweepSlice(batchId, stableFilter);
    },
    enabled: isEnabled,
    refetchInterval: opts.refetchIntervalMs ?? false,
  });
}
