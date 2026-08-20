/**
 * cartesianProduct — pure helper that materialises the full grid of
 * sweep points from a set of parameter ranges.
 *
 * Given `{ a: [1, 2], b: [10, 20] }`, returns
 * `[{ a: 1, b: 10 }, { a: 1, b: 20 }, { a: 2, b: 10 }, { a: 2, b: 20 }]`.
 *
 * Iteration order walks the *last* key's values innermost. This matches
 * the common expectation for tornado chart ordering (the first parameter
 * listed is the "outer" axis) and is stable regardless of insertion
 * order — consumers get the same sequence for the same input map.
 *
 * Edge cases:
 *   - Empty map → `[{}]` (one empty point, the vacuous product). This is
 *     the mathematical identity; callers that treat it as "no sweep"
 *     filter on `Object.keys(ranges).length === 0` themselves.
 *   - Any range with zero values collapses the whole product to `[]`.
 *     This is also standard; we explicitly return `[]` rather than
 *     `[{}]` so a UI that shows "no children" can distinguish.
 *
 * Exported from `./cartesianProduct` for testability and re-exported
 * from `./useSweepConfig` via `generateChildrenParams` (which wraps
 * this with NaN filtering and numeric coercion).
 */

/**
 * A mapping of parameter name → list of sample values. Values are
 * `unknown` because parameters can be numeric, string-enum, or boolean
 * — coercion is the caller's responsibility. The Sweep UI restricts to
 * numeric by construction (range editor only emits `number[]`), but the
 * helper stays general so Monte Carlo / Trade Study can reuse it.
 */
export type ParameterRanges = Record<string, readonly unknown[]>;

/** A single cartesian-product point — one value per parameter. */
export type SweepPoint = Record<string, unknown>;

/**
 * Compute the cartesian product of the per-parameter value lists.
 *
 * O(∏|Ai|) time / space — callers are responsible for bounding the
 * input. A nominal guard of 10_000 points is enforced by the UI; this
 * helper itself does not cap.
 */
export function cartesianProduct(ranges: ParameterRanges): SweepPoint[] {
  const keys = Object.keys(ranges);

  // Empty input → vacuous product. The mathematical identity makes
  // consumer logic simpler ("if no ranges, issue one default run").
  if (keys.length === 0) return [{}];

  // Any zero-length range collapses the product to empty — otherwise
  // we'd silently drop that parameter, which is surprising.
  for (const k of keys) {
    if ((ranges[k]?.length ?? 0) === 0) return [];
  }

  // Iterative accumulation — avoids recursion depth issues for
  // heavily-dimensioned sweeps (e.g. 6 parameters × 5 values each).
  let out: SweepPoint[] = [{}];
  for (const k of keys) {
    const values = ranges[k] ?? [];
    const next: SweepPoint[] = [];
    for (const point of out) {
      for (const v of values) {
        next.push({ ...point, [k]: v });
      }
    }
    out = next;
  }
  return out;
}

/**
 * A single range specification — either an inclusive numeric grid
 * `{min, max, step}` or an explicit list of values.
 *
 * Both variants produce a `number[]` via `expandRange`. The range
 * editor UI emits whichever is easier for the user; the hook normalises
 * everything to `number[]` before calling `cartesianProduct`.
 */
export type RangeSpec =
  | { kind: 'grid'; min: number; max: number; step: number }
  | { kind: 'list'; values: number[] };

/**
 * Materialise a `RangeSpec` into its explicit number list.
 *
 * For a `grid` spec, walks `min, min+step, min+2*step, …` up to (and
 * including) `max` using a tolerance equal to half the step to guard
 * against floating-point drift (`0.1 + 0.1 + 0.1 !== 0.3`). Returns an
 * empty list when the spec is ill-defined (non-finite values, zero /
 * negative step, min > max) — the hook surfaces that to the UI as a
 * "this range is empty" hint.
 */
export function expandRange(spec: RangeSpec): number[] {
  if (spec.kind === 'list') {
    return spec.values.filter((v) => Number.isFinite(v));
  }
  const { min, max, step } = spec;
  if (!Number.isFinite(min) || !Number.isFinite(max) || !Number.isFinite(step)) return [];
  if (step <= 0) return [];
  if (min > max) return [];

  const out: number[] = [];
  const tol = step / 2;
  // Cap iterations defensively so a malformed spec can never lock the
  // event loop. 10_001 is far beyond any realistic sweep resolution;
  // the UI caps visible points at 10_000 anyway.
  const MAX_POINTS = 10_001;
  let i = 0;
  for (let v = min; v <= max + tol; v += step) {
    out.push(round(v, step));
    if (++i >= MAX_POINTS) break;
  }
  return out;
}

/**
 * Round to the precision implied by `step` (prevents `0.30000000000000004`
 * from poisoning display / hashing). Falls back to a bounded toFixed so
 * integer-step grids stay integer.
 */
function round(v: number, step: number): number {
  if (!Number.isFinite(v) || !Number.isFinite(step)) return v;
  // Derive a precision from the step — 1 → 0, 0.1 → 1, 0.01 → 2, etc.
  const s = Math.abs(step);
  const precision = s >= 1 ? 0 : Math.min(12, Math.ceil(-Math.log10(s)));
  const factor = 10 ** precision;
  return Math.round(v * factor) / factor;
}
