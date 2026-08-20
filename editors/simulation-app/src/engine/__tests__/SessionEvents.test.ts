/**
 * Unit tests for SessionEventBus — R1.5 event stream.
 *
 * Covers the acceptance list in the agent brief:
 *   1. polling derives tick events from snapshot diffs
 *   2. unsub stops polling when last subscriber leaves
 *   3. breakpoint-hit fires when phase transitions to paused + marker present
 *   4. verdict-flip fires when a constraint result's verdict changes
 *
 * The bus is tested with a controlled mock SnapshotSource + Scheduler,
 * so no real timers or backend calls are involved.
 */

import { describe, it, expect, beforeEach } from 'vitest';
import {
  SessionEventBus,
  type SessionEvent,
  type SnapshotSource,
  type Scheduler,
} from '../SessionEvents';
import type {
  SessionDetail,
  SessionPhase,
} from '../../features/sessions/types';

// ── Mocks ────────────────────────────────────────────────────────────

/**
 * Scheduler that records the fn passed to setInterval and lets the test
 * drive ticks manually. Never actually schedules real timers.
 */
function makeManualScheduler(): {
  scheduler: Scheduler;
  runOnce: () => void;
  clearedCount: () => number;
  active: () => boolean;
} {
  // Map of opaque handle -> handler fn. The bus creates one timer per
  // subscribed session; the test driver runs every active handler on
  // each `runOnce()` call, mirroring a real setInterval firing.
  const handlers = new Map<object, () => void>();
  let cleared = 0;

  const scheduler: Scheduler = {
    setInterval: (fn) => {
      const handle = {};
      handlers.set(handle, fn);
      return handle;
    },
    clearInterval: (h) => {
      if (handlers.delete(h as object)) cleared += 1;
    },
  };

  return {
    scheduler,
    runOnce: () => {
      for (const fn of Array.from(handlers.values())) fn();
    },
    clearedCount: () => cleared,
    active: () => handlers.size > 0,
  };
}

interface MockSource extends SnapshotSource {
  set(session: string, detail: SessionDetail | null, phase: SessionPhase | null): void;
}

function makeMockSource(): MockSource {
  const detailMap = new Map<string, SessionDetail | null>();
  const phaseMap = new Map<string, SessionPhase | null>();
  return {
    getDetail: (s) => detailMap.get(s) ?? null,
    getPhase: (s) => phaseMap.get(s) ?? null,
    set(s, d, p) {
      detailMap.set(s, d);
      phaseMap.set(s, p);
    },
  };
}

function makeDetail(
  tick: number,
  opts: {
    subsystems?: Array<{ name: string; state: string }>;
    constraints?: Array<{ name: string; satisfied: boolean }>;
    completed?: boolean;
    /**
     * BreakpointId that triggered the current pause, or omitted/null
     * for none. Lives on `summary.paused_at_breakpoint` (BP1) — NOT
     * the snapshot; an earlier version of this fixture (and of the
     * production `extractBreakpointMarker` it exercises) put it on
     * `latest_snapshot` before the real backend field landed.
     */
    pausedAtBreakpoint?: string | null;
    timeMs?: number;
  } = {},
): SessionDetail {
  const timeMs = opts.timeMs ?? tick;
  const subsystems = (opts.subsystems ?? []).map((s) => ({
    name: s.name,
    kind_label: 'sm',
    current_state: s.state,
    completed: false,
    available_transitions: [] as [string, string][],
  }));
  const latestSnapshot: Record<string, unknown> | null = opts.constraints
    ? {
        constraint_results: (opts.constraints ?? []).map((c) => ({
          name: c.name,
          expression: '',
          satisfied: c.satisfied,
        })),
      }
    : null;
  return {
    summary: {
      id: 'sid',
      kind: 'simulation',
      uri: 'test://foo',
      subsystem_name: null,
      label: null,
      created_at_ms: 0,
      elapsed_ms: 0,
      tick,
      time_ms: timeMs,
      current_state: null,
      completed: !!opts.completed,
      is_expired: false,
      history_len: tick,
      subsystem_count: subsystems.length,
      fork_point_tick: null,
      paused: opts.pausedAtBreakpoint != null,
      paused_at_breakpoint: opts.pausedAtBreakpoint ?? null,
      ticks_advanced: 0,
    },
    subsystems,
    latest_snapshot: latestSnapshot,
  };
}

