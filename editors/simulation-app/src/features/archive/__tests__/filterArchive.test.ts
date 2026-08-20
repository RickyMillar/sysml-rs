/**
 * Pure-function tests for `filterArchive` and its helpers (R4.1).
 */

import { describe, expect, it } from 'vitest';
import {
  filterArchive,
  sinceCutoffMs,
  worstVerdict,
} from '../filterArchive';
import {
  DEFAULT_ARCHIVE_FILTER,
  type ArchiveFilter,
  type ArchivedSessionSummary,
  type SessionOrigin,
} from '../types';

const NOW = 1_700_000_000_000; // arbitrary anchor — deterministic

function makeEntry(
  overrides: Partial<ArchivedSessionSummary> & Pick<ArchivedSessionSummary, 'id'>,
): ArchivedSessionSummary {
  return {
    id: overrides.id,
    label: overrides.label ?? `entry-${overrides.id}`,
    origin: (overrides.origin ?? 'run') as SessionOrigin,
    workspace_uri: overrides.workspace_uri ?? 'file:///ws/root',
    created_at: overrides.created_at ?? NOW - 60_000,
    ended_at: overrides.ended_at ?? null,
    ticks: overrides.ticks ?? 10,
    is_golden: overrides.is_golden ?? false,
    golden_label: overrides.golden_label,
    verdict_counts: overrides.verdict_counts,
  };
}

function withFilter(partial: Partial<ArchiveFilter>): ArchiveFilter {
  return { ...DEFAULT_ARCHIVE_FILTER, ...partial };
}

describe('sinceCutoffMs', () => {
  it('returns null for "all"', () => {
    expect(sinceCutoffMs('all', NOW)).toBeNull();
  });

  it('computes the 1h cutoff', () => {
    expect(sinceCutoffMs('1h', NOW)).toBe(NOW - 60 * 60 * 1000);
  });

  it('computes the 24h cutoff', () => {
    expect(sinceCutoffMs('24h', NOW)).toBe(NOW - 24 * 60 * 60 * 1000);
  });

  it('computes the 7d cutoff', () => {
    expect(sinceCutoffMs('7d', NOW)).toBe(NOW - 7 * 24 * 60 * 60 * 1000);
  });
});

describe('worstVerdict', () => {
  it('returns null when counts are absent', () => {
    expect(worstVerdict(undefined)).toBeNull();
  });

  it('returns null when every count is zero', () => {
    expect(
      worstVerdict({ pass: 0, fail: 0, inconclusive: 0, error: 0 }),
    ).toBeNull();
  });

  it('returns "pass" when only pass > 0', () => {
    expect(
      worstVerdict({ pass: 5, fail: 0, inconclusive: 0, error: 0 }),
    ).toBe('pass');
  });

  it('returns "inconclusive" when it beats pass', () => {
    expect(
      worstVerdict({ pass: 5, fail: 0, inconclusive: 1, error: 0 }),
    ).toBe('inconclusive');
  });

  it('returns "fail" when it beats inconclusive', () => {
    expect(
      worstVerdict({ pass: 5, fail: 1, inconclusive: 3, error: 0 }),
    ).toBe('fail');
  });

  it('returns "error" as the most severe', () => {
    expect(
      worstVerdict({ pass: 5, fail: 3, inconclusive: 2, error: 1 }),
    ).toBe('error');
  });
});

