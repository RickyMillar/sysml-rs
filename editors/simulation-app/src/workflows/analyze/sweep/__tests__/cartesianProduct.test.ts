/**
 * cartesianProduct + expandRange unit tests.
 *
 * Brief-specified cases:
 *   - `{a: [1,2], b: [10,20]}` → full 2×2 grid in the declared order.
 *   - Empty input → `[{}]` (vacuous identity).
 *   - Any empty-range entry collapses to `[]`.
 *   - Single-value ranges behave like any other.
 *
 * Plus coverage for `expandRange`'s floating-point tolerance and the
 * round-trip `generateChildrenParams` wrapper used by the UI.
 */

import { describe, it, expect } from 'vitest';
import { cartesianProduct, expandRange } from '../cartesianProduct';
import { generateChildrenParams } from '../useSweepConfig';

describe('cartesianProduct — brief case', () => {
  it('produces the 2×2 grid in the stated order', () => {
    const pts = cartesianProduct({ a: [1, 2], b: [10, 20] });
    expect(pts).toEqual([
      { a: 1, b: 10 },
      { a: 1, b: 20 },
      { a: 2, b: 10 },
      { a: 2, b: 20 },
    ]);
  });

  it('single-value ranges behave like any other (one-point degenerate axis)', () => {
    expect(cartesianProduct({ a: [42], b: [1, 2] })).toEqual([
      { a: 42, b: 1 },
      { a: 42, b: 2 },
    ]);
  });

  it('preserves insertion order of keys in each point', () => {
    const pts = cartesianProduct({ b: [1, 2], a: [10, 20] });
    // First key encountered in the input is the outer loop.
    expect(pts[0]).toEqual({ b: 1, a: 10 });
    expect(pts[1]).toEqual({ b: 1, a: 20 });
  });
});

describe('cartesianProduct — edge cases', () => {
  it('empty input returns a single vacuous point', () => {
    expect(cartesianProduct({})).toEqual([{}]);
  });

  it('any zero-length range collapses the product to empty', () => {
    expect(cartesianProduct({ a: [1, 2], b: [] })).toEqual([]);
    expect(cartesianProduct({ a: [], b: [10, 20] })).toEqual([]);
    expect(cartesianProduct({ a: [], b: [] })).toEqual([]);
  });

  it('handles string-valued axes (generic product over unknown)', () => {
    expect(cartesianProduct({ mode: ['off', 'on'] })).toEqual([
      { mode: 'off' },
      { mode: 'on' },
    ]);
  });

  it('explodes combinatorially — 3×3×2 = 18 points', () => {
    const pts = cartesianProduct({
      a: [1, 2, 3],
      b: [10, 20, 30],
      c: [100, 200],
    });
    expect(pts.length).toBe(18);
    // spot-check first + last
    expect(pts[0]).toEqual({ a: 1, b: 10, c: 100 });
    expect(pts[pts.length - 1]).toEqual({ a: 3, b: 30, c: 200 });
  });
});

describe('expandRange — grid spec', () => {
  it('inclusive walk from min to max with integer step', () => {
    expect(expandRange({ kind: 'grid', min: 0, max: 4, step: 1 })).toEqual([0, 1, 2, 3, 4]);
  });

  it('handles sub-unit step without floating-point drift', () => {
    const pts = expandRange({ kind: 'grid', min: 0, max: 0.3, step: 0.1 });
    // Without tolerance guard the third point is 0.30000000000000004 →
    // must still land on the nose at 0.3.
    expect(pts).toEqual([0, 0.1, 0.2, 0.3]);
  });

  it('returns empty for ill-defined specs (step ≤ 0 or min > max)', () => {
    expect(expandRange({ kind: 'grid', min: 0, max: 1, step: 0 })).toEqual([]);
    expect(expandRange({ kind: 'grid', min: 0, max: 1, step: -0.1 })).toEqual([]);
    expect(expandRange({ kind: 'grid', min: 2, max: 1, step: 0.1 })).toEqual([]);
  });

  it('returns empty for non-finite inputs', () => {
    expect(expandRange({ kind: 'grid', min: NaN, max: 1, step: 0.1 })).toEqual([]);
    expect(expandRange({ kind: 'grid', min: 0, max: Infinity, step: 0.1 })).toEqual([]);
  });

  it('min === max + positive step → single point', () => {
    expect(expandRange({ kind: 'grid', min: 5, max: 5, step: 0.1 })).toEqual([5]);
  });
});

describe('expandRange — list spec', () => {
  it('passes through finite numbers unchanged', () => {
    expect(expandRange({ kind: 'list', values: [0.5, 1.5, 2.5] })).toEqual([0.5, 1.5, 2.5]);
  });

  it('drops non-finite values defensively', () => {
    expect(expandRange({ kind: 'list', values: [1, NaN, 2, Infinity, 3] })).toEqual([1, 2, 3]);
  });

  it('empty list stays empty', () => {
    expect(expandRange({ kind: 'list', values: [] })).toEqual([]);
  });
});

describe('generateChildrenParams — hook wrapper', () => {
  it('full pipeline: list specs → cartesian product', () => {
    const out = generateChildrenParams([
      { parameterId: 'a', spec: { kind: 'list', values: [1, 2] } },
      { parameterId: 'b', spec: { kind: 'list', values: [10, 20] } },
    ]);
    expect(out).toEqual([
      { a: 1, b: 10 },
      { a: 1, b: 20 },
      { a: 2, b: 10 },
      { a: 2, b: 20 },
    ]);
  });

  it('empty range entries → empty result (not vacuous [{}])', () => {
    expect(generateChildrenParams([])).toEqual([]);
  });

  it('grid spec expands then products', () => {
    const out = generateChildrenParams([
      { parameterId: 'x', spec: { kind: 'grid', min: 0, max: 2, step: 1 } },
      { parameterId: 'y', spec: { kind: 'list', values: [100] } },
    ]);
    expect(out).toEqual([
      { x: 0, y: 100 },
      { x: 1, y: 100 },
      { x: 2, y: 100 },
    ]);
  });

  it('a single empty range collapses everything to []', () => {
    const out = generateChildrenParams([
      { parameterId: 'a', spec: { kind: 'list', values: [1, 2] } },
      { parameterId: 'b', spec: { kind: 'grid', min: 5, max: 1, step: 1 } }, // empty
    ]);
    expect(out).toEqual([]);
  });
});
