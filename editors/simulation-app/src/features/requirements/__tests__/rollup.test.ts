/**
 * Pure-logic tests for the Requirements workbench presentation helpers:
 * document-order grouping, coverage stats, the three-state verified
 * chip ruling (design doc §5), and the v1 view presets.
 */

import { describe, expect, it } from 'vitest';
import {
  coverageStats,
  filterRows,
  groupRowsByPackage,
  maturityGlyph,
  rowDisplayId,
  summarizePackages,
  verifiedChipModel,
} from '../rollup';
import type { RequirementRow, RequirementVerificationRollup } from '../types';

function row(overrides: Partial<RequirementRow>): RequirementRow {
  return {
    id: 'e-0',
    kind: 'RequirementUsage',
    req_id: null,
    name: null,
    text: null,
    qualified_name: null,
    owning_package: null,
    source_span: null,
    outline_depth: 0,
    maturity: null,
    satisfied_by: [],
    verified_by: [],
    verification: { state: 'incomplete', cases_total: 0, cases_passed: 0 },
    verification_methods: [],
    derived_from: [],
    derives: [],
    refines: [],
    ...overrides,
  };
}

const pkg = (id: string, name: string) => ({ id, name, kind: 'Package' });

describe('groupRowsByPackage', () => {
  it('groups contiguous document-order runs, never re-sorting', () => {
    const rows = [
      row({ id: 'a', owning_package: pkg('p1', 'TripUnit') }),
      row({ id: 'b', owning_package: pkg('p1', 'TripUnit') }),
      row({ id: 'c', owning_package: pkg('p2', 'Sensing') }),
      // p1 reappears later in the document — a second, separate group.
      row({ id: 'd', owning_package: pkg('p1', 'TripUnit') }),
    ];
    const groups = groupRowsByPackage(rows);
    expect(groups.map((g) => g.label)).toEqual(['TripUnit', 'Sensing', 'TripUnit']);
    expect(groups[0].rows.map((r) => r.id)).toEqual(['a', 'b']);
    expect(groups[2].rows.map((r) => r.id)).toEqual(['d']);
  });

  it('labels package-less rows "(no package)"', () => {
    const groups = groupRowsByPackage([row({ id: 'a' })]);
    expect(groups[0].label).toBe('(no package)');
    expect(groups[0].packageId).toBeNull();
  });
});

describe('summarizePackages', () => {
  it('returns unique packages in first-appearance order with counts', () => {
    const rows = [
      row({ id: 'a', owning_package: pkg('p1', 'TripUnit') }),
      row({ id: 'b', owning_package: pkg('p2', 'Sensing') }),
      row({ id: 'c', owning_package: pkg('p1', 'TripUnit') }),
    ];
    expect(summarizePackages(rows)).toEqual([
      { packageId: 'p1', label: 'TripUnit', count: 2 },
      { packageId: 'p2', label: 'Sensing', count: 1 },
    ]);
  });
});

describe('verifiedChipModel — the three-state ruling', () => {
  const rollup = (
    state: RequirementVerificationRollup['state'],
    total: number,
    passed: number,
  ): RequirementVerificationRollup => ({
    state,
    cases_total: total,
    cases_passed: passed,
  });

  it('recorded fail → filled fail chip', () => {
    const model = verifiedChipModel(rollup('fail', 4, 1));
    expect(model.variant).toBe('fail');
    expect(model.label).toBe('1/4');
    expect(model.title).toMatch(/recorded fail/);
  });

  it('all cases passed → filled pass chip', () => {
    const model = verifiedChipModel(rollup('pass', 3, 3));
    expect(model.variant).toBe('pass');
    expect(model.label).toBe('3/3 ✓');
  });

  it('incomplete with linked cases → neutral outline', () => {
    const model = verifiedChipModel(rollup('incomplete', 2, 1));
    expect(model.variant).toBe('outline');
    expect(model.label).toBe('1/2');
    expect(model.title).toMatch(/incomplete/);
  });

  it('zero linked cases → plain dash (unverified)', () => {
    const model = verifiedChipModel(rollup('incomplete', 0, 0));
    expect(model.variant).toBe('none');
    expect(model.label).toBe('—');
    expect(model.title).toMatch(/unverified/i);
  });
});

describe('coverageStats + view presets', () => {
  const rows = [
    row({ id: 'a', verification: { state: 'pass', cases_total: 2, cases_passed: 2 }, maturity: 'done' }),
    row({ id: 'b', verification: { state: 'fail', cases_total: 4, cases_passed: 1 }, maturity: 'tbc' }),
    row({ id: 'c', verification: { state: 'incomplete', cases_total: 1, cases_passed: 0 } }),
    row({ id: 'd', verification: { state: 'incomplete', cases_total: 0, cases_passed: 0 }, maturity: 'closed' }),
  ];

  it('computes the honest rollup counts', () => {
    const stats = coverageStats(rows);
    expect(stats).toMatchObject({
      total: 4,
      passed: 1,
      failed: 1,
      incomplete: 2,
      unverified: 3,
      maturityOpen: 2,
    });
    expect(stats.coverage).toBeCloseTo(0.25);
  });

  it('empty set → zero coverage without NaN', () => {
    expect(coverageStats([]).coverage).toBe(0);
  });

  it('filters by view preset and package', () => {
    expect(filterRows(rows, 'all', undefined)).toHaveLength(4);
    expect(filterRows(rows, 'unverified', undefined).map((r) => r.id)).toEqual(['b', 'c', 'd']);
    expect(filterRows(rows, 'failing', undefined).map((r) => r.id)).toEqual(['b']);
    // No fixture row has satisfiers — all are orphans under R7's lens.
    expect(filterRows(rows, 'orphans', undefined)).toHaveLength(4);
    // packageId null = "rows with no package ancestor".
    expect(filterRows(rows, 'all', null)).toHaveLength(4);
  });
});

describe('display helpers', () => {
  it('maturityGlyph maps StatusKind literals to fill glyphs', () => {
    expect(maturityGlyph('done')).toBe('●');
    expect(maturityGlyph('closed')).toBe('●');
    expect(maturityGlyph('tbc')).toBe('◐');
    expect(maturityGlyph('tbr')).toBe('◐');
    expect(maturityGlyph('tbd')).toBe('○');
    expect(maturityGlyph('open')).toBe('○');
  });

  it('rowDisplayId prefers req_id, then name, then qualified name', () => {
    expect(rowDisplayId(row({ req_id: 'REQ-1', name: 'X' }))).toBe('REQ-1');
    expect(rowDisplayId(row({ name: 'X' }))).toBe('X');
    expect(rowDisplayId(row({ qualified_name: 'P::X' }))).toBe('P::X');
    expect(rowDisplayId(row({ id: 'e-9' }))).toBe('e-9');
  });
});
