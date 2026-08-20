/**
 * Pure helpers for the Monte Carlo pass-rate dashboard (R5.8).
 *
 * A `ChildDescriptor` is the per-iteration record produced by the Monte
 * Carlo batch runner. The shape here is the minimum needed to compute
 * pass-rates and CSV exports — EE's shell defines the richer runtime
 * contract; keeping these helpers agnostic lets them compose with any
 * descriptor shape that includes the fields below.
 *
 * `verdicts` is optional + flat. When present, helpers identify a
 * constraint by its stable `id` (fallback: `metadata.requirement_id`,
 * then synthetic `_verdict_${i}`). "Overall pass" means every listed
 * constraint id has a `pass` verdict in the child's verdict bag — AND
 * semantics, NOT OR.
 */

import type { Verdict, Value } from '../../../engine/types';

/** Status classification for a child iteration. */
export type ChildStatus =
  | 'pending'
  | 'running'
  | 'complete'
  | 'failed'
  | 'cancelled';

/**
 * Minimal per-iteration record. Agent EE's shell emits a richer type;
 * this interface captures just the load-bearing fields for R5.7/8/9.
 */
export interface ChildDescriptor {
  /** Deterministic iteration index (0-based). Ordering key across UI. */
  index: number;
  /** Backend session id (present once the iteration has started). */
  session_id?: string | null;
  /** Life-cycle classification — "complete" means the metrics are final. */
  status: ChildStatus;
  /** Input parameter overrides used to seed the iteration. */
  params?: Record<string, Value>;
  /** Terminal metric values harvested at session end. */
  metrics?: Record<string, Value>;
  /** Flat verdict list (same shape as VerifyWorkflow emits). */
  verdicts?: Verdict[];
}

/** Pass-rate breakdown for a single constraint across all children. */
export interface PassRateBreakdown {
  pass: number;
  fail: number;
  inconclusive: number;
  error: number;
  /** Total children that have any verdict for this constraint. */
  total: number;
  /** `pass / total`; `0` when `total === 0` so UI can render a flat bar. */
  passRate: number;
}

/** Roll-up across every tracked constraint. */
export interface OverallPassRate {
  /** Children where every tracked constraint was `pass`. */
  allPass: number;
  /** Children where at least one tracked constraint was `fail`. */
  anyFail: number;
  /** Children that have answered every tracked constraint. */
  evaluated: number;
  /** Total children considered (complete + terminal). */
  total: number;
  /** `allPass / total`; `0` when `total === 0`. */
  rate: number;
}

/**
 * Derive a stable id for a verdict. Falls back to `metadata.requirement_id`,
 * then to `metadata.case_name`, then the synthetic `_verdict_${i}` tag.
 * Matches the metadata conventions PassFailGridViewer set up in R3.3.
 */
function verdictId(v: Verdict, idx: number): string {
  if (v.id) return v.id;
  const meta = v.metadata;
  if (meta) {
    const req = meta['requirement_id'];
    if (typeof req === 'string' && req.length > 0) return req;
    const name = meta['case_name'];
    if (typeof name === 'string' && name.length > 0) return name;
  }
  return `_verdict_${idx}`;
}

/**
 * Status classes that count toward the "evaluated" denominator. `pending`
 * and `running` are intentionally excluded — they represent iterations
 * whose outcome is still unknown, and including them would make the
 * streaming dashboard drift toward 0% during the run.
 */
function isTerminal(status: ChildStatus): boolean {
  return status === 'complete' || status === 'failed' || status === 'cancelled';
}

/**
 * Look up a child's verdict for a specific constraint id. First match
 * wins when duplicates exist — mirrors PassFailGridViewer's cell-index
 * behaviour.
 */
function findVerdict(child: ChildDescriptor, constraintId: string): Verdict | undefined {
  if (!child.verdicts) return undefined;
  for (let i = 0; i < child.verdicts.length; i++) {
    const v = child.verdicts[i];
    if (verdictId(v, i) === constraintId) return v;
  }
  return undefined;
}

/**
 * Compute pass-rate for a single constraint across a child list.
 *
 * Empty / pre-run children yield `{pass: 0, fail: 0, ..., rate: 0}` so
 * the dashboard can render a flat bar during streaming mode.
 */
export function computePassRate(
  children: ChildDescriptor[],
  constraintId: string,
): PassRateBreakdown {
  const out: PassRateBreakdown = {
    pass: 0,
    fail: 0,
    inconclusive: 0,
    error: 0,
    total: 0,
    passRate: 0,
  };
  for (const c of children) {
    if (!isTerminal(c.status)) continue;
    const v = findVerdict(c, constraintId);
    if (!v) continue;
    out.total += 1;
    switch (v.verdict) {
      case 'pass':
        out.pass += 1;
        break;
      case 'fail':
        out.fail += 1;
        break;
      case 'inconclusive':
        out.inconclusive += 1;
        break;
      case 'error':
        out.error += 1;
        break;
    }
  }
  out.passRate = out.total === 0 ? 0 : out.pass / out.total;
  return out;
}

/**
 * Compute overall pass-rate across a set of tracked constraints. "Pass"
 * for a child means every tracked constraint's verdict is `pass` (AND
 * semantics). A `fail` on any tracked constraint classifies the child
 * as "anyFail". Children missing any tracked verdict are excluded from
 * `evaluated` but still counted in `total`.
 */
export function computeOverallPassRate(
  children: ChildDescriptor[],
  constraintIds: string[],
): OverallPassRate {
  const out: OverallPassRate = {
    allPass: 0,
    anyFail: 0,
    evaluated: 0,
    total: 0,
    rate: 0,
  };
  if (constraintIds.length === 0) {
    for (const c of children) if (isTerminal(c.status)) out.total += 1;
    return out;
  }
  for (const c of children) {
    if (!isTerminal(c.status)) continue;
    out.total += 1;
    let hasAll = true;
    let anyFail = false;
    for (const id of constraintIds) {
      const v = findVerdict(c, id);
      if (!v) {
        hasAll = false;
        continue;
      }
      if (v.verdict === 'fail') anyFail = true;
      if (v.verdict !== 'pass') hasAll = false;
    }
    // "Evaluated" = child has a verdict for every tracked constraint. A
    // child that failed to run and therefore has no verdicts is counted
    // toward `total` but not `evaluated`, matching the PassFailGridViewer
    // convention of not penalising absent cells.
    let answeredAll = true;
    for (const id of constraintIds) {
      if (!findVerdict(c, id)) {
        answeredAll = false;
        break;
      }
    }
    if (answeredAll) out.evaluated += 1;
    if (hasAll && answeredAll) out.allPass += 1;
    if (anyFail) out.anyFail += 1;
  }
  out.rate = out.total === 0 ? 0 : out.allPass / out.total;
  return out;
}

// Re-export for test convenience — kept alongside the helpers so tests
// can exercise the private id derivation without reaching into module
// internals.
export const __internals = { verdictId, isTerminal, findVerdict };
