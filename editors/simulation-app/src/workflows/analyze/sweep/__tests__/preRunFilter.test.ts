/**
 * Tests for applyPreRunFilter (R5.4).
 *
 * Covers:
 *   - All retained (no predicate)
 *   - All filtered (predicate never holds)
 *   - Partial (some retained)
 *   - Predicate on non-existent param → warn + retain all
 *   - Every `CompareOp` branch through compareNumbers
 *   - Input is not mutated
 */

import { describe, expect, it, vi } from 'vitest';
import { applyPreRunFilter, compareNumbers } from '../preRunFilter';
import type { ChildDescriptor, ParamPredicate } from '@/engine/types';

let nextIndex = 0;
function descriptor(
  id: string,
  params: Record<string, number>,
): ChildDescriptor {
  return { id, session_id: null, index: nextIndex++, status: 'pending', params };
}

describe('compareNumbers', () => {
  it('evaluates every CompareOp branch', () => {
    expect(compareNumbers(1, 'lt', 2)).toBe(true);
    expect(compareNumbers(2, 'lt', 2)).toBe(false);
    expect(compareNumbers(2, 'le', 2)).toBe(true);
    expect(compareNumbers(3, 'le', 2)).toBe(false);
    expect(compareNumbers(3, 'gt', 2)).toBe(true);
    expect(compareNumbers(2, 'gt', 2)).toBe(false);
    expect(compareNumbers(2, 'ge', 2)).toBe(true);
    expect(compareNumbers(1, 'ge', 2)).toBe(false);
    expect(compareNumbers(5, 'eq', 5)).toBe(true);
    expect(compareNumbers(4, 'eq', 5)).toBe(false);
    expect(compareNumbers(5, 'ne', 5)).toBe(false);
    expect(compareNumbers(4, 'ne', 5)).toBe(true);
  });
});

describe('applyPreRunFilter', () => {
  const children = [
    descriptor('c0', { voltage: 10, current: 1 }),
    descriptor('c1', { voltage: 12, current: 1.2 }),
    descriptor('c2', { voltage: 14, current: 1.4 }),
  ];

  it('retains all children when the predicate is null', () => {
    expect(applyPreRunFilter(children, null)).toEqual(children);
  });

  it('retains all children when the predicate is undefined', () => {
    expect(applyPreRunFilter(children, undefined)).toEqual(children);
  });

  it('retains no children when the predicate is never satisfied', () => {
    const p: ParamPredicate = { param: 'voltage', op: 'gt', value: 100 };
    expect(applyPreRunFilter(children, p)).toEqual([]);
  });

  it('retains a subset when the predicate is partially satisfied', () => {
    const p: ParamPredicate = { param: 'voltage', op: 'gt', value: 10 };
    const result = applyPreRunFilter(children, p);
    expect(result.map((c) => c.id)).toEqual(['c1', 'c2']);
  });

  it('warns and retains all when the predicate references a missing param', () => {
    const warn = vi.fn();
    const p: ParamPredicate = { param: 'unknownParam', op: 'gt', value: 0 };
    const result = applyPreRunFilter(children, p, { warn });
    expect(result).toEqual(children);
    expect(warn).toHaveBeenCalledTimes(1);
    expect(warn.mock.calls[0][0]).toMatch(/unknownParam/);
  });

  it('warns exactly once when the missing param affects multiple children', () => {
    const warn = vi.fn();
    const big = Array.from({ length: 20 }, (_, i) =>
      descriptor(`c${i}`, { voltage: i }),
    );
    applyPreRunFilter(big, { param: 'missing', op: 'eq', value: 1 }, { warn });
    expect(warn).toHaveBeenCalledTimes(1);
  });

  it('does not mutate the input array', () => {
    const p: ParamPredicate = { param: 'voltage', op: 'gt', value: 10 };
    const copy = children.map((c) => ({ ...c, params: { ...c.params } }));
    applyPreRunFilter(children, p);
    expect(children).toEqual(copy);
  });

  it('returns a new array reference even when the filter is a no-op', () => {
    const out = applyPreRunFilter(children, null);
    expect(out).not.toBe(children);
    expect(out).toEqual(children);
  });
});
