/**
 * Requirements workbench wire types — mirrors the B2 backend shapes
 * (`sysml_query::RequirementRow` et al.; the verbatim row JSON is
 * recorded in ninebar-implementation-plan.md §1.5).
 *
 * The one rule that matters (design doc §5 correction): the three-state
 * verification classification ships ON the row (`row.verification`).
 * The UI reads that field — it never joins `sysml.aggregate`, which is
 * owner-keyed two-bucket counts and cannot distinguish these states.
 */

/** Three-state rollup: a recorded fail beats everything; anything short
 *  of "every linked case evaluated Pass" (including zero linked cases)
 *  is `incomplete`. */
export type RequirementVerificationState = 'fail' | 'incomplete' | 'pass';

/** A linked element on a row (satisfier, verification case, derivation
 *  endpoint, refinement target). Identity is the ElementId; `name` is
 *  display-only. */
export interface RequirementLinkRef {
  id: string;
  name: string | null;
  kind: string;
}

export interface RequirementVerificationRollup {
  state: RequirementVerificationState;
  /** Number of verification cases linked via Verify edges. */
  cases_total: number;
  /** How many of those cases passed. */
  cases_passed: number;
  /**
   * HOW the case verdicts were computed — 'static' (against current/
   * default values) or 'trajectory' (against a live run). BINDING label
   * (§2.1a ruling (d)): rendered always, never only on disagreement — a
   * static verdict on an ODE-backed case answers a different question.
   * Missing/empty only from a pre-ruling backend.
   */
  evaluation_mode?: string;
}

export interface RequirementSourceSpan {
  file: string;
  start: number;
  end: number;
  line?: number | null;
  col?: number | null;
}

export interface RequirementRow {
  id: string;
  kind: string; // 'RequirementDefinition' | 'RequirementUsage'
  /** The requirement ID — the declared short name `<'REQ-001'>` (spec §7.21.2). */
  req_id: string | null;
  name: string | null;
  /** All owned doc bodies joined in source order with a blank line. */
  text: string | null;
  qualified_name: string | null;
  /** Nearest Package ancestor, when one exists. */
  owning_package: RequirementLinkRef | null;
  source_span: RequirementSourceSpan | null;
  /** Requirement-kind ANCESTORS only — not raw containment depth. */
  outline_depth: number;
  /** @StatusInfo status as a bare enum literal ("tbd", "done", …). */
  maturity: string | null;
  satisfied_by: RequirementLinkRef[];
  verified_by: RequirementLinkRef[];
  verification: RequirementVerificationRollup;
  /**
   * DECLARED verification methods (B4): union of `@VerificationMethod`
   * kinds across this row's verifying cases (spec enum literals —
   * 'inspect' | 'analyze' | 'demo' | 'test' — though unknown declared
   * values pass through). Model INTENT — distinct from
   * `verification.evaluation_mode` (what the tool computed); never merge
   * the two chips. Empty when no verifying case declares a method.
   */
  verification_methods: string[];
  /** Requirements this row derives FROM (the originals). */
  derived_from: RequirementLinkRef[];
  /** Requirements derived from this row. */
  derives: RequirementLinkRef[];
  /** Requirements this row refines. */
  refines: RequirementLinkRef[];
}

/** Paged response envelope for `sysml.workspace.requirement_rows`. */
export interface RequirementRowsResult {
  rows: RequirementRow[];
  total_estimate: number | null;
  cursor: string | null;
  cursor_invalidated: boolean;
  revision: number;
}

// ── Requirement detail (B2.1 / R18) — mirrors sysml_query::RequirementDetail ──

/** One assume/require constraint on a requirement — a VERDICT INPUT. */
export interface RequirementConstraint {
  id: string;
  name: string | null;
  /** Inline body — pretty-printed from the expression AST since the v2
   *  unification (verbatim-source fallback only for pre-AST graphs).
   *  Null for the pure reference form. */
  text: string | null;
  /** Reference-form target (`require constraint : SomeDef;`) when the
   *  name resolves unambiguously; null otherwise — never a guessed link. */
  referenced_definition: RequirementLinkRef | null;
  /** The chain ancestor this constraint was inherited from; absent/null
   *  for OWNED constraints. Rendering the provenance is BINDING (steward
   *  ruling 2026-07-16, upheld by §2.1a) — an unlabeled inherited row
   *  misleads about where to edit it. */
  inherited_from?: RequirementLinkRef | null;
  /** HOW the ancestor was reached — 'typing' (usage : Def) or
   *  'specialization' (def A :> B). BINDING with the full-chain closure
   *  (§2.1a): a two-hop row must never be misreported as one hop. */
  inherited_via?: string | null;
}

/** An attribute owned by the requirement — the values its constraints read. */
export interface RequirementAttribute {
  id: string;
  name: string | null;
  /** Declared/default value as display text (`40 [ms]`). */
  value: string | null;
  /** Live session value when one is running (not wired in v1.6). */
  live_value: string | null;
}

/**
 * The evaluated contract + narrative context of one requirement.
 * Bucket separation is BINDING (design doc §2.1): subject, constraints,
 * and referenced_attributes are verdict inputs (render next to the
 * verified chips); framed_concerns/actors/stakeholders are narrative —
 * never place them beside the verdict.
 */
export interface RequirementDetail {
  id: string;
  subject: RequirementLinkRef | null;
  /** OWNED constraints only (mirrors the spec's owned-only
   *  assumedConstraint/requiredConstraint derived properties). */
  assumed_constraints: RequirementConstraint[];
  required_constraints: RequirementConstraint[];
  /** Constraints the verdict evaluates from the FULL inheritance chain
   *  (§2.1a: typing + def specialization, transitive, redefinition-
   *  suppressed; populated UNCONDITIONALLY — inherited rows show even
   *  when the requirement owns constraints, because the evaluator
   *  aggregates both). Rows carry `inherited_from` + `inherited_via`. */
  inherited_assumed_constraints: RequirementConstraint[];
  inherited_required_constraints: RequirementConstraint[];
  /** CONTENT usages typed by this element (reverse of the "· from" edge;
   *  check occurrences ride verified_by instead — one fact, one home). */
  instantiated_by: RequirementLinkRef[];
  framed_concerns: RequirementLinkRef[];
  actors: RequirementLinkRef[];
  stakeholders: RequirementLinkRef[];
  referenced_attributes: RequirementAttribute[];
  rationale: string | null;
  /** DECLARED verification methods — same union-over-verifying-cases read
   *  as `RequirementRow.verification_methods` (one backend home), so the
   *  rail and the grid column can never disagree. */
  verification_methods: string[];
}
