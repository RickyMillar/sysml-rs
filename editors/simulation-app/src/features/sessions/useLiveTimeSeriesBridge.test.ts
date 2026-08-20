/**
 * Unit tests for useLiveTimeSeriesBridge — the big-model lazy
 * registration guard (UX closeout #4 / #17) is the behaviorally
 * sensitive part: it must be byte-identical to before below
 * `BIG_MODEL_VAR_THRESHOLD` (hybrid and every existing fixture live well
 * under it) and must correctly start collecting a variable from the
 * moment it's selected, without ever dropping already-recorded history
 * for a variable that gets deselected later.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, renderHook } from '@testing-library/react';
import {
  BIG_MODEL_VAR_THRESHOLD,
  selectValuesToBuffer,
  useLiveTimeSeriesBridge,
} from './useLiveTimeSeriesBridge';
import { useSessionLiveStore } from './sessionLiveStore';
import { usePlotSelectionStore } from '@/features/results/usePlotSelectionStore';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';

function bigScalarVars(count: number, overrides: Record<string, number> = {}): Record<string, number> {
  const out: Record<string, number> = {};
  for (let i = 0; i < count; i++) out[`v${i}`] = i;
  return { ...out, ...overrides };
}

describe('selectValuesToBuffer — pure selectivity logic', () => {
  it('passes every variable through unchanged at or below the threshold (small/medium models: zero behaviour change)', () => {
    const vars = bigScalarVars(BIG_MODEL_VAR_THRESHOLD);
    const tracked = new Set<string>();
    const out = selectValuesToBuffer(vars, 'sess-1', tracked);
    expect(out).toEqual(vars);
    // No tracking bookkeeping happens below the threshold.
    expect(tracked.size).toBe(0);
  });

  it('above the threshold, buffers nothing when nothing is selected', () => {
    const vars = bigScalarVars(BIG_MODEL_VAR_THRESHOLD + 1);
    const tracked = new Set<string>();
    const out = selectValuesToBuffer(vars, 'sess-1', tracked);
    expect(out).toEqual({});
  });

  it('above the threshold, buffers only the selected variables', () => {
    const vars = bigScalarVars(BIG_MODEL_VAR_THRESHOLD + 1, { v0: 42, v1: 7 });
    usePlotSelectionStore.getState().setSelected('sess-1', ['v0', 'v1']);
    const tracked = new Set<string>();
    const out = selectValuesToBuffer(vars, 'sess-1', tracked);
    expect(out).toEqual({ v0: 42, v1: 7 });
  });

  it('a variable selected on a later tick starts buffering from that point (does not require re-registration)', () => {
    const tracked = new Set<string>();
    usePlotSelectionStore.getState().setSelected('sess-1', []);
    const tick0 = bigScalarVars(BIG_MODEL_VAR_THRESHOLD + 1, { v5: 1 });
    expect(selectValuesToBuffer(tick0, 'sess-1', tracked)).toEqual({});

    // User adds v5 to a plot after the fact.
    usePlotSelectionStore.getState().setSelected('sess-1', ['v5']);
    const tick1 = bigScalarVars(BIG_MODEL_VAR_THRESHOLD + 1, { v5: 2 });
    expect(selectValuesToBuffer(tick1, 'sess-1', tracked)).toEqual({ v5: 2 });
  });

  it('once tracked, a variable keeps buffering even after being deselected (history is never blanked)', () => {
    const tracked = new Set<string>();
    usePlotSelectionStore.getState().setSelected('sess-1', ['v9']);
    const tick0 = bigScalarVars(BIG_MODEL_VAR_THRESHOLD + 1, { v9: 10 });
    expect(selectValuesToBuffer(tick0, 'sess-1', tracked)).toEqual({ v9: 10 });

    // Deselect v9 — it should still keep buffering.
    usePlotSelectionStore.getState().setSelected('sess-1', []);
    const tick1 = bigScalarVars(BIG_MODEL_VAR_THRESHOLD + 1, { v9: 11 });
    expect(selectValuesToBuffer(tick1, 'sess-1', tracked)).toEqual({ v9: 11 });
  });
});

describe('useLiveTimeSeriesBridge — integration (big-model gating)', () => {
  const originalEnv = { ...(import.meta as any).env };

  beforeEach(() => {
    (import.meta as any).env = { ...originalEnv, VITE_STREAM_V1: '1' };
    useSessionLiveStore.getState().reset();
    useTimeSeriesStore.getState().reset();
    usePlotSelectionStore.setState({ selectionsBySession: {} });
  });

  afterEach(() => {
    cleanup();
    (import.meta as any).env = originalEnv;
    vi.restoreAllMocks();
  });

  it('buffers only selected variables once the model crosses the big-model threshold', () => {
    usePlotSelectionStore.getState().setSelected('sess-1', ['v0']);
    renderHook(() => useLiveTimeSeriesBridge());

    act(() => {
      useSessionLiveStore.getState().applyHello('sess-1', {
        tick: 0,
        time_ms: 0,
        completed: false,
        subsystems: {},
        scalar_vars: bigScalarVars(BIG_MODEL_VAR_THRESHOLD + 1, { v0: 100 }),
        string_vars: {},
        constraint_results: [],
      });
    });

    const series = useTimeSeriesStore.getState().getTimeSeries();
    expect(Object.keys(series)).toEqual(['v0']);
    expect(series.v0!.map((p) => p.v)).toEqual([100]);
  });

  it('resets the tracked-variable set on session switch (no leakage into a new session)', () => {
    usePlotSelectionStore.getState().setSelected('sess-1', ['v0']);
    renderHook(() => useLiveTimeSeriesBridge());

    act(() => {
      useSessionLiveStore.getState().applyHello('sess-1', {
        tick: 0,
        time_ms: 0,
        completed: false,
        subsystems: {},
        scalar_vars: bigScalarVars(BIG_MODEL_VAR_THRESHOLD + 1, { v0: 1 }),
        string_vars: {},
        constraint_results: [],
      });
    });
    expect(Object.keys(useTimeSeriesStore.getState().getTimeSeries())).toEqual(['v0']);

    // Switch to a new session with a different selection — v0 must not
    // reappear just because it was tracked in the old session.
    usePlotSelectionStore.getState().setSelected('sess-2', ['v1']);
    act(() => {
      useSessionLiveStore.getState().applyHello('sess-2', {
        tick: 0,
        time_ms: 0,
        completed: false,
        subsystems: {},
        scalar_vars: bigScalarVars(BIG_MODEL_VAR_THRESHOLD + 1, { v0: 1, v1: 2 }),
        string_vars: {},
        constraint_results: [],
      });
    });
    const series = useTimeSeriesStore.getState().getTimeSeries();
    expect(Object.keys(series)).toEqual(['v1']);
  });

  it('below the threshold every variable buffers exactly as before (hybrid-scale unaffected)', () => {
    renderHook(() => useLiveTimeSeriesBridge());
    act(() => {
      useSessionLiveStore.getState().applyHello('sess-1', {
        tick: 0,
        time_ms: 0,
        completed: false,
        subsystems: {},
        scalar_vars: { i_drive: 1, B: 2, temp: 3 },
        string_vars: {},
        constraint_results: [],
      });
    });
    const series = useTimeSeriesStore.getState().getTimeSeries();
    expect(Object.keys(series).sort()).toEqual(['B', 'i_drive', 'temp']);
  });
});
