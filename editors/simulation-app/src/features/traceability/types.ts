/**
 * features/traceability/types — panel-local types for the trace matrix (R6.2).
 *
 * Wire mirrors (`TraceMatrixRow`, `TraceMatrix`, …) live in
 * `engine/types.ts`. Re-exported here so component files only import
 * `./types` and never touch the engine barrel directly.
 *
 * Local types cover the UI filter shape + the default ElementKind /
 * RelationshipKind tuple the hook uses when no caller override is
 * provided. The backend requires these three selectors, so the hook
 * picks sensible defaults (`PartUsage` / `Satisfy` / `RequirementUsage`)
 * and exposes them for the panel UI to tweak later.
 *
 * Direction note: Satisfy/Verify edges are minted source=satisfier/case
 * → target=requirement (spec §8.2.3.21 doc convention; B1a flip). The
 * requirement therefore sits on the TARGET endpoint — the viewer keys
 * its rows on `target`, not `source`.
 */

import type {
  TraceMatrixRow,
  TraceRow,
  TraceColumn,
  TraceLink,
  TraceMatrix,
} from '@/engine/types';

export type {
  TraceMatrixRow,
  TraceRow,
  TraceColumn,
  TraceLink,
  TraceMatrix,
};

/**
 * UI-side filter state held by the viewer. Every field is optional at
 * the wire layer but required in the UI so the component never has to
 * carry `undefined`s.
 */
export interface TraceFilter {
  /** Free-text needle — matches row label (case-insensitive substring). */
  search: string;
  /**
   * When `true`, hide rows whose links are all `pass` verdicts. The
   * "show only unsatisfied" toggle in the top bar drives this.
   */
  onlyUnsatisfied: boolean;
  /**
   * When `true`, keep only rows that have zero links at all — the
   * "no coverage" gap rows a requirements engineer wants to spot.
   */
  onlyNoCoverage: boolean;
}

/** Default filter used on panel mount — widest possible view. */
export const DEFAULT_TRACE_FILTER: TraceFilter = {
  search: '',
  onlyUnsatisfied: false,
  onlyNoCoverage: false,
};

/** Display density for the matrix body — affects row height + padding. */
export type TraceDensity = 'compact' | 'roomy';

/**
 * Canonical trio of selectors forwarded to `sysml.trace_matrix`. Kept
 * here (not in the hook) so tests and the panel can share one default
 * without triggering a round-trip per prop change.
 *
 * Defaults cover the 99% case: "which parts satisfy which requirements".
 * Callers that want Verify / Derive / Allocate pass their own triple.
 */
export interface TraceSelectors {
  /** ElementKind of the source (satisfier — columns). Defaults to `PartUsage`. */
  source_kind: string;
  /** ElementKind of the target (requirement — rows). Defaults to `RequirementUsage`. */
  target_kind: string;
  /** RelationshipKind connecting source→target. Defaults to `Satisfy`. */
  relation_kind: string;
}

export const DEFAULT_TRACE_SELECTORS: TraceSelectors = {
  // `source_kind` / `target_kind` are `ElementKind` (serde default →
  // PascalCase on the wire: `PartUsage`, `RequirementUsage`). Satisfy
  // edges run satisfier→requirement, so the requirement is the TARGET.
  source_kind: 'PartUsage',
  target_kind: 'RequirementUsage',
  // `relation_kind` is `RelationshipKind`, which is `#[serde(rename_all =
  // "camelCase")]` in `sysml-core/src/relationship.rs` — so the wire form is
  // `satisfy`, NOT `Satisfy`. Sending PascalCase makes `sysml.trace_matrix`
  // reject the request with `400 "unknown variant Satisfy"`.
  relation_kind: 'satisfy',
};
