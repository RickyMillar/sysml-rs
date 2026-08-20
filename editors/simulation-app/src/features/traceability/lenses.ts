/**
 * Trace lenses — the named (source · relation · target) triples the matrix
 * can be read through.
 *
 * The matrix has always taken a `TraceSelectors` triple, but only one was ever
 * reachable: the default `PartUsage · satisfy · RequirementUsage`. On
 * `espresso-production-cell` that lens has ZERO edges while
 * `VerificationCaseDefinition · verify · RequirementDefinition` has eight — so
 * the Browse trace view showed an empty grid for a workspace whose
 * traceability is fully modelled, with nothing on screen saying which
 * question had just been asked.
 *
 * The default is NOT changed. "Which parts satisfy which requirements" is the
 * right first question for most models, and rewriting it to suit one fixture
 * would trade this bug for its mirror image. What changes is that the lens is
 * visible, switchable, and — when it comes back empty — able to say which
 * other lens would not.
 */

import type { TraceSelectors } from './types';

export interface TraceLens {
  /** Stable key for selection state and testids. */
  id: string;
  /** Short label for the picker. */
  label: string;
  /** One line naming the question this lens answers, in the user's terms. */
  question: string;
  selectors: TraceSelectors;
}

/**
 * The lenses offered, in the order they are shown.
 *
 * `relation_kind` is camelCase on the wire (`RelationshipKind` is
 * `#[serde(rename_all = "camelCase")]`); the kinds are PascalCase
 * (`ElementKind` uses serde's default). Sending the wrong case makes
 * `sysml.trace_matrix` reject the request with a 400, so these strings are
 * load-bearing — see `DEFAULT_TRACE_SELECTORS` for the full note.
 *
 * Definition-level and usage-level are separate entries rather than one lens
 * that tries both: a model authored at one level and read at the other should
 * show the reader WHICH level had the edges, not silently merge them.
 */
export const TRACE_LENSES: TraceLens[] = [
  {
    id: 'satisfy-usage',
    label: 'Parts satisfy requirements',
    question: 'which part usages satisfy which requirement usages',
    selectors: {
      source_kind: 'PartUsage',
      target_kind: 'RequirementUsage',
      relation_kind: 'satisfy',
    },
  },
  {
    id: 'satisfy-def',
    label: 'Parts satisfy requirement defs',
    question: 'which part usages satisfy which requirement definitions',
    selectors: {
      source_kind: 'PartUsage',
      target_kind: 'RequirementDefinition',
      relation_kind: 'satisfy',
    },
  },
  {
    id: 'verify-def',
    label: 'Cases verify requirement defs',
    question: 'which verification case definitions verify which requirement definitions',
    selectors: {
      source_kind: 'VerificationCaseDefinition',
      target_kind: 'RequirementDefinition',
      relation_kind: 'verify',
    },
  },
  {
    id: 'verify-usage',
    label: 'Cases verify requirements',
    question: 'which verification case usages verify which requirement usages',
    selectors: {
      source_kind: 'VerificationCaseUsage',
      target_kind: 'RequirementUsage',
      relation_kind: 'verify',
    },
  },
  {
    id: 'derive',
    label: 'Requirements derive requirements',
    question: 'which requirements are derived from which',
    selectors: {
      source_kind: 'RequirementUsage',
      target_kind: 'RequirementUsage',
      relation_kind: 'derive',
    },
  },
  {
    id: 'refine',
    label: 'Requirements refine requirements',
    question: 'which requirements refine which',
    selectors: {
      source_kind: 'RequirementUsage',
      target_kind: 'RequirementUsage',
      relation_kind: 'refine',
    },
  },
  {
    id: 'allocate',
    label: 'Parts allocate to parts',
    question: 'which part usages are allocated to which',
    selectors: {
      source_kind: 'PartUsage',
      target_kind: 'PartUsage',
      relation_kind: 'allocate',
    },
  },
];

/**
 * The lens the matrix opens on — unchanged from the historical default, so no
 * existing caller's first read moves.
 */
export const DEFAULT_TRACE_LENS_ID = 'satisfy-usage';

export function lensById(id: string): TraceLens {
  return TRACE_LENSES.find((l) => l.id === id) ?? TRACE_LENSES[0];
}

/** Does this triple match a named lens? Used to label a caller-supplied one. */
export function lensForSelectors(selectors: TraceSelectors): TraceLens | null {
  return (
    TRACE_LENSES.find(
      (l) =>
        l.selectors.source_kind === selectors.source_kind &&
        l.selectors.target_kind === selectors.target_kind &&
        l.selectors.relation_kind === selectors.relation_kind,
    ) ?? null
  );
}

/** Human-readable form of an arbitrary triple, for a lens with no name. */
export function describeSelectors(selectors: TraceSelectors): string {
  return `${selectors.source_kind} · ${selectors.relation_kind} · ${selectors.target_kind}`;
}
