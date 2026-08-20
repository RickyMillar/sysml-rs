/**
 * Tests for pure helpers consumed by every R5.3 sweep viewer.
 *
 * These lock down the contract promised by the task spec:
 *   - computeSensitivity: monotone +, monotone −, constant, single-value, NaN
 *   - buildHeatmapGrid : 2×2 grid exact, missing → NaN, non-2-param → empty
 *   - normaliseAxisValues: single-value mid-point, negative range, empty
 */
import { describe, it, expect } from 'vitest';
import type { Verdict } from '../../../engine/types';
import {
  buildHeatmapGrid,
  collectParamNames,
  colourForNormalised,
  computeSensitivity,
  extractorFor,
  failCount,
  firstMargin,
  normaliseAxisValues,
  rollupVerdict,
  toNumber,
  type ChildDescriptor,
} from '../sweepViewerHelpers';

function mkVerdict(v: Verdict['verdict'], margin?: number | null): Verdict {
  return { verdict: v, margin: margin ?? null };
}

function mkChild(
  index: number,
  params: Record<string, unknown>,
  verdicts: Verdict[] = [],
  status: ChildDescriptor['status'] = 'complete',
  reason?: string | null,
): ChildDescriptor {
  return {
    session_id: `s${index}`,
    index,
    params,
    status,
    verdicts,
    reason: reason ?? null,
  };
}

// ── toNumber ───────────────────────────────────────────────────────

describe('toNumber', () => {
  it('returns finite numbers unchanged', () => {
    expect(toNumber(42)).toBe(42);
    expect(toNumber(-3.14)).toBe(-3.14);
  });
  it('parses numeric strings', () => {
    expect(toNumber('12')).toBe(12);
  });
  it('returns NaN for unparsable', () => {
    expect(Number.isNaN(toNumber('abc'))).toBe(true);
    expect(Number.isNaN(toNumber(null))).toBe(true);
    expect(Number.isNaN(toNumber(undefined))).toBe(true);
    expect(Number.isNaN(toNumber({}))).toBe(true);
  });
  it('booleans become 1/0', () => {
    expect(toNumber(true)).toBe(1);
    expect(toNumber(false)).toBe(0);
  });
  it('Infinity is not finite → NaN', () => {
    expect(Number.isNaN(toNumber(Infinity))).toBe(true);
  });
});

// ── collectParamNames ──────────────────────────────────────────────

describe('collectParamNames', () => {
  it('preserves insertion order across children', () => {
    const cs = [
      mkChild(0, { a: 1, b: 2 }),
      mkChild(1, { b: 2, c: 3 }),
      mkChild(2, { a: 1 }),
    ];
    expect(collectParamNames(cs)).toEqual(['a', 'b', 'c']);
  });

  it('is stable when children stream in', () => {
    const cs = [mkChild(0, { x: 1 })];
    expect(collectParamNames(cs)).toEqual(['x']);
    cs.push(mkChild(1, { x: 2, y: 3 }));
    expect(collectParamNames(cs)).toEqual(['x', 'y']);
  });

  it('returns [] for empty input', () => {
    expect(collectParamNames([])).toEqual([]);
  });
});

// ── computeSensitivity ─────────────────────────────────────────────

