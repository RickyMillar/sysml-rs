/**
 * Tests for useVerifyConfig — the pure local state hook that backs the
 * VerifyWorkflow config panel.
 *
 * Exercises selection mechanics, suite toggling, and the derived
 * summary fields that the UI reads (`hasSelection`, `selectedCount`,
 * `suiteLabel`).
 */

import { describe, it, expect } from 'vitest';
import { act, renderHook } from '@testing-library/react';
import {
  useVerifyConfig,
  VERIFY_SUITES,
  DEFAULT_SUITE,
  isVerifySuite,
} from '../useVerifyConfig';

describe('useVerifyConfig — defaults', () => {
  it('starts with an empty selection and default suite', () => {
    const { result } = renderHook(() => useVerifyConfig());
    expect(result.current.selectedCaseIds.size).toBe(0);
    expect(result.current.suite).toBe(DEFAULT_SUITE);
    expect(result.current.hasSelection).toBe(false);
    expect(result.current.selectedCount).toBe(0);
  });

  it('seeds from options when provided', () => {
    const { result } = renderHook(() =>
      useVerifyConfig({
        initialSelection: ['c1', 'c2'],
        initialSuite: 'evaluate_constraints',
      }),
    );
    expect(result.current.selectedCaseIds.has('c1')).toBe(true);
    expect(result.current.selectedCaseIds.has('c2')).toBe(true);
    expect(result.current.suite).toBe('evaluate_constraints');
    expect(result.current.selectedCount).toBe(2);
    expect(result.current.hasSelection).toBe(true);
  });
});

describe('useVerifyConfig — selection mechanics', () => {
  it('toggleCase adds then removes an id', () => {
    const { result } = renderHook(() => useVerifyConfig());

    act(() => result.current.toggleCase('c1'));
    expect(result.current.selectedCaseIds.has('c1')).toBe(true);
    expect(result.current.selectedCount).toBe(1);
    expect(result.current.hasSelection).toBe(true);

    act(() => result.current.toggleCase('c1'));
    expect(result.current.selectedCaseIds.has('c1')).toBe(false);
    expect(result.current.selectedCount).toBe(0);
    expect(result.current.hasSelection).toBe(false);
  });

  it('selectAll merges ids into the selection (idempotent)', () => {
    const { result } = renderHook(() => useVerifyConfig({ initialSelection: ['c1'] }));

    act(() => result.current.selectAll(['c1', 'c2', 'c3']));
    expect(result.current.selectedCount).toBe(3);
    expect(result.current.selectedCaseIds.has('c1')).toBe(true);
    expect(result.current.selectedCaseIds.has('c2')).toBe(true);
    expect(result.current.selectedCaseIds.has('c3')).toBe(true);

    // Calling again is a no-op.
    act(() => result.current.selectAll(['c2', 'c3']));
    expect(result.current.selectedCount).toBe(3);
  });

  it('clearSelection empties the selection without changing the suite', () => {
    const { result } = renderHook(() =>
      useVerifyConfig({
        initialSelection: ['c1', 'c2'],
        initialSuite: 'evaluate_constraints',
      }),
    );

    act(() => result.current.clearSelection());
    expect(result.current.selectedCount).toBe(0);
    expect(result.current.hasSelection).toBe(false);
    expect(result.current.suite).toBe('evaluate_constraints');
  });

  it('setSelection replaces the selection wholesale', () => {
    const { result } = renderHook(() => useVerifyConfig({ initialSelection: ['a', 'b'] }));

    act(() => result.current.setSelection(['x', 'y', 'z']));
    expect(result.current.selectedCount).toBe(3);
    expect(result.current.selectedCaseIds.has('a')).toBe(false);
    expect(result.current.selectedCaseIds.has('x')).toBe(true);
    expect(result.current.selectedCaseIds.has('y')).toBe(true);
    expect(result.current.selectedCaseIds.has('z')).toBe(true);
  });
});

describe('useVerifyConfig — suite toggle', () => {
  it('setSuite cycles through every suite the UI exposes', () => {
    const { result } = renderHook(() => useVerifyConfig());
    for (const opt of VERIFY_SUITES) {
      act(() => result.current.setSuite(opt.id));
      expect(result.current.suite).toBe(opt.id);
      expect(result.current.suiteLabel).toBe(opt.label);
    }
  });

  it('suiteLabel reflects the current suite', () => {
    const { result } = renderHook(() =>
      useVerifyConfig({ initialSuite: 'evaluate_constraints' }),
    );
    expect(result.current.suiteLabel).toBe('Constraints');
    act(() => result.current.setSuite('evaluate_calculations'));
    expect(result.current.suiteLabel).toBe('Calculations');
  });
});

describe('isVerifySuite', () => {
  it('accepts every canonical suite id', () => {
    for (const s of VERIFY_SUITES) {
      expect(isVerifySuite(s.id)).toBe(true);
    }
  });

  it('rejects unknown strings', () => {
    expect(isVerifySuite('does_not_exist')).toBe(false);
    expect(isVerifySuite('')).toBe(false);
  });
});
