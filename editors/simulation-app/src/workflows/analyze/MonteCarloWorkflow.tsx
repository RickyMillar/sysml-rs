/**
 * MonteCarloWorkflow — route /analyze/montecarlo (R5.6).
 *
 * The real implementation lives at `./montecarlo/MonteCarloWorkflow.tsx`;
 * this file stays as the module the workflow barrel (`workflows/index.ts`)
 * imports from, so parallel agents and the existing route wiring do not
 * need to move.
 *
 * R5.7 (FF/FF2) fills in the viewers / CSV export behind the results
 * shell — the shell's `{ batchId, children }` prop shape is stable and
 * part of the R5.6 contract.
 */

export { MonteCarloWorkflow } from './montecarlo/MonteCarloWorkflow';