describe('computeSensitivity', () => {
  it('monotone positive: metric rises with param', () => {
    const cs = [
      mkChild(0, { p: 1 }, [mkVerdict('pass', 0)]),
      mkChild(1, { p: 2 }, [mkVerdict('pass', 5)]),
      mkChild(2, { p: 3 }, [mkVerdict('pass', 10)]),
    ];
    const stat = computeSensitivity(cs, 'p', firstMargin);
    expect(stat.low).toBe(0);
    expect(stat.high).toBe(10);
    expect(stat.range).toBe(10);
    expect(stat.samples).toBe(3);
  });

  it('monotone negative: range is still absolute', () => {
    const cs = [
      mkChild(0, { p: 1 }, [mkVerdict('pass', 10)]),
      mkChild(1, { p: 2 }, [mkVerdict('pass', 5)]),
      mkChild(2, { p: 3 }, [mkVerdict('pass', -4)]),
    ];
    const stat = computeSensitivity(cs, 'p', firstMargin);
    expect(stat.range).toBe(14);
    expect(stat.low).toBe(-4);
    expect(stat.high).toBe(10);
  });

  it('constant metric → range 0, samples reflect distinct param values', () => {
    const cs = [
      mkChild(0, { p: 1 }, [mkVerdict('pass', 7)]),
      mkChild(1, { p: 2 }, [mkVerdict('pass', 7)]),
    ];
    const stat = computeSensitivity(cs, 'p', firstMargin);
    expect(stat.range).toBe(0);
    expect(stat.samples).toBe(2);
  });

  it('single-value param → samples = 1', () => {
    const cs = [mkChild(0, { p: 5 }, [mkVerdict('pass', 1)])];
    const stat = computeSensitivity(cs, 'p', firstMargin);
    expect(stat.samples).toBe(1);
    expect(stat.range).toBe(0);
  });

  it('NaN metrics are skipped', () => {
    const cs = [
      mkChild(0, { p: 1 }, []), // firstMargin → NaN
      mkChild(1, { p: 2 }, [mkVerdict('pass', 3)]),
    ];
    const stat = computeSensitivity(cs, 'p', firstMargin);
    expect(stat.samples).toBe(1);
    expect(stat.low).toBe(3);
    expect(stat.high).toBe(3);
  });

  it('empty children → zeroed stat with NaN endpoints', () => {
    const stat = computeSensitivity([], 'p', firstMargin);
    expect(stat.samples).toBe(0);
    expect(stat.range).toBe(0);
    expect(Number.isNaN(stat.low)).toBe(true);
    expect(Number.isNaN(stat.high)).toBe(true);
  });

  it('works with failCount extractor', () => {
    const cs = [
      mkChild(0, { p: 1 }, [mkVerdict('pass'), mkVerdict('pass')]),
      mkChild(1, { p: 2 }, [mkVerdict('fail'), mkVerdict('fail'), mkVerdict('fail')]),
    ];
    const stat = computeSensitivity(cs, 'p', failCount);
    expect(stat.low).toBe(0);
    expect(stat.high).toBe(3);
    expect(stat.range).toBe(3);
  });
});

// ── normaliseAxisValues ────────────────────────────────────────────

describe('normaliseAxisValues', () => {
  it('single value → normalise always returns 0.5', () => {
    const n = normaliseAxisValues([7]);
    expect(n.min).toBe(7);
    expect(n.max).toBe(7);
    expect(n.normalise(7)).toBe(0.5);
    expect(n.normalise(9999)).toBe(0.5);
  });

  it('handles negative range', () => {
    const n = normaliseAxisValues([-10, 0, 10]);
    expect(n.min).toBe(-10);
    expect(n.max).toBe(10);
    expect(n.normalise(-10)).toBe(0);
    expect(n.normalise(0)).toBe(0.5);
    expect(n.normalise(10)).toBe(1);
  });

  it('empty → constant 0.5', () => {
    const n = normaliseAxisValues([]);
    expect(Number.isNaN(n.min)).toBe(true);
    expect(n.normalise(123)).toBe(0.5);
  });

  it('filters NaN inputs', () => {
    const n = normaliseAxisValues([Number.NaN, 2, 6]);
    expect(n.min).toBe(2);
    expect(n.max).toBe(6);
    expect(n.normalise(4)).toBe(0.5);
  });

  it('normalise of NaN returns 0.5 (gap sentinel)', () => {
    const n = normaliseAxisValues([1, 5]);
    expect(n.normalise(Number.NaN)).toBe(0.5);
  });
});

// ── buildHeatmapGrid ───────────────────────────────────────────────

