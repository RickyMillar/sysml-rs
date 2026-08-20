/**
 * tradeHelpers — pure utilities for the R5.11 trade-study table viewer.
 *
 * Responsibilities:
 *   - `isPareto` / `computeParetoFront` — given alternatives + per-criterion
 *     objectives (min | max), decide which alternatives are non-dominated.
 *   - `computeWeightedScore` — linear weighted sum over criteria, with
 *     "Min" objectives inverted (negated) so that a higher weighted score
 *     always means "better" regardless of per-criterion polarity.
 *   - `rankAlternatives` — stable, tie-breaking-by-index ordering.
 *
 * Everything here is pure — no React, no side effects. The viewer
 * component (`TradeStudyTableViewer`) calls these helpers, as does Agent
 * GG's TradeStudy config / workflow layer.
 *
 * Shape contracts:
 *   - `ChildDescriptor` mirrors the wire shape that Agent AA's R5.0
 *     backend batch sessions emit (snake_case, e.g. `session_id`). The
 *     authoritative definition lives in `engine/types.ts` once R5.0
 *     lands upstream; we define a structurally-compatible local type
 *     here so this module typechecks on any base that does not yet
 *     carry the descriptor, *and* continues to typecheck once the
 *     canonical type appears (structural subtype).
 *   - `TradeCriterion` / `TradeObjective` / `AlternativeScore` are the
 *     scoring contract GG's config emits and the viewer consumes.
 *
 * NB on objectives:
 *   - "max" — higher raw value is better.
 *   - "min" — lower raw value is better (we negate when aggregating so
 *     higher *weighted score* is still better).
 *
 * Dominance:
 *   - Alternative A dominates B iff A is at least as good on every
 *     criterion AND strictly better on at least one.
 *   - "At least as good" and "strictly better" are polarised by the
 *     criterion's objective (min vs. max) — `normalise` below does that.
 */

// ── Wire-compatible ChildDescriptor (structural) ─────────────────────

/**
 * Minimal structural shape of one alternative row.
 *
 * Intentionally declared `type` (not `interface`) so downstream files
 * that import the canonical `ChildDescriptor` from `engine/types.ts`
 * (once R5.0 lands) can pass those values to our helpers without
 * nominal-type friction — TypeScript will structurally widen.
 *
 * `metrics` is the per-criterion raw-value dictionary the trade study
 * config evaluates against each alternative (one entry per
 * `TradeCriterion.key`). Keeping it separate from `params` (sweep
 * inputs) keeps the viewer decoupled from the generating workflow.
 */
export type ChildDescriptorLike = {
  /** Stable id of the alternative (the sweep-point / design id). */
  id: string;
  /** Backend session id once the run has materialised; null before kickoff. */
  session_id: string | null;
  /** Display label — falls back to `id` in the viewer when absent. */
  label?: string;
  /** Final status — pending rows are shown with the skeleton. */
  status?: 'pending' | 'running' | 'completed' | 'failed' | 'cancelled';
  /** Sweep parameter values (free-form; not read by the Pareto math). */
  params?: Record<string, number>;
  /**
   * Per-criterion raw metric values, keyed by `TradeCriterion.key`.
   * Missing / non-finite entries cause the alternative to be excluded
   * from the Pareto calc (treated as "data not yet available").
   */
  metrics?: Record<string, number>;
};

// ── Objective + criterion types ──────────────────────────────────────

/** Objective polarity for a single criterion. */
export type TradeObjective = 'min' | 'max';

/**
 * One scoring criterion.
 *
 * `weight` is the caller-supplied positive weight; `computeWeightedScore`
 * does NOT re-normalise weights (callers should feed
 * `normaliseWeights` → `criteria` if they want sum-to-1 behaviour).
 */
export interface TradeCriterion {
  /** Stable key into `ChildDescriptorLike.metrics`. */
  key: string;
  /** Display label (falls back to `key` in the viewer). */
  label?: string;
  /** Min (lower is better) vs Max (higher is better). */
  objective: TradeObjective;
  /** Non-negative weight. `0` → ignored. */
  weight: number;
  /** Optional unit label, e.g. "ms" / "$" / "kg". Viewer only. */
  unit?: string;
}

/**
 * Scored view of one alternative — what helpers consume.
 *
 * The viewer composes this from `(ChildDescriptorLike, TradeCriterion[])`
 * before rendering. Keeping it separate means the helpers stay
 * independent of the descriptor's full shape.
 */
export interface AlternativeScore {
  /** Stable alternative id (matches `ChildDescriptorLike.id`). */
  id: string;
  /**
   * Per-criterion raw values, in `criteria` order (matches
   * `TradeCriterion[]` passed to the helper). May contain `NaN` /
   * `Infinity` when a metric is missing — dominance excludes those rows.
   */
  values: number[];
}

// ── Helpers ─────────────────────────────────────────────────────────

/** Treat `NaN`, `Infinity`, and `-Infinity` as "missing data". */
function isFiniteNumber(v: number): boolean {
  return Number.isFinite(v);
}

/**
 * Normalise a single criterion value to "higher is better" space.
 * For `max`: identity. For `min`: negate.
 */
function normalise(value: number, objective: TradeObjective): number {
  return objective === 'max' ? value : -value;
}

