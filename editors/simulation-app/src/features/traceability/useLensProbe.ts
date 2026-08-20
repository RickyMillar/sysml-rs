/**
 * useLensProbe — when the active lens is empty, find out which lenses are not.
 *
 * An empty trace matrix has two very different causes and the UI could not
 * tell them apart: the workspace genuinely has no traceability, or it has
 * plenty and the question being asked was the wrong one. On
 * `espresso-production-cell` it was the second — the default satisfy lens
 * returns 0 while the verify lens returns 8 — and the grid said nothing
 * either way.
 *
 * The probe runs ONLY when the active lens came back empty, so the normal
 * path costs nothing extra. Each probe is a `count`-shaped read of the same
 * REST endpoint the matrix uses and lands in the same react-query cache, so
 * switching to a suggested lens is served from cache rather than refetched.
 */

import { useQueries } from '@tanstack/react-query';
import { httpGet } from '@/shared/api/http';
import { TRACE_LENSES, type TraceLens } from './lenses';
import type { TraceMatrixRow, TraceSelectors } from './types';
import { traceMatrixKeys } from './useTraceMatrix';

async function fetchRows(selectors: TraceSelectors): Promise<TraceMatrixRow[]> {
  const query = new URLSearchParams({
    source_kind: selectors.source_kind,
    relation_kind: selectors.relation_kind,
    target_kind: selectors.target_kind,
  });
  const res = await httpGet<TraceMatrixRow[] | { rows?: TraceMatrixRow[] }>(
    `/models/__workspace__/trace?${query}`,
  );
  return Array.isArray(res) ? res : (res.rows ?? []);
}

export interface LensSuggestion {
  lens: TraceLens;
  edgeCount: number;
}

export interface UseLensProbeResult {
  /** Lenses with at least one edge, richest first. Empty until probes land. */
  suggestions: LensSuggestion[];
  /** True while any probe is in flight — the empty state waits rather than
   *  claiming "no links anywhere" before it knows. */
  isProbing: boolean;
  /** True once every probe has settled and none found an edge. */
  probedAndEmpty: boolean;
}

/**
 * @param workspaceUri gates the probe exactly as the matrix query is gated.
 * @param activeLensId excluded from the probe — it is the one we know is empty.
 * @param enabled pass `false` unless the active lens actually returned zero.
 */
export function useLensProbe(
  workspaceUri: string | null,
  activeLensId: string,
  enabled: boolean,
): UseLensProbeResult {
  const candidates = TRACE_LENSES.filter((l) => l.id !== activeLensId);
  const active = enabled && workspaceUri !== null;

  const results = useQueries({
    queries: candidates.map((lens) => ({
      queryKey: traceMatrixKeys.list({
        workspace_uri: workspaceUri ?? '',
        ...lens.selectors,
      }),
      queryFn: () => fetchRows(lens.selectors),
      enabled: active,
      staleTime: 30_000,
      // A probe that fails is not a finding. Stay silent rather than telling
      // the reader a lens is empty when the request never landed.
      retry: false,
    })),
  });

  const suggestions: LensSuggestion[] = [];
  let settled = 0;
  results.forEach((r, i) => {
    if (!r.isPending) settled += 1;
    const rows = r.data;
    if (Array.isArray(rows) && rows.length > 0) {
      suggestions.push({ lens: candidates[i], edgeCount: rows.length });
    }
  });
  suggestions.sort((a, b) => b.edgeCount - a.edgeCount);

  return {
    suggestions,
    isProbing: active && settled < candidates.length,
    probedAndEmpty: active && settled === candidates.length && suggestions.length === 0,
  };
}
