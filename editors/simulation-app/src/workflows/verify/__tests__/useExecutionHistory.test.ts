/**
 * Tests for the execution-history pure helpers (design turn 3 data layer):
 * row composition (per-mode, never flat), model-structure grouping with
 * failing-rows-first, the recent-verdicts trend slot, and relative ages.
 */

import { describe, it, expect } from 'vitest';
import {
  composeLatestRows,
  groupKeyOf,
  groupLatestRows,
  latestByCase,
  recentVerdicts,
  relativeAge,
  ROOT_GROUP_LABEL,
  type CaseLatestWire,
  type ExecutionRowWire,
} from '../useExecutionHistory';
import type { VerificationCaseRow } from '../useVerificationCases';

const CASES: VerificationCaseRow[] = [
  {
    element_id: 'e-5x',
    case_name: 'TripAt5xComplianceCase',
    qualified_name: 'IECCompliance::TripCompliance::TripAt5xComplianceCase',
    verdict: 'Inconclusive',
    total_requirements: 1,
  } as VerificationCaseRow,
  {
    element_id: 'e-1x',
    case_name: 'TripAt1xComplianceCase',
    qualified_name: 'IECCompliance::TripCompliance::TripAt1xComplianceCase',
    verdict: 'Inconclusive',
    total_requirements: 1,
  } as VerificationCaseRow,
  {
    element_id: 'e-nuis',
    case_name: 'NuisanceTripFloorGap',
    qualified_name: 'IECCompliance::NuisanceImmunity::NuisanceTripFloorGap',
    verdict: 'Inconclusive',
    total_requirements: 1,
  } as VerificationCaseRow,
];

const LATEST: CaseLatestWire[] = [
  {
    case_id: 'TripAt5xComplianceCase',
    case_element_id: 'e-5x',
    latest: {
      trajectory: {
        verdict: 'pass',
        execution_id: 'S-1842',
        timestamp: 1_000,
        case_changed_since: false,
      },
      external: {
        verdict: 'pass',
        execution_id: 'x-1',
        timestamp: 900,
        tool: 'hil-bench-2',
        matches_current_model: false,
      },
    },
  },
  {
    case_id: 'TripAt1xComplianceCase',
    case_element_id: 'e-1x',
    latest: {
      trajectory: {
        verdict: 'fail',
        execution_id: 'S-1842',
        timestamp: 1_000,
        case_changed_since: true,
      },
    },
  },
];

describe('composeLatestRows — per-mode, never flat', () => {
  it('keeps the three modes as separate slots and derives the flags', () => {
    const rows = composeLatestRows(CASES, latestByCase(LATEST));
    const r5x = rows.find((r) => r.caseName === 'TripAt5xComplianceCase')!;
    expect(r5x.staticVerdict).toBe('inconclusive');
    expect(r5x.trajectory?.verdict).toBe('pass');
    expect(r5x.external?.tool).toBe('hil-bench-2');
    expect(r5x.stale).toBe(true); // matches_current_model === false
    expect(r5x.failing).toBe(false);

    const r1x = rows.find((r) => r.caseName === 'TripAt1xComplianceCase')!;
    expect(r1x.failing).toBe(true);
    expect(r1x.changed).toBe(true);
    expect(r1x.external).toBeNull(); // absent mode key stays absent
  });

  it('a case with no executions carries only the static slot', () => {
    const rows = composeLatestRows(CASES, latestByCase(LATEST));
    const rNuis = rows.find((r) => r.caseName === 'NuisanceTripFloorGap')!;
    expect(rNuis.trajectory).toBeNull();
    expect(rNuis.external).toBeNull();
    expect(rNuis.staticVerdict).toBe('inconclusive');
  });
});

describe('grouping — model structure, failing rows first', () => {
  it('groups by the owner chain of qualified_name', () => {
    expect(groupKeyOf(CASES[0])).toBe('IECCompliance::TripCompliance');
    expect(groupKeyOf({ case_name: 'x' } as VerificationCaseRow)).toBe('');
  });

  it('keeps model order between groups and floats failing rows within', () => {
    const rows = composeLatestRows(CASES, latestByCase(LATEST));
    const groups = groupLatestRows(rows);
    expect(groups.map((g) => g.label)).toEqual([
      'IECCompliance::TripCompliance',
      'IECCompliance::NuisanceImmunity',
    ]);
    // 5x came first in model order, but 1x FAILS → it sorts first in-group.
    expect(groups[0].rows.map((r) => r.caseName)).toEqual([
      'TripAt1xComplianceCase',
      'TripAt5xComplianceCase',
    ]);
    expect(groups[0].counts.fail).toBe(1);
  });

  it('labels the empty group as the model root', () => {
    const rows = composeLatestRows(
      [{ case_name: 'Loose', verdict: 'Pass', total_requirements: 1 } as VerificationCaseRow],
      new Map(),
    );
    expect(groupLatestRows(rows)[0].label).toBe(ROOT_GROUP_LABEL);
  });
});

describe('recentVerdicts — the trend slot', () => {
  const EXECUTIONS: ExecutionRowWire[] = [
    {
      execution_id: 'ex-new',
      origin: 'run',
      timestamp: 2_000,
      evaluation_mode: 'trajectory',
      results: [
        { case_id: 'A', verdict: 'fail', evaluation_mode: 'trajectory', timestamp: 2_000 },
        { case_id: 'B', verdict: 'pass', evaluation_mode: 'trajectory', timestamp: 2_000 },
      ],
      counts: { pass: 1, fail: 1, inconclusive: 0, error: 0 },
    },
    {
      execution_id: 'ex-old',
      origin: 'external',
      timestamp: 1_000,
      evaluation_mode: 'external',
      results: [{ case_id: 'A', verdict: 'pass', evaluation_mode: 'external', timestamp: 1_000 }],
      counts: { pass: 1, fail: 0, inconclusive: 0, error: 0 },
    },
  ];

  it('collects newest-first across modes, filtered to the case', () => {
    expect(recentVerdicts(EXECUTIONS, 'A')).toEqual(['fail', 'pass']);
    expect(recentVerdicts(EXECUTIONS, 'B')).toEqual(['pass']);
    expect(recentVerdicts(EXECUTIONS, 'missing')).toEqual([]);
  });

  it('honours the limit', () => {
    expect(recentVerdicts(EXECUTIONS, 'A', 1)).toEqual(['fail']);
  });
});

describe('relativeAge', () => {
  it('formats minutes, hours, days off a fixed now', () => {
    const now = 10 * 24 * 60 * 60_000;
    expect(relativeAge(now - 30_000, now)).toBe('now');
    expect(relativeAge(now - 5 * 60_000, now)).toBe('5m');
    expect(relativeAge(now - 2 * 60 * 60_000, now)).toBe('2h');
    expect(relativeAge(now - 3 * 24 * 60 * 60_000, now)).toBe('3d');
  });
});
