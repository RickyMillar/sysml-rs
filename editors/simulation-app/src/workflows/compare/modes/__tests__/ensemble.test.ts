/**
 * Tests for ensemble-mode pure helpers.
 *
 * Focus: the numerical core — percentile boundaries, σ correctness,
 * outlier detection with NaN + single-element + degenerate inputs.
 */
import { describe, it, expect } from 'vitest';
import {
  buildEnsembleReport,
  computeEnsembleStats,
  detectOutliers,
  detectOutlierSessions,
  ensembleMode,
  percentile,
  summarizeSamples,
  type EnsembleRun,
} from '../ensemble';

describe('percentile', () => {
  it('returns null for empty arrays', () => {
    expect(percentile([], 0.5)).toBeNull();
  });

  it('returns the sole element for single-element arrays', () => {
    expect(percentile([42], 0)).toBe(42);
    expect(percentile([42], 0.5)).toBe(42);
    expect(percentile([42], 1)).toBe(42);
  });

  it('returns endpoints for q=0 and q=1', () => {
    expect(percentile([1, 2, 3, 4, 5], 0)).toBe(1);
    expect(percentile([1, 2, 3, 4, 5], 1)).toBe(5);
  });

  it('interpolates linearly', () => {
    // [0, 10], q=0.5 → midpoint = 5
    expect(percentile([0, 10], 0.5)).toBe(5);
    // [0, 100], q=0.25 → 25
    expect(percentile([0, 100], 0.25)).toBe(25);
  });

  it('clamps out-of-range q', () => {
    expect(percentile([1, 2, 3], -0.5)).toBe(1);
    expect(percentile([1, 2, 3], 1.5)).toBe(3);
  });
});

describe('summarizeSamples', () => {
  it('returns empty stats when no finite samples', () => {
    const s = summarizeSamples([null, Number.NaN, Number.POSITIVE_INFINITY]);
    expect(s.n).toBe(0);
    expect(s.mean).toBe(0);
    expect(s.sigma).toBe(0);
    expect(s.p5).toBeNull();
    expect(s.p95).toBeNull();
  });

  it('computes mean + population σ correctly', () => {
    // Values: 2, 4, 4, 4, 5, 5, 7, 9 → mean 5, pop σ 2
    const s = summarizeSamples([2, 4, 4, 4, 5, 5, 7, 9]);
    expect(s.mean).toBe(5);
    expect(s.sigma).toBeCloseTo(2, 6);
    expect(s.n).toBe(8);
    expect(s.min).toBe(2);
    expect(s.max).toBe(9);
  });

  it('drops NaN / null silently', () => {
    const s = summarizeSamples([1, Number.NaN, 3, null, 5]);
    expect(s.n).toBe(3);
    expect(s.mean).toBe(3);
  });

  it('handles a single finite sample', () => {
    const s = summarizeSamples([7]);
    expect(s.n).toBe(1);
    expect(s.mean).toBe(7);
    expect(s.sigma).toBe(0);
    expect(s.p5).toBe(7);
    expect(s.p95).toBe(7);
  });
});

describe('computeEnsembleStats', () => {
  it('samples each run at the given tick', () => {
    const runs: EnsembleRun[] = [
      { sessionId: 'a', points: [{ t: 0, v: 10 }, { t: 5, v: 20 }] },
      { sessionId: 'b', points: [{ t: 0, v: 12 }, { t: 5, v: 22 }] },
      { sessionId: 'c', points: [{ t: 0, v: 14 }, { t: 5, v: 24 }] },
    ];
    const stats = computeEnsembleStats(runs, 5);
    expect(stats.n).toBe(3);
    expect(stats.mean).toBeCloseTo(22, 6);
  });

  it('returns empty stats when no run has samples at/before tick', () => {
    const runs: EnsembleRun[] = [
      { sessionId: 'a', points: [{ t: 10, v: 5 }] },
    ];
    expect(computeEnsembleStats(runs, 0).n).toBe(0);
  });
});

describe('detectOutliers (pure)', () => {
  it('returns empty for empty / single-element arrays', () => {
    expect(detectOutliers([], 2)).toEqual([]);
    expect(detectOutliers([42], 2)).toEqual([]);
  });

  it('returns empty when σ is zero (all identical)', () => {
    expect(detectOutliers([5, 5, 5], 2)).toEqual([]);
  });

  it('returns empty for non-positive thresholds', () => {
    expect(detectOutliers([0, 10], 0)).toEqual([]);
    expect(detectOutliers([0, 10], -1)).toEqual([]);
    expect(detectOutliers([0, 10], Number.NaN)).toEqual([]);
  });

  it('flags indices outside 2σ', () => {
    // Values 0,0,0,0,0,100 → mean=100/6≈16.67, σ≈37.27, 2σ≈74.54.
    // |100 − 16.67| = 83.33 > 74.54 → index 5 flagged.
    expect(detectOutliers([0, 0, 0, 0, 0, 100], 2)).toEqual([5]);
  });

  it('ignores NaN samples when deciding outliers', () => {
    // Same as above but with a NaN at index 5 — stats are computed
    // from the six finite values; the giant value sits at index 6.
    const values = [0, 0, 0, 0, 0, Number.NaN, 100];
    const out = detectOutliers(values, 2);
    expect(out).toContain(6);
    expect(out).not.toContain(5);
  });

  it('handles the 2σ boundary (strict >)', () => {
    // Symmetric ensemble: -2,-1,0,1,2 → mean 0, σ = sqrt(2) ≈ 1.414,
    // 2σ ≈ 2.828. All samples are inside.
    expect(detectOutliers([-2, -1, 0, 1, 2], 2)).toEqual([]);
  });

  it('samples exactly at the threshold are NOT flagged', () => {
    // Construct a population whose σ is exactly 1 and a sample at
    // distance exactly 2 from the mean. Boundary rule says strict >,
    // so that sample is considered inside.
    // Values 0,0,0,0,4 → mean 0.8, σ² = (0.64·4 + 10.24)/5 = 2.56, σ=1.6
    // 2σ = 3.2. |4-0.8| = 3.2 exactly → strict > fails → NOT flagged.
    expect(detectOutliers([0, 0, 0, 0, 4], 2)).toEqual([]);
  });
});

describe('detectOutlierSessions (session wrapper)', () => {
  function mkRun(id: string, v: number): EnsembleRun {
    return { sessionId: id, points: [{ t: 0, v }] };
  }

  it('maps indices back to session ids', () => {
    const runs = [
      mkRun('a', 0),
      mkRun('b', 0),
      mkRun('c', 0),
      mkRun('d', 0),
      mkRun('e', 0),
      mkRun('f', 100),
    ];
    expect(detectOutlierSessions(runs, 0, 2)).toEqual(['f']);
  });
});

describe('buildEnsembleReport', () => {
  it('produces one row per variable, sorted alphabetically', () => {
    const runs: EnsembleRun[] = [
      { sessionId: 'a', points: [{ t: 0, v: 1 }] },
      { sessionId: 'b', points: [{ t: 0, v: 2 }] },
    ];
    const rows = buildEnsembleReport({ zVolts: runs, aAmps: runs }, 0, 2);
    expect(rows.map((r) => r.variable)).toEqual(['aAmps', 'zVolts']);
    expect(rows[0].stats.n).toBe(2);
  });

  it('handles empty input', () => {
    expect(buildEnsembleReport({}, 0, 2)).toEqual([]);
  });
});

describe('ensembleMode registration', () => {
  it('has the stable id', () => {
    expect(ensembleMode.id).toBe('ensemble');
    expect(typeof ensembleMode.configRender).toBe('function');
  });
});
