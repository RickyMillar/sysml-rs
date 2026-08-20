/**
 * verdictMapping — normalise raw backend verification payloads into the
 * universal `Verdict` shape (E2 contract) used by every workflow.
 *
 * Three backend shapes feed this module:
 *
 *  1. `sysml.verify` (per-case) returns `VerifyResult { verdict: "Pass" | ...,
 *     requirements: [{ requirement_id, verdict, message }], diagnostics }`.
 *     We emit one `Verdict` per case (the case-level outcome), carrying the
 *     per-requirement detail in `metadata.requirements`.
 *
 *  2. `sysml.evaluate.verification_cases` returns a JSON array of case
 *     objects `{ element_id, case_name, verdict, total_requirements,
 *     passed_requirements, display }`. One `Verdict` per case.
 *
 *  3. `sysml.evaluate.constraints` returns `{ element_id, satisfied, detail,
 *     ast, verdict: Verdict-shaped }`. The backend already emits a
 *     `verdict` key that matches our shape — we normalise the verdict
 *     string, passthrough the payload, and attach the element_id into
 *     metadata.
 *
 * Round 3 scope (per the R3.2 brief):
 *   - `verdict = Pass | Fail | Inconclusive | Error` normalised from any
 *     case-sensitivity of the backend enum serialisation.
 *   - `actual` / `expected` — pulled from the result where available.
 *   - `evidence = None` (Agent R fills this in on a parallel branch).
 *   - `metadata` carries the raw reason, display string, element id,
 *     requirement breakdown, and anything else the backend surfaced.
 *
 * Edge cases handled explicitly:
 *   - Missing `verdict` field → `inconclusive` (never silently drops).
 *   - Unknown verdict enum value → `inconclusive`.
 *   - Missing `reason` / `detail` / `message` → metadata key omitted.
 *   - Neither actual nor expected → both `null`.
 *   - Both actual and expected present → both preserved; margin not
 *     computed here (Round 2 backend work owns margin extraction).
 */

import type { Verdict, VerdictKind } from '@/engine/types';

// ── Verdict kind normalisation ───────────────────────────────────────

/**
 * Coerce a possibly-string / possibly-missing verdict value into the
 * canonical `VerdictKind`. Accepts the backend's Rust-debug form
 * ("Pass") and serde-serialised form ("pass") interchangeably.
 *
 * Exported for tests and for callers that need the fallback behaviour.
 */
export function normalizeVerdictKind(raw: unknown): VerdictKind {
  if (raw == null) return 'inconclusive';
  const lower = String(raw).toLowerCase();
  if (lower === 'pass' || lower === 'fail' || lower === 'error') return lower;
  if (lower === 'inconclusive') return 'inconclusive';
  return 'inconclusive';
}

/** Construct a bare `Verdict` with every optional field zeroed. */
export function emptyVerdict(kind: VerdictKind): Verdict {
  return {
    verdict: kind,
    actual: null,
    expected: null,
    margin: null,
    error: null,
    sensitivity: null,
    evidence: null,
    metadata: {},
  };
}

// ── Shape 1 — sysml.evaluate.constraints ─────────────────────────────

/**
 * Shape of a single entry in the `sysml.evaluate.constraints` response.
 * Deliberately permissive — every field is optional because the backend
 * may evolve.
 */
export interface RawConstraintResult {
  element_id?: string;
  satisfied?: boolean;
  detail?: string;
  ast?: unknown;
  /** The backend already emits a structured verdict here (R1.3). */
  verdict?: {
    verdict?: unknown;
    actual?: unknown;
    expected?: unknown;
    margin?: number | null;
    error?: string | null;
    sensitivity?: Record<string, number> | null;
    evidence?: unknown;
    metadata?: Record<string, unknown>;
  } | null;
}

/**
 * Map a single `constraint_result` entry into a Verdict.
 *
 * Priority when deciding the verdict kind:
 *   1. `result.verdict.verdict` if the backend nested verdict is present.
 *   2. `result.satisfied` → Pass if true, Fail if false.
 *   3. `inconclusive` otherwise.
 */
