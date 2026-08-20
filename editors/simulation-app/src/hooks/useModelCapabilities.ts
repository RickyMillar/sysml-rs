/**
 * useModelCapabilities — derives a typed capability profile from backend stats.
 *
 * Reads statsCache from workspace store (populated per-file from the
 * /models/{uri}/stats endpoint). This is authoritative — no tree-walk
 * depth issues.
 */

import { useMemo } from 'react';
import { useWorkspaceStore } from '../store/workspace';

export interface ModelCapabilities {
  hasStateMachines: boolean;
  hasODEs: boolean;
  hasConstraints: boolean;
  hasRequirements: boolean;
  hasVerification: boolean;
  hasAnalysisCases: boolean;
  hasFlows: boolean;
  hasPlots: boolean;
  hasActionFlows: boolean;

  smCount: number;
  smInstanceCount: number;
  odeCount: number;
  flowCount: number;
  constraintCount: number;
  requirementCount: number;
  verificationCount: number;
  analysisCaseCount: number;
  partCount: number;
  actionDefCount: number;

  sessionType: 'orchestrator' | 'sm' | 'action';
  isMultiFile: boolean;
}

const EMPTY: ModelCapabilities = {
  hasStateMachines: false,
  hasODEs: false,
  hasConstraints: false,
  hasRequirements: false,
  hasVerification: false,
  hasAnalysisCases: false,
  hasFlows: false,
  hasPlots: false,
  hasActionFlows: false,
  smCount: 0,
  smInstanceCount: 0,
  odeCount: 0,
  flowCount: 0,
  constraintCount: 0,
  requirementCount: 0,
  verificationCount: 0,
  analysisCaseCount: 0,
  partCount: 0,
  actionDefCount: 0,
  sessionType: 'sm',
  isMultiFile: false,
};

export function useModelCapabilities(): ModelCapabilities {
  const statsCache = useWorkspaceStore((s) => s.statsCache);
  const caps = useWorkspaceStore((s) => s.capabilities);
  const fileCount = useWorkspaceStore((s) => s.loadedFiles.size);

  return useMemo(() => {
    if (fileCount === 0) return EMPTY;

    // Sum stats across all loaded URIs
    let smDefCount = 0;
    let smInstanceCount = 0;
    let actionDefCount = 0;
    let constraintCount = 0;
    let requirementCount = 0;
    let verificationCount = 0;
    let analysisCaseCount = 0;
    let flowCount = 0;
    let partCount = 0;

    let metadataCount = 0;

    for (const stats of statsCache.values()) {
      smDefCount += (stats.StateDefinition ?? 0);
      smInstanceCount += (stats.StateUsage ?? 0) + (stats.ExhibitStateUsage ?? 0);
      actionDefCount += (stats.ActionDefinition ?? 0);
      constraintCount += (stats.ConstraintUsage ?? 0) + (stats.ConstraintDefinition ?? 0)
        + (stats.AssertConstraintUsage ?? 0);
      requirementCount += (stats.RequirementUsage ?? 0) + (stats.RequirementDefinition ?? 0);
      verificationCount += (stats.VerificationCaseUsage ?? 0) + (stats.VerificationCaseDefinition ?? 0);
      analysisCaseCount += (stats.AnalysisCaseUsage ?? 0) + (stats.AnalysisCaseDefinition ?? 0);
      flowCount += (stats.FlowConnectionUsage ?? 0);
      partCount += (stats.PartUsage ?? 0);
      metadataCount += (stats.MetadataUsage ?? 0);
    }

    const isMultiFile = fileCount > 1;
    const hasStateMachines = smDefCount > 0 || caps.hasStateMachines;
    const hasODEs = caps.hasOdeDynamics || (isMultiFile && hasStateMachines);
    const hasActionFlows = actionDefCount > 0 || caps.hasActionFlows;

    let sessionType: ModelCapabilities['sessionType'] = 'sm';
    if (hasODEs || isMultiFile) {
      sessionType = 'orchestrator';
    } else if (hasActionFlows && !hasStateMachines) {
      sessionType = 'action';
    }

    return {
      hasStateMachines,
      hasODEs,
      hasConstraints: constraintCount > 0 || caps.hasConstraints,
      hasRequirements: requirementCount > 0 || caps.hasRequirements,
      hasVerification: verificationCount > 0,
      hasAnalysisCases: analysisCaseCount > 0 || caps.hasTradeStudies,
      hasFlows: flowCount > 0 || caps.hasPortFlows,
      hasPlots: false,
      smCount: Math.max(smDefCount, caps.stateMachineNames.length),
      smInstanceCount,
      odeCount: hasODEs ? Math.max(metadataCount, 1) : 0,
      flowCount,
      constraintCount,
      requirementCount,
      verificationCount,
      analysisCaseCount,
      hasActionFlows,
      partCount,
      actionDefCount,
      sessionType,
      isMultiFile,
    };
  }, [statsCache, caps, fileCount]);
}

export { EMPTY as EMPTY_CAPABILITIES };
