/**
 * Unit tests for sessionCreate: deriving `sysml.sessions.create` parameters
 * from the selected run target.
 *
 * The backend now infers the session kind, so the client only says *what to
 * run* — there is no capability→command decision tree to test (the old
 * `commandForTarget` / `extractSessionKey` pair is gone). What remains worth
 * pinning is that the mapping is shape-stable regardless of element kind: the
 * client never branches on it.
 */
import { describe, it, expect } from 'vitest';
import { createParamsForTarget } from '../features/sessions/sessionCreate';
import type { RunTargetSummary } from '../features/run-targets/types';

function makeTarget(
  overrides: Partial<RunTargetSummary> & { elementKind?: string } = {},
): RunTargetSummary {
  const { elementKind = 'StateDefinition', ...rest } = overrides;
  return {
    id: 'tgt-1',
    name: 'Target',
    kind: 'simulation',
    uri: 'file:///tmp/x.sysml',
    metadata: { elementKind },
    ...rest,
  };
}

describe('createParamsForTarget', () => {
  it('no target → whole-workspace orchestrator', () => {
    expect(createParamsForTarget(null, 0.5)).toEqual({
      uri: '__workspace__',
      dtMs: 0.5,
    });
  });

  it('a target → its source URI + element name (server infers the kind)', () => {
    const target = makeTarget({ name: 'TrafficLight', uri: 'file:///tmp/tl.sysml' });
    expect(createParamsForTarget(target, 1.0)).toEqual({
      uri: 'file:///tmp/tl.sysml',
      target: 'TrafficLight',
      dtMs: 1.0,
    });
  });

  it('does not branch on element kind — kind is decided server-side', () => {
    // State machine, action, and part all map the same way: the client only
    // names what to run; `sysml.sessions.create` resolves the SessionKind.
    const sm = makeTarget({ elementKind: 'StateDefinition', name: 'SM' });
    const action = makeTarget({ elementKind: 'ActionDefinition', name: 'Act' });
    const part = makeTarget({ elementKind: 'PartUsage', name: 'P' });
    expect(createParamsForTarget(sm).target).toBe('SM');
    expect(createParamsForTarget(action).target).toBe('Act');
    expect(createParamsForTarget(part).target).toBe('P');
  });

  it('a nameless target omits `target` (falls back to workspace orchestration)', () => {
    const target = makeTarget({ name: null as unknown as string });
    const params = createParamsForTarget(target);
    expect(params.uri).toBe('file:///tmp/x.sysml');
    expect(params.target).toBeUndefined();
  });
});
