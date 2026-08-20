/**
 * Create-time scenario overrides — the client half of J3's
 * "a severe run is configured with restrictionConductance = 0.3".
 *
 * The rule these gates hold down: a scenario is chosen when the session is
 * BUILT, and it travels on the one `sessions.create` call. If it leaked into
 * the step path instead, the first ticks of a "severe" run would have executed
 * nominal — which is exactly the failure the journey is meant to catch, and it
 * would be invisible in the UI.
 */
import { describe, expect, it, beforeEach } from 'vitest';
import { createParamsForTarget } from '../sessionCreate';
import { toSessionRecord } from '../normalize';
import { useSessionStore } from '../store';
import type { SessionSummary } from '../types';
import type { RunTargetSummary } from '@/features/run-targets/types';

const SEVERE: [string, string][] = [['restrictionConductance', '0.3']];

function target(): RunTargetSummary {
  return {
    id: 'el-1',
    name: 'PumpCycle',
    kind: 'simulation',
    uri: 'file:///pump/Behaviour/PumpCycle.sysml',
    metadata: { elementKind: 'StateDefinition' },
  };
}

function summary(overrides?: [string, string][]): SessionSummary {
  return {
    id: 'sess-1',
    kind: 'orchestrator',
    uri: '__workspace__',
    subsystem_name: null,
    label: null,
    created_at_ms: 0,
    elapsed_ms: 0,
    tick: 1,
    time_ms: 2,
    current_state: 'intake',
    completed: false,
    is_expired: false,
    history_len: 1,
    subsystem_count: 2,
    fork_point_tick: null,
    paused: false,
    ...(overrides ? { create_overrides: overrides } : {}),
  } as SessionSummary;
}

describe('createParamsForTarget — scenario travels with creation', () => {
  it('sends the scenario alongside the whole-workspace run', () => {
    const params = createParamsForTarget(null, 2, SEVERE);
    expect(params.uri).toBe('__workspace__');
    expect(params.overrides).toEqual(SEVERE);
  });

  it('sends the scenario alongside a named target too', () => {
    const params = createParamsForTarget(target(), 2, SEVERE);
    expect(params.target).toBe('PumpCycle');
    expect(params.overrides).toEqual(SEVERE);
  });

  // An empty list must not become `overrides: []` on the wire: the backend
  // would record an empty `create_overrides` either way, but sending the key
  // implies a scenario was configured. "No scenario" and "an empty scenario"
  // have to look the same, because they are the same.
  it('omits the field entirely when nothing is staged', () => {
    expect(createParamsForTarget(null, 2, []).overrides).toBeUndefined();
    expect(createParamsForTarget(null, 2).overrides).toBeUndefined();
  });
});

describe('session store — scenario is run configuration, not view state', () => {
  beforeEach(() => {
    useSessionStore.setState({ scenarioOverrides: [] });
  });

  it('stages and replaces the scenario wholesale', () => {
    useSessionStore.getState().setScenarioOverrides(SEVERE);
    expect(useSessionStore.getState().scenarioOverrides).toEqual(SEVERE);
  });

  // The regression this guards: if the scenario lived in `SessionViewState`,
  // `resetViewState()` — which fires on session switch, i.e. immediately after
  // creating the very run it configured — would wipe it, and Configure would
  // report "None" for a session that is demonstrably severe.
  it('survives resetViewState, which fires when the new session is selected', () => {
    useSessionStore.getState().setScenarioOverrides(SEVERE);
    useSessionStore.getState().resetViewState();
    expect(useSessionStore.getState().scenarioOverrides).toEqual(SEVERE);
  });

  it('keeps draft (step-time) overrides in a separate bucket', () => {
    useSessionStore.getState().setScenarioOverrides(SEVERE);
    useSessionStore.getState().setDraftOverride('restrictionConductance', '0.9');
    expect(useSessionStore.getState().scenarioOverrides).toEqual(SEVERE);
    expect(useSessionStore.getState().draftOverrides).toEqual({
      restrictionConductance: '0.9',
    });
  });
});

describe('toSessionRecord — scenario provenance reaches the UI', () => {
  it('carries create_overrides through', () => {
    expect(toSessionRecord(summary(SEVERE)).createOverrides).toEqual(SEVERE);
  });

  it('treats an absent field as the baseline, not as unknown', () => {
    expect(toSessionRecord(summary()).createOverrides).toEqual([]);
  });
});
