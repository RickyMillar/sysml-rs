/**
 * Workflow barrel — components + re-exports of the route descriptor
 * table.
 *
 * Source of truth for the workflow-as-route architecture introduced in
 *
 * Every workflow is a route. Each route file owns its own config,
 * result layout, viewers, and report, per Layer 3 of the plan. The
 * shared engine (Layer 2) and primitives (Layer 1) are consumed by
 * each workflow; they are not re-implemented per workflow.
 *
 * Adding a new workflow = add a route file + append a descriptor to
 * `./routes.ts`.
 */

export { BrowseWorkflow } from './browse/BrowseWorkflow';
export { RequirementsWorkflow } from './requirements/RequirementsWorkflow';
export { RunWorkflow } from './run/RunWorkflow';
export { VerifyWorkflow } from './verify/VerifyWorkflow';
export { AnalyzeWorkflow, AnalyzeIndexRedirect } from './analyze/AnalyzeWorkflow';
export { SweepWorkflow } from './analyze/SweepWorkflow';
export { MonteCarloWorkflow } from './analyze/MonteCarloWorkflow';
export { TradeStudyWorkflow } from './analyze/TradeStudyWorkflow';
export { SensitivityWorkflow } from './analyze/SensitivityWorkflow';
export { CompareWorkflow } from './compare/CompareWorkflow';

export {
  WORKFLOWS,
  workflowIdForPath,
  pathForWorkflowId,
} from './routes';
export type { WorkflowDescriptor } from './routes';
