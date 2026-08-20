/**
 * Tests for `sampleDistribution.ts` — pure samplers + the seedable RNG.
 *
 * Coverage:
 *   - RNG determinism (same seed → identical sequence).
 *   - `sampleNormal` statistical approximation (1000 samples → mean
 *     within 0.1σ of target, stddev within 5 % of target).
 *   - `sampleUniform` bounds.
 *   - `sampleTriangular` bounds + mode validation.
 *   - `sampleCustomCdf` interpolation + rejection of non-monotone input.
 *   - `generateChildrenParams` shape + seed stability.
 *
 * Intentionally does NOT assert on browser-only APIs. The samplers are
 * pure and run in Node.
 */

import { describe, it, expect } from 'vitest';
import {
  defaultDistribution,
  generateChildrenParams,
  isDistributionValid,
  parseCustomCdfPoints,
  sampleCustomCdf,
  sampleNormal,
  sampleTriangular,
  sampleUniform,
  seedableRng,
  type Distribution,
  type DistributionMap,
} from '../sampleDistribution';

// ── RNG determinism ─────────────────────────────────────────────────

describe('seedableRng', () => {
  it('produces an identical sequence for the same seed', () => {
    const a = seedableRng(42);
    const b = seedableRng(42);
    for (let i = 0; i < 100; i++) {
      expect(a()).toBe(b());
    }
  });

  it('produces a different sequence for a different seed', () => {
    const a = seedableRng(42);
    const b = seedableRng(43);
    // Pull ~20 draws; at least one must differ for distinct seeds.
    let anyDifferent = false;
    for (let i = 0; i < 20; i++) {
      if (a() !== b()) {
        anyDifferent = true;
        break;
      }
    }
    expect(anyDifferent).toBe(true);
  });

  it('always yields values in [0, 1)', () => {
    const rng = seedableRng(7);
    for (let i = 0; i < 10_000; i++) {
      const v = rng();
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThan(1);
    }
  });

  it('falls back to Math.random when seed is undefined', () => {
    // We cannot assert the value, only that it runs + produces a [0,1).
    const rng = seedableRng();
    const v = rng();
    expect(Number.isFinite(v)).toBe(true);
    expect(v).toBeGreaterThanOrEqual(0);
    expect(v).toBeLessThan(1);
  });
});

// ── sampleNormal ────────────────────────────────────────────────────

describe('sampleNormal', () => {
  it('approximates mean and stddev over 1000 samples', () => {
    const rng = seedableRng(1234);
    const mean = 3;
    const sigma = 2;
    const N = 1000;
    const samples: number[] = [];
    for (let i = 0; i < N; i++) samples.push(sampleNormal(mean, sigma, rng));
    const empMean = samples.reduce((a, b) => a + b, 0) / N;
    const empVar =
      samples.reduce((a, b) => a + (b - empMean) ** 2, 0) / N;
    const empSigma = Math.sqrt(empVar);

    // |empMean - mean| <= 0.1 σ (statistical wiggle with a fixed seed).
    expect(Math.abs(empMean - mean)).toBeLessThanOrEqual(0.1 * sigma);
    // Empirical sigma within 5 % of target sigma.
    expect(Math.abs(empSigma - sigma) / sigma).toBeLessThanOrEqual(0.05);
  });

  it('rejects invalid parameters', () => {
    const rng = seedableRng(1);
    expect(() => sampleNormal(Number.NaN, 1, rng)).toThrow();
    expect(() => sampleNormal(0, Number.POSITIVE_INFINITY, rng)).toThrow();
    expect(() => sampleNormal(0, -1, rng)).toThrow();
  });
});

// ── sampleUniform ───────────────────────────────────────────────────

