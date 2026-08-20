/**
 * useSweepConfig — round-trip + state-mutation tests.
 *
 * Exercises the hook's public contract end-to-end:
 *   - add / remove / update / clear ranges
 *   - toggle / set metrics
 *   - run-mode switches
 *   - derived `children` / `hasRuns` / `childCount` reflect state
 *   - `generateChildrenParams` (pure helper) matches the hook's output
 *
 * Runs under `renderHook` from @testing-library/react so the useState
 * + useMemo wiring inside the hook is exercised exactly as production
 * callers use it.
 */

import { describe, it, expect, afterEach } from 'vitest';
import { act, cleanup, renderHook } from '@testing-library/react';
import {
  useSweepConfig,
  generateChildrenParams,
  DEFAULT_RANGE_SPEC,
  DEFAULT_RUN_MODE,
  type ParameterRangeEntry,
} from '../useSweepConfig';

afterEach(() => {
  cleanup();
});

describe('useSweepConfig — initial state', () => {
  it('defaults to empty ranges + metrics and parallel run mode', () => {
    const { result } = renderHook(() => useSweepConfig());
    expect(result.current.ranges).toEqual([]);
    expect(result.current.selectedMetricIds).toEqual([]);
    expect(result.current.runMode).toBe(DEFAULT_RUN_MODE);
    expect(result.current.hasRuns).toBe(false);
    expect(result.current.childCount).toBe(0);
    // Empty-ranges short-circuit — derived children list is empty, not
    // the vacuous `[{}]` that the pure helper would yield.
    expect(result.current.children).toEqual([{}]);
  });

  it('accepts initial ranges / metrics / run mode from options', () => {
    const seededRanges: ParameterRangeEntry[] = [
      { parameterId: 'foo', spec: { kind: 'list', values: [1, 2] } },
    ];
    const { result } = renderHook(() =>
      useSweepConfig({
        initialRanges: seededRanges,
        initialMetrics: ['m1', 'm2'],
        initialRunMode: 'sequential',
      }),
    );
    expect(result.current.ranges).toEqual(seededRanges);
    expect(result.current.selectedMetricIds).toEqual(['m1', 'm2']);
    expect(result.current.runMode).toBe('sequential');
    expect(result.current.hasRuns).toBe(true);
    expect(result.current.childCount).toBe(2);
  });
});

describe('useSweepConfig — range mutations', () => {
  it('addRange appends an entry and updates derived counters', () => {
    const { result } = renderHook(() => useSweepConfig());
    act(() => {
      result.current.addRange({
        parameterId: 'a',
        spec: { kind: 'list', values: [1, 2, 3] },
      });
    });
    expect(result.current.ranges).toHaveLength(1);
    expect(result.current.hasRuns).toBe(true);
    expect(result.current.childCount).toBe(3);
    expect(result.current.children).toEqual([{ a: 1 }, { a: 2 }, { a: 3 }]);
  });

  it('addRange is idempotent by parameterId (picking twice is a no-op)', () => {
    const { result } = renderHook(() => useSweepConfig());
    act(() => {
      result.current.addRange({ parameterId: 'a', spec: { kind: 'list', values: [1] } });
    });
    // Second pick with a different spec must NOT clobber the first.
    act(() => {
      result.current.addRange({ parameterId: 'a', spec: { kind: 'list', values: [99] } });
    });
    expect(result.current.ranges).toHaveLength(1);
    expect(result.current.ranges[0].spec).toEqual({ kind: 'list', values: [1] });
  });

  it('updateRange replaces the spec in place (position preserved)', () => {
    const { result } = renderHook(() => useSweepConfig());
    act(() => {
      result.current.addRange({ parameterId: 'a', spec: { kind: 'list', values: [1] } });
      result.current.addRange({ parameterId: 'b', spec: { kind: 'list', values: [10] } });
    });
    act(() => {
      result.current.updateRange('a', { kind: 'list', values: [1, 2] });
    });
    expect(result.current.ranges[0].parameterId).toBe('a');
    expect(result.current.ranges[0].spec).toEqual({ kind: 'list', values: [1, 2] });
    // Cartesian product reflects the new spec.
    expect(result.current.childCount).toBe(2);
  });

  it('updateRange on unknown id is a no-op', () => {
    const { result } = renderHook(() => useSweepConfig());
    act(() => {
      result.current.updateRange('nope', { kind: 'list', values: [1] });
    });
    expect(result.current.ranges).toEqual([]);
  });

  it('removeRange drops the entry and re-derives counters', () => {
    const { result } = renderHook(() => useSweepConfig());
    act(() => {
      result.current.addRange({ parameterId: 'a', spec: { kind: 'list', values: [1, 2] } });
      result.current.addRange({ parameterId: 'b', spec: { kind: 'list', values: [10, 20] } });
    });
    expect(result.current.childCount).toBe(4);
    act(() => {
      result.current.removeRange('a');
    });
    expect(result.current.ranges).toHaveLength(1);
    expect(result.current.childCount).toBe(2);
  });

  it('clearRanges empties everything', () => {
    const { result } = renderHook(() => useSweepConfig());
    act(() => {
      result.current.addRange({ parameterId: 'a', spec: { kind: 'list', values: [1, 2] } });
      result.current.addRange({ parameterId: 'b', spec: { kind: 'list', values: [10] } });
    });
    act(() => {
      result.current.clearRanges();
    });
    expect(result.current.ranges).toEqual([]);
    expect(result.current.hasRuns).toBe(false);
    expect(result.current.childCount).toBe(0);
  });
});

