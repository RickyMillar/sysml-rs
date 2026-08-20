/**
 * Tests for the stats metric registry extension (R7.2).
 *
 * Pins:
 *   - `registerStatsMetrics` upserts all five descriptors (idempotent).
 *   - `computeStatsMetric` returns correct numeric / categorical values.
 *   - `computeAllStatsMetrics` shares a bootstrap RNG across the two
 *     CI bounds so they pair cleanly.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { MetricRegistry } from '../../../shared/metrics/registry';
import {
  computeAllStatsMetrics,
  computeStatsMetric,
  registerStatsMetrics,
  STATS_METRIC_IDS,
} from '../../../shared/metrics/statsMetrics';
import { createSeededRng } from '../statsHelpers';
import type { ChildDescriptor as SweepChild } from '../../../shared/viewers/sweepViewerHelpers';

function mkChild(index: number, value: number): SweepChild {
  return {
    session_id: `s-${index}`,
    index,
    params: {},
    status: 'complete',
    verdicts: [{ verdict: 'pass', margin: value }],
  };
}

describe('registerStatsMetrics', () => {
  let registry: MetricRegistry;
  beforeEach(() => {
    registry = new MetricRegistry();
  });

  it('registers all five stats metrics', () => {
    registerStatsMetrics(registry);
    for (const id of STATS_METRIC_IDS) {
      expect(registry.get(id)).toBeDefined();
    }
    expect(registry.size).toBe(5);
  });

  it('is idempotent when called twice', () => {
    registerStatsMetrics(registry);
    registerStatsMetrics(registry);
    expect(registry.size).toBe(5);
  });

  it('descriptors carry the expected labels', () => {
    registerStatsMetrics(registry);
    expect(registry.get('mean')?.name).toBe('Mean');
    expect(registry.get('distribution_family')?.name).toBe('Fitted distribution');
  });
});

describe('computeStatsMetric', () => {
  const extractor = (c: SweepChild): number => {
    const v = c.verdicts[0]?.margin;
    return typeof v === 'number' ? v : Number.NaN;
  };

  it('mean returns correct value', () => {
    const kids = [1, 2, 3, 4, 5].map((v, i) => mkChild(i, v));
    const m = computeStatsMetric('mean', kids, extractor);
    expect(m.id).toBe('mean');
    expect(m.value).toBe(3);
    expect(m.n).toBe(5);
  });

  it('stddev returns sample stddev', () => {
    const kids = [2, 4, 4, 4, 5, 5, 7, 9].map((v, i) => mkChild(i, v));
    const m = computeStatsMetric('stddev', kids, extractor);
    expect(m.value).toBeCloseTo(Math.sqrt(32 / 7), 10);
  });

  it('ci95_lower / ci95_upper bracket the sample mean', () => {
    const rng1 = createSeededRng(55);
    const kids: SweepChild[] = [];
    for (let i = 0; i < 100; i++) {
      // Normal-ish via two-uniforms sum.
      const v = rng1() + rng1();
      kids.push(mkChild(i, v));
    }
    const lo = computeStatsMetric('ci95_lower', kids, extractor, {
      rng: createSeededRng(7),
    });
    const hi = computeStatsMetric('ci95_upper', kids, extractor, {
      rng: createSeededRng(7),
    });
    expect(lo.value).toBeLessThan(hi.value);
    // Sample mean should be within the CI (most of the time for n=100).
    const mm = computeStatsMetric('mean', kids, extractor);
    expect(mm.value).toBeGreaterThan(lo.value);
    expect(mm.value).toBeLessThan(hi.value);
  });

  it('distribution_family returns a category string', () => {
    const kids: SweepChild[] = [];
    // Uniform sample.
    const rng = createSeededRng(909);
    for (let i = 0; i < 500; i++) kids.push(mkChild(i, rng()));
    const fam = computeStatsMetric('distribution_family', kids, extractor);
    expect(fam.category).toBe('uniform');
    expect(Number.isNaN(fam.value)).toBe(true);
  });

  it('handles empty input', () => {
    const m = computeStatsMetric('mean', [], extractor);
    expect(Number.isNaN(m.value)).toBe(true);
    expect(m.n).toBe(0);
  });
});

describe('computeAllStatsMetrics', () => {
  const extractor = (c: SweepChild): number => {
    const v = c.verdicts[0]?.margin;
    return typeof v === 'number' ? v : Number.NaN;
  };

  it('returns all five keys', () => {
    const kids = [1, 2, 3, 4, 5].map((v, i) => mkChild(i, v));
    const all = computeAllStatsMetrics(kids, extractor, { rng: createSeededRng(10) });
    expect(Object.keys(all).sort()).toEqual([...STATS_METRIC_IDS].sort());
  });

  it('ci bounds are ordered lower <= upper', () => {
    const kids: SweepChild[] = [];
    const rng = createSeededRng(42);
    for (let i = 0; i < 100; i++) kids.push(mkChild(i, rng() * 10));
    const all = computeAllStatsMetrics(kids, extractor, { rng: createSeededRng(11) });
    expect(all.ci95_lower.value).toBeLessThanOrEqual(all.ci95_upper.value);
  });

  it('deterministic under seeded RNG', () => {
    const kids = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10].map((v, i) => mkChild(i, v));
    const a = computeAllStatsMetrics(kids, extractor, { rng: createSeededRng(999) });
    const b = computeAllStatsMetrics(kids, extractor, { rng: createSeededRng(999) });
    expect(a.ci95_lower.value).toBe(b.ci95_lower.value);
    expect(a.ci95_upper.value).toBe(b.ci95_upper.value);
  });
});