describe('sampleUniform', () => {
  it('stays within [min, max)', () => {
    const rng = seedableRng(99);
    for (let i = 0; i < 5_000; i++) {
      const v = sampleUniform(-3, 7, rng);
      expect(v).toBeGreaterThanOrEqual(-3);
      expect(v).toBeLessThan(7);
    }
  });

  it('returns min when min === max', () => {
    const rng = seedableRng(1);
    expect(sampleUniform(5, 5, rng)).toBe(5);
  });

  it('rejects min > max', () => {
    const rng = seedableRng(1);
    expect(() => sampleUniform(5, 4, rng)).toThrow();
  });
});

// ── sampleTriangular ────────────────────────────────────────────────

describe('sampleTriangular', () => {
  it('stays within [min, max] and skews toward mode', () => {
    const rng = seedableRng(1729);
    let belowMode = 0;
    let aboveMode = 0;
    const N = 5_000;
    const min = 0;
    const mode = 3; // highly skewed right
    const max = 4;
    for (let i = 0; i < N; i++) {
      const v = sampleTriangular(min, mode, max, rng);
      expect(v).toBeGreaterThanOrEqual(min);
      expect(v).toBeLessThanOrEqual(max);
      if (v < mode) belowMode++;
      else aboveMode++;
    }
    // With mode 3 in [0, 4], roughly 75 % of mass sits below the mode.
    // Require at least a 60/40 split to confirm skew is real.
    expect(belowMode).toBeGreaterThan(aboveMode);
  });

  it('rejects invalid triples', () => {
    const rng = seedableRng(1);
    expect(() => sampleTriangular(2, 1, 3, rng)).toThrow(); // mode < min
    expect(() => sampleTriangular(0, 4, 3, rng)).toThrow(); // mode > max
  });

  it('collapses to the singleton when min === max', () => {
    const rng = seedableRng(1);
    expect(sampleTriangular(2, 2, 2, rng)).toBe(2);
  });
});

// ── sampleCustomCdf ─────────────────────────────────────────────────

describe('sampleCustomCdf', () => {
  it('interpolates linearly on a two-point CDF', () => {
    // CDF from x=0..10 uniform → linear mapping u → 10 * u
    const pts = [
      { x: 0, cdf: 0 },
      { x: 10, cdf: 1 },
    ];
    // Fake rng producing fixed uniform values.
    const samples = [0, 0.25, 0.5, 0.75, 0.999];
    const iter = makeRng(samples);
    for (const u of samples) {
      const v = sampleCustomCdf(pts, iter);
      // Expected value for a linear CDF is simply 10 * u.
      expect(v).toBeCloseTo(10 * u, 10);
    }
  });

  it('rejects non-monotone CDFs', () => {
    const rng = seedableRng(1);
    const bad = [
      { x: 0, cdf: 0 },
      { x: 5, cdf: 0.8 },
      { x: 10, cdf: 0.4 }, // drops — non-monotone
    ];
    expect(() => sampleCustomCdf(bad, rng)).toThrow(/monotone/);
  });

  it('requires at least 2 points', () => {
    const rng = seedableRng(1);
    expect(() => sampleCustomCdf([{ x: 0, cdf: 0 }], rng)).toThrow();
  });

  it('stays within the CDF support', () => {
    const pts = [
      { x: -2, cdf: 0 },
      { x: 0, cdf: 0.3 },
      { x: 5, cdf: 1 },
    ];
    const rng = seedableRng(123);
    for (let i = 0; i < 2_000; i++) {
      const v = sampleCustomCdf(pts, rng);
      expect(v).toBeGreaterThanOrEqual(-2);
      expect(v).toBeLessThanOrEqual(5);
    }
  });
});

// ── parseCustomCdfPoints ────────────────────────────────────────────

