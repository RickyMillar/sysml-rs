/**
 * Tests for the shared series helpers.
 */
import { describe, it, expect } from 'vitest';
import { isFiniteNumber, sampleAtTick, zipSeries } from '../seriesAccess';

describe('isFiniteNumber', () => {
  it('accepts normal numbers', () => {
    expect(isFiniteNumber(0)).toBe(true);
    expect(isFiniteNumber(-1.5)).toBe(true);
    expect(isFiniteNumber(1e10)).toBe(true);
  });

  it('rejects NaN, Infinity, and non-numbers', () => {
    expect(isFiniteNumber(Number.NaN)).toBe(false);
    expect(isFiniteNumber(Number.POSITIVE_INFINITY)).toBe(false);
    expect(isFiniteNumber(Number.NEGATIVE_INFINITY)).toBe(false);
    expect(isFiniteNumber('0')).toBe(false);
    expect(isFiniteNumber(null)).toBe(false);
    expect(isFiniteNumber(undefined)).toBe(false);
  });
});

describe('sampleAtTick', () => {
  it('returns null for empty arrays', () => {
    expect(sampleAtTick([], 0)).toBeNull();
  });

  it('returns null when tick is before first sample', () => {
    expect(sampleAtTick([{ t: 5, v: 1 }], 0)).toBeNull();
  });

  it('returns last-known-value at an exact tick', () => {
    const pts = [
      { t: 0, v: 10 },
      { t: 5, v: 20 },
      { t: 10, v: 30 },
    ];
    expect(sampleAtTick(pts, 0)).toBe(10);
    expect(sampleAtTick(pts, 5)).toBe(20);
    expect(sampleAtTick(pts, 10)).toBe(30);
  });

  it('returns LKV between samples', () => {
    const pts = [
      { t: 0, v: 10 },
      { t: 5, v: 20 },
    ];
    expect(sampleAtTick(pts, 3)).toBe(10);
    expect(sampleAtTick(pts, 4.9)).toBe(10);
    expect(sampleAtTick(pts, 5.1)).toBe(20);
    expect(sampleAtTick(pts, 1000)).toBe(20);
  });

  it('returns null for non-finite values', () => {
    expect(sampleAtTick([{ t: 0, v: Number.NaN }], 0)).toBeNull();
  });
});

describe('zipSeries', () => {
  it('zips on the union of tick grids', () => {
    const a = [
      { t: 0, v: 1 },
      { t: 2, v: 3 },
    ];
    const b = [
      { t: 1, v: 10 },
      { t: 2, v: 20 },
    ];
    const zipped = zipSeries(a, b);
    expect(zipped.map((z) => z.t)).toEqual([0, 1, 2]);
    expect(zipped[0]).toEqual({ t: 0, a: 1, b: null });
    expect(zipped[1]).toEqual({ t: 1, a: 1, b: 10 });
    expect(zipped[2]).toEqual({ t: 2, a: 3, b: 20 });
  });

  it('returns empty for two empty inputs', () => {
    expect(zipSeries([], [])).toEqual([]);
  });
});
