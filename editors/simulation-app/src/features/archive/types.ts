/**
 * features/archive/types — panel-local types for the session archive (R4.1).
 *
 * Wire mirrors (ArchivedSessionSummary / ArchivedSession / SessionOrigin)
 * live in `engine/types.ts`. Re-exported here so component files only
 * import `./types` and never touch the engine surface directly.
 *
 * Local types include the UI filter shape and a worst-wins verdict
 * aggregate — both are UI-only and would pollute the engine barrel.
 */

import type {
  ArchivedSessionSummary,
  ArchivedSession,
  SessionOrigin,
  VerdictKind,
} from '@/engine/types';

export type { ArchivedSessionSummary, ArchivedSession, SessionOrigin };

/**
 * Quick-chip bucket for the "since" filter. Wire shape is `number | null`
 * (unix ms cutoff, or `null` for "all"); UI carries a string enum that
 * encapsulates the rendering label + the conversion policy.
 */
export type ArchiveSinceKey = '1h' | '24h' | '7d' | 'all';

/**
 * All-or-nothing segmented-control value. `'all'` is the UI "no filter"
 * sentinel; the wire shape drops `origin` entirely when the user hasn't
 * narrowed.
 */
export type ArchiveOriginFilter = 'all' | SessionOrigin;

/**
 * Full filter state held by `<ArchivePanel />` and consumed by the pure
 * `filterArchive` helper. Each field is optional at the wire layer but
 * required in the UI so the component never has to carry `undefined`s.
 */
export interface ArchiveFilter {
  /** Free-text search (matches label + workspace_uri, case-insensitive). */
  search: string;
  /** Segmented-control value. `'all'` disables origin narrowing. */
  origin: ArchiveOriginFilter;
  /** Quick-chip value for the "since" cutoff. */
  since: ArchiveSinceKey;
  /** When `true`, only entries with `is_golden: true` pass the filter. */
  onlyGolden: boolean;
}

/**
 * Default filter used on panel mount. `'all'` and empty search mirror the
 * "no narrowing" state — `useArchiveList` sends the narrowest possible
 * wire payload (no origin, no since, no only_golden) for this value.
 */
export const DEFAULT_ARCHIVE_FILTER: ArchiveFilter = {
  search: '',
  origin: 'all',
  since: 'all',
  onlyGolden: false,
};

/**
 * Worst-wins aggregate of a summary's verdict counts, used by the row
 * verdict badge. Resolves to the most severe outcome present, mirroring
 * the four-valued VerdictKind space.
 *
 * Severity: `error` > `fail` > `inconclusive` > `pass` > `null` (no data).
 */
export type WorstVerdict = VerdictKind | null;

/**
 * Canonical ordered list of origins for the segmented control. Kept
 * in one place so the Panel and tests stay in lock-step as new origins
 * land (each new `SessionOrigin` becomes a pill).
 */
export const ARCHIVE_ORIGIN_OPTIONS: readonly ArchiveOriginFilter[] = [
  'all',
  'run',
  'verify',
  'sweep',
  'montecarlo',
  'tradestudy',
] as const;

/**
 * Human labels for the origin chip / segmented control. Separate from the
 * enum so we can tweak wording without touching the wire contract.
 */
export const ARCHIVE_ORIGIN_LABELS: Record<ArchiveOriginFilter, string> = {
  all: 'All',
  run: 'Run',
  verify: 'Verify',
  sweep: 'Sweep',
  montecarlo: 'Monte Carlo',
  tradestudy: 'Trade Study',
};

/** Canonical order for the "since" quick chips. */
export const ARCHIVE_SINCE_OPTIONS: readonly ArchiveSinceKey[] = [
  '1h',
  '24h',
  '7d',
  'all',
] as const;

/** Human labels for the "since" chips. */
export const ARCHIVE_SINCE_LABELS: Record<ArchiveSinceKey, string> = {
  '1h': '1h',
  '24h': '24h',
  '7d': '7d',
  all: 'All',
};