describe('filterArchive', () => {
  const entries: ArchivedSessionSummary[] = [
    makeEntry({ id: 'a', label: 'Regression A', origin: 'run', created_at: NOW - 30 * 60 * 1000 }),
    makeEntry({ id: 'b', label: 'Nightly verify', origin: 'verify', is_golden: true, created_at: NOW - 2 * 60 * 60 * 1000 }),
    makeEntry({ id: 'd', label: 'Sweep scan', origin: 'sweep', workspace_uri: 'file:///other/ws', created_at: NOW - 10 * 24 * 60 * 60 * 1000 }),
    makeEntry({ id: 'e', label: 'MC run', origin: 'montecarlo', is_golden: true, created_at: NOW - 5 * 60 * 1000 }),
  ];

  it('passes everything through with defaults', () => {
    const out = filterArchive(entries, DEFAULT_ARCHIVE_FILTER, NOW);
    expect(out).toHaveLength(entries.length);
  });

  it('narrows by origin', () => {
    const out = filterArchive(entries, withFilter({ origin: 'verify' }), NOW);
    expect(out.map((e) => e.id)).toEqual(['b']);
  });

  it('does not filter when origin is "all"', () => {
    const out = filterArchive(entries, withFilter({ origin: 'all' }), NOW);
    expect(out).toHaveLength(entries.length);
  });

  it('filters by only_golden', () => {
    const out = filterArchive(entries, withFilter({ onlyGolden: true }), NOW);
    expect(out.map((e) => e.id).sort()).toEqual(['b', 'e']);
  });

  it('respects since=1h', () => {
    const out = filterArchive(entries, withFilter({ since: '1h' }), NOW);
    // a (30m ago) and e (5m ago) remain
    expect(out.map((e) => e.id).sort()).toEqual(['a', 'e']);
  });

  it('respects since=24h', () => {
    const out = filterArchive(entries, withFilter({ since: '24h' }), NOW);
    // a (30m), b (2h), e (5m)
    expect(out.map((e) => e.id).sort()).toEqual(['a', 'b', 'e']);
  });

  it('respects since=7d', () => {
    const out = filterArchive(entries, withFilter({ since: '7d' }), NOW);
    // excludes d (10d old)
    expect(out.map((e) => e.id).sort()).toEqual(['a', 'b', 'e']);
  });

  it('search narrows by label substring, case-insensitive', () => {
    const out = filterArchive(entries, withFilter({ search: 'VERIFY' }), NOW);
    expect(out.map((e) => e.id)).toEqual(['b']);
  });

  it('search narrows by workspace_uri substring', () => {
    const out = filterArchive(entries, withFilter({ search: 'other' }), NOW);
    expect(out.map((e) => e.id)).toEqual(['d']);
  });

  it('combines only_golden + since + search', () => {
    const out = filterArchive(
      entries,
      withFilter({ onlyGolden: true, since: '1h', search: 'mc' }),
      NOW,
    );
    // entry e: golden, 5m ago, label "MC run"
    expect(out.map((e) => e.id)).toEqual(['e']);
  });

  it('returns an empty array when filters exclude everything', () => {
    const out = filterArchive(
      entries,
      withFilter({ origin: 'tradestudy', search: 'nonexistent' }),
      NOW,
    );
    expect(out).toEqual([]);
  });

  it('does not mutate the input', () => {
    const snapshot = JSON.parse(JSON.stringify(entries));
    filterArchive(entries, withFilter({ origin: 'verify' }), NOW);
    expect(entries).toEqual(snapshot);
  });

  it('preserves input ordering among matches', () => {
    const many: ArchivedSessionSummary[] = [
      makeEntry({ id: '1', origin: 'run', created_at: NOW - 10 * 60 * 1000 }),
      makeEntry({ id: '2', origin: 'run', created_at: NOW - 20 * 60 * 1000 }),
      makeEntry({ id: '3', origin: 'verify', created_at: NOW - 30 * 60 * 1000 }),
      makeEntry({ id: '4', origin: 'run', created_at: NOW - 40 * 60 * 1000 }),
    ];
    const out = filterArchive(many, withFilter({ origin: 'run' }), NOW);
    expect(out.map((e) => e.id)).toEqual(['1', '2', '4']);
  });

  it('handles empty input', () => {
    expect(filterArchive([], DEFAULT_ARCHIVE_FILTER, NOW)).toEqual([]);
  });

  it('trims search whitespace', () => {
    const out = filterArchive(
      entries,
      withFilter({ search: '   verify   ' }),
      NOW,
    );
    expect(out.map((e) => e.id)).toEqual(['b']);
  });
});
