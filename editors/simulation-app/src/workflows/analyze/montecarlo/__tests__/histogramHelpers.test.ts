/**
 * Tests for the pure histogram helpers (R5.7).
 *
 * Focus on the numerical contracts other agents will lean on:
 *   - `quantile` agrees with hand-computed interpolation on 10-element input
 *   - `buildHistogram` emits monotone-increasing bins, correct count sum,
 *     and recognisable stats on a known distribution
 *   - edge cases: empty arrays, all-equal, one-element
 */
import { describe, it, expect } from 'vitest';
import {
  buildHistogram,
  computeStats,
  kde,
  quantile,
} from '../histogramHelpers';

describe('quantile', () => {
  it('returns NaN on empty input', () => {
    expect(Number.isNaN(quantile([], 0.5))).toBe(true);
  });

  it('returns the sole element for length-1 input', () => {
    expect(quantile([42], 0.0)).toBe(42);
    expect(quantile([42], 1.0)).toBe(42);
    expect(quantile([42], 0.5)).toBe(42);
  });

  it('q=0 is min, q=1 is max', () => {
    const v = [3, 1, 4, 1, 5, 9, 2, 6];
    expect(quantile(v, 0)).toBe(1);
    expect(quantile(v, 1)).toBe(9);
  });

  it('q=0.5 is median on odd-length samples', () => {
    expect(quantile([1, 3, 5], 0.5)).toBe(3);
  });

  it('linear interpolation on 10-element series', () => {
    // 1..10; n-1 = 9; pos(q) = 9q
    const v = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    expect(quantile(v, 0.0)).toBe(1);
    expect(quantile(v, 1.0)).toBe(10);
    // pos=4.5 → linear interp between index 4 (value 5) and index 5 (value 6)
    expect(quantile(v, 0.5)).toBeCloseTo(5.5, 10);
    // pos=0.9 → between 1 and 2 with frac 0.9
    expect(quantile(v, 0.1)).toBeCloseTo(1.9, 10);
    // pos=8.1 → between 9 and 10 with frac 0.1
    expect(quantile(v, 0.9)).toBeCloseTo(9.1, 10);
  });

  it('clamps out-of-range q', () => {
    const v = [10, 20, 30];
    expect(quantile(v, -5)).toBe(10);
    expect(quantile(v, 5)).toBe(30);
  });

  it('does not mutate the input', () => {
    const v = [3, 1, 2];
    const copy = [...v];
    quantile(v, 0.5);
    expect(v).toEqual(copy);
  });
});

describe('computeStats', () => {
  it('returns NaN stats for empty input', () => {
    const s = computeStats([]);
    expect(Number.isNaN(s.mean)).toBe(true);
    expect(Number.isNaN(s.sigma)).toBe(true);
  });

  it('computes mean / σ correctly for a known sample', () => {
    // 1..5; mean 3, population variance 2, σ = √2
    const s = computeStats([1, 2, 3, 4, 5]);
    expect(s.mean).toBeCloseTo(3, 10);
    expect(s.sigma).toBeCloseTo(Math.sqrt(2), 10);
    expect(s.p50).toBeCloseTo(3, 10);
  });

  it('matches quantile positions for p5 / p95', () => {
    const v = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    const s = computeStats(v);
    // pos(0.05) = 0.45 → 1 + 0.45 = 1.45
    expect(s.p5).toBeCloseTo(1.45, 10);
    expect(s.p95).toBeCloseTo(9.55, 10);
  });
});

describe('buildHistogram', () => {
  it('returns zero bins and NaN stats on empty input', () => {
    const r = buildHistogram([], 10);
    expect(r.bins).toEqual([]);
    expect(Number.isNaN(r.stats.mean)).toBe(true);
  });

  it('emits a single synthetic bin when all values are equal', () => {
    const r = buildHistogram([7, 7, 7, 7], 10);
    expect(r.bins).toHaveLength(1);
    expect(r.bins[0].count).toBe(4);
    expect(r.bins[0].lower).toBeLessThan(7);
    expect(r.bins[0].upper).toBeGreaterThan(7);
  });

  it('bins are monotone-increasing and cover [min,max]', () => {
    const v = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    const r = buildHistogram(v, 5);
    expect(r.bins).toHaveLength(5);
    for (let i = 0; i < r.bins.length - 1; i++) {
      expect(r.bins[i].upper).toBeCloseTo(r.bins[i + 1].lower, 10);
      expect(r.bins[i].lower).toBeLessThan(r.bins[i].upper);
    }
    expect(r.bins[0].lower).toBeCloseTo(0, 10);
    expect(r.bins[r.bins.length - 1].upper).toBeCloseTo(10, 10);
  });

  it('counts sum to the number of finite input samples', () => {
    const v = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    const r = buildHistogram(v, 5);
    const total = r.bins.reduce((a, b) => a + b.count, 0);
    expect(total).toBe(v.length);
  });

  it('uniform samples produce roughly even bin counts', () => {
    // 500 evenly-spaced samples in [0, 10) → 5 bins of ~100 each
    const v = Array.from({ length: 500 }, (_, i) => (i / 500) * 10);
    const r = buildHistogram(v, 5);
    expect(r.bins).toHaveLength(5);
    for (const b of r.bins) {
      expect(b.count).toBe(100);
    }
  });

  it('stats match an analytic reference to 1e-6', () => {
    // Population stats over 1..100: mean = 50.5, σ = sqrt((n^2 - 1)/12)
    const v = Array.from({ length: 100 }, (_, i) => i + 1);
    const r = buildHistogram(v, 10);
    expect(r.stats.mean).toBeCloseTo(50.5, 6);
    expect(r.stats.sigma).toBeCloseTo(Math.sqrt((100 * 100 - 1) / 12), 6);
    // Final-bin-inclusive boundary means max lands inside the last bin.
    expect(r.bins[r.bins.length - 1].count).toBeGreaterThan(0);
  });

  it('clamps binCount to 1 when given zero/negative', () => {
    const r = buildHistogram([0, 1, 2, 3], 0);
    expect(r.bins).toHaveLength(1);
    expect(r.bins[0].count).toBe(4);
  });
});

describe('kde', () => {
  it('returns empty array for empty input', () => {
    expect(kde([], [0, 1, 2])).toEqual([]);
    expect(kde([1, 2, 3], [])).toEqual([]);
  });

  it('density is peaked near the mean of a tight cluster', () => {
    const v = [0, 0, 0.1, -0.1, 0.05, -0.05];
    const grid = [-2, -1, 0, 1, 2];
    const d = kde(v, grid);
    expect(d).toHaveLength(5);
    // Centre density strictly greater than either tail
    expect(d[2]).toBeGreaterThan(d[0]);
    expect(d[2]).toBeGreaterThan(d[4]);
  });
});