describe('buildHeatmapGrid', () => {
  it('produces exact 2×2 grid for 2-param sweep', () => {
    const cs = [
      mkChild(0, { a: 1, b: 10 }, [mkVerdict('pass', 11)]),
      mkChild(1, { a: 2, b: 10 }, [mkVerdict('pass', 12)]),
      mkChild(2, { a: 1, b: 20 }, [mkVerdict('pass', 21)]),
      mkChild(3, { a: 2, b: 20 }, [mkVerdict('pass', 22)]),
    ];
    const grid = buildHeatmapGrid(cs, 'a', 'b', firstMargin);
    expect(grid.x).toEqual([1, 2]);
    expect(grid.y).toEqual([10, 20]);
    expect(grid.values.length).toBe(2);
    expect(grid.values[0]).toEqual([11, 12]); // y=10 row
    expect(grid.values[1]).toEqual([21, 22]); // y=20 row
  });

  it('fills missing cells with NaN (streaming)', () => {
    const cs = [
      mkChild(0, { a: 1, b: 10 }, [mkVerdict('pass', 11)]),
      mkChild(1, { a: 2, b: 20 }, [mkVerdict('pass', 22)]),
    ];
    const grid = buildHeatmapGrid(cs, 'a', 'b', firstMargin);
    expect(grid.values[0][0]).toBe(11);
    expect(Number.isNaN(grid.values[0][1])).toBe(true);
    expect(Number.isNaN(grid.values[1][0])).toBe(true);
    expect(grid.values[1][1]).toBe(22);
  });

  it('empty input returns empty grid (no throw)', () => {
    const grid = buildHeatmapGrid([], 'a', 'b', firstMargin);
    expect(grid.x).toEqual([]);
    expect(grid.y).toEqual([]);
    expect(grid.values).toEqual([]);
  });

  it('non-2-param input (missing axis) produces empty dimension', () => {
    // Only 'a' is swept; 'b' has no values on any child.
    const cs = [mkChild(0, { a: 1 }, [mkVerdict('pass', 1)])];
    const grid = buildHeatmapGrid(cs, 'a', 'b', firstMargin);
    expect(grid.x).toEqual([1]);
    expect(grid.y).toEqual([]); // no finite y values → empty
    expect(grid.values).toEqual([]);
  });

  it('later writes win on duplicate (x, y) — streaming override', () => {
    const cs = [
      mkChild(0, { a: 1, b: 1 }, []), // firstMargin → NaN initially
      mkChild(1, { a: 1, b: 1 }, [mkVerdict('pass', 99)]),
    ];
    const grid = buildHeatmapGrid(cs, 'a', 'b', firstMargin);
    // Second child rewrites the single cell with the real margin.
    expect(grid.values[0][0]).toBe(99);
  });
});

// ── rollupVerdict ──────────────────────────────────────────────────

describe('rollupVerdict', () => {
  it('returns null when no verdicts present', () => {
    expect(rollupVerdict(mkChild(0, {}, []))).toBe(null);
  });
  it('error dominates fail/pass', () => {
    expect(
      rollupVerdict(mkChild(0, {}, [mkVerdict('pass'), mkVerdict('error'), mkVerdict('fail')])),
    ).toBe('error');
  });
  it('fail dominates inconclusive', () => {
    expect(
      rollupVerdict(mkChild(0, {}, [mkVerdict('inconclusive'), mkVerdict('fail')])),
    ).toBe('fail');
  });
  it('inconclusive dominates pass', () => {
    expect(rollupVerdict(mkChild(0, {}, [mkVerdict('pass'), mkVerdict('inconclusive')]))).toBe(
      'inconclusive',
    );
  });
  it('all pass → pass', () => {
    expect(rollupVerdict(mkChild(0, {}, [mkVerdict('pass'), mkVerdict('pass')]))).toBe('pass');
  });
});

// ── extractorFor / MetricName ──────────────────────────────────────

describe('extractorFor', () => {
  it('fail_count counts fails', () => {
    const ex = extractorFor('fail_count');
    expect(ex(mkChild(0, {}, [mkVerdict('fail'), mkVerdict('pass'), mkVerdict('fail')]))).toBe(2);
  });
  it('margin returns first numeric margin, NaN when none', () => {
    const ex = extractorFor('margin');
    expect(ex(mkChild(0, {}, [mkVerdict('pass', 3.5)]))).toBe(3.5);
    expect(Number.isNaN(ex(mkChild(0, {}, [mkVerdict('pass')])))).toBe(true);
    expect(Number.isNaN(ex(mkChild(0, {}, [])))).toBe(true);
  });
});

// ── colourForNormalised ────────────────────────────────────────────

describe('colourForNormalised', () => {
  it('returns an rgb() string for valid input', () => {
    expect(colourForNormalised(0)).toMatch(/^rgb\(/);
    expect(colourForNormalised(0.5)).toMatch(/^rgb\(/);
    expect(colourForNormalised(1)).toMatch(/^rgb\(/);
  });
  it('clamps out-of-range inputs', () => {
    expect(colourForNormalised(-1)).toMatch(/^rgb\(/);
    expect(colourForNormalised(2)).toMatch(/^rgb\(/);
  });
  it('NaN → transparent sentinel', () => {
    expect(colourForNormalised(Number.NaN)).toBe('transparent');
  });
});
