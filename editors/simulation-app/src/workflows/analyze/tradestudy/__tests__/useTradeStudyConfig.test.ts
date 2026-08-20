/**
 * Tests for useTradeStudyConfig — the pure local state hook backing the
 * TradeStudyWorkflow config panel.
 *
 * Covers:
 *   - round-trip: alternatives / criteria / weights add + remove + mutate
 *   - weight normalisation rules (pure `normalizeWeights` helper)
 *   - objective default heuristic (cost/latency/error/penalty → Min;
 *     anything else → Max)
 *   - validation contract (≥2 alternatives AND ≥1 criterion).
 */

import { describe, it, expect } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import {
  useTradeStudyConfig,
  normalizeWeights,
  defaultObjectiveForMetric,
  validateTradeStudyConfig,
} from '../useTradeStudyConfig';

describe('useTradeStudyConfig — defaults + round-trip', () => {
  it('starts with empty alternatives, criteria, weights', () => {
    const { result } = renderHook(() => useTradeStudyConfig());
    expect(result.current.alternatives).toEqual([]);
    expect(result.current.criteria).toEqual([]);
    expect(result.current.weights).toEqual([]);
    expect(result.current.normalizedWeights).toEqual([]);
    expect(result.current.hasAlternatives).toBe(false);
    expect(result.current.hasCriteria).toBe(false);
    expect(result.current.validation.canRun).toBe(false);
  });

  it('addAlternative appends with a default label and empty overrides', () => {
    const { result } = renderHook(() => useTradeStudyConfig());
    act(() => result.current.addAlternative());
    act(() => result.current.addAlternative());
    expect(result.current.alternatives).toHaveLength(2);
    expect(result.current.alternatives[0].label).toBe('Design A');
    expect(result.current.alternatives[1].label).toBe('Design B');
    expect(result.current.alternatives[0].overrides).toEqual({});
  });

  it('rename / setOverride / removeOverride round-trip on alternatives', () => {
    const { result } = renderHook(() => useTradeStudyConfig());
    act(() => result.current.addAlternative('Baseline'));
    const id = result.current.alternatives[0].id;

    act(() => result.current.renameAlternative(id, 'Tweaked'));
    expect(result.current.alternatives[0].label).toBe('Tweaked');

    act(() => result.current.setOverride(id, 'gain', 2.5));
    expect(result.current.alternatives[0].overrides.gain).toBe(2.5);

    act(() => result.current.setOverride(id, 'gain', 3.0));
    expect(result.current.alternatives[0].overrides.gain).toBe(3.0);

    act(() => result.current.removeOverride(id, 'gain'));
    expect('gain' in result.current.alternatives[0].overrides).toBe(false);
  });

  it('removeAlternative drops by id', () => {
    const { result } = renderHook(() => useTradeStudyConfig());
    act(() => result.current.addAlternative('A'));
    act(() => result.current.addAlternative('B'));
    const firstId = result.current.alternatives[0].id;
    act(() => result.current.removeAlternative(firstId));
    expect(result.current.alternatives.map((a) => a.label)).toEqual(['B']);
  });

  it('addCriterion dedupes by metricId and seeds weights equal', () => {
    const { result } = renderHook(() => useTradeStudyConfig());
    act(() => result.current.addCriterion('cost_usd'));
    act(() => result.current.addCriterion('performance_score'));
    act(() => result.current.addCriterion('cost_usd')); // duplicate
    expect(result.current.criteria.map((c) => c.metricId)).toEqual([
      'cost_usd',
      'performance_score',
    ]);
    expect(result.current.weights).toEqual([0.5, 0.5]);
  });

  it('setWeight / setObjective / removeCriterion round-trip', () => {
    const { result } = renderHook(() => useTradeStudyConfig());
    act(() => result.current.addCriterion('cost_usd'));
    act(() => result.current.addCriterion('performance_score'));

    act(() => result.current.setWeight('cost_usd', 3));
    expect(result.current.weights[0]).toBe(3);

    act(() => result.current.setObjective('performance_score', 'min'));
    expect(result.current.criteria[1].objective).toBe('min');

    act(() => result.current.removeCriterion('cost_usd'));
    expect(result.current.criteria.map((c) => c.metricId)).toEqual([
      'performance_score',
    ]);
    // Leftover weights should sum to 1 after removal.
    const sum = result.current.weights.reduce((a, b) => a + b, 0);
    expect(sum).toBeCloseTo(1, 10);
  });

  it('resetWeights reseeds to equal shares', () => {
    const { result } = renderHook(() => useTradeStudyConfig());
    act(() => result.current.addCriterion('a'));
    act(() => result.current.addCriterion('b'));
    act(() => result.current.addCriterion('c'));
    act(() => result.current.setWeight('a', 10));
    expect(result.current.weights[0]).toBe(10);
    act(() => result.current.resetWeights());
    expect(result.current.weights).toEqual([1 / 3, 1 / 3, 1 / 3]);
  });
});