export function mapConstraintResult(result: RawConstraintResult): Verdict {
  const nested = result.verdict ?? null;

  // Kind
  let kind: VerdictKind;
  if (nested && nested.verdict !== undefined && nested.verdict !== null) {
    kind = normalizeVerdictKind(nested.verdict);
  } else if (typeof result.satisfied === 'boolean') {
    kind = result.satisfied ? 'pass' : 'fail';
  } else {
    kind = 'inconclusive';
  }

  const out: Verdict = emptyVerdict(kind);

  // actual / expected
  if (nested?.actual !== undefined) out.actual = nested.actual;
  if (nested?.expected !== undefined) out.expected = nested.expected;

  // margin
  if (typeof nested?.margin === 'number') out.margin = nested.margin;

  // error
  if (typeof nested?.error === 'string' && nested.error.length > 0) {
    out.error = nested.error;
  }

  // sensitivity
  if (nested?.sensitivity && typeof nested.sensitivity === 'object') {
    out.sensitivity = nested.sensitivity as Record<string, number>;
  }

  // metadata: carry element_id + detail + any nested metadata.
  const meta: Record<string, unknown> = {};
  if (result.element_id) meta.element_id = result.element_id;
  if (typeof result.detail === 'string' && result.detail.length > 0) {
    meta.reason = result.detail;
  }
  if (nested?.metadata && typeof nested.metadata === 'object') {
    Object.assign(meta, nested.metadata);
  }
  meta.source = 'constraint';
  out.metadata = meta;

  return out;
}

// ── Shape 2 — sysml.evaluate.verification_cases ──────────────────────

/** One entry of the evaluate.verification_cases response. */
export interface RawVerificationCaseRequirement {
  requirement_id?: string;
  requirement_name?: string;
  requirement_element_id?: string;
  element_id?: string;
  requirement_text?: string | null;
  verdict?: unknown;
  actual?: unknown;
  expected?: unknown;
  margin?: number | null;
  error?: string | null;
  message?: string;
  constraints?: unknown[];
  subrequirements?: RawVerificationCaseRequirement[];
}

export interface RawVerificationCaseResult {
  element_id?: string;
  case_id?: string;
  case_name?: string;
  subject?: string | null;
  /** DECLARED @VerificationMethod kinds ([1..*] by spec — plural). Model
   *  intent, distinct from how the verdict was computed. Replaces the
   *  singular `method`, which the backend never populated (always null). */
  methods?: string[];
  /** How the verdict was COMPUTED (B10 layer 2, §2.1a(d)). This read is
   *  always `"static"` — recomputed against the current graph, never a
   *  trajectory or external verdict — but carried explicitly so the mode
   *  badge has a source and the label stays honest per the ruling. */
  evaluation_mode?: string;
  verdict?: unknown;
  total_requirements?: number;
  passed_requirements?: number;
  display?: string;
  requirements?: RawVerificationCaseRequirement[];
  diagnostics?: string[];
}

/**
 * Map the `evaluate.verification_cases` response entry to a Verdict.
 *
 * Because this command returns aggregate data only (no per-requirement
 * actual/expected values), `actual` / `expected` are left null. The
 * pass/total counts and display string land in `metadata`.
 */
