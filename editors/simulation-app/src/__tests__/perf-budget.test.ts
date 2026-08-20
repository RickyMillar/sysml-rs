/**
 * Performance budget validation tests per ADR-008.
 *
 * Ensures LTTB decimation, memory estimates, and overlay computation
 * stay within their allocated budgets.
 */

import { describe, it, expect } from 'vitest';
import { lttbDecimate } from '../features/results/selectors';
import { PerfBudget, estimateMemoryUsage } from '../shared/perf';
import type { TimePoint } from '../features/sessions/types';

// ── Helpers ──────────────────────────────────────────────────────────

/** Generate a synthetic time-series of `n` points. */
function syntheticSeries(n: number): TimePoint[] {
  const pts: TimePoint[] = new Array(n);
  for (let i = 0; i < n; i++) {
    pts[i] = { t: i, v: Math.sin(i * 0.01) * 100 };
  }
  return pts;
}

// ── LTTB output-size tests ───────────────────────────────────────────

describe('lttbDecimate output size', () => {
  it.each([10_000, 60_000, 100_000])(
    'produces <= 1500 points for %i input points',
    (n) => {
      const decimated = lttbDecimate(syntheticSeries(n));
      expect(decimated.length).toBeLessThanOrEqual(PerfBudget.MAX_RENDER_POINTS);
    },
  );

  it('returns input unchanged when <= threshold', () => {
    const small = syntheticSeries(500);
    expect(lttbDecimate(small)).toBe(small);
  });
});

// ── LTTB timing test ─────────────────────────────────────────────────

describe('lttbDecimate performance', () => {
  it('decimates 60k points in < 50ms', () => {
    const pts = syntheticSeries(60_000);

    const start = performance.now();
    lttbDecimate(pts);
    const elapsed = performance.now() - start;

    expect(elapsed).toBeLessThan(50);
  });
});

// ── Memory estimation test ───────────────────────────────────────────

describe('estimateMemoryUsage', () => {
  it('50 variables x 60k ticks stays under 100 MB', () => {
    const bytes = estimateMemoryUsage(50, 60_000);
    const mb = bytes / (1024 * 1024);
    expect(mb).toBeLessThan(PerfBudget.MAX_MEMORY_MB);
  });

  it('returns correct byte count for known input', () => {
    // 1 series, 1 point => 2 channels * 8 bytes = 16
    expect(estimateMemoryUsage(1, 1)).toBe(16);
  });
});
