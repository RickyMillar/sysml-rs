/**
 * filterArchive — pure client-side narrowing helper for the archive panel (R4.1).
 *
 * The backend already narrows on `origin` / `since` / `only_golden` via
 * `sysml.sessions.archive.list`, but the panel re-runs those filters in
 * the browser as well so the free-text search can combine with server
 * facets without round-trips. Keeping the helper pure makes it trivial
 * to unit-test every combination.
 *
 * Also exports the helpers used inside the filter:
 *   - `sinceCutoffMs` — resolves a `ArchiveSinceKey` to a unix-ms cutoff.
 *   - `worstVerdict` — collapses a `verdict_counts` record into the most
 *     severe `VerdictKind`, or `null` when the record is absent/empty.
 *
 * Exported from this module so the panel and its tests share one
 * implementation — no inline severity maps sprinkled across components.
 */

import type {
  ArchivedSessionSummary,
  ArchiveFilter,
  ArchiveSinceKey,
  WorstVerdict,
} from './types';

/**
 * Returns a unix-ms cutoff for a given `since` key, or `null` when the
 * key is `'all'` (no cutoff). Anchored on `now` so tests can inject a
 * deterministic reference point.
 */
export function sinceCutoffMs(
  since: ArchiveSinceKey,
  now: number = Date.now(),
): number | null {
  switch (since) {
    case '1h':
      return now - 60 * 60 * 1000;
    case '24h':
      return now - 24 * 60 * 60 * 1000;
    case '7d':
      return now - 7 * 24 * 60 * 60 * 1000;
    case 'all':
      return null;
  }
}

/**
 * Collapse a `verdict_counts` record into the worst-severity VerdictKind
 * present. Returns `null` when the record is absent or every count is
 * zero — the row then renders without a verdict badge rather than a
 * misleading "pass" glyph.
 *
 * Severity (highest first): error > fail > inconclusive > pass.
 */
export function worstVerdict(
  counts: ArchivedSessionSummary['verdict_counts'] | undefined,
): WorstVerdict {
  if (!counts) return null;
  if ((counts.error ?? 0) > 0) return 'error';
  if ((counts.fail ?? 0) > 0) return 'fail';
  if ((counts.inconclusive ?? 0) > 0) return 'inconclusive';
  if ((counts.pass ?? 0) > 0) return 'pass';
  return null;
}

/**
 * Pure filter — takes the server-side list plus the client filter state
 * and returns only the entries the UI should render. Idempotent and
 * allocation-frugal: the input array is never mutated.
 *
 * Ordering is preserved — callers sort server-side (newest-first in the
 * expected backend; client re-sort lives outside this helper).
 */
export function filterArchive(
  entries: ArchivedSessionSummary[],
  filter: ArchiveFilter,
  now: number = Date.now(),
): ArchivedSessionSummary[] {
  const needle = filter.search.trim().toLowerCase();
  const cutoff = sinceCutoffMs(filter.since, now);
  const originGate = filter.origin === 'all' ? null : filter.origin;

  return entries.filter((entry) => {
    if (filter.onlyGolden && !entry.is_golden) return false;
    if (originGate !== null && entry.origin !== originGate) return false;
    if (cutoff !== null && entry.created_at < cutoff) return false;
    if (needle.length > 0) {
      const hay =
        entry.label.toLowerCase() + ' ' + entry.workspace_uri.toLowerCase();
      if (!hay.includes(needle)) return false;
    }
    return true;
  });
}
