/**
 * Tests for the pure statistical helpers (R7.2).
 *
 * Focus points:
 *   - CI helpers agree with textbook normal assumptions on large samples
 *     and degrade gracefully on small / edge-case samples.
 *   - `fitDistribution` picks the right family for known generators.
 *   - `kolmogorovSmirnov` returns exactly the expected D-statistic for a
 *     small hand-computed example.
 *   - `tTestOneSided` matches a textbook example.
 */

import { describe, it, expect } from 'vitest';
import {
  bootstrapCI,
  confidenceInterval,
  createSeededRng,
  fitDistribution,
  kolmogorovSmirnov,
  kurtosis,
  mean,
  normalCdf,
  normalQuantile,
  qqPoints,
  quantile,
  skewness,
  stddev,
  tCdf,
  tQuantile,
  tTestOneSided,
} from '../statsHelpers';

// ── Helpers for generating deterministic samples ────────────────────

/** Box-Muller normal samples using a seeded RNG. */
function normalSamples(n: number, mu: number, sigma: number, rng: () => number): number[] {
  const out: number[] = [];
  while (out.length < n) {
    const u1 = Math.max(rng(), 1e-12);
    const u2 = rng();
    const r = Math.sqrt(-2 * Math.log(u1));
    const theta = 2 * Math.PI * u2;
    out.push(mu + sigma * r * Math.cos(theta));
    if (out.length < n) out.push(mu + sigma * r * Math.sin(theta));
  }
  return out;
}

/** Uniform samples on [lo, hi). */
function uniformSamples(n: number, lo: number, hi: number, rng: () => number): number[] {
  const out = new Array<number>(n);
  for (let i = 0; i < n; i++) out[i] = lo + rng() * (hi - lo);
  return out;
}

/** Lognormal samples — exp(normal). */
function lognormalSamples(n: number, mu: number, sigma: number, rng: () => number): number[] {
  return normalSamples(n, mu, sigma, rng).map((x) => Math.exp(x));
}

describe('basic moments', () => {
  it('mean returns NaN on empty input', () => {
    expect(Number.isNaN(mean([]))).toBe(true);
  });

  it('mean / stddev agree with hand-computation', () => {
    const v = [2, 4, 4, 4, 5, 5, 7, 9];
    expect(mean(v)).toBeCloseTo(5, 10);
    // Sample stddev (Bessel): sqrt(32 / 7) ≈ 2.138...
    expect(stddev(v)).toBeCloseTo(Math.sqrt(32 / 7), 10);
  });

  it('stddev returns 0 on single-element and NaN on empty', () => {
    expect(stddev([7])).toBe(0);
    expect(Number.isNaN(stddev([]))).toBe(true);
  });

  it('skewness is ~0 on a symmetric sample and positive on a right-skew sample', () => {
    const symmetric = [-3, -2, -1, 0, 1, 2, 3];
    expect(Math.abs(skewness(symmetric))).toBeLessThan(1e-9);
    const rightSkew = [1, 1, 1, 1, 2, 2, 3, 10];
    expect(skewness(rightSkew)).toBeGreaterThan(0);
  });

  it('kurtosis is ~0 on a normal sample (sanity check)', () => {
    const rng = createSeededRng(1234);
    const v = normalSamples(2000, 0, 1, rng);
    expect(Math.abs(kurtosis(v))).toBeLessThan(0.3);
  });

  it('filters NaN from inputs without throwing', () => {
    const v = [1, 2, Number.NaN, 3, Number.POSITIVE_INFINITY];
    expect(mean(v)).toBe(2);
    expect(stddev(v)).toBeCloseTo(1, 10);
  });
});

describe('quantile', () => {
  it('handles edge cases', () => {
    expect(Number.isNaN(quantile([], 0.5))).toBe(true);
    expect(quantile([7], 0.5)).toBe(7);
  });

  it('matches NumPy "linear" method on a 10-element series', () => {
    const v = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    expect(quantile(v, 0.5)).toBeCloseTo(5.5, 10);
    expect(quantile(v, 0.1)).toBeCloseTo(1.9, 10);
    expect(quantile(v, 0.9)).toBeCloseTo(9.1, 10);
  });
});

