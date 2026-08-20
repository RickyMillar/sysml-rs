/**
 * Client for the backend `sysml.workspace.capabilities` service command.
 *
 * The backend owns capability detection; this module just translates
 * the snake_case wire fields to the camelCase `Capabilities` interface
 * the workspace store and panel registry consume.
 */
import { httpPost } from './http';
import type { Capabilities } from '@/store/workspace';

/** Wire shape returned by `sysml.workspace.capabilities`. */
export interface WorkspaceCapabilitiesWire {
  has_state_machines: boolean;
  has_action_flows: boolean;
  has_ode_dynamics: boolean;
  has_port_flows: boolean;
  has_multiple_subsystems: boolean;
  has_constraints: boolean;
  has_requirements: boolean;
  has_trade_studies: boolean;
  state_machine_names: string[];
  action_flow_names: string[];
  trade_study_names: string[];
}

/** Map the snake_case wire response onto the camelCase store interface. */
export function capabilitiesFromWire(wire: WorkspaceCapabilitiesWire): Capabilities {
  return {
    hasStateMachines: wire.has_state_machines,
    hasActionFlows: wire.has_action_flows,
    hasOdeDynamics: wire.has_ode_dynamics,
    hasPortFlows: wire.has_port_flows,
    hasMultipleSubsystems: wire.has_multiple_subsystems,
    hasConstraints: wire.has_constraints,
    hasRequirements: wire.has_requirements,
    hasTradeStudies: wire.has_trade_studies,
    stateMachineNames: wire.state_machine_names ?? [],
    actionFlowNames: wire.action_flow_names ?? [],
    tradeStudyNames: wire.trade_study_names ?? [],
  };
}

/**
 * Fetch the backend-owned capability profile for the currently loaded
 * workspace. The backend keys its tracked query on the elaborated
 * workspace so repeated calls between loads are cheap.
 */
export async function fetchWorkspaceCapabilities(): Promise<Capabilities> {
  const wire = await httpPost<WorkspaceCapabilitiesWire>('/api/command', {
    command: 'sysml.workspace.capabilities',
    params: {},
  });
  return capabilitiesFromWire(wire);
}
