/**
 * features/readiness/types — domain types for the model-readiness
 * aggregation (ninebar Phase 1.5, plan "Model readiness & Browse floor").
 *
 * `useModelReadiness()` fans out three existing backend surfaces —
 * `sysml.diagnostics` (via the existing `useDiagnostics`),
 * `sysml.dependency.status` (via the new `useDependencyStatus`,
 * `features/packages/queries.ts`), and `sysml.workspace.capabilities`
 * (already cached on `useWorkspaceStore.capabilities`) — into one
 * `ReadinessSummary` that the frame chip (`ReadinessChip`) and the Run
 * workflow's teaching line both consume.
 *
 * The `sysml.dependency.status` / `sysml.workspace.verify` wire shapes
 * live in `features/packages/queries.ts` (next to the hooks that fetch
 * them, matching that file's existing convention for `WorkspaceLoadRaw`
 * etc.) and are re-exported here for readiness call sites.
 */

import type { DiagnosticSeverity } from '@/engine/types';
import type { Capabilities } from '@/store/workspace';
import type {
  DependencyStatusWire,
  WorkspaceVerifyResult,
} from '@/features/packages/queries';

export type { Capabilities, DependencyStatusWire, WorkspaceVerifyResult };

export type ReadinessLevel = 'ready' | 'warnings' | 'errors' | 'unknown';

/**
 * One row in the readiness drill list — a diagnostic or a resolved
 * dependency failure, normalised to a single shape so the chip's
 * popover can render one list.
 *
 * `elementId` is `undefined` for every row today: diagnostics don't
 * carry a resolved element id on the wire (see
 * `features/diagnostics/DiagnosticsPanel.tsx`'s `defaultExtractor`,
 * which returns `null` for the identical reason — span-to-element
 * resolution isn't implemented), and dependency failures are
 * root/manifest-level, not element-level. Kept optional (not omitted)
 * so a future extractor can populate it without a shape change.
 */
export interface ReadinessDrillEntry {
  /** File URI (or workspace-root path, for dependency failures) the problem was reported against. */
  file: string;
  severity: DiagnosticSeverity;
  message: string;
  elementId?: string | null;
}

export interface ReadinessCounts {
  errors: number;
  warnings: number;
}

export interface ReadinessSummary {
  level: ReadinessLevel;
  /** Diagnostic-only counts (error/warning severities from `sysml.diagnostics`). */
  counts: ReadinessCounts;
  /** Dependency names that failed to resolve, flattened across every workspace root. */
  unresolvedDeps: string[];
  /**
   * Capability flags `sysml.workspace.capabilities` reports as absent
   * for the loaded model (e.g. `'constraints'`, `'stateMachines'`).
   *
   * Informational only — it does NOT affect `level`. A model simply not
   * containing state machines isn't a validity problem, so capabilities
   * can't feed a pass/fail judgement the way diagnostics and dependency
   * resolution do. This is the data the plan's other per-tool teaching
   * empty-states are meant to read from ("this model has constraints ->
   * Verify", "sweepable parameters -> Analyze" — audit F12, plan
   * "Round 2+" task list).
   */
  missingCapabilities: string[];
  drill: ReadinessDrillEntry[];
}