describe('normalCdf / normalQuantile / tQuantile', () => {
  it('normalCdf(0) ≈ 0.5 and is symmetric', () => {
    expect(normalCdf(0)).toBeCloseTo(0.5, 6);
    expect(normalCdf(1) + normalCdf(-1)).toBeCloseTo(1, 6);
  });

  it('normalQuantile is the inverse of normalCdf', () => {
    for (const p of [0.05, 0.1, 0.25, 0.5, 0.75, 0.9, 0.95]) {
      const z = normalQuantile(p);
      expect(normalCdf(z)).toBeCloseTo(p, 3);
    }
  });

  it('normalQuantile(0.975) ≈ 1.96 (classic 95% CI z)', () => {
    expect(normalQuantile(0.975)).toBeCloseTo(1.96, 2);
  });

  it('tQuantile(0.975, df=10) ≈ 2.228 (textbook t)', () => {
    expect(tQuantile(0.975, 10)).toBeCloseTo(2.228, 2);
  });

  it('tQuantile approaches the normal z for df ≥ 30', () => {
    expect(tQuantile(0.975, 30)).toBeCloseTo(normalQuantile(0.975), 3);
  });

  it('tCdf(0, df) ≈ 0.5', () => {
    expect(tCdf(0, 5)).toBeCloseTo(0.5, 6);
    expect(tCdf(0, 50)).toBeCloseTo(0.5, 6);
  });
});

describe('confidenceInterval', () => {
  it('returns NaN bounds on empty input', () => {
    const ci = confidenceInterval([], 0.95);
    expect(Number.isNaN(ci.lower)).toBe(true);
    expect(Number.isNaN(ci.upper)).toBe(true);
    expect(Number.isNaN(ci.mean)).toBe(true);
  });

  it('n=1 returns a degenerate point interval', () => {
    const ci = confidenceInterval([42], 0.95);
    expect(ci.lower).toBe(42);
    expect(ci.upper).toBe(42);
    expect(ci.sem).toBe(0);
  });

  it('n=1000 normal sample → tight CI centred near the true mean', () => {
    const rng = createSeededRng(42);
    const v = normalSamples(1000, 10, 2, rng);
    const ci = confidenceInterval(v, 0.95);
    // The interval ~= mean ± 1.96 * σ/√n. With σ≈2 and n=1000, half-
    // width is ≈ 0.124. Assert the observed half-width is in that
    // ballpark and that the interval covers the true mean.
    const halfWidth = (ci.upper - ci.lower) / 2;
    expect(halfWidth).toBeLessThan(0.2);
    expect(halfWidth).toBeGreaterThan(0.05);
    expect(ci.lower).toBeLessThan(10);
    expect(ci.upper).toBeGreaterThan(10);
  });

  it('small n uses the wider Student-t critical value', () => {
    const v = [4, 5, 6, 5, 4, 5, 6];
    const ci = confidenceInterval(v, 0.95);
    // df=6 → t ≈ 2.447 — wider than the normal 1.96.
    // Interval half-width should be ~ 2.447 * σ/√n.
    const sd = stddev(v);
    const n = v.length;
    const expectedHalf = 2.447 * (sd / Math.sqrt(n));
    const actualHalf = (ci.upper - ci.lower) / 2;
    expect(actualHalf).toBeCloseTo(expectedHalf, 2);
  });
});

describe('bootstrapCI', () => {
  it('empty input returns NaN bounds', () => {
    const rng = createSeededRng(1);
    const ci = bootstrapCI([], 0.95, 500, rng);
    expect(Number.isNaN(ci.lower)).toBe(true);
    expect(Number.isNaN(ci.upper)).toBe(true);
  });

  it('single-element returns the point as both bounds', () => {
    const rng = createSeededRng(1);
    const ci = bootstrapCI([7], 0.95, 100, rng);
    expect(ci.lower).toBe(7);
    expect(ci.upper).toBe(7);
  });

  it('deterministic with a fixed-seed RNG', () => {
    const v = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
    const ci1 = bootstrapCI(v, 0.95, 500, createSeededRng(777));
    const ci2 = bootstrapCI(v, 0.95, 500, createSeededRng(777));
    expect(ci1.lower).toBe(ci2.lower);
    expect(ci1.upper).toBe(ci2.upper);
  });

  it('normal-distributed sample → CI includes the true mean', () => {
    const rng = createSeededRng(99);
    const v = normalSamples(500, 5, 1, rng);
    const ciRng = createSeededRng(31337);
    const ci = bootstrapCI(v, 0.95, 1000, ciRng);
    expect(ci.lower).toBeLessThan(5);
    expect(ci.upper).toBeGreaterThan(5);
    // Half-width should be close to 1.96 * σ / √n ≈ 0.0877.
    const half = (ci.upper - ci.lower) / 2;
    expect(half).toBeGreaterThan(0.03);
    expect(half).toBeLessThan(0.2);
  });
});