describe('useTradeStudyConfig — normalizedWeights', () => {
  it('normalizedWeights sums to 1 when weights are arbitrary', () => {
    const { result } = renderHook(() => useTradeStudyConfig());
    act(() => result.current.addCriterion('a'));
    act(() => result.current.addCriterion('b'));
    act(() => result.current.addCriterion('c'));
    act(() => result.current.setWeight('a', 2));
    act(() => result.current.setWeight('b', 3));
    act(() => result.current.setWeight('c', 5));
    const norm = result.current.normalizedWeights;
    expect(norm).toEqual([0.2, 0.3, 0.5]);
    expect(norm.reduce((a, b) => a + b, 0)).toBeCloseTo(1, 10);
  });

  it('normalizedWeights falls back to equal when weights degenerate', () => {
    const { result } = renderHook(() => useTradeStudyConfig());
    act(() => result.current.addCriterion('a'));
    act(() => result.current.addCriterion('b'));
    act(() => result.current.setWeight('a', 0));
    act(() => result.current.setWeight('b', 0));
    expect(result.current.normalizedWeights).toEqual([0.5, 0.5]);
  });
});

describe('normalizeWeights (pure)', () => {
  it('empty input yields empty output', () => {
    expect(normalizeWeights([])).toEqual([]);
  });

  it('arbitrary positive weights scale to sum 1', () => {
    const out = normalizeWeights([1, 1, 2]);
    expect(out).toEqual([0.25, 0.25, 0.5]);
    expect(out.reduce((a, b) => a + b, 0)).toBeCloseTo(1, 10);
  });

  it('all zero input yields equal weights', () => {
    expect(normalizeWeights([0, 0, 0, 0])).toEqual([0.25, 0.25, 0.25, 0.25]);
  });

  it('negative + NaN entries are treated as zero', () => {
    const out = normalizeWeights([NaN, -1, 2, 8]);
    expect(out[0]).toBe(0);
    expect(out[1]).toBe(0);
    expect(out[2]).toBeCloseTo(0.2, 10);
    expect(out[3]).toBeCloseTo(0.8, 10);
  });

  it('all-negative input yields equal weights (degenerate fallback)', () => {
    expect(normalizeWeights([-1, -2, -3])).toEqual([1 / 3, 1 / 3, 1 / 3]);
  });
});

describe('defaultObjectiveForMetric heuristic', () => {
  it('cost_usd → Min', () => {
    expect(defaultObjectiveForMetric('cost_usd')).toBe('min');
  });
  it('latency_ms → Min', () => {
    expect(defaultObjectiveForMetric('latency_ms')).toBe('min');
  });
  it('error_rate → Min', () => {
    expect(defaultObjectiveForMetric('error_rate')).toBe('min');
  });
  it('steady_state_penalty → Min', () => {
    expect(defaultObjectiveForMetric('steady_state_penalty')).toBe('min');
  });
  it('performance_score → Max', () => {
    expect(defaultObjectiveForMetric('performance_score')).toBe('max');
  });
  it('throughput → Max', () => {
    expect(defaultObjectiveForMetric('throughput')).toBe('max');
  });
  it('something_random → Max (default branch)', () => {
    expect(defaultObjectiveForMetric('efficiency_ratio')).toBe('max');
  });
  it('matches case-insensitively on embedded keywords', () => {
    expect(defaultObjectiveForMetric('Total COST (USD)')).toBe('min');
  });
});

describe('validateTradeStudyConfig', () => {
  const crit = { metricId: 'cost_usd', objective: 'min' as const };
  const alt = (label: string) => ({ id: label, label, overrides: {} });

  it('fewer than two alternatives → cannot run', () => {
    const v = validateTradeStudyConfig([], [crit]);
    expect(v.canRun).toBe(false);
    expect(v.reason).toMatch(/two alternatives/i);

    const v1 = validateTradeStudyConfig([alt('A')], [crit]);
    expect(v1.canRun).toBe(false);
    expect(v1.reason).toMatch(/two alternatives/i);
  });

  it('zero criteria → cannot run', () => {
    const v = validateTradeStudyConfig([alt('A'), alt('B')], []);
    expect(v.canRun).toBe(false);
    expect(v.reason).toMatch(/criterion/i);
  });

  it('≥2 alternatives AND ≥1 criterion → can run', () => {
    const v = validateTradeStudyConfig([alt('A'), alt('B')], [crit]);
    expect(v.canRun).toBe(true);
    expect(v.reason).toBeNull();
  });
});

describe('useTradeStudyConfig.validation integration', () => {
  it('flips to canRun=true once both conditions are met', () => {
    const { result } = renderHook(() => useTradeStudyConfig());
    expect(result.current.validation.canRun).toBe(false);

    act(() => result.current.addAlternative('A'));
    expect(result.current.validation.canRun).toBe(false);

    act(() => result.current.addAlternative('B'));
    expect(result.current.validation.canRun).toBe(false); // still no criteria

    act(() => result.current.addCriterion('cost_usd'));
    expect(result.current.validation.canRun).toBe(true);
  });
});
