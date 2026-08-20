/**
 * Tests for generateNestedChildrenParams (R5.4).
 *
 * Covers:
 *   - 1×1, 2×3 cartesian products
 *   - 3-level nesting (A → B → C)
 *   - Empty outer / inner → empty result
 *   - Error paths: unknown parent, duplicate name, cycle
 *   - Iteration order is stable and predictable (parent-before-child)
 */

import { describe, expect, it } from 'vitest';
import {
  generateNestedChildrenParams,
  NestedRangeError,
  type NestedRange,
} from '../nestedRanges';

describe('generateNestedChildrenParams — basics', () => {
  it('returns a single row for a 1×1 sweep', () => {
    const ranges: NestedRange[] = [{ param: 'a', values: [5] }];
    expect(generateNestedChildrenParams(ranges)).toEqual([{ a: 5 }]);
  });

  it('returns the full 2×3 cartesian product with parent → child nesting', () => {
    const ranges: NestedRange[] = [
      { param: 'temperature', values: [20, 40] },
      { param: 'voltage', values: [10, 12, 14], parent: 'temperature' },
    ];
    expect(generateNestedChildrenParams(ranges)).toEqual([
      { temperature: 20, voltage: 10 },
      { temperature: 20, voltage: 12 },
      { temperature: 20, voltage: 14 },
      { temperature: 40, voltage: 10 },
      { temperature: 40, voltage: 12 },
      { temperature: 40, voltage: 14 },
    ]);
  });

  it('expands a 3-level nesting (A → B → C) to A·B·C rows', () => {
    const ranges: NestedRange[] = [
      { param: 'a', values: [1, 2] },
      { param: 'b', values: [10, 20], parent: 'a' },
      { param: 'c', values: [100, 200], parent: 'b' },
    ];
    const result = generateNestedChildrenParams(ranges);
    expect(result).toHaveLength(2 * 2 * 2);
    expect(result[0]).toEqual({ a: 1, b: 10, c: 100 });
    expect(result[1]).toEqual({ a: 1, b: 10, c: 200 });
    expect(result[result.length - 1]).toEqual({ a: 2, b: 20, c: 200 });
  });
});

describe('generateNestedChildrenParams — empty sets', () => {
  it('returns [] when the list of ranges is empty', () => {
    expect(generateNestedChildrenParams([])).toEqual([]);
  });

  it('returns [] when the outer range is empty', () => {
    const ranges: NestedRange[] = [
      { param: 'outer', values: [] },
      { param: 'inner', values: [1, 2], parent: 'outer' },
    ];
    expect(generateNestedChildrenParams(ranges)).toEqual([]);
  });

  it('returns [] when the inner range is empty', () => {
    const ranges: NestedRange[] = [
      { param: 'outer', values: [1, 2] },
      { param: 'inner', values: [], parent: 'outer' },
    ];
    expect(generateNestedChildrenParams(ranges)).toEqual([]);
  });
});

describe('generateNestedChildrenParams — ordering', () => {
  it('processes parents before descendants even when input order is reversed', () => {
    const ranges: NestedRange[] = [
      { param: 'c', values: [100], parent: 'b' },
      { param: 'b', values: [10], parent: 'a' },
      { param: 'a', values: [1] },
    ];
    const result = generateNestedChildrenParams(ranges);
    expect(result).toEqual([{ a: 1, b: 10, c: 100 }]);
  });

  it('supports multiple independent roots (cartesian across roots)', () => {
    const ranges: NestedRange[] = [
      { param: 'x', values: [1, 2] },
      { param: 'y', values: [10] },
    ];
    // Two roots → full cartesian: 2×1 = 2 children.
    expect(generateNestedChildrenParams(ranges)).toEqual([
      { x: 1, y: 10 },
      { x: 2, y: 10 },
    ]);
  });
});

describe('generateNestedChildrenParams — errors', () => {
  it('throws on unknown parent', () => {
    const ranges: NestedRange[] = [
      { param: 'a', values: [1], parent: 'missing' },
    ];
    expect(() => generateNestedChildrenParams(ranges)).toThrow(
      NestedRangeError,
    );
  });

  it('throws on duplicate range name', () => {
    const ranges: NestedRange[] = [
      { param: 'a', values: [1] },
      { param: 'a', values: [2] },
    ];
    expect(() => generateNestedChildrenParams(ranges)).toThrow(
      /duplicate/i,
    );
  });

  it('throws on a cycle', () => {
    const ranges: NestedRange[] = [
      { param: 'a', values: [1], parent: 'b' },
      { param: 'b', values: [2], parent: 'a' },
    ];
    expect(() => generateNestedChildrenParams(ranges)).toThrow(/cycle/i);
  });
});
