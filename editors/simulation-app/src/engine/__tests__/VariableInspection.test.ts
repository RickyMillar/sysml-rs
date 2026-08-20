/**
 * Unit tests for the engine-layer VariableInspection adapter.
 *
 * We exercise the pure read helpers (`readCurrent`, `readAtTick`,
 * `readSeries`, `readAcrossSessions`) directly, plus
 * `createVariableInspection` with a real QueryClient so we pin the
 * contract shape that every workflow depends on.
 */

import { describe, it, expect } from 'vitest';
import { QueryClient } from '@tanstack/react-query';
import {
  createVariableInspection,
  readAcrossSessions,
  readAtTick,
  readCurrent,
  readSeries,
} from '../VariableInspection';
import { sessionKeys } from '../../features/sessions/queries';
import type {
  SessionDetail,
  SessionSummary,
  TimePoint,
} from '../../features/sessions/types';

// ── Fixtures ─────────────────────────────────────────────────────────

function makeSummary(id: string, overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    id,
    kind: 'simulation',
    uri: 'file:///m.sysml',
    subsystem_name: null,
    label: null,
    created_at_ms: 0,
    elapsed_ms: 0,
    tick: 0,
    time_ms: 0,
    current_state: null,
    completed: false,
    is_expired: false,
    history_len: 0,
    subsystem_count: 0,
    fork_point_tick: null,
    paused: false,
    paused_at_breakpoint: null,
    ticks_advanced: 0,
    ...overrides,
  };
}

function makeDetail(
  id: string,
  snapshot: Record<string, unknown> | null,
  summaryOverrides: Partial<SessionSummary> = {},
): SessionDetail {
  return {
    summary: makeSummary(id, summaryOverrides),
    subsystems: [],
    latest_snapshot: snapshot,
  };
}

// ── readCurrent ──────────────────────────────────────────────────────

describe('readCurrent', () => {
  it('returns null when detail is missing', () => {
    expect(readCurrent(null, 'x')).toBeNull();
    expect(readCurrent(undefined, 'x')).toBeNull();
  });

  it('returns null when latest_snapshot is missing', () => {
    const detail = makeDetail('s1', null);
    expect(readCurrent(detail, 'x')).toBeNull();
  });

  it('returns null when variables is missing or not an object', () => {
    expect(readCurrent(makeDetail('s1', {}), 'x')).toBeNull();
    expect(
      readCurrent(makeDetail('s1', { variables: 42 }), 'x'),
    ).toBeNull();
  });

  it('returns the variable value when present', () => {
    const detail = makeDetail('s1', {
      variables: { voltage: 3.3, label: 'on', active: true, empty: null },
    });
    expect(readCurrent(detail, 'voltage')).toBe(3.3);
    expect(readCurrent(detail, 'label')).toBe('on');
    expect(readCurrent(detail, 'active')).toBe(true);
    expect(readCurrent(detail, 'empty')).toBeNull();
  });

  it('returns null for missing variable name', () => {
    const detail = makeDetail('s1', { variables: { a: 1 } });
    expect(readCurrent(detail, 'missing')).toBeNull();
  });

  it('returns structured Quantity values intact', () => {
    const q = { value: 4.2, dimension: [0, 0, 0, 1, 0, 0, 0], unit: 'A' };
    const detail = makeDetail('s1', { variables: { amps: q } });
    expect(readCurrent(detail, 'amps')).toEqual(q);
  });
});

// ── readAtTick ───────────────────────────────────────────────────────

describe('readAtTick', () => {
  const series: TimePoint[] = [
    { t: 0, v: 10 },
    { t: 100, v: 20 },
    { t: 200, v: 30 },
  ];

  it('returns the value at the requested tick index', () => {
    expect(readAtTick(series, 0)).toBe(10);
    expect(readAtTick(series, 1)).toBe(20);
    expect(readAtTick(series, 2)).toBe(30);
  });

  it('returns null when tick is out of range', () => {
    expect(readAtTick(series, -1)).toBeNull();
    expect(readAtTick(series, 3)).toBeNull();
    expect(readAtTick(series, 999)).toBeNull();
  });

  it('returns null for non-finite ticks', () => {
    expect(readAtTick(series, NaN)).toBeNull();
    expect(readAtTick(series, Infinity)).toBeNull();
  });

  it('returns null for an empty series', () => {
    expect(readAtTick([], 0)).toBeNull();
  });

  it('floors fractional ticks', () => {
    expect(readAtTick(series, 1.9)).toBe(20);
  });
});