describe('kolmogorovSmirnov', () => {
  it('returns NaN on empty input', () => {
    expect(Number.isNaN(kolmogorovSmirnov([], (x) => x))).toBe(true);
  });

  it('D = 0.1 for uniform samples at (0.1, 0.3, 0.5, 0.7, 0.9) vs uniform CDF', () => {
    // F_n(x_i) = (i+1)/n lands 0.1 above and 0.1 below the uniform CDF at
    // each sample; sup |F_n − F| = 0.1.
    const values = [0.1, 0.3, 0.5, 0.7, 0.9];
    const d = kolmogorovSmirnov(values, (x) => Math.min(1, Math.max(0, x)));
    expect(d).toBeCloseTo(0.1, 6);
  });

  it('small D against a well-fitted normal distribution', () => {
    const rng = createSeededRng(222);
    const v = normalSamples(500, 0, 1, rng);
    const d = kolmogorovSmirnov(v, (x) => normalCdf(x));
    // Critical value at α=0.05 for n=500 is 1.36/√500 ≈ 0.061.
    expect(d).toBeLessThan(0.08);
  });
});

describe('fitDistribution', () => {
  it('returns unknown on tiny samples', () => {
    const fit = fitDistribution([1, 2]);
    expect(fit.family).toBe('unknown');
  });

  it('identifies a pure-normal sample', () => {
    const rng = createSeededRng(101);
    const v = normalSamples(1000, 0, 1, rng);
    const fit = fitDistribution(v);
    expect(fit.family).toBe('normal');
    expect(fit.params.mu).toBeCloseTo(0, 1);
    expect(fit.params.sigma).toBeCloseTo(1, 1);
  });

  it('identifies a pure-uniform sample', () => {
    const rng = createSeededRng(202);
    const v = uniformSamples(1000, 0, 10, rng);
    const fit = fitDistribution(v);
    expect(fit.family).toBe('uniform');
    expect(fit.params.min).toBeLessThan(0.5);
    expect(fit.params.max).toBeGreaterThan(9.5);
  });

  it('identifies a heavy-tailed (lognormal) sample', () => {
    const rng = createSeededRng(303);
    const v = lognormalSamples(1000, 0, 1, rng);
    const fit = fitDistribution(v);
    expect(fit.family).toBe('lognormal');
  });

  it('falls back to unknown for high-entropy junk', () => {
    // Mix of three generators → no single family wins.
    const rng = createSeededRng(404);
    const a = normalSamples(50, -5, 1, rng);
    const b = uniformSamples(50, 100, 200, rng);
    const c = normalSamples(50, 50, 20, rng);
    const fit = fitDistribution([...a, ...b, ...c]);
    expect(fit.family).toBe('unknown');
  });
});

describe('tTestOneSided', () => {
  it('returns NaN fields for tiny samples', () => {
    const r = tTestOneSided([1], 0);
    expect(Number.isNaN(r.tStatistic)).toBe(true);
    expect(Number.isNaN(r.pValue)).toBe(true);
  });

  it('textbook example: bulb lifetime vs 1000 h', () => {
    // Values borrowed from a standard stats example — sample mean ~1034,
    // n=10, σ≈45. Under H0: μ=1000, t ≈ 2.39, one-sided p ≈ 0.02.
    const v = [1007, 1032, 1014, 1105, 1087, 978, 1010, 996, 1053, 1063];
    const r = tTestOneSided(v, 1000);
    expect(r.df).toBe(9);
    expect(r.tStatistic).toBeGreaterThan(2);
    expect(r.tStatistic).toBeLessThan(3);
    expect(r.pValue).toBeGreaterThan(0.001);
    expect(r.pValue).toBeLessThan(0.05);
  });

  it('p-value is ~0.5 when the sample mean equals H0 mean', () => {
    const v = [9, 10, 11, 10, 9, 11, 10];
    const r = tTestOneSided(v, 10);
    expect(r.pValue).toBeCloseTo(0.5, 2);
  });
});

describe('qqPoints', () => {
  it('empty input returns empty array', () => {
    expect(qqPoints([], (p) => p)).toEqual([]);
  });

  it('sorted observed values paired with expected quantiles', () => {
    // Against identity CDF the Q-Q plot diagonals should match the sample
    // (in uniform-like ordering).
    const v = [0.9, 0.1, 0.5];
    const pts = qqPoints(v, (p) => p);
    expect(pts.length).toBe(3);
    expect(pts[0].observed).toBeCloseTo(0.1, 6);
    expect(pts[2].observed).toBeCloseTo(0.9, 6);
    // Plotting positions are (i + 0.5)/n: 1/6, 3/6, 5/6.
    expect(pts[0].expected).toBeCloseTo(1 / 6, 6);
    expect(pts[1].expected).toBeCloseTo(3 / 6, 6);
    expect(pts[2].expected).toBeCloseTo(5 / 6, 6);
  });
});

describe('createSeededRng', () => {
  it('produces deterministic streams for the same seed', () => {
    const r1 = createSeededRng(5);
    const r2 = createSeededRng(5);
    for (let i = 0; i < 10; i++) expect(r1()).toBe(r2());
  });

  it('produces values in [0, 1)', () => {
    const r = createSeededRng(9);
    for (let i = 0; i < 500; i++) {
      const v = r();
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThan(1);
    }
  });
});
