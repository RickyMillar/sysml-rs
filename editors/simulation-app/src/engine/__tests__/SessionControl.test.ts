/**
 * Unit tests for the engine-layer SessionControl adapter.
 *
 * The React hook itself (`useSessionControl`) is thin glue around
 * `useSessionController` + mutations + the breakpoint REST client,
 * exercised by higher-level render tests elsewhere. These tests pin
 * the **pure contract pieces** that every future workflow depends on:
 *
 *   - buildCreateParams    — SessionStartConfig → sysml.sessions.create params
 *   - overridesToTuples    — engine Overrides map → [name, value] tuples
 *   - serializeValue       — Value → string for the override wire format
 *   - createBreakpointClient — REST shape for sysml.breakpoint.{set,clear,list}
 */

import { describe, it, expect } from 'vitest';
import {
  buildCreateParams,
  overridesToTuples,
  serializeValue,
  createBreakpointClient,
} from '../SessionControl';
import type { Breakpoint, SessionStartConfig } from '../types';
import type { RunTargetSummary } from '../../features/run-targets/types';

// ── Fixtures ─────────────────────────────────────────────────────────

function makeTarget(overrides: Partial<RunTargetSummary> & {
  elementKind: string;
}): RunTargetSummary {
  const { elementKind, ...rest } = overrides;
  return {
    id: 'tgt-1',
    name: 'Target',
    kind: 'simulation',
    uri: 'file:///tmp/x.sysml',
    metadata: { elementKind },
    ...rest,
  };
}

// ── buildCreateParams ────────────────────────────────────────────────
//
// The server infers the SessionKind, so there is no command/capability
// dispatch on the client. buildCreateParams only resolves *what to run* into
// `sysml.sessions.create` parameters.

describe('buildCreateParams', () => {
  it('uses an explicit config.uri + config.target (server infers the kind)', () => {
    const config: SessionStartConfig = {
      uri: 'file:///model.sysml',
      target: 'Door',
    };
    const result = buildCreateParams(config, null, 1.0);
    expect(result).toEqual({
      uri: 'file:///model.sysml',
      target: 'Door',
      dtMs: 1.0,
    });
  });

  it('falls back to the selected run target when no explicit uri', () => {
    const target = makeTarget({ elementKind: 'StateDefinition', name: 'SM1' });
    const result = buildCreateParams({}, target, 1.0);
    expect(result).toEqual({
      uri: 'file:///tmp/x.sysml',
      target: 'SM1',
      dtMs: 1.0,
    });
  });

  it('does not forward config.overrides at creation (deliberate hold)', () => {
    // `sessions.create` DOES take `overrides` now (create-time scenarios), and
    // `SessionStartConfig.overrides` means exactly that. It is still not
    // forwarded here because every caller is an analyze sweep, where moving
    // from per-tick to per-build application changes the numbers those runs
    // produce — see the rationale on `buildCreateParams`. This test pins the
    // hold so the switch is a conscious change with re-blessed baselines,
    // not a silent drift.
    const target = makeTarget({ elementKind: 'StateDefinition', name: 'SM1' });
    const result = buildCreateParams(
      { overrides: { 'm.g': 9.81, 'm.kind': 'Earth' } },
      target,
      2.5,
    );
    expect(result).not.toHaveProperty('overrides');
    expect(result.dtMs).toBe(2.5);
  });

  it('uses config.dtMs over the default when provided', () => {
    const result = buildCreateParams({ dtMs: 0.5 }, null, 10.0);
    expect(result.dtMs).toBe(0.5);
  });

  it('no uri and no target → whole-workspace orchestrator', () => {
    const result = buildCreateParams({}, null, 1.0);
    expect(result).toEqual({ uri: '__workspace__', dtMs: 1.0 });
  });
});

// ── overridesToTuples / serializeValue ───────────────────────────────

describe('overridesToTuples + serializeValue', () => {
  it('serializes numbers, booleans, strings, nulls directly', () => {
    expect(serializeValue(3.14)).toBe('3.14');
    expect(serializeValue(true)).toBe('true');
    expect(serializeValue(false)).toBe('false');
    expect(serializeValue('hello')).toBe('hello');
    expect(serializeValue(null)).toBe('null');
  });

  it('JSON-encodes structured values', () => {
    expect(serializeValue({ re: 1, im: 2 })).toBe('{"re":1,"im":2}');
    expect(serializeValue([1, 2, 3])).toBe('[1,2,3]');
  });

  it('produces tuples preserving iteration order', () => {
    const tuples = overridesToTuples({ a: 1, b: true, c: 'x' });
    expect(tuples).toEqual([
      ['a', '1'],
      ['b', 'true'],
      ['c', 'x'],
    ]);
  });

  it('returns an empty array for an empty map', () => {
    expect(overridesToTuples({})).toEqual([]);
  });
});

// ── createBreakpointClient ───────────────────────────────────────────

