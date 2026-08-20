/**
 * selectors — pure divergence/variance math for the compare workflow.
 *
 * These helpers drive the heat-band gutter and the top-6 variable
 * auto-pick. Both are performance-sensitive and called on every tick,
 * so edge cases (1 session, all-equal, NaNs, ragged lengths) are
 * pinned here.
 */

import { describe, expect, it } from 'vitest';
import {
  autoPickVariables,
  computeDivergence,
  crossSessionVariance,
  divergenceColor,
  normaliseSamples,
  peakDivergenceTick,
  playheadMaxTick,
} from '../selectors';

describe('computeDivergence', () => {
  it('returns empty array for zero sessions', () => {
    expect(computeDivergence([])).toEqual([]);
  });

  it('returns all-zeros when only one session is picked', () => {
    expect(computeDivergence([[1, 2, 3, 4]])).toEqual([0, 0, 0, 0]);
  });

  it('returns all-zeros when all sessions are identical (no spread)', () => {
    const d = computeDivergence([
      [1, 2, 3, 4],
      [1, 2, 3, 4],
      [1, 2, 3, 4],
    ]);
    expect(d).toEqual([0, 0, 0, 0]);
  });

  it('returns all-zeros when values are constant across ticks and sessions', () => {
    expect(computeDivergence([[5, 5, 5], [5, 5, 5]])).toEqual([0, 0, 0]);
  });

  it('scales the peak to 1.0 at the tick of maximum spread', () => {
    // global range = [0, 10], spread at tick 2 = 10 → score 1.0
    const d = computeDivergence([
      [0, 0, 0, 0],
      [0, 0, 10, 0],
    ]);
    expect(d[2]).toBeCloseTo(1, 6);
    expect(d[0]).toBe(0);
  });

  it('monotonically increases the score as two-session spread grows', () => {
    const d = computeDivergence([
      [0, 0, 0, 0, 0],
      [0, 1, 2, 3, 4],
    ]);
    expect(d[0]).toBe(0);
    for (let t = 1; t < d.length; t++) {
      expect(d[t]).toBeGreaterThanOrEqual(d[t - 1]);
    }
  });

  it('pads ragged rows with NaN and ignores NaNs in the spread', () => {
    // session A has 4 ticks, session B has 2 — beyond tick 1 B contributes NaN.
    const d = computeDivergence([
      [0, 0, 5, 10],
      [0, 10],
    ]);
    // global range spans [0, 10]. Tick 0: both present, spread 0. Tick 1:
    // spread 10 → 1.0. Tick 2..3: only A present → score 0.
    expect(d).toHaveLength(4);
    expect(d[0]).toBe(0);
    expect(d[1]).toBeCloseTo(1, 6);
    expect(d[2]).toBe(0);
    expect(d[3]).toBe(0);
  });

  it('ignores ticks where fewer than 2 sessions have values', () => {
    const d = computeDivergence([
      [NaN, 1, 2],
      [0, NaN, NaN],
    ]);
    // Tick 0: only B. Tick 1: only A. Tick 2: only A. All should be 0.
    expect(d).toEqual([0, 0, 0]);
  });

  it('clamps scores into [0, 1]', () => {
    const d = computeDivergence([
      [0, 0],
      [100, 100],
    ]);
    for (const v of d) expect(v).toBeGreaterThanOrEqual(0);
    for (const v of d) expect(v).toBeLessThanOrEqual(1);
  });
});

describe('peakDivergenceTick', () => {
  it('returns -1 when the input is empty or all zero', () => {
    expect(peakDivergenceTick([])).toBe(-1);
    expect(peakDivergenceTick([0, 0, 0])).toBe(-1);
  });
  it('returns the index of the first peak on ties', () => {
    expect(peakDivergenceTick([0.2, 0.9, 0.9, 0.3])).toBe(1);
  });
  it('picks the strict maximum', () => {
    expect(peakDivergenceTick([0.1, 0.3, 0.99, 0.5])).toBe(2);
  });
});

describe('crossSessionVariance', () => {
  it('is 0 for single-session or empty inputs', () => {
    expect(crossSessionVariance([])).toBe(0);
    expect(crossSessionVariance([[1, 2, 3]])).toBe(0);
  });
  it('is 0 for identical sessions', () => {
    expect(crossSessionVariance([[1, 2, 3], [1, 2, 3]])).toBe(0);
  });
  it('is positive when sessions diverge', () => {
    expect(
      crossSessionVariance([[0, 0, 0], [10, 10, 10]]),
    ).toBeGreaterThan(0);
  });
});

describe('autoPickVariables', () => {
  it('returns top-N variables by descending variance', () => {
    const vars = {
      quiet: [[1, 1, 1], [1, 1, 1]],
      loud: [[0, 0, 0], [10, 10, 10]],
      medium: [[0, 1, 2], [2, 1, 0]],
    };
    const picks = autoPickVariables(vars, 2);
    expect(picks[0]).toBe('loud');
    expect(picks).toContain('medium');
    expect(picks).not.toContain('quiet');
  });

  it('breaks ties alphabetically for deterministic output', () => {
    // All zero variance → ties everywhere → alpha order wins.
    const vars: Record<string, number[][]> = {
      beta: [[1, 1], [1, 1]],
      alpha: [[1, 1], [1, 1]],
      gamma: [[1, 1], [1, 1]],
    };
    expect(autoPickVariables(vars, 2)).toEqual(['alpha', 'beta']);
  });

  it('caps output to N', () => {
    const vars: Record<string, number[][]> = {};
    for (let i = 0; i < 10; i++) {
      vars[`v${i}`] = [[0, 0], [i, i]];
    }
    expect(autoPickVariables(vars, 3)).toHaveLength(3);
  });
});

describe('normaliseSamples', () => {
  it('pads shorter rows with NaN', () => {
    const rect = normaliseSamples([[1, 2, 3], [4, 5]]);
    expect(rect[0]).toEqual([1, 2, 3]);
    expect(rect[1][0]).toBe(4);
    expect(rect[1][1]).toBe(5);
    expect(Number.isNaN(rect[1][2])).toBe(true);
  });
  it('is a no-op on rectangular inputs', () => {
    const rect = normaliseSamples([[1, 2], [3, 4]]);
    expect(rect).toEqual([[1, 2], [3, 4]]);
  });
});

describe('playheadMaxTick', () => {
  it('returns 0 when no sessions are picked', () => {
    expect(playheadMaxTick([])).toBe(0);
  });
  it('returns max - 1 (0-based) of the longest session', () => {
    expect(playheadMaxTick([3, 5, 2])).toBe(4);
  });
  it('clamps to 0 for zero-length sessions', () => {
    expect(playheadMaxTick([0, 0])).toBe(0);
  });
});

describe('divergenceColor', () => {
  it('returns transparent for zero', () => {
    expect(divergenceColor(0)).toBe('transparent');
  });
  it('returns an oklch() string for positive scores', () => {
    expect(divergenceColor(0.5)).toMatch(/^oklch\(/);
    expect(divergenceColor(1.0)).toMatch(/^oklch\(/);
  });
  it('clamps negative values to transparent', () => {
    expect(divergenceColor(-0.5)).toBe('transparent');
  });
});
