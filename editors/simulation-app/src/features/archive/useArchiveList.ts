/**
 * useArchiveList — react-query hook that fetches archived sessions (R4.1).
 *
 * Thin wrapper around `sysml.sessions.archive.list`. The server-side
 * filters (`workspace_uri`, `origin`, `since`, `only_golden`) are
 * forwarded verbatim; the client-side search ('label' / 'workspace_uri'
 * substring) runs in-component via `filterArchive` so typing a needle
 * doesn't cause a round-trip per keystroke.
 *
 * Also exports `useArchivedSession` — used by the panel's three-dot menu
 * when the user wants to preview the full payload (verdicts + overrides
 * + snapshots) before restoring. The list view intentionally only needs
 * the summary shape.
 */

import { useQuery, type UseQueryResult } from '@tanstack/react-query';
import { httpPost } from '@/shared/api/http';
import type {
  ArchivedSession,
  ArchivedSessionSummary,
  SessionOrigin,
} from './types';
import { sinceCutoffMs } from './filterArchive';
import type { ArchiveFilter } from './types';

// ── Query key factory ────────────────────────────────────────────────

/**
 * Query keys for the archive surface. Exported so mutations (golden
 * mark / unmark) can invalidate the list without stringly-coupling.
 */
export const archiveKeys = {
  all: ['archive'] as const,
  lists: () => [...archiveKeys.all, 'list'] as const,
  /** Keyed by the *server-forwarded* slice of the filter. */
  list: (opts: ArchiveListParams) =>
    [...archiveKeys.lists(), opts] as const,
  details: () => [...archiveKeys.all, 'detail'] as const,
  detail: (id: string) => [...archiveKeys.details(), id] as const,
};

// ── Wire shapes ──────────────────────────────────────────────────────

/** Slice of the UI filter that is forwarded to the backend verbatim. */
export interface ArchiveListParams {
  workspace_uri?: string;
  origin?: SessionOrigin;
  since?: number;
  only_golden?: boolean;
}

/** Response envelope from `sysml.sessions.archive.list`. */
interface ArchiveListResponse {
  entries: Array<ArchivedSessionSummary & { origin: SessionOrigin | 'compliance' }>;
}

/** Response envelope from `sysml.sessions.archive.get`. */
interface ArchiveGetResponse {
  entry: (ArchivedSession & { origin: SessionOrigin | 'compliance' }) | null;
}

// ── Helpers ──────────────────────────────────────────────────────────

function cmd<T>(command: string, params: Record<string, unknown> = {}): Promise<T> {
  return httpPost<T>('/api/command', { command, params });
}

function normalizeLegacyOrigin(origin: SessionOrigin | 'compliance'): SessionOrigin {
  return origin === 'compliance' ? 'verify' : origin;
}

function normalizeSummaryOrigin(
  entry: ArchivedSessionSummary & { origin: SessionOrigin | 'compliance' },
): ArchivedSessionSummary {
  return { ...entry, origin: normalizeLegacyOrigin(entry.origin) };
}

function normalizeEntryOrigin(
  entry: (ArchivedSession & { origin: SessionOrigin | 'compliance' }) | null,
): ArchivedSession | null {
  return entry ? { ...entry, origin: normalizeLegacyOrigin(entry.origin) } : null;
}

/**
 * Translate the full UI filter into the narrow server-side slice. The
 * free-text `search` never reaches the wire — it's a client-side overlay.
 * Shape mirrors the backend `ArchiveListInput` one-to-one (snake_case).
 */
export function toListParams(
  filter: ArchiveFilter,
  workspaceUri?: string | null,
  now: number = Date.now(),
): ArchiveListParams {
  const params: ArchiveListParams = {};
  if (workspaceUri) params.workspace_uri = workspaceUri;
  if (filter.origin !== 'all') params.origin = filter.origin;
  const cutoff = sinceCutoffMs(filter.since, now);
  if (cutoff !== null) params.since = cutoff;
  if (filter.onlyGolden) params.only_golden = true;
  return params;
}

// ── Hooks ────────────────────────────────────────────────────────────

export interface UseArchiveListOpts {
  /** Optional workspace narrowing — forwarded as `workspace_uri`. */
  workspaceUri?: string | null;
  /** Poll interval in ms. Default is disabled (manual refetch only). */
  refetchIntervalMs?: number | false;
  /** When `false`, the query is parked — useful for gated panels. */
  enabled?: boolean;
}

/**
 * Fetch the archive list. Returns the raw react-query result so callers
 * can render loading / error / empty states with the canonical flags.
 */
export function useArchiveList(
  filter: ArchiveFilter,
  opts: UseArchiveListOpts = {},
): UseQueryResult<ArchivedSessionSummary[], Error> {
  const params = toListParams(filter, opts.workspaceUri ?? null);
  return useQuery<ArchivedSessionSummary[], Error>({
    queryKey: archiveKeys.list(params),
    queryFn: async () => {
      const res = await cmd<ArchiveListResponse>(
        'sysml.sessions.archive.list',
        params as Record<string, unknown>,
      );
      return (res.entries ?? []).map(normalizeSummaryOrigin);
    },
    enabled: opts.enabled ?? true,
    refetchInterval: opts.refetchIntervalMs ?? false,
  });
}

/**
 * Fetch one full archived session by id. Lazy — only enabled when `id`
 * is non-null, so callers can flip a preview pane open/closed without
 * juggling a manual refetch.
 */
export function useArchivedSession(
  id: string | null,
): UseQueryResult<ArchivedSession | null, Error> {
  return useQuery<ArchivedSession | null, Error>({
    queryKey: archiveKeys.detail(id ?? ''),
    queryFn: async () => {
      const res = await cmd<ArchiveGetResponse>(
        'sysml.sessions.archive.get',
        { id },
      );
      return normalizeEntryOrigin(res.entry ?? null);
    },
    enabled: !!id,
  });
}
