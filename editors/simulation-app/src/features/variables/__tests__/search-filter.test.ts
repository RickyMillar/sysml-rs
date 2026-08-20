/**
 * filterEntries / glob / filter-chip semantics.
 *
 * These tests pin the user-visible "chip + search" behaviour. Any regression
 * that breaks the glob pattern, substring fall-back, or chip filter will
 * fail here first.
 */

import { describe, it, expect } from 'vitest';
import {
  filterEntries,
  buildGlobMatcher,
  buildSubstringMatcher,
  type VariableEntry,
} from '../VariableTree';

function e(
  name: string,
  overrides: Partial<VariableEntry> = {},
): VariableEntry {
  return { name, value: 0, ...overrides };
}

const SAMPLE: VariableEntry[] = [
  e('circuit1.breaker.temp',    { constraint: 'pass',         lastChangedTick: 10 }),
  e('circuit1.breaker.current', { constraint: 'fail',         lastChangedTick: 10 }),
  e('circuit2.breaker.temp',    { constraint: 'inconclusive', lastChangedTick: 5  }),
  e('circuit2.busbar.temp',     { constraint: 'error',        lastChangedTick: 10 }),
  e('ambientTemp',              {                             lastChangedTick: 2  }),
  e('__hidden',                 {                             lastChangedTick: 10 }),
];

describe('filterEntries — search', () => {
  it('substring search is case-insensitive', () => {
    const out = filterEntries(SAMPLE, { search: 'BUSBAR' });
    expect(out.map((e) => e.name)).toEqual(['circuit2.busbar.temp']);
  });

  it('glob search uses wildcard expansion', () => {
    const out = filterEntries(SAMPLE, { search: 'circuit*.temp' });
    expect(out.map((e) => e.name).sort()).toEqual([
      'circuit1.breaker.temp',
      'circuit2.breaker.temp',
      'circuit2.busbar.temp',
    ].sort());
  });

  it('empty / whitespace search returns all visible entries', () => {
    const out = filterEntries(SAMPLE, { search: '   ' });
    expect(out.map((e) => e.name)).not.toContain('__hidden');
    expect(out).toHaveLength(SAMPLE.length - 1);
  });

  it('non-matching search returns an empty list', () => {
    expect(filterEntries(SAMPLE, { search: 'nomatch' })).toEqual([]);
  });
});

describe('filterEntries — chips', () => {
  it('passing chip keeps only satisfied constraints', () => {
    expect(filterEntries(SAMPLE, { filter: 'passing' }).map((e) => e.name))
      .toEqual(['circuit1.breaker.temp']);
  });

  it('failing chip keeps only violated constraints', () => {
    expect(filterEntries(SAMPLE, { filter: 'failing' }).map((e) => e.name))
      .toEqual(['circuit1.breaker.current']);
  });

  it('inconclusive chip keeps the amber set', () => {
    expect(filterEntries(SAMPLE, { filter: 'inconclusive' }).map((e) => e.name))
      .toEqual(['circuit2.breaker.temp']);
  });

  it('error chip keeps the magenta set', () => {
    expect(filterEntries(SAMPLE, { filter: 'error' }).map((e) => e.name))
      .toEqual(['circuit2.busbar.temp']);
  });

  it('pinned chip restricts to the provided pin set', () => {
    const out = filterEntries(SAMPLE, {
      filter: 'pinned',
      pinned: new Set(['ambientTemp']),
    });
    expect(out.map((e) => e.name)).toEqual(['ambientTemp']);
  });

  it('changed chip uses currentTick + recentWindow', () => {
    const out = filterEntries(SAMPLE, {
      filter: 'changed',
      currentTick: 10,
      recentWindow: 2,
    });
    // tick 10 with window 2 → 8..10 inclusive
    expect(out.map((e) => e.name).sort()).toEqual([
      'circuit1.breaker.current',
      'circuit1.breaker.temp',
      'circuit2.busbar.temp',
    ].sort());
  });

  it('changed chip returns empty when currentTick is undefined', () => {
    const out = filterEntries(SAMPLE, { filter: 'changed' });
    expect(out).toEqual([]);
  });
});

describe('filterEntries — chip + search compose', () => {
  it('intersects both filters', () => {
    const out = filterEntries(SAMPLE, {
      filter: 'passing',
      search: 'breaker',
    });
    expect(out.map((e) => e.name)).toEqual(['circuit1.breaker.temp']);
  });
});

describe('buildGlobMatcher / buildSubstringMatcher', () => {
  it('glob anchors at both ends', () => {
    const m = buildGlobMatcher('foo*');
    expect(m('foo')).toBe(true);
    expect(m('foobar')).toBe(true);
    expect(m('xfoo')).toBe(false);
  });

  it('glob escapes regex metachars', () => {
    const m = buildGlobMatcher('a.b*');
    expect(m('a.bcd')).toBe(true);
    expect(m('aXbcd')).toBe(false); // `.` is literal, not a wildcard
  });

  it('substring is case-insensitive', () => {
    const m = buildSubstringMatcher('TEMP');
    expect(m('ambientTemp')).toBe(true);
  });
});