// ── Tests ────────────────────────────────────────────────────────────

describe('SessionEventBus — polling', () => {
  const SESSION = 'sess-1';
  let source: MockSource;
  let sched: ReturnType<typeof makeManualScheduler>;
  let bus: SessionEventBus;

  beforeEach(() => {
    source = makeMockSource();
    sched = makeManualScheduler();
    bus = new SessionEventBus({
      source,
      scheduler: sched.scheduler,
      intervalMs: 100,
    });
  });

  it('derives tick events from snapshot diffs', () => {
    // Seed: session at tick 0.
    source.set(
      SESSION,
      makeDetail(0, { subsystems: [{ name: 'A', state: 'Idle' }] }),
      'running',
    );
    const ticks: SessionEvent[] = [];
    bus.on(SESSION, 'tick', (ev) => ticks.push(ev));

    // Baseline is seeded. Advance to tick 1 — should emit one 'tick'.
    source.set(
      SESSION,
      makeDetail(1, { subsystems: [{ name: 'A', state: 'Idle' }] }),
      'running',
    );
    sched.runOnce();
    expect(ticks).toHaveLength(1);
    expect(ticks[0].tick).toBe(1);
    expect(ticks[0].kind).toBe('tick');

    // Advance to tick 2 — another tick.
    source.set(
      SESSION,
      makeDetail(2, { subsystems: [{ name: 'A', state: 'Idle' }] }),
      'running',
    );
    sched.runOnce();
    expect(ticks).toHaveLength(2);
    expect(ticks[1].tick).toBe(2);

    // Same tick — no new tick event.
    sched.runOnce();
    expect(ticks).toHaveLength(2);
  });

  it('unsub stops polling when the last subscriber leaves', () => {
    source.set(SESSION, makeDetail(0), 'running');

    const unsubA = bus.on(SESSION, 'tick', () => {});
    const unsubB = bus.on(SESSION, 'transition', () => {});
    expect(sched.active()).toBe(true);

    unsubA();
    // One subscriber left — still polling.
    expect(sched.active()).toBe(true);
    expect(sched.clearedCount()).toBe(0);

    unsubB();
    // Last subscriber left — polling stops.
    expect(sched.active()).toBe(false);
    expect(sched.clearedCount()).toBe(1);
  });

  it('breakpoint-hit fires when phase transitions to paused-at-breakpoint', () => {
    // Running with no marker.
    source.set(
      SESSION,
      makeDetail(5, { subsystems: [{ name: 'A', state: 'Idle' }] }),
      'running',
    );
    const hits: SessionEvent[] = [];
    bus.on(SESSION, 'breakpoint-hit', (ev) => hits.push(ev));

    // Phase changes to paused AND a BreakpointId appears on the summary.
    source.set(
      SESSION,
      makeDetail(6, {
        subsystems: [{ name: 'A', state: 'Idle' }],
        pausedAtBreakpoint: 'bp-1',
      }),
      'paused',
    );
    sched.runOnce();
    expect(hits).toHaveLength(1);
    expect(hits[0].kind).toBe('breakpoint-hit');
    expect(hits[0].context).toMatchObject({ breakpointId: 'bp-1' });

    // Stays paused, marker unchanged — no re-fire.
    sched.runOnce();
    expect(hits).toHaveLength(1);
  });

  it('does not fire breakpoint-hit when pause happens without a marker', () => {
    source.set(SESSION, makeDetail(0), 'running');
    const hits: SessionEvent[] = [];
    bus.on(SESSION, 'breakpoint-hit', (ev) => hits.push(ev));

    // Just a manual pause — no marker.
    source.set(SESSION, makeDetail(1), 'paused');
    sched.runOnce();
    expect(hits).toHaveLength(0);
  });

  it('verdict-flip fires when a constraint result verdict changes', () => {
    // Seed with one passing constraint.
    source.set(
      SESSION,
      makeDetail(0, {
        constraints: [{ name: 'c1', satisfied: true }],
      }),
      'running',
    );
    const flips: SessionEvent[] = [];
    bus.on(SESSION, 'verdict-flip', (ev) => flips.push(ev));

    // Same verdict -> no flip.
    source.set(
      SESSION,
      makeDetail(1, {
        constraints: [{ name: 'c1', satisfied: true }],
      }),
      'running',
    );
    sched.runOnce();
    expect(flips).toHaveLength(0);

    // Flip true -> false.
    source.set(
      SESSION,
      makeDetail(2, {
        constraints: [{ name: 'c1', satisfied: false }],
      }),
      'running',
    );
    sched.runOnce();
    expect(flips).toHaveLength(1);
    expect(flips[0].kind).toBe('verdict-flip');
    expect(flips[0].context).toMatchObject({
      constraint: 'c1',
      fromSatisfied: true,
      toSatisfied: false,
    });
  });

  it('emits transition events when a subsystem state changes', () => {
    source.set(
      SESSION,
      makeDetail(0, { subsystems: [{ name: 'A', state: 'Off' }] }),
      'running',
    );
    const transitions: SessionEvent[] = [];
    bus.on(SESSION, 'transition', (ev) => transitions.push(ev));

    source.set(
      SESSION,
      makeDetail(1, { subsystems: [{ name: 'A', state: 'On' }] }),
      'running',
    );
    sched.runOnce();
    expect(transitions).toHaveLength(1);
    expect(transitions[0].context).toMatchObject({
      subsystem: 'A',
      fromState: 'Off',
      toState: 'On',
    });
  });

  it('fires completed exactly once on transition to completed', () => {
    source.set(SESSION, makeDetail(0, { completed: false }), 'running');
    const completions: SessionEvent[] = [];
    bus.on(SESSION, 'completed', (ev) => completions.push(ev));

    source.set(SESSION, makeDetail(1, { completed: true }), 'completed');
    sched.runOnce();
    expect(completions).toHaveLength(1);

    // Subsequent polls don't re-emit.
    sched.runOnce();
    expect(completions).toHaveLength(1);
  });

  it('fires error exactly once on transition to error phase', () => {
    source.set(SESSION, makeDetail(0), 'running');
    const errors: SessionEvent[] = [];
    bus.on(SESSION, 'error', (ev) => errors.push(ev));

    source.set(SESSION, makeDetail(1), 'error');
    sched.runOnce();
    expect(errors).toHaveLength(1);

    sched.runOnce();
    expect(errors).toHaveLength(1);
  });

  it('refcounts subscribers per session independently', () => {
    const S1 = 'sess-1';
    const S2 = 'sess-2';
    source.set(S1, makeDetail(0), 'running');
    source.set(S2, makeDetail(0), 'running');

    const unsub1 = bus.on(S1, 'tick', () => {});
    const unsub2 = bus.on(S2, 'tick', () => {});
    expect(sched.clearedCount()).toBe(0);

    unsub1();
    expect(sched.clearedCount()).toBe(1); // S1 poller gone

    unsub2();
    expect(sched.clearedCount()).toBe(2); // S2 poller gone
  });

  it('handles a subscriber added after the first poll seeds baseline', () => {
    // No detail at first.
    const ticks: SessionEvent[] = [];
    source.set(SESSION, makeDetail(0), 'running');
    bus.on(SESSION, 'tick', (ev) => ticks.push(ev));

    // Another subscriber arrives late on the same session — should
    // share the same poller, not re-seed and not double-fire.
    const extra: SessionEvent[] = [];
    bus.on(SESSION, 'tick', (ev) => extra.push(ev));

    source.set(SESSION, makeDetail(1), 'running');
    sched.runOnce();
    expect(ticks).toHaveLength(1);
    expect(extra).toHaveLength(1);
  });

  it('isolates a subscriber error so other subscribers still fire', () => {
    source.set(SESSION, makeDetail(0), 'running');
    const good: SessionEvent[] = [];
    bus.on(SESSION, 'tick', () => {
      throw new Error('boom');
    });
    bus.on(SESSION, 'tick', (ev) => good.push(ev));

    source.set(SESSION, makeDetail(1), 'running');
    sched.runOnce();
    expect(good).toHaveLength(1);
  });
});