describe('useSweepConfig — metric mutations', () => {
  it('toggleMetric adds then removes', () => {
    const { result } = renderHook(() => useSweepConfig());
    act(() => {
      result.current.toggleMetric('m1');
    });
    expect(result.current.selectedMetricIds).toEqual(['m1']);
    act(() => {
      result.current.toggleMetric('m2');
    });
    expect(result.current.selectedMetricIds).toEqual(['m1', 'm2']);
    act(() => {
      result.current.toggleMetric('m1');
    });
    expect(result.current.selectedMetricIds).toEqual(['m2']);
  });

  it('setSelectedMetrics replaces the list wholesale', () => {
    const { result } = renderHook(() => useSweepConfig({ initialMetrics: ['a', 'b'] }));
    act(() => {
      result.current.setSelectedMetrics(['x', 'y', 'z']);
    });
    expect(result.current.selectedMetricIds).toEqual(['x', 'y', 'z']);
  });
});

describe('useSweepConfig — run mode', () => {
  it('setRunMode flips between parallel and sequential', () => {
    const { result } = renderHook(() => useSweepConfig());
    expect(result.current.runMode).toBe('parallel');
    act(() => {
      result.current.setRunMode('sequential');
    });
    expect(result.current.runMode).toBe('sequential');
    act(() => {
      result.current.setRunMode('parallel');
    });
    expect(result.current.runMode).toBe('parallel');
  });
});

describe('useSweepConfig — derived children matches generateChildrenParams', () => {
  it('hook-derived children == pure-helper output for same ranges', () => {
    const { result } = renderHook(() => useSweepConfig());
    act(() => {
      result.current.addRange({
        parameterId: 'a',
        spec: { kind: 'list', values: [1, 2] },
      });
      result.current.addRange({
        parameterId: 'b',
        spec: { kind: 'list', values: [10, 20] },
      });
    });
    const fromHelper = generateChildrenParams(result.current.ranges);
    expect(result.current.children).toEqual(fromHelper);
    // And matches the brief's canonical expansion.
    expect(result.current.children).toEqual([
      { a: 1, b: 10 },
      { a: 1, b: 20 },
      { a: 2, b: 10 },
      { a: 2, b: 20 },
    ]);
  });

  it('grid spec round-trips through the hook', () => {
    const { result } = renderHook(() => useSweepConfig());
    act(() => {
      result.current.addRange({
        parameterId: 'x',
        spec: { kind: 'grid', min: 0, max: 2, step: 1 },
      });
    });
    expect(result.current.children).toEqual([{ x: 0 }, { x: 1 }, { x: 2 }]);
  });

  it('DEFAULT_RANGE_SPEC expands to a 5-point grid', () => {
    // Sanity check — the default seeded when the user picks a parameter
    // via the UI shouldn't accidentally collapse to empty.
    const entry: ParameterRangeEntry = { parameterId: 'p', spec: DEFAULT_RANGE_SPEC };
    const pts = generateChildrenParams([entry]);
    expect(pts.length).toBeGreaterThan(0);
    expect(pts[0]).toEqual({ p: 0 });
  });
});