// ── readSeries ───────────────────────────────────────────────────────

describe('readSeries', () => {
  it('returns the named series', () => {
    const all = {
      v: [{ t: 0, v: 1 }],
      i: [{ t: 0, v: 0.5 }],
    };
    expect(readSeries(all, 'v')).toEqual([{ t: 0, v: 1 }]);
  });

  it('returns an empty array for unknown names', () => {
    expect(readSeries({ v: [{ t: 0, v: 1 }] }, 'missing')).toEqual([]);
  });

  it('returns an empty array when given an empty map', () => {
    expect(readSeries({}, 'x')).toEqual([]);
  });
});

// ── readAcrossSessions ───────────────────────────────────────────────

describe('readAcrossSessions', () => {
  it('collects current values from each session', () => {
    const details: Record<string, SessionDetail> = {
      s1: makeDetail('s1', { variables: { m: 1.0 } }),
      s2: makeDetail('s2', { variables: { m: 2.5 } }),
      s3: makeDetail('s3', { variables: { m: 0.1 } }),
    };
    const result = readAcrossSessions(
      ['s1', 's2', 's3'],
      'm',
      (id) => details[id],
    );
    expect(result.size).toBe(3);
    expect(result.get('s1')).toBe(1.0);
    expect(result.get('s2')).toBe(2.5);
    expect(result.get('s3')).toBe(0.1);
  });

  it('skips sessions that have no snapshot for the variable', () => {
    const details: Record<string, SessionDetail | null> = {
      s1: makeDetail('s1', { variables: { m: 1.0 } }),
      s2: null,
      s3: makeDetail('s3', { variables: {} }),
    };
    const result = readAcrossSessions(
      ['s1', 's2', 's3'],
      'm',
      (id) => details[id],
    );
    expect(Array.from(result.keys())).toEqual(['s1']);
  });

  it('returns an empty map for an empty session list', () => {
    const result = readAcrossSessions([], 'm', () => null);
    expect(result.size).toBe(0);
  });
});

// ── createVariableInspection ─────────────────────────────────────────

describe('createVariableInspection', () => {
  function setupClient(
    details: Record<string, SessionDetail>,
    series: Record<string, TimePoint[]> = {},
  ) {
    const qc = new QueryClient();
    for (const [id, detail] of Object.entries(details)) {
      qc.setQueryData(sessionKeys.detail(id), detail);
    }
    const inspector = createVariableInspection(qc, () => series);
    return { qc, inspector };
  }

  it('current() reads the variable from the query cache', () => {
    const { inspector } = setupClient({
      s1: makeDetail('s1', { variables: { voltage: 5.0 } }),
    });
    expect(inspector.current('s1', 'voltage')).toBe(5.0);
    expect(inspector.current('s1', 'missing')).toBeNull();
    expect(inspector.current('no-such-session', 'voltage')).toBeNull();
  });

  it('series() reads from the injected time-series accessor', () => {
    const points: TimePoint[] = [
      { t: 0, v: 1 },
      { t: 10, v: 2 },
    ];
    const { inspector } = setupClient({}, { voltage: points });
    expect(inspector.series('s1', 'voltage')).toEqual(points);
    expect(inspector.series('s1', 'missing')).toEqual([]);
  });

  it('atTick() indexes into the time-series accessor', () => {
    const points: TimePoint[] = [
      { t: 0, v: 10 },
      { t: 10, v: 20 },
      { t: 20, v: 30 },
    ];
    const { inspector } = setupClient({}, { v: points });
    expect(inspector.atTick('s1', 0, 'v')).toBe(10);
    expect(inspector.atTick('s1', 2, 'v')).toBe(30);
    expect(inspector.atTick('s1', 99, 'v')).toBeNull();
  });

  it('acrossSessions() collects values from the query cache', () => {
    const { inspector } = setupClient({
      a: makeDetail('a', { variables: { x: 1 } }),
      b: makeDetail('b', { variables: { x: 2 } }),
      c: makeDetail('c', { variables: { x: 3 } }),
    });
    const result = inspector.acrossSessions(['a', 'b', 'c'], 'x');
    expect(result.get('a')).toBe(1);
    expect(result.get('b')).toBe(2);
    expect(result.get('c')).toBe(3);
  });
});