export function mapEvaluateVerificationCaseResult(
  result: RawVerificationCaseResult,
): Verdict {
  const kind = normalizeVerdictKind(result.verdict);
  const out = emptyVerdict(kind);

  const meta: Record<string, unknown> = { source: 'verification-case' };
  if (result.element_id) meta.element_id = result.element_id;
  if (result.case_id) meta.case_id = result.case_id;
  if (result.case_name) meta.case_name = result.case_name;
  if (result.subject) meta.subject = result.subject;
  if (Array.isArray(result.methods) && result.methods.length > 0) {
    meta.methods = result.methods;
  }
  if (typeof result.evaluation_mode === 'string' && result.evaluation_mode.length > 0) {
    meta.evaluation_mode = result.evaluation_mode;
  }
  if (typeof result.total_requirements === 'number') {
    meta.total_requirements = result.total_requirements;
  }
  if (typeof result.passed_requirements === 'number') {
    meta.passed_requirements = result.passed_requirements;
  }
  if (result.display) meta.display = result.display;
  if (Array.isArray(result.requirements) && result.requirements.length > 0) {
    meta.requirements = result.requirements.map((requirement) => ({
      requirement_id: requirement.requirement_id ?? '',
      requirement_name: requirement.requirement_name,
      requirement_element_id: requirement.requirement_element_id,
      element_id: requirement.element_id,
      requirement_text: requirement.requirement_text,
      verdict: normalizeVerdictKind(requirement.verdict),
      actual: requirement.actual,
      expected: requirement.expected,
      margin: requirement.margin,
      error: requirement.error ?? null,
      message: requirement.message ?? '',
      constraints: requirement.constraints,
      subrequirements: requirement.subrequirements,
    }));
    const failed = result.requirements
      .filter((requirement) => normalizeVerdictKind(requirement.verdict) !== 'pass')
      .map((requirement) => requirement.message)
      .filter((message): message is string => !!message);
    if (failed.length > 0) meta.reason = failed.join('; ');
  }
  if (Array.isArray(result.diagnostics) && result.diagnostics.length > 0) {
    meta.diagnostics = result.diagnostics;
    if (meta.reason === undefined) meta.reason = result.diagnostics.join('; ');
  }
  out.metadata = meta;

  return out;
}

// ── Shape 3 — sysml.verify (per-case) ────────────────────────────────

/** One requirement inside a `sysml.verify` response. */
export interface RawRequirementResult {
  requirement_id?: string;
  requirement_element_id?: string;
  element_id?: string;
  requirement_text?: string | null;
  verdict?: unknown;
  actual?: unknown;
  expected?: unknown;
  margin?: number | null;
  error?: string | null;
  constraints?: unknown[];
  message?: string;
}

/** Shape of the `sysml.verify` response. */
export interface RawVerifyResult {
  verdict?: unknown;
  /** How the verdict was COMPUTED (B10 layer 2, §2.1a(d)): `"static"` for
   *  `sysml.verify` (against current/default values), `"trajectory"` for
   *  `sysml.sessions.verify` (against a live run). Carried so the mode
   *  badge distinguishes a desk check from a simulation run. */
  evaluation_mode?: string;
  requirements?: RawRequirementResult[];
  diagnostics?: string[];
}

/**
 * Map a `sysml.verify` response to a single case-level Verdict.
 *
 * Unlike constraint evaluation, `sysml.verify` gives us per-requirement
 * detail. We roll it up into the case-level verdict and stash the
 * per-requirement breakdown in `metadata.requirements`.
 */
export function mapVerifyResult(
  caseId: string,
  result: RawVerifyResult,
): Verdict {
  const kind = normalizeVerdictKind(result.verdict);
  const out = emptyVerdict(kind);

  const meta: Record<string, unknown> = {
    source: 'verify',
    case_id: caseId,
  };
  if (typeof result.evaluation_mode === 'string' && result.evaluation_mode.length > 0) {
    meta.evaluation_mode = result.evaluation_mode;
  }

  if (Array.isArray(result.requirements) && result.requirements.length > 0) {
    meta.requirements = result.requirements.map((r) => ({
      requirement_id: r.requirement_id ?? '',
      requirement_element_id: r.requirement_element_id,
      element_id: r.element_id,
      requirement_text: r.requirement_text,
      verdict: normalizeVerdictKind(r.verdict),
      actual: r.actual,
      expected: r.expected,
      margin: r.margin,
      error: r.error ?? null,
      constraints: r.constraints,
      message: r.message ?? '',
    }));
    // Concatenate non-pass requirement messages into a `reason` hint
    // for the badge tooltip. Tested via verdictMapping.test.ts.
    const failReasons = result.requirements
      .filter((r) => normalizeVerdictKind(r.verdict) !== 'pass')
      .map((r) => r.message)
      .filter((m): m is string => !!m);
    if (failReasons.length > 0) meta.reason = failReasons.join('; ');
  }

  if (Array.isArray(result.diagnostics) && result.diagnostics.length > 0) {
    meta.diagnostics = result.diagnostics;
    if (meta.reason === undefined) meta.reason = result.diagnostics.join('; ');
  }

  out.metadata = meta;
  return out;
}

