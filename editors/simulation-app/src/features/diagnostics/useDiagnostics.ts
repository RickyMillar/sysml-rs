/**
 * useDiagnostics — react-query hook for the Diagnostics panel (R6.1).
 *
 * Thin wrapper around the `sysml.diagnostics` backend command (exposed
 * via the shared `/api/command` dispatcher). The command returns every
 * diagnostic — parse plus stored semantic — for a single loaded URI; to
 * cover the whole workspace, the hook fans out one query per URI and
 * merges the results into a single flat `DiagnosticEntry[]`.
 *
 * Per-URI queries are the unit of caching so the typical "user edits
 * file A" flow only re-fetches A, not the entire workspace.
 *
 * Wire shape (confirmed against
 * `crates/tooling/sysml-service/src/lib.rs` on Apr 19 2026):
 *   request : { command: "sysml.diagnostics", params: { uri: "..." } }
 *   response: Vec<Diagnostic>
 *
 * `Diagnostic` serialises every field directly — severity is lowercase
 * (`"info" | "warning" | "error"`), span is optional, code is optional,
 * notes / related / tags default to `[]`.
 */

import { useQueries, type UseQueryOptions } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import type { Diagnostic, DiagnosticEntry } from './types';

// ── Query key factory ────────────────────────────────────────────────

/** Query keys for the diagnostics surface. */
export const diagnosticsKeys = {
  all: ['diagnostics'] as const,
  /** Keyed by URI — one entry per loaded file. */
  byUri: (uri: string) => [...diagnosticsKeys.all, 'uri', uri] as const,
};

// ── Wire helper ──────────────────────────────────────────────────────

/** Dispatch a single `sysml.*` command through the shared HTTP wrapper. */
function cmd<T>(
  command: string,
  params: Record<string, unknown> = {},
): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

/**
 * Fetch raw diagnostics for a single URI. Exported for test reuse — the
 * panel consumes the memoised `useDiagnostics` hook, not this function.
 */
export async function fetchDiagnosticsForUri(
  uri: string,
): Promise<Diagnostic[]> {
  const raw = await cmd<Diagnostic[] | { diagnostics: Diagnostic[] }>(
    'sysml.diagnostics',
    { uri },
  );
  // The backend emits a bare `Vec<Diagnostic>`, but future service
  // revisions could wrap it in an envelope without breaking this hook.
  if (Array.isArray(raw)) return raw;
  if (raw && Array.isArray((raw as { diagnostics?: Diagnostic[] }).diagnostics)) {
    return (raw as { diagnostics: Diagnostic[] }).diagnostics;
  }
  return [];
}

// ── Hook ─────────────────────────────────────────────────────────────

export interface UseDiagnosticsOpts {
  /**
   * URIs to fan out over. Typically `wsData?.uris ?? []` from the
   * packages query. The hook parks (no query) when the list is empty.
   */
  uris: string[];
  /** Poll interval in ms. Default: disabled — panel refetches manually. */
  refetchIntervalMs?: number | false;
  /** When `false`, every per-URI query is parked. */
  enabled?: boolean;
}

export interface UseDiagnosticsResult {
  /** Flat list of diagnostic entries across every URI. */
  entries: DiagnosticEntry[];
  /** `true` while any per-URI query is still loading on first mount. */
  isLoading: boolean;
  /** `true` when at least one per-URI query has errored. */
  isError: boolean;
  /** First error observed (for error-state copy). */
  error: Error | null;
  /** `true` while any query is revalidating (stale-while-revalidate). */
  isFetching: boolean;
  /** Refetch every per-URI query. */
  refetch: () => void;
}

/**
 * Fetch diagnostics for every URI in the loaded workspace and flatten
 * the result. The panel filters / groups the flat list client-side so
 * the severity / search / scope controls operate on a single source of
 * truth.
 *
 * Behavioural notes:
 *   - An empty `uris` array yields `entries: []` with no loading state,
 *     letting the panel render the "no diagnostics" empty-state
 *     immediately rather than flashing a spinner.
 *   - A per-URI fetch that fails bubbles into `isError` / `error`, but
 *     other URIs continue to produce entries — a single bad file does
 *     not blank the whole panel.
 */
export function useDiagnostics(opts: UseDiagnosticsOpts): UseDiagnosticsResult {
  const enabled = opts.enabled ?? true;
  const refetchInterval = opts.refetchIntervalMs ?? false;

  const queries = useQueries({
    queries: opts.uris.map(
      (uri): UseQueryOptions<Diagnostic[], Error> => ({
        queryKey: diagnosticsKeys.byUri(uri),
        queryFn: () => fetchDiagnosticsForUri(uri),
        enabled,
        refetchInterval,
      }),
    ),
  });

  // Flatten per-URI results into a single list of DiagnosticEntry. Order
  // follows `opts.uris` — the group renderer relies on this for stable
  // section ordering across renders.
  const entries: DiagnosticEntry[] = [];
  let isLoading = false;
  let isError = false;
  let error: Error | null = null;
  let isFetching = false;

  queries.forEach((q, idx) => {
    const uri = opts.uris[idx];
    if (uri == null) return;
    if (q.isLoading) isLoading = true;
    if (q.isFetching) isFetching = true;
    if (q.isError && !isError) {
      isError = true;
      error = q.error ?? new Error('diagnostics fetch failed');
    }
    const data = q.data ?? [];
    for (const diagnostic of data) {
      entries.push({ uri, diagnostic });
    }
  });

  const refetch = () => {
    for (const q of queries) {
      void q.refetch();
    }
  };

  return { entries, isLoading, isError, error, isFetching, refetch };
}