/**
 * Strict-dominance check in normalised space.
 *
 * Returns true if `a` dominates `b`: a ≥ b on every criterion AND a > b
 * on at least one. Undefined / non-finite values on either side fall
 * through to "cannot determine" — that criterion is ignored for the
 * comparison, which matches the streaming-data contract (partial rows
 * cannot dominate, nor be dominated on, unmeasured criteria).
 */
function dominatesInNormSpace(a: number[], b: number[], objectives: TradeObjective[]): boolean {
  let strictlyBetter = false;
  for (let i = 0; i < objectives.length; i++) {
    const av = a[i];
    const bv = b[i];
    if (!isFiniteNumber(av) || !isFiniteNumber(bv)) {
      // Missing data on either side → skip this criterion.
      continue;
    }
    const an = normalise(av, objectives[i]);
    const bn = normalise(bv, objectives[i]);
    if (an < bn) return false; // a is worse on some criterion → cannot dominate
    if (an > bn) strictlyBetter = true;
  }
  return strictlyBetter;
}

/**
 * True iff no *other* alternative strictly dominates `alt`.
 *
 * A single-alternative input is trivially Pareto-optimal (the only
 * entry in the set). Missing-data rows still pass this gate — they
 * are merely undominated on any criterion with no paired value.
 */
export function isPareto(
  alt: AlternativeScore,
  others: AlternativeScore[],
  objectives: TradeObjective[],
): boolean {
  for (const other of others) {
    if (other === alt) continue;
    if (other.id === alt.id) continue;
    if (dominatesInNormSpace(other.values, alt.values, objectives)) {
      return false;
    }
  }
  return true;
}

/**
 * Indices of all Pareto-optimal alternatives in input order.
 *
 * O(n²) in the number of alternatives — fine for the table sizes we
 * serve (≤ a few hundred rows). If that ever stops being true, the
 * standard skyline sweep-and-filter can drop this to O(n log n).
 */
export function computeParetoFront(
  alts: AlternativeScore[],
  objectives: TradeObjective[],
): number[] {
  const out: number[] = [];
  for (let i = 0; i < alts.length; i++) {
    if (isPareto(alts[i], alts, objectives)) out.push(i);
  }
  return out;
}

/**
 * Linear weighted sum — higher is better, regardless of per-criterion
 * polarity. For `min` objectives, the raw value is negated before
 * multiplying by the weight, so:
 *
 *   cost = 100, weight = 0.5, objective = 'min' → contribution = -50
 *   throughput = 200, weight = 0.5, objective = 'max' → contribution = +100
 *
 * Non-finite metrics contribute `0` (the row is partial; callers can
 * detect this via `Number.isFinite` on individual values if they care
 * to show a "?" marker).
 */
export function computeWeightedScore(
  alt: AlternativeScore,
  criteria: TradeCriterion[],
  objectives: TradeObjective[],
): number {
  let score = 0;
  for (let i = 0; i < criteria.length; i++) {
    const v = alt.values[i];
    if (!isFiniteNumber(v)) continue;
    const w = criteria[i].weight;
    if (!Number.isFinite(w) || w === 0) continue;
    score += normalise(v, objectives[i]) * w;
  }
  return score;
}

/**
 * Ranked indices (best first) by weighted score, with a deterministic
 * tie-break on original index so equal scores produce a stable ordering
 * every render.
 *
 * Returns indices into `alts`, not ids — the viewer re-materialises to
 * rows. Length equals `alts.length`.
 */
export function rankAlternatives(
  alts: AlternativeScore[],
  criteria: TradeCriterion[],
  objectives: TradeObjective[],
): number[] {
  const decorated = alts.map((a, idx) => ({
    idx,
    score: computeWeightedScore(a, criteria, objectives),
  }));
  decorated.sort((a, b) => {
    if (b.score !== a.score) return b.score - a.score;
    return a.idx - b.idx; // stable tie-break
  });
  return decorated.map((d) => d.idx);
}

// ── Convenience: build AlternativeScore[] from descriptors ───────────

/**
 * Project a `ChildDescriptorLike[]` into `AlternativeScore[]` in the
 * criterion order the caller provides. Missing metrics become `NaN`,
 * which the helpers above treat as "data not yet available" (rows stay
 * undominated on those columns and contribute 0 to the weighted sum).
 *
 * Exported because GG's `<TradeStudyResultsPanel>` wrapper builds the
 * same projection before rendering; pulling it out keeps the two sites
 * from drifting on "what counts as a missing value".
 */
export function buildAlternativeScores(
  rows: ChildDescriptorLike[],
  criteria: TradeCriterion[],
): AlternativeScore[] {
  return rows.map((r) => ({
    id: r.id,
    values: criteria.map((c) => {
      const raw = r.metrics?.[c.key];
      return typeof raw === 'number' ? raw : Number.NaN;
    }),
  }));
}

/**
 * Sum of the `criteria` weights, guarded against non-finite inputs.
 * Consumers can use this to display a "sum of weights" hint or
 * normalise before calling `computeWeightedScore`.
 */
export function sumWeights(criteria: TradeCriterion[]): number {
  let s = 0;
  for (const c of criteria) {
    if (Number.isFinite(c.weight)) s += Math.max(0, c.weight);
  }
  return s;
}

/**
 * Convenience: extract objectives array in the same order as criteria.
 */
export function objectivesOf(criteria: TradeCriterion[]): TradeObjective[] {
  return criteria.map((c) => c.objective);
}
