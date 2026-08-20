import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  capabilitiesFromWire,
  fetchWorkspaceCapabilities,
  type WorkspaceCapabilitiesWire,
} from '../capabilities';

const ORIGINAL_FETCH = globalThis.fetch;

afterEach(() => {
  globalThis.fetch = ORIGINAL_FETCH;
  vi.restoreAllMocks();
});

function mockJsonResponse(body: unknown, status = 200) {
  return {
    ok: status >= 200 && status < 300,
    status,
    statusText: 'OK',
    json: () => Promise.resolve(body),
  } as Response;
}

const SAMPLE_WIRE: WorkspaceCapabilitiesWire = {
  has_state_machines: true,
  has_action_flows: false,
  has_ode_dynamics: true,
  has_port_flows: true,
  has_multiple_subsystems: true,
  has_constraints: false,
  has_requirements: false,
  has_trade_studies: true,
  state_machine_names: ['Engine', 'Brake'],
  action_flow_names: [],
  trade_study_names: ['TS1'],
};

describe('capabilitiesFromWire', () => {
  it('maps snake_case wire fields onto camelCase Capabilities', () => {
    const caps = capabilitiesFromWire(SAMPLE_WIRE);
    expect(caps).toEqual({
      hasStateMachines: true,
      hasActionFlows: false,
      hasOdeDynamics: true,
      hasPortFlows: true,
      hasMultipleSubsystems: true,
      hasConstraints: false,
      hasRequirements: false,
      hasTradeStudies: true,
      stateMachineNames: ['Engine', 'Brake'],
      actionFlowNames: [],
      tradeStudyNames: ['TS1'],
    });
  });

  it('defaults missing name lists to empty arrays', () => {
    const partial = {
      ...SAMPLE_WIRE,
      state_machine_names: undefined,
      action_flow_names: undefined,
      trade_study_names: undefined,
    } as unknown as WorkspaceCapabilitiesWire;
    const caps = capabilitiesFromWire(partial);
    expect(caps.stateMachineNames).toEqual([]);
    expect(caps.actionFlowNames).toEqual([]);
    expect(caps.tradeStudyNames).toEqual([]);
  });
});

describe('fetchWorkspaceCapabilities', () => {
  it('POSTs the workspace.capabilities envelope and returns mapped Capabilities', async () => {
    const fetchMock = vi.fn().mockResolvedValue(mockJsonResponse(SAMPLE_WIRE));
    globalThis.fetch = fetchMock as unknown as typeof fetch;

    const caps = await fetchWorkspaceCapabilities();

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [path, init] = fetchMock.mock.calls[0]!;
    expect(path).toBe('/api/command');
    expect((init as RequestInit).method).toBe('POST');
    const body = JSON.parse((init as RequestInit).body as string);
    expect(body).toEqual({
      command: 'sysml.workspace.capabilities',
      params: {},
    });
    expect(caps.hasStateMachines).toBe(true);
    expect(caps.stateMachineNames).toEqual(['Engine', 'Brake']);
  });

  it('throws on non-2xx responses', async () => {
    globalThis.fetch = vi
      .fn()
      .mockResolvedValue(mockJsonResponse({ error: 'no workspace' }, 500)) as unknown as typeof fetch;
    await expect(fetchWorkspaceCapabilities()).rejects.toThrow(/no workspace/);
  });
});