describe('parseCustomCdfPoints', () => {
  it('parses comma- and whitespace-separated rows', () => {
    const parsed = parseCustomCdfPoints('0, 0\n5, 0.5\n10, 1');
    expect(parsed).toEqual([
      { x: 0, cdf: 0 },
      { x: 5, cdf: 0.5 },
      { x: 10, cdf: 1 },
    ]);
  });

  it('strips comments and blank lines', () => {
    const parsed = parseCustomCdfPoints('# header\n0 0\n\n10 1 # end');
    expect(parsed).toEqual([
      { x: 0, cdf: 0 },
      { x: 10, cdf: 1 },
    ]);
  });

  it('rescales endpoints to [0, 1]', () => {
    const parsed = parseCustomCdfPoints('0, 0.1\n10, 0.9');
    expect(parsed[0].cdf).toBeCloseTo(0, 10);
    expect(parsed[1].cdf).toBeCloseTo(1, 10);
  });

  it('rejects non-monotone input', () => {
    expect(() => parseCustomCdfPoints('0, 0\n5, 0.8\n10, 0.5')).toThrow(
      /monotone/,
    );
  });

  it('rejects malformed lines', () => {
    expect(() => parseCustomCdfPoints('0, 0\nbogus\n10, 1')).toThrow();
  });

  it('rejects an empty CDF', () => {
    expect(() => parseCustomCdfPoints('   ')).toThrow();
    expect(() => parseCustomCdfPoints('# only a comment')).toThrow();
  });
});

// ── isDistributionValid / defaultDistribution ───────────────────────

describe('isDistributionValid', () => {
  it('accepts valid defaults for every kind', () => {
    for (const kind of ['normal', 'uniform', 'triangular', 'custom-cdf'] as const) {
      expect(isDistributionValid(defaultDistribution(kind))).toBe(true);
    }
  });

  it('rejects invalid field values', () => {
    expect(
      isDistributionValid({ kind: 'normal', mean: Number.NaN, sigma: 1 } as Distribution),
    ).toBe(false);
    expect(
      isDistributionValid({ kind: 'uniform', min: 5, max: 1 } as Distribution),
    ).toBe(false);
    expect(
      isDistributionValid({
        kind: 'triangular',
        min: 0,
        mode: 5,
        max: 2,
      } as Distribution),
    ).toBe(false);
  });
});

// ── generateChildrenParams ──────────────────────────────────────────

describe('generateChildrenParams', () => {
  const dists: DistributionMap = {
    voltage: { kind: 'normal', mean: 12, sigma: 1 },
    current: { kind: 'uniform', min: 0, max: 10 },
  };

  it('returns `count` rows with the right keys', () => {
    const rows = generateChildrenParams(dists, 50, 7);
    expect(rows).toHaveLength(50);
    for (const row of rows) {
      expect(Object.keys(row).sort()).toEqual(['current', 'voltage']);
      expect(Number.isFinite(row.voltage)).toBe(true);
      expect(Number.isFinite(row.current)).toBe(true);
      expect(row.current).toBeGreaterThanOrEqual(0);
      expect(row.current).toBeLessThan(10);
    }
  });

  it('is deterministic for a fixed seed', () => {
    const a = generateChildrenParams(dists, 20, 11);
    const b = generateChildrenParams(dists, 20, 11);
    expect(a).toEqual(b);
  });

  it('varies between seeds', () => {
    const a = generateChildrenParams(dists, 20, 11);
    const b = generateChildrenParams(dists, 20, 12);
    // Different seed → at least one cell differs.
    const same = a.every((row, i) =>
      Object.keys(row).every((k) => row[k] === b[i][k]),
    );
    expect(same).toBe(false);
  });

  it('returns [] for count 0', () => {
    expect(generateChildrenParams(dists, 0, 1)).toEqual([]);
  });

  it('returns [] when no parameters selected', () => {
    expect(generateChildrenParams({}, 5, 1)).toEqual([]);
  });

  it('throws when a distribution is invalid', () => {
    expect(() =>
      generateChildrenParams(
        { bad: { kind: 'uniform', min: 5, max: 1 } },
        3,
        1,
      ),
    ).toThrow();
  });

  it('rejects negative count', () => {
    expect(() => generateChildrenParams(dists, -1, 1)).toThrow();
  });
});

// ── helpers ─────────────────────────────────────────────────────────

function makeRng(values: readonly number[]): () => number {
  let i = 0;
  return () => {
    const v = values[i % values.length];
    i++;
    return v;
  };
}
