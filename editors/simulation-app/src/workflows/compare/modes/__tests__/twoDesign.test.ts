/**
 * Tests for two-design pure helpers.
 */
import { describe, it, expect } from 'vitest';
import {
  buildDesignWinnerTable,
  compareDesignDelta,
  hashOverrides,
  twoDesignMode,
  type DesignSeries,
} from '../twoDesign';

describe('compareDesignDelta', () => {
  it('zero delta for identical series', () => {
    const a = [
      { t: 0, v: 1 },
      { t: 1, v: 2 },
      { t: 2, v: 3 },
    ];
    const r = compareDesignDelta(a, a);
    expect(r.integral).toBe(0);
    expect(r.peakDelta).toBe(0);
    // Peak tick is null when delta never rises above zero.
    expect(r.peakTick).toBeNull();
  });

  it('integrates |A − B| trapezoidally', () => {
    // A = 0 everywhere, B = 2 at t=0, 2 at t=1 → |Δ| = 2 constant over [0,1]
    // ∫|Δ|dt = 2*1 = 2
    const a = [
      { t: 0, v: 0 },
      { t: 1, v: 0 },
    ];
    const b = [
      { t: 0, v: 2 },
      { t: 1, v: 2 },
    ];
    const r = compareDesignDelta(a, b);
    expect(r.integral).toBeCloseTo(2, 6);
    expect(r.peakDelta).toBe(2);
  });

  it('records peak delta and tick', () => {
    const a = [
      { t: 0, v: 0 },
      { t: 1, v: 0 },
      { t: 2, v: 0 },
    ];
    const b = [
      { t: 0, v: 1 },
      { t: 1, v: 10 },
      { t: 2, v: 2 },
    ];
    const r = compareDesignDelta(a, b);
    expect(r.peakDelta).toBe(10);
    expect(r.peakTick).toBe(1);
  });

  it('returns zeros for empty inputs', () => {
    expect(compareDesignDelta([], []).integral).toBe(0);
    expect(compareDesignDelta([], []).peakTick).toBeNull();
    expect(compareDesignDelta([], []).evaluatedTicks).toBe(0);
  });

  it('monotone series agree → zero delta', () => {
    const a = [
      { t: 0, v: 0 },
      { t: 1, v: 1 },
      { t: 2, v: 2 },
      { t: 3, v: 3 },
    ];
    const r = compareDesignDelta(a, a);
    expect(r.integral).toBe(0);
    expect(r.peakDelta).toBe(0);
  });

  it('detects peak at the right tick for a pulse', () => {
    const a = [
      { t: 0, v: 0 },
      { t: 1, v: 0 },
      { t: 2, v: 0 },
      { t: 3, v: 0 },
    ];
    const b = [
      { t: 0, v: 0 },
      { t: 1, v: 0 },
      { t: 2, v: 10 },
      { t: 3, v: 0 },
    ];
    const r = compareDesignDelta(a, b);
    expect(r.peakDelta).toBe(10);
    expect(r.peakTick).toBe(2);
  });

  it('returns zeros for one-sided empty', () => {
    const r = compareDesignDelta([], [{ t: 0, v: 1 }]);
    expect(r.integral).toBe(0);
    // No evaluable pair tick (A is null at t=0) → peakTick null
    expect(r.peakTick).toBeNull();
  });

  it('skips non-finite samples without breaking integration', () => {
    const a = [
      { t: 0, v: 0 },
      { t: 1, v: Number.NaN },
      { t: 2, v: 0 },
    ];
    const b = [
      { t: 0, v: 1 },
      { t: 1, v: 1 },
      { t: 2, v: 1 },
    ];
    const r = compareDesignDelta(a, b);
    expect(r.evaluatedTicks).toBe(2);
    expect(r.peakDelta).toBe(1);
  });
});

describe('hashOverrides', () => {
  it('matching maps → matching hash', () => {
    expect(hashOverrides({ a: 1, b: 2 })).toBe(hashOverrides({ b: 2, a: 1 }));
  });

  it('differing values → different hash', () => {
    expect(hashOverrides({ a: 1 })).not.toBe(hashOverrides({ a: 2 }));
  });

  it('empty + null are the same', () => {
    expect(hashOverrides({})).toBe(hashOverrides(null));
    expect(hashOverrides({})).toBe(hashOverrides(undefined));
  });
});

describe('buildDesignWinnerTable', () => {
  function mkDesign(id: string, vs: Record<string, Array<[number, number]>>): DesignSeries {
    return {
      sessionId: id,
      label: id,
      variables: Object.fromEntries(
        Object.entries(vs).map(([k, pts]) => [k, pts.map(([t, v]) => ({ t, v }))]),
      ),
    };
  }

  it('sorts rows by integral descending', () => {
    const a = mkDesign('A', {
      x: [[0, 0], [1, 0]],
      y: [[0, 0], [1, 0]],
    });
    const b = mkDesign('B', {
      x: [[0, 10], [1, 10]], // integral 10
      y: [[0, 2], [1, 2]], // integral 2
    });
    const table = buildDesignWinnerTable(a, b, ['x', 'y']);
    expect(table.rows.map((r) => r.variable)).toEqual(['x', 'y']);
  });

  it('flags overrides mismatch', () => {
    const a = { ...mkDesign('A', { x: [[0, 0]] }), overrides: { p: 1 } };
    const b = { ...mkDesign('B', { x: [[0, 0]] }), overrides: { p: 2 } };
    const table = buildDesignWinnerTable(a, b, ['x']);
    expect(table.overridesMismatch).toBe(true);
  });

  it('marks worseAt side', () => {
    const a = mkDesign('A', { x: [[0, 0], [1, 0]] });
    const b = mkDesign('B', { x: [[0, 5], [1, 5]] });
    const table = buildDesignWinnerTable(a, b, ['x']);
    expect(table.rows[0].worseAt).toBe('B');
  });

  it('worseAt is null when identical', () => {
    const a = mkDesign('A', { x: [[0, 1], [1, 2]] });
    const b = mkDesign('B', { x: [[0, 1], [1, 2]] });
    const table = buildDesignWinnerTable(a, b, ['x']);
    expect(table.rows[0].worseAt).toBeNull();
  });
});

describe('twoDesignMode registration', () => {
  it('has the stable id', () => {
    expect(twoDesignMode.id).toBe('two-design');
  });
});
