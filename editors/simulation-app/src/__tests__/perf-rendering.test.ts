/**
 * Performance tests for rendering-path selectors per ADR-008.
 *
 * Validates that selectDecimatedTimeSeries, selectConstraintResults,
 * and selectKPISummaries stay within their time and size budgets.
 */

import { describe, it, expect } from 'vitest';
import {
  selectDecimatedTimeSeries,
  selectConstraintResults,
  selectKPISummaries,
} from '../features/results/selectors';
import { PerfBudget } from '../shared/perf';
import type { TimePoint } from '../features/sessions/types';
import type { SessionDetail } from '../features/sessions/types';

// ── Helpers ──────────────────────────────────────────────────────────

function syntheticSeries(n: number): TimePoint[] {
  const pts: TimePoint[] = new Array(n);
  for (let i = 0; i < n; i++) {
    pts[i] = { t: i, v: Math.sin(i * 0.01) * 100 };
  }
  return pts;
}

function fakeSessionWithConstraints(count: number): SessionDetail {
  return {
    summary: {
      id: 'test',
      kind: 'simulation',
      uri: 'test::Model',
      subsystem_name: null,
      label: null,
      created_at_ms: 0,
      elapsed_ms: 0,
      tick: 100,
      time_ms: 1000,
      current_state: null,
      completed: false,
      is_expired: false,
      history_len: 100,
      subsystem_count: 1,
      fork_point_tick: null,
      paused: false,
      paused_at_breakpoint: null,
      ticks_advanced: 0,
    },
    subsystems: [],
    latest_snapshot: {
      constraint_results: Array.from({ length: count }, (_, i) => ({
        name: `constraint_${i}`,
        expression: `x_${i} > 0`,
        verdict: i % 3 !== 0 ? 'Pass' : 'Fail',
        actual: `${i * 1.5}`,
      })),
    },
  };
}

function fakeTimeSeries(varCount: number, pointsPerVar: number): Record<string, TimePoint[]> {
  const ts: Record<string, TimePoint[]> = {};
  for (let v = 0; v < varCount; v++) {
    // Mix in names that trigger KPI detection
    const name = v < 10 ? `i_current_${v}` : v < 20 ? `t_temp_${v}` : `var_${v}`;
    ts[name] = syntheticSeries(pointsPerVar);
  }
  return ts;
}

// ── selectDecimatedTimeSeries ────────────────────────────────────────

describe('selectDecimatedTimeSeries', () => {
  it('60k raw points per series => <= 1500 points per series', () => {
    const raw = fakeTimeSeries(5, 60_000);
    const result = selectDecimatedTimeSeries(raw);

    for (const [name, pts] of Object.entries(result)) {
      expect(pts.length).toBeLessThanOrEqual(PerfBudget.MAX_RENDER_POINTS);
    }
  });

  it('preserves first and last points', () => {
    const raw = { signal: syntheticSeries(10_000) };
    const result = selectDecimatedTimeSeries(raw);
    const dec = result.signal;

    expect(dec[0]).toEqual(raw.signal[0]);
    expect(dec[dec.length - 1]).toEqual(raw.signal[raw.signal.length - 1]);
  });
});

// ── selectConstraintResults ──────────────────────────────────────────

describe('selectConstraintResults', () => {
  it('100 constraints complete in < 10ms', () => {
    const detail = fakeSessionWithConstraints(100);

    const start = performance.now();
    selectConstraintResults(detail);
    const elapsed = performance.now() - start;

    expect(elapsed).toBeLessThan(10);
  });

  it('returns correct count', () => {
    const detail = fakeSessionWithConstraints(100);
    const results = selectConstraintResults(detail);
    expect(results.length).toBe(100);
  });
});

// ── selectKPISummaries ───────────────────────────────────────────────

describe('selectKPISummaries', () => {
  it('50 variables complete in < 5ms', () => {
    const ts = fakeTimeSeries(50, 1_000);

    const start = performance.now();
    selectKPISummaries(ts, 5000);
    const elapsed = performance.now() - start;

    expect(elapsed).toBeLessThan(5);
  });

  it('produces KPI entries for current and temp variables', () => {
    const ts = fakeTimeSeries(50, 100);
    const kpis = selectKPISummaries(ts, 1000);

    // Should have entries: 10 current vars + 10 temp vars + 1 sim time = 21, capped at 8
    expect(kpis.length).toBeLessThanOrEqual(8);
    expect(kpis.length).toBeGreaterThan(0);
  });
});
