/**
 * Workflow sidecar wire types — TS mirrors of
 * `sysml_store::workflow_store` (+ the service's folded-state wrapper).
 *
 * Events are internally tagged on `kind` and FLATTENED into the event
 * envelope (serde `#[serde(flatten)]`): one JSON object carries both
 * the envelope fields and the variant fields.
 *
 * Rust tuples serialize as arrays: `approval: [state, by, at_ms]`,
 * `sign_offs: [statement, by, at_ms][]`.
 */

/** Closed approval vocabulary (steward ruling 2026-07-16) — mirrors
 *  `sysml_service::workflow::APPROVAL_STATES`; the backend rejects
 *  anything else. "draft" is every element's initial state (no event
 *  yet IS draft, never a sentinel). */
export const APPROVAL_STATES = ['draft', 'in_review', 'approved', 'rejected'] as const;
export const APPROVAL_INITIAL = 'draft';

interface WorkflowEventEnvelope {
  /** Store-assigned, monotonic per project; doubles as the event id. */
  seq: number;
  schema_version: number;
  project: string;
  element_id: string;
  /** Explicit identity — never an OS/`"local"` default. */
  actor: string;
  timestamp_ms: number;
}

export type WorkflowEventWire = WorkflowEventEnvelope &
  (
    | { kind: 'approval_state_changed'; from: string; to: string }
    | { kind: 'comment'; body: string }
    | { kind: 'sign_off_attestation'; statement: string }
    | {
        kind: 'suspect_clearing_attestation';
        baseline_name: string;
        baseline_commit: string;
        attested_commit: string;
        rationale: string;
      }
    | { kind: 'engineer_assigned'; assignee: string }
    | { kind: 'relinked'; from: string; to: string; rationale: string }
  );

export interface ClearingRecordWire {
  seq: number;
  baseline_name: string;
  baseline_commit: string;
  attested_commit: string;
  actor: string;
  timestamp_ms: number;
  rationale: string;
  /** True when the requirement changed again after this attestation. */
  superseded: boolean;
}

/** One manual-verification attestation (B10) — a signed human act, NEVER
 *  a verdict. Mirrors `sysml_store::workflow_store::VerificationAttestationRecord`.
 *  Renders only in history/attestation surfaces, actor-attributed. */
export interface VerificationAttestationWire {
  seq: number;
  /** Layer-1 method vocabulary (`inspect | analyze | demo | test`). */
  method: string;
  statement: string;
  attested_commit: string;
  actor: string;
  timestamp_ms: number;
  /** True when the element changed again after this attestation
   *  (display-time computed). */
  superseded: boolean;
}

/** `sysml.workflow.state` — folded, display-time-computed. */
export interface ElementWorkflowStateWire {
  approval: [state: string, by: string, atMs: number] | null;
  assignee: string | null;
  sign_offs: Array<[statement: string, by: string, atMs: number]>;
  suspect_clearings: ClearingRecordWire[];
  /** Manual-verification attestations, oldest-first (B10). */
  verification_attestations: VerificationAttestationWire[];
  comment_count: number;
  /** The element id no longer exists in the current graph (ADR-009:
   *  history belongs to a prior identity — never auto re-attached). */
  orphaned: boolean;
}
