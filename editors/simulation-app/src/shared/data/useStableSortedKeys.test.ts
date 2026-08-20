import { describe, expect, it } from 'vitest';
import { computeStableSortedKeys } from './useStableSortedKeys';

describe('computeStableSortedKeys', () => {
  it('returns a sorted key array on first call (no previous)', () => {
    const out = computeStableSortedKeys({ b: 1, a: 2 }, null);
    expect(out).toEqual(['a', 'b']);
  });

  it('returns the SAME reference when only values change, not the key set', () => {
    const prev = computeStableSortedKeys({ a: 1, b: 2 }, null);
    const next = computeStableSortedKeys({ a: 99, b: -1 }, prev);
    expect(next).toBe(prev); // reference equality — this is the whole point
  });

  it('returns a NEW array when a key is added', () => {
    const prev = computeStableSortedKeys({ a: 1 }, null);
    const next = computeStableSortedKeys({ a: 1, b: 2 }, prev);
    expect(next).not.toBe(prev);
    expect(next).toEqual(['a', 'b']);
  });

  it('returns a NEW array when a key is removed', () => {
    const prev = computeStableSortedKeys({ a: 1, b: 2 }, null);
    const next = computeStableSortedKeys({ a: 1 }, prev);
    expect(next).not.toBe(prev);
    expect(next).toEqual(['a']);
  });

  it('returns a NEW array when the key set is the same size but different names', () => {
    const prev = computeStableSortedKeys({ a: 1, b: 2 }, null);
    const next = computeStableSortedKeys({ a: 1, c: 2 }, prev);
    expect(next).not.toBe(prev);
    expect(next).toEqual(['a', 'c']);
  });
});
