/**
 * Two-factor sweep expansion — the study the crash report was about.
 *
 * The reported failure was "a two-parameter sweep hung for a while and then
 * crashed". Measurement located that in execution, not expansion: the client
 * drove all 25 children at once and each child's archive-on-stop landed
 * simultaneously, taking the backend past the machine's RAM.
 *
 * Expansion is pinned here anyway, because the fix must not be allowed to
 * become "run fewer of them". A 5×5 study is 25 points, every combination
 * distinct, every one carrying both factors — and it stays that way.
 */

import { describe, it, expect } from 'vitest';
import { expandStudyChildren } from '../useSweepStudyStore';
import { generateChildrenParams, type ParameterRangeEntry } from '../useSweepConfig';

/** The exact repro: `ambientTemp` × `emissivity`, five values each. */
const TWO_FACTOR: ParameterRangeEntry[] = [
  { parameterId: 'ambientTemp', spec: { kind: 'grid', min: 250, max: 350, step: 25 } },
  { parameterId: 'emissivity', spec: { kind: 'grid', min: 0.5, max: 0.9, step: 0.1 } },
];

describe('two-factor sweep expansion', () => {
  it('produces all 25 combinations', () => {
    expect(expandStudyChildren(TWO_FACTOR)).toHaveLength(25);
  });

  it('gives every child both factors', () => {
    for (const point of expandStudyChildren(TWO_FACTOR)) {
      expect(Object.keys(point).sort()).toEqual(['ambientTemp', 'emissivity']);
    }
  });

  it('produces 25 DISTINCT points — no duplicates padding the count', () => {
    const points = expandStudyChildren(TWO_FACTOR);
    const keys = new Set(points.map((p) => JSON.stringify(p)));
    expect(keys.size).toBe(25);
  });

  it('covers the full cross product of both factors', () => {
    const points = expandStudyChildren(TWO_FACTOR);
    const ambient = new Set(points.map((p) => p.ambientTemp));
    const emissivity = new Set(points.map((p) => p.emissivity));
    expect([...ambient].sort((a, b) => Number(a) - Number(b))).toEqual([250, 275, 300, 325, 350]);
    expect(emissivity.size).toBe(5);
    // Each ambient value appears with each emissivity value exactly once.
    for (const a of ambient) {
      expect(points.filter((p) => p.ambientTemp === a)).toHaveLength(5);
    }
  });

  it('scales multiplicatively with a third factor', () => {
    const three = [
      ...TWO_FACTOR,
      { parameterId: 'surfaceArea', spec: { kind: 'grid' as const, min: 0.1, max: 0.3, step: 0.1 } },
    ];
    expect(expandStudyChildren(three)).toHaveLength(75);
  });

  it('is the same expansion the legacy hook produces — one home', () => {
    expect(expandStudyChildren(TWO_FACTOR)).toEqual(generateChildrenParams(TWO_FACTOR));
  });

  it('has no children at all with no factors', () => {
    expect(expandStudyChildren([])).toEqual([]);
  });
});