// ── Requirement row expansion ───────────────────────────────────────

/**
 * Promote nested requirement checks to first-class Verdict rows for the
 * PassFailGrid. If a backend result has no requirement breakdown, keep the
 * aggregate verdict unchanged.
 */
export function expandRequirementVerdicts(verdict: Verdict): Verdict[] {
  const requirements = verdict.metadata?.requirements;
  if (!Array.isArray(requirements) || requirements.length === 0) return [verdict];

  return requirements
    .filter((requirement): requirement is Record<string, unknown> => !!requirement && typeof requirement === 'object')
    .map((requirement, index) => {
      const requirementId = stringOrUndefined(requirement.requirement_id) ?? `requirement-${index + 1}`;
      const requirementElementId = stringOrUndefined(requirement.requirement_element_id) ?? stringOrUndefined(requirement.element_id);
      const firstConstraint = firstObject(requirement.constraints);
      const constraintId = firstConstraint ? stringOrUndefined(firstConstraint.constraint_id) ?? stringOrUndefined(firstConstraint.element_id) ?? stringOrUndefined(firstConstraint.expression_id) : undefined;
      const requirementName = stringOrUndefined(requirement.requirement_name);
      const message = stringOrUndefined(requirement.message);
      const row = emptyVerdict(normalizeVerdictKind(requirement.verdict));
      row.id = `${stringOrUndefined(verdict.metadata?.case_id) ?? stringOrUndefined(verdict.metadata?.case_name) ?? 'case'}:${requirementId}`;
      row.label = requirementName ?? requirementId;
      row.actual = requirement.actual ?? null;
      row.expected = requirement.expected ?? null;
      row.margin = typeof requirement.margin === 'number' ? requirement.margin : null;
      row.error = typeof requirement.error === 'string' && requirement.error.length > 0
        ? requirement.error
        : null;
      row.evidence = verdict.evidence ?? null;
      row.reason = message ?? verdict.reason ?? metadataString(verdict, 'reason');
      row.runtimeMs = verdict.runtimeMs ?? null;
      row.metadata = {
        ...verdict.metadata,
        source: `${stringOrUndefined(verdict.metadata?.source) ?? 'verify'}:requirement`,
        parent_source: verdict.metadata?.source,
        parent_verdict: verdict.verdict,
        parent_reason: verdict.reason ?? metadataString(verdict, 'reason'),
        element_id: constraintId ?? requirementElementId,
        requirement_id: requirementId,
        requirement_element_id: requirementElementId,
        constraint_id: constraintId,
        expression_id: constraintId,
        requirement_name: requirementName,
        requirement_text: requirement.requirement_text,
        message,
        actual: requirement.actual,
        expected: requirement.expected,
        margin: requirement.margin,
        error: row.error,
        constraints: requirement.constraints,
        subrequirements: requirement.subrequirements,
      };
      delete row.metadata.requirements;
      return row;
    });
}

function firstObject(value: unknown): Record<string, unknown> | undefined {
  return Array.isArray(value) ? value.find((item): item is Record<string, unknown> => !!item && typeof item === 'object') : undefined;
}

function stringOrUndefined(value: unknown): string | undefined {
  if (typeof value === 'string' && value.length > 0) return value;
  if (typeof value === 'number' || typeof value === 'boolean') return String(value);
  return undefined;
}

function metadataString(verdict: Verdict, key: string): string | null {
  const value = verdict.metadata?.[key];
  return typeof value === 'string' ? value : null;
}

// ── Summary helper ──────────────────────────────────────────────────

/** Count verdicts by kind for the run summary block. */
export function summarize(verdicts: Verdict[]): {
  pass: number;
  fail: number;
  inconclusive: number;
  error: number;
} {
  const out = { pass: 0, fail: 0, inconclusive: 0, error: 0 };
  for (const v of verdicts) out[v.verdict] += 1;
  return out;
}
