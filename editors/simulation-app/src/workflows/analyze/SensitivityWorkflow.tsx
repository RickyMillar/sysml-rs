/**
 * SensitivityWorkflow — route /analyze/sensitivity (R7.4).
 *
 * The real implementation lives at `./sensitivity/SensitivityWorkflow.tsx`;
 * this file stays as the module the workflow barrel
 * (`workflows/index.ts`) imports from, matching the Monte Carlo /
 * Trade Study layout convention.
 */

export { SensitivityWorkflow } from './sensitivity/SensitivityWorkflow';
