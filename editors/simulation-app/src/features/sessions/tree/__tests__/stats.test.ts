import { describe, expect, it } from 'vitest';
import { computeAttributeStats } from '../detail/stats';

describe('computeAttributeStats', () => {
  it('returns null on empty input', () => {
    expect(computeAttributeStats([])).toBeNull();
  });

  it('returns null when every sample is non-finite', () => {
    expect(
      computeAttributeStats([
        { t: 0, v: NaN },
        { t: 1, v: Number.POSITIVE_INFINITY },
      ]),
    ).toBeNull();
  });

  it('min / max / mean / stddev / count / t-range for a clean series', () => {
    const s = computeAttributeStats([
      { t: 0, v: 2 },
      { t: 1, v: 4 },
      { t: 2, v: 4 },
      { t: 3, v: 4 },
      { t: 4, v: 5 },
      { t: 5, v: 5 },
      { t: 6, v: 7 },
      { t: 7, v: 9 },
    ]);
    expect(s?.count).toBe(8);
    expect(s?.min).toBe(2);
    expect(s?.max).toBe(9);
    expect(s?.mean).toBe(5);
    // Sum of squared deviations: 9+1+1+1+0+0+4+16 = 32.
    // Sample stddev (n-1) = sqrt(32/7) ≈ 2.1381.
    expect(s?.stddev).toBeCloseTo(Math.sqrt(32 / 7), 6);
    expect(s?.tFirst).toBe(0);
    expect(s?.tLast).toBe(7);
  });

  it('single-sample series → stddev 0 (no divide-by-zero)', () => {
    const s = computeAttributeStats([{ t: 3, v: 42 }]);
    expect(s).toEqual({
      count: 1,
      min: 42,
      max: 42,
      mean: 42,
      stddev: 0,
      tFirst: 3,
      tLast: 3,
    });
  });

  it('skips NaN / Infinity samples mid-series', () => {
    const s = computeAttributeStats([
      { t: 0, v: 10 },
      { t: 1, v: NaN },
      { t: 2, v: Number.POSITIVE_INFINITY },
      { t: 3, v: 20 },
    ]);
    expect(s?.count).toBe(2);
    expect(s?.min).toBe(10);
    expect(s?.max).toBe(20);
    expect(s?.mean).toBe(15);
  });

  it('constant series stddev is 0', () => {
    const s = computeAttributeStats([
      { t: 0, v: 7 },
      { t: 1, v: 7 },
      { t: 2, v: 7 },
    ]);
    expect(s?.stddev).toBe(0);
  });
});