describe('createBreakpointClient', () => {
  function makeMockPoster() {
    const calls: Array<{ path: string; body: unknown }> = [];
    const responses = new Map<string, unknown>();
    const poster = <T>(path: string, body?: unknown): Promise<T> => {
      calls.push({ path, body });
      const cmd = (body as { command?: string } | undefined)?.command ?? '';
      const resp = responses.get(cmd);
      return Promise.resolve(resp as T);
    };
    return { poster, calls, responses };
  }

  const SID = 'session-abc';

  it('serialises state-entry breakpoint to backend shape (element_id)', async () => {
    const { poster, calls, responses } = makeMockPoster();
    responses.set('sysml.breakpoint.set', { id: 'bp-42' });
    const client = createBreakpointClient(poster);
    const bp: Breakpoint = {
      kind: 'state-entry',
      target: 'SM::Running',
    };
    const id = await client.set(SID, bp);
    expect(id).toBe('bp-42');
    expect(calls).toHaveLength(1);
    expect(calls[0].path).toBe('/api/command');
    expect(calls[0].body).toEqual({
      command: 'sysml.breakpoint.set',
      params: {
        session_id: SID,
        breakpoint: { kind: 'state-entry', element_id: 'SM::Running' },
      },
    });
  });

  it('serialises threshold-crossing direction → backend op + value', async () => {
    const { poster, calls, responses } = makeMockPoster();
    responses.set('sysml.breakpoint.set', 'bp-1');
    const client = createBreakpointClient(poster);
    await client.set(SID, {
      kind: 'threshold-crossing',
      target: 'circuit5.bimetalTemp',
      variable: 'circuit5.bimetalTemp',
      threshold: 350,
      direction: 'rising',
    });
    expect(calls[0].body).toEqual({
      command: 'sysml.breakpoint.set',
      params: {
        session_id: SID,
        breakpoint: {
          kind: 'threshold-crossing',
          variable: 'circuit5.bimetalTemp',
          op: 'gt',
          value: 350,
          debounce_ticks: 0,
        },
      },
    });
  });

  it('serialises threshold-crossing falling → op:lt', async () => {
    const { poster, calls, responses } = makeMockPoster();
    responses.set('sysml.breakpoint.set', 'bp-1');
    const client = createBreakpointClient(poster);
    await client.set(SID, {
      kind: 'threshold-crossing',
      target: 'x',
      variable: 'x',
      threshold: 10,
      direction: 'falling',
    });
    const bp = (calls[0].body as { params: { breakpoint: { op: string } } }).params.breakpoint;
    expect(bp.op).toBe('lt');
  });

  it('accepts a plain string response from sysml.breakpoint.set', async () => {
    const { poster, responses } = makeMockPoster();
    responses.set('sysml.breakpoint.set', 'bp-9');
    const client = createBreakpointClient(poster);
    const id = await client.set(SID, { kind: 'transition-fire', target: 't.1' });
    expect(id).toBe('bp-9');
  });

  it('throws on an unrecognised set response shape', async () => {
    const { poster, responses } = makeMockPoster();
    responses.set('sysml.breakpoint.set', 12345);
    const client = createBreakpointClient(poster);
    await expect(
      client.set(SID, { kind: 'state-entry', target: 'x' }),
    ).rejects.toThrow(/sysml.breakpoint.set/);
  });

  it('posts sysml.breakpoint.clear with session_id+breakpoint_id', async () => {
    const { poster, calls } = makeMockPoster();
    const client = createBreakpointClient(poster);
    await client.clear(SID, 'bp-42');
    expect(calls[0].body).toEqual({
      command: 'sysml.breakpoint.clear',
      params: { session_id: SID, breakpoint_id: 'bp-42' },
    });
  });

  it('lifts wire-shaped pairs back into UI Breakpoint shape', async () => {
    const { poster, calls, responses } = makeMockPoster();
    // Backend returns [id, Breakpoint] pairs in the Rust struct shape:
    //   StateEntry { element_id }, ThresholdCrossing { variable, op, value }
    responses.set('sysml.breakpoint.list', [
      ['bp-1', { kind: 'state-entry', element_id: 'SM::A' }],
      [
        'bp-2',
        {
          kind: 'threshold-crossing',
          variable: 'busbar.T',
          op: 'gt',
          value: 105,
          debounce_ticks: 5,
        },
      ],
    ]);
    const client = createBreakpointClient(poster);
    const result = await client.list(SID);
    expect(result).toHaveLength(2);
    expect(result[0]).toEqual([
      'bp-1',
      { kind: 'state-entry', target: 'SM::A' },
    ]);
    expect(result[1][0]).toBe('bp-2');
    const bp = result[1][1];
    expect(bp.kind).toBe('threshold-crossing');
    if (bp.kind === 'threshold-crossing') {
      expect(bp.variable).toBe('busbar.T');
      expect(bp.threshold).toBe(105);
      expect(bp.direction).toBe('rising');
      expect(bp.debounce_ticks).toBe(5);
    }
    expect(calls[0].body).toEqual({
      command: 'sysml.breakpoint.list',
      params: { session_id: SID },
    });
  });

  it('unwraps { breakpoints: [...] } responses from list', async () => {
    const { poster, responses } = makeMockPoster();
    responses.set('sysml.breakpoint.list', {
      breakpoints: [['bp-9', { kind: 'constraint-violation', element_id: 'req1' }]],
    });
    const client = createBreakpointClient(poster);
    const r = await client.list(SID);
    expect(r).toEqual([['bp-9', { kind: 'constraint-violation', target: 'req1' }]]);
  });

  it('returns an empty array when list response is malformed', async () => {
    const { poster, responses } = makeMockPoster();
    responses.set('sysml.breakpoint.list', null);
    const client = createBreakpointClient(poster);
    expect(await client.list(SID)).toEqual([]);
  });
});
