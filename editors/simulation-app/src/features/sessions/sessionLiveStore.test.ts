/**
 * Unit tests for sessionLiveStore — the delta reducer and store actions.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import {
  applyDelta,
  useSessionLiveStore,
  type NormalizedSnapshot,
  type DeltaFrame,
} from './sessionLiveStore';

function baseSnap(overrides: Partial<NormalizedSnapshot> = {}): NormalizedSnapshot {
  return {
    tick: 0,
    time_ms: 0,
    completed: false,
    subsystems: {},
    scalar_vars: {},
    string_vars: {},
    constraint_results: [],
    ...overrides,
  };
}

describe('applyDelta', () => {
  it('adds and updates scalars', () => {
    const base = baseSnap({ scalar_vars: { x: 1, y: 2 } });
    const delta: DeltaFrame = {
      tick: 1,
      time_ms: 10,
      completed: false,
      scalar_changed: { y: 3, z: 9 },
    };
    const out = applyDelta(base, delta);
    expect(out.scalar_vars).toEqual({ x: 1, y: 3, z: 9 });
    expect(out.tick).toBe(1);
  });

  it('removes scalars listed in scalar_removed', () => {
    const base = baseSnap({ scalar_vars: { x: 1, y: 2, z: 3 } });
    const delta: DeltaFrame = {
      tick: 1,
      time_ms: 10,
      completed: false,
      scalar_removed: ['x', 'z'],
    };
    expect(applyDelta(base, delta).scalar_vars).toEqual({ y: 2 });
  });

  it('keeps existing constraint rows when delta carries null/undefined', () => {
    const rows = [
      { name: 'c1', expression: 'x < 10', verdict: 'Pass' as const },
    ];
    const base = baseSnap({ constraint_results: rows });
    const delta: DeltaFrame = { tick: 1, time_ms: 10, completed: false };
    expect(applyDelta(base, delta).constraint_results).toEqual(rows);

    const delta2: DeltaFrame = {
      tick: 2, time_ms: 20, completed: false, constraint_results: null,
    };
    expect(applyDelta(base, delta2).constraint_results).toEqual(rows);
  });

  it('replaces constraint rows when delta carries an array', () => {
    const oldRows = [{ name: 'c1', expression: null, verdict: 'Pass' as const }];
    const newRows = [{ name: 'c1', expression: null, verdict: 'Fail' as const }];
    const base = baseSnap({ constraint_results: oldRows });
    const delta: DeltaFrame = {
      tick: 1, time_ms: 10, completed: false, constraint_results: newRows,
    };
    expect(applyDelta(base, delta).constraint_results).toEqual(newRows);
  });

  it('adds and updates port_values entries (GAP-FLOW-001)', () => {
    const base = baseSnap({
      port_values: {
        'tank.waterOut': { flowRate: 1, pressure: 100 },
      },
    });
    const delta: DeltaFrame = {
      tick: 1,
      time_ms: 10,
      completed: false,
      port_values_changed: {
        'tank.waterOut': { flowRate: 1.5, pressure: 100 },
        'valve.inlet': { flowRate: 0.8 },
      },
    };
    const out = applyDelta(base, delta);
    expect(out.port_values?.['tank.waterOut']).toEqual({
      flowRate: 1.5,
      pressure: 100,
    });
    expect(out.port_values?.['valve.inlet']).toEqual({ flowRate: 0.8 });
  });

  it('removes port_values keys listed in port_values_removed', () => {
    const base = baseSnap({
      port_values: {
        'tank.waterOut': { flowRate: 1 },
        'pump.in': { flowRate: 2 },
      },
    });
    const delta: DeltaFrame = {
      tick: 1,
      time_ms: 10,
      completed: false,
      port_values_removed: ['pump.in'],
    };
    const out = applyDelta(base, delta);
    expect(out.port_values).toEqual({
      'tank.waterOut': { flowRate: 1 },
    });
  });

  it('preserves undefined port_values on a base that never had any', () => {
    const base = baseSnap();
    const delta: DeltaFrame = { tick: 1, time_ms: 10, completed: false };
    expect(applyDelta(base, delta).port_values).toBeUndefined();
  });

  it('applies derivatives add/change/remove (GAP-ODE-002)', () => {
    const base = baseSnap({ derivatives: { T: 0.1, q: -0.02 } });
    const out = applyDelta(base, {
      tick: 1,
      time_ms: 10,
      completed: false,
      derivatives_changed: { T: 0.2, new_state: 0.5 },
      derivatives_removed: ['q'],
    });
    expect(out.derivatives).toEqual({ T: 0.2, new_state: 0.5 });
  });

  it('preserves undefined derivatives on a base that never had any', () => {
    const base = baseSnap();
    const delta: DeltaFrame = { tick: 1, time_ms: 10, completed: false };
    expect(applyDelta(base, delta).derivatives).toBeUndefined();
  });

  it('lazy-creates derivatives on the first delta that touches them', () => {
    const base = baseSnap();
    const out = applyDelta(base, {
      tick: 1,
      time_ms: 10,
      completed: false,
      derivatives_changed: { T: 0.1 },
    });
    expect(out.derivatives).toEqual({ T: 0.1 });
  });

  it('lazy-creates port_values on the first delta that touches ports', () => {
    const base = baseSnap();
    const delta: DeltaFrame = {
      tick: 1,
      time_ms: 10,
      completed: false,
      port_values_changed: { 'tank.waterOut': { flowRate: 1 } },
    };
    const out = applyDelta(base, delta);
    expect(out.port_values).toEqual({
      'tank.waterOut': { flowRate: 1 },
    });
  });

  it('adds/updates/removes subsystems and strings', () => {
    const base = baseSnap({
      subsystems: {
        sm1: { current_state: 'Idle', completed: false, kind_label: 'stateMachine' },
      },
      string_vars: { mode: 'normal', phase: 'A' },
    });
    const delta: DeltaFrame = {
      tick: 1,
      time_ms: 10,
      completed: false,
      subsystem_changed: {
        sm1: { current_state: 'Running', completed: false, kind_label: 'stateMachine' },
        sm2: { current_state: 'Idle', completed: false, kind_label: 'action' },
      },
      string_changed: { phase: 'B' },
      string_removed: ['mode'],
    };
    const out = applyDelta(base, delta);
    expect(out.subsystems.sm1.current_state).toBe('Running');
    expect(out.subsystems.sm2).toBeDefined();
    expect(out.string_vars).toEqual({ phase: 'B' });
  });
});

describe('useSessionLiveStore', () => {
  beforeEach(() => {
    useSessionLiveStore.getState().reset();
  });

  it('applyHello sets the baseline + open phase', () => {
    const base = baseSnap({ tick: 5, time_ms: 500, scalar_vars: { x: 1 } });
    useSessionLiveStore.getState().applyHello('sess-1', base);

    const s = useSessionLiveStore.getState();
    expect(s.sessionId).toBe('sess-1');
    expect(s.snapshot?.tick).toBe(5);
    expect(s.lastTick).toBe(5);
    expect(s.phase).toBe('open');
  });

  it('applyTick folds deltas onto the Hello base', () => {
    const base = baseSnap({ scalar_vars: { x: 1 } });
    const s = useSessionLiveStore.getState();
    s.applyHello('sess', base);
    s.applyTick({
      tick: 1, time_ms: 10, completed: false, scalar_changed: { x: 2, y: 3 },
    });
    const snap = useSessionLiveStore.getState().snapshot!;
    expect(snap.scalar_vars).toEqual({ x: 2, y: 3 });
    expect(snap.tick).toBe(1);
  });

  it('markCompleted sets completed flag + phase stays as applied by caller', () => {
    const s = useSessionLiveStore.getState();
    s.applyHello('sess', baseSnap({ tick: 0 }));
    s.markCompleted(42, 4_200);
    const snap = useSessionLiveStore.getState().snapshot!;
    expect(snap.completed).toBe(true);
    expect(snap.tick).toBe(42);
    expect(snap.time_ms).toBe(4_200);
  });

  it('reset clears everything back to idle', () => {
    const s = useSessionLiveStore.getState();
    s.applyHello('sess', baseSnap({ scalar_vars: { x: 1 } }));
    s.applyVerdict(1, { pass: 1, fail: 0, inconclusive: 0, error: 0 });
    s.reset();
    const state = useSessionLiveStore.getState();
    expect(state.sessionId).toBeNull();
    expect(state.snapshot).toBeNull();
    expect(state.verdicts).toBeNull();
    expect(state.phase).toBe('idle');
    expect(state.lastTick).toBeNull();
  });
});
