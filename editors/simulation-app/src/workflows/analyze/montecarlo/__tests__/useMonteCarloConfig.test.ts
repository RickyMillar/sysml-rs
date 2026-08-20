/**
 * Tests for `useMonteCarloConfig` — pure local-state hook behind the
 * Monte Carlo config panel.
 *
 * Covers:
 *   - Defaults + seeding from `initialDistributions`.
 *   - Parameter selection add/remove + `setParameters` bulk sync.
 *   - Distribution-kind flipping resets fields.
 *   - Sample-count clamping + seed parsing.
 *   - Derived fields: `hasParameters`, `isValid`, `validityByName`.
 */

import { describe, it, expect } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import {
  clampSampleCount,
  DEFAULT_SAMPLE_COUNT,
  MAX_SAMPLE_COUNT,
  useMonteCarloConfig,
} from '../useMonteCarloConfig';

describe('clampSampleCount', () => {
  it('coerces non-finite to default', () => {
    expect(clampSampleCount(Number.NaN)).toBe(DEFAULT_SAMPLE_COUNT);
    expect(clampSampleCount(Number.POSITIVE_INFINITY)).toBe(DEFAULT_SAMPLE_COUNT);
  });

  it('clamps below 1 to 1 and above MAX to MAX', () => {
    expect(clampSampleCount(0)).toBe(1);
    expect(clampSampleCount(-100)).toBe(1);
    expect(clampSampleCount(MAX_SAMPLE_COUNT + 1)).toBe(MAX_SAMPLE_COUNT);
  });

  it('floors fractional values', () => {
    expect(clampSampleCount(3.7)).toBe(3);
  });
});

describe('useMonteCarloConfig — defaults', () => {
  it('starts empty with 1000 samples and no seed', () => {
    const { result } = renderHook(() => useMonteCarloConfig());
    expect(result.current.parameterCount).toBe(0);
    expect(result.current.sampleCount).toBe(DEFAULT_SAMPLE_COUNT);
    expect(result.current.seed).toBeNull();
    expect(result.current.hasParameters).toBe(false);
    expect(result.current.isValid).toBe(false);
  });

  it('seeds from initialDistributions when provided', () => {
    const { result } = renderHook(() =>
      useMonteCarloConfig({
        initialDistributions: {
          alpha: { kind: 'normal', mean: 0, sigma: 1 },
        },
        initialSampleCount: 15,
        initialSeed: 42,
      }),
    );
    expect(result.current.parameterNames).toEqual(['alpha']);
    expect(result.current.sampleCount).toBe(15);
    expect(result.current.seed).toBe(42);
    expect(result.current.isValid).toBe(true);
  });
});

describe('useMonteCarloConfig — parameter selection', () => {
  it('adds and removes parameters idempotently', () => {
    const { result } = renderHook(() => useMonteCarloConfig());

    act(() => result.current.addParameter('voltage'));
    expect(result.current.parameterNames).toEqual(['voltage']);
    // Re-add is a no-op.
    act(() => result.current.addParameter('voltage'));
    expect(result.current.parameterNames).toEqual(['voltage']);

    act(() => result.current.removeParameter('voltage'));
    expect(result.current.parameterNames).toEqual([]);
    // Remove of a missing key is a no-op (doesn't throw).
    act(() => result.current.removeParameter('missing'));
    expect(result.current.parameterNames).toEqual([]);
  });

  it('setParameters replaces the selection in the given order', () => {
    const { result } = renderHook(() => useMonteCarloConfig());
    act(() => result.current.setParameters(['a', 'b', 'c']));
    expect(result.current.parameterNames).toEqual(['a', 'b', 'c']);
    // Reducing selection drops entries.
    act(() => result.current.setParameters(['b']));
    expect(result.current.parameterNames).toEqual(['b']);
  });

  it('flipping the kind resets fields to defaults', () => {
    const { result } = renderHook(() => useMonteCarloConfig());
    act(() => result.current.addParameter('x', 'normal'));
    act(() => result.current.setDistribution('x', { kind: 'normal', mean: 99, sigma: 2 }));
    expect(result.current.distributions.x).toMatchObject({ mean: 99, sigma: 2 });

    act(() => result.current.setDistributionKind('x', 'uniform'));
    expect(result.current.distributions.x.kind).toBe('uniform');
    // Uniform defaults — not the old normal(99, 2) fields.
    expect(result.current.distributions.x).toMatchObject({ min: 0, max: 1 });
  });
});

describe('useMonteCarloConfig — sample count / seed', () => {
  it('clamps sampleCount on set', () => {
    const { result } = renderHook(() => useMonteCarloConfig());
    act(() => result.current.setSampleCount(99_999));
    expect(result.current.sampleCount).toBe(MAX_SAMPLE_COUNT);
    act(() => result.current.setSampleCount(0));
    expect(result.current.sampleCount).toBe(1);
  });

  it('accepts null seed (auto) + integer seeds', () => {
    const { result } = renderHook(() => useMonteCarloConfig());
    act(() => result.current.setSeed(null));
    expect(result.current.seed).toBeNull();
    act(() => result.current.setSeed(42));
    expect(result.current.seed).toBe(42);
  });
});

describe('useMonteCarloConfig — derived validity', () => {
  it('is invalid with no parameters', () => {
    const { result } = renderHook(() => useMonteCarloConfig());
    expect(result.current.isValid).toBe(false);
  });

  it('is valid once one valid distribution is selected', () => {
    const { result } = renderHook(() => useMonteCarloConfig());
    act(() => result.current.addParameter('alpha'));
    expect(result.current.hasParameter('alpha')).toBe(true);
    expect(result.current.isValid).toBe(true);
    expect(result.current.validityByName.alpha).toBe(true);
  });

  it('flips invalid if any distribution is malformed', () => {
    const { result } = renderHook(() => useMonteCarloConfig());
    act(() => result.current.addParameter('alpha', 'uniform'));
    act(() =>
      result.current.setDistribution('alpha', { kind: 'uniform', min: 5, max: 1 }),
    );
    expect(result.current.validityByName.alpha).toBe(false);
    expect(result.current.isValid).toBe(false);
  });
});
