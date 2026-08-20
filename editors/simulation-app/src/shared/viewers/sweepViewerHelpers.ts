/**
 * Sweep viewer helpers — pure utilities shared by every R5.3 sweep viewer.
 *
 * Every helper is side-effect-free and exported so viewers and tests can
 * import them independently. Keeping the numeric crunch outside the render
 * layer means:
 *   - the Tornado ranker, ParallelCoords normaliser, and Heatmap builder
 *     can each be unit-tested without JSDOM,
 *   - viewers stay declarative — they describe geometry, not maths,
 *   - BB's `<SweepResultsShell>` integration and DD's drill workflow can
 *     reuse the same `ChildDescriptor` shape via the re-export below.
 *
 * ChildDescriptor shape is the R5.2 streaming contract: the sweep shell
 * polls `sysml.batch.status`, each response carries an array of these, and
 * every viewer consumes them incrementally (pending rows are legal, failed
 * rows must render with a reason).
 */
import type { OutcomeReading, Verdict, VerdictKind } from '../../engine/types';

// ── The streaming contract ──────────────────────────────────────────

/**
 * Backend-authored summary of one sweep point.
 *
 * The four statuses map to the R5.2 streaming state machine. Viewers MUST
 * cope with any subset (an empty `children` list is a valid first render);
 * anything that locks up while `status === 'running'` is a regression.
 */
export type ChildStatus = 'pending' | 'running' | 'complete' | 'failed';

/**
 * One row in the streaming batch status response.
 *
 * Mirrors the contract that Agent BB's `useSweepRunner` hook produces. The
 * `params` map carries the sweep-point coordinates — keys are parameter
 * names, values are whatever the backend emitted (numbers, strings,
 * booleans, etc.). Viewers narrow to numerics with `toNumber()`.
 */
export interface ChildDescriptor {
  session_id: string;
  index: number;
  params: Record<string, unknown>;
  status: ChildStatus;
  verdicts: Verdict[];
  /**
   * Optional failure reason. Populated when `status === 'failed'`; rendered
   * in red by Table / Tornado / Heatmap so the user sees *why* the point
   * did not settle without opening the drill.
   */
  reason?: string | null;
  /**
   * Final readings for the outcomes the batch was asked to measure, keyed
   * by variable name. Backend-populated with the terminal status transition;
   * absent for batches that requested none.
   */
  outcomes?: Record<string, OutcomeReading>;
}

/**
 * Shape returned by `computeSensitivity`. Keep it tiny so the tornado
 * viewer can lay bars out with zero further transformation.
 */
export interface SensitivityStat {
  /** Minimum observed metric value across the swept parameter's points. */
  low: number;
  /** Maximum observed metric value. */
  high: number;
  /** Absolute range = `high - low`; used as bar magnitude. */
  range: number;
  /**
   * Number of distinct parameter values represented. A 1-value parameter
   * cannot drive sensitivity so the tornado viewer hides it — tests pin
   * this behaviour.
   */
  samples: number;
}

// ── Small numeric coercion ──────────────────────────────────────────

/**
 * Narrow an `unknown` (from `params` or `Verdict.actual`) to a real number.
 *
 * Returns `NaN` for anything that isn't a finite number. Viewers treat NaN
 * as "unknown / skip" — this matches how `uPlot` already handles gap cells
 * elsewhere in the kit so the behaviour is consistent.
 */
export function toNumber(value: unknown): number {
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (typeof value === 'string') {
    const n = Number(value);
    return Number.isFinite(n) ? n : Number.NaN;
  }
  if (typeof value === 'boolean') return value ? 1 : 0;
  return Number.NaN;
}

/**
 * Collect the set of parameter names present across every child, preserving
 * insertion order of first appearance. This keeps the rendered column /
 * axis order deterministic even when later children expose more params
 * (the streaming scenario).
 */
export function collectParamNames(children: ChildDescriptor[]): string[] {
  const seen = new Set<string>();
  const ordered: string[] = [];
  for (const c of children) {
    for (const key of Object.keys(c.params)) {
      if (!seen.has(key)) {
        seen.add(key);
        ordered.push(key);
      }
    }
  }
  return ordered;
}

// ── computeSensitivity (Tornado) ────────────────────────────────────

/**
 * Rank a parameter by the spread of an outcome metric across its sweep
 * values. The tornado viewer pipes every parameter through this and sorts
 * by `range` descending.
 *
 * `metricExtractor` runs once per child so callers can compose whatever
 * verdict-aggregation rule they like (first margin, max margin, verdict
 * count, …) without this helper growing a taxonomy.
 *
 * Edge cases (all pinned by tests):
 *   - Empty `children`  → `{ low: NaN, high: NaN, range: 0, samples: 0 }`.
 *   - All-NaN metrics   → `range: 0, samples: 0` (we only count points
 *                         whose metric is finite, because a metric we
 *                         cannot read has no sensitivity signal).
 *   - Single value      → `range: 0, samples: 1`.
 *   - Negative range    → `range` is absolute, never signed.
 */
export function computeSensitivity(
  children: ChildDescriptor[],
  paramName: string,
  metricExtractor: (child: ChildDescriptor) => number,
): SensitivityStat {
  let low = Number.POSITIVE_INFINITY;
  let high = Number.NEGATIVE_INFINITY;
  const distinctParamValues = new Set<number>();

  for (const child of children) {
    const paramValue = toNumber(child.params[paramName]);
    const metric = metricExtractor(child);
    // Drop points whose metric or parameter reading is non-numeric —
    // including them would poison the range calculation with NaN.
    if (!Number.isFinite(paramValue) || !Number.isFinite(metric)) continue;
    distinctParamValues.add(paramValue);
    if (metric < low) low = metric;
    if (metric > high) high = metric;
  }

  const samples = distinctParamValues.size;
  if (samples === 0) {
    return { low: Number.NaN, high: Number.NaN, range: 0, samples: 0 };
  }

  const range = Math.abs(high - low);
  return { low, high, range, samples };
}

// ── normaliseAxisValues (ParallelCoords) ────────────────────────────

/**
 * Compute a 0..1 normaliser over a set of numeric samples. Parallel-coords
 * uses this once per axis so every polyline can be laid out in the same
 * normalised y-space regardless of the axis's raw units.
 *
 * Edge cases (all pinned by tests):
 *   - Empty input     → constant normaliser mapping any input to 0.5
 *                       (renders the axis as a single centre line).
 *   - Single value    → same: `{ min: v, max: v, normalise: () => 0.5 }`.
 *   - All-NaN input   → constant normaliser, `min = max = NaN`.
 *   - Negative range  → handled like any other finite range.
 */
export function normaliseAxisValues(values: number[]): {
  min: number;
  max: number;
  normalise: (v: number) => number;
} {
  const finite = values.filter((v) => Number.isFinite(v));
  if (finite.length === 0) {
    return { min: Number.NaN, max: Number.NaN, normalise: () => 0.5 };
  }
  let min = finite[0];
  let max = finite[0];
  for (const v of finite) {
    if (v < min) min = v;
    if (v > max) max = v;
  }
  if (min === max) {
    return { min, max, normalise: () => 0.5 };
  }
  const span = max - min;
  return {
    min,
    max,
    normalise: (v: number) => {
      if (!Number.isFinite(v)) return 0.5;
      return (v - min) / span;
    },
  };
}

// ── buildHeatmapGrid (Heatmap) ──────────────────────────────────────

export interface HeatmapGrid {
  /** Sorted, unique x-axis parameter values. */
  x: number[];
  /** Sorted, unique y-axis parameter values. */
  y: number[];
  /**
   * `values[yi][xi]` — the metric at that cell, or NaN when no child has
   * produced that coordinate yet (a very common streaming state). The
   * viewer renders NaN cells with a hatched placeholder so the user can
   * tell "no data yet" from "data but zero".
   */
  values: number[][];
}

/**
 * Build a 2D heatmap grid for exactly two swept parameters.
 *
 * Behaviour:
 *   - Duplicates on the same (x, y) keep the *last-written* metric. In the
 *     streaming case this is what we want — a `complete` child overwrites
 *     a prior `running` placeholder.
 *   - Missing cells (no child at that grid point) stay NaN.
 *   - Empty input returns `{ x: [], y: [], values: [] }` so the viewer
 *     can fall through to its empty-state without an extra guard.
 */
export function buildHeatmapGrid(
  children: ChildDescriptor[],
  xParam: string,
  yParam: string,
  metricExtractor: (child: ChildDescriptor) => number,
): HeatmapGrid {
  if (children.length === 0) return { x: [], y: [], values: [] };

  const xSet = new Set<number>();
  const ySet = new Set<number>();
  for (const child of children) {
    const x = toNumber(child.params[xParam]);
    const y = toNumber(child.params[yParam]);
    if (Number.isFinite(x)) xSet.add(x);
    if (Number.isFinite(y)) ySet.add(y);
  }
  const x = [...xSet].sort((a, b) => a - b);
  const y = [...ySet].sort((a, b) => a - b);
  const xi = new Map(x.map((v, i) => [v, i]));
  const yi = new Map(y.map((v, i) => [v, i]));

  const values: number[][] = y.map(() => new Array<number>(x.length).fill(Number.NaN));
  for (const child of children) {
    const xv = toNumber(child.params[xParam]);
    const yv = toNumber(child.params[yParam]);
    if (!Number.isFinite(xv) || !Number.isFinite(yv)) continue;
    const row = yi.get(yv);
    const col = xi.get(xv);
    if (row === undefined || col === undefined) continue;
    const metric = metricExtractor(child);
    values[row][col] = metric;
  }
  return { x, y, values };
}

// ── Verdict-flavoured metric extractors ─────────────────────────────

/** Count of failing verdicts on a child. Default metric for tornado. */
export function failCount(child: ChildDescriptor): number {
  let n = 0;
  for (const v of child.verdicts) if (v.verdict === 'fail') n += 1;
  return n;
}

/** First numeric `margin` in the child's verdicts, NaN if none. */
export function firstMargin(child: ChildDescriptor): number {
  for (const v of child.verdicts) {
    if (typeof v.margin === 'number' && Number.isFinite(v.margin)) return v.margin;
  }
  return Number.NaN;
}

/**
 * Pick a verdict-based metric by name. `'fail_count'` / `'margin'` are the
 * two the viewers expose in their config dropdowns; anything else falls
 * back to `firstMargin`. Exported so BB's config surface can offer the
 * same menu without duplicating the list.
 */
export type MetricName = 'fail_count' | 'margin';

/**
 * Namespace marker for a model outcome used as a viewer metric.
 *
 * Outcome names come from the model, so an attribute could legitimately be
 * called `margin` and collide with the built-in verdict metric. Prefixing
 * keeps the two families addressable without either shadowing the other, and
 * makes an id self-describing at any point it is read back.
 */
export const OUTCOME_METRIC_PREFIX = 'outcome:';

/**
 * A viewer metric selection: one of the two verdict-derived built-ins, or an
 * `outcome:<name>` id naming a measured model outcome.
 *
 * This is `string` rather than a closed union because the outcome half is not
 * knowable at compile time — it is whatever the model declared `out`. Use
 * `metricOptionsFor` to enumerate what is actually selectable for a given
 * batch, rather than assuming either family is present.
 */
export type SweepMetricId = string;

/** Metric id addressing a model outcome by name. */
export function outcomeMetricId(name: string): SweepMetricId {
  return `${OUTCOME_METRIC_PREFIX}${name}`;
}

/** The outcome name inside a metric id, or `null` for the built-ins. */
export function outcomeNameFromMetricId(metric: SweepMetricId): string | null {
  return metric.startsWith(OUTCOME_METRIC_PREFIX)
    ? metric.slice(OUTCOME_METRIC_PREFIX.length)
    : null;
}

/**
 * Outcome names present across the children, in first-appearance order.
 *
 * Reads the children rather than the study config so the viewers describe the
 * run they are actually showing. Streaming-safe: children that have not
 * reached a terminal state carry no outcomes yet and simply contribute
 * nothing, so the option list grows as results land.
 */
export function collectOutcomeNames(children: ChildDescriptor[]): string[] {
  const seen = new Set<string>();
  const ordered: string[] = [];
  for (const c of children) {
    for (const name of Object.keys(c.outcomes ?? {})) {
      if (!seen.has(name)) {
        seen.add(name);
        ordered.push(name);
      }
    }
  }
  return ordered;
}

/** The raw reading for one outcome on one child, if the child has it. */
export function outcomeReading(
  child: ChildDescriptor,
  name: string,
): OutcomeReading | undefined {
  return child.outcomes?.[name];
}

/**
 * One outcome as a number for the numeric viewers.
 *
 * Returns `NaN` — the kit's established "unknown / skip" signal, which
 * `computeSensitivity`, `normaliseAxisValues`, and `buildHeatmapGrid` all
 * already drop rather than plot — whenever the reading is missing, carries an
 * error, or is non-finite. Never `0`: a run that could not produce a value and
 * a run that produced zero must not land on the same pixel.
 */
export function outcomeValue(child: ChildDescriptor, name: string): number {
  const reading = outcomeReading(child, name);
  if (!reading || reading.error !== undefined) return Number.NaN;
  return typeof reading.value === 'number' && Number.isFinite(reading.value)
    ? reading.value
    : Number.NaN;
}

/** The two verdict-derived metrics, always available. */
export const METRIC_OPTIONS: { value: MetricName; label: string }[] = [
  { value: 'fail_count', label: 'Fail count' },
  { value: 'margin', label: 'First margin' },
];

/**
 * Everything selectable for this batch: the verdict built-ins plus one entry
 * per measured outcome. This is what the viewers' metric dropdowns render, so
 * a selected outcome becomes choosable everywhere a metric is choosable.
 */
export function metricOptionsFor(
  children: ChildDescriptor[],
): { value: SweepMetricId; label: string }[] {
  return [
    ...METRIC_OPTIONS,
    ...collectOutcomeNames(children).map((name) => ({
      value: outcomeMetricId(name),
      label: unitSuffixed(name, outcomeUnit(children, name)),
    })),
  ];
}

/** Display label for a metric id — the outcome's own name, or the built-in's. */
export function metricLabelFor(
  metric: SweepMetricId,
  children: ChildDescriptor[] = [],
): string {
  const name = outcomeNameFromMetricId(metric);
  if (name !== null) return unitSuffixed(name, outcomeUnit(children, name));
  return METRIC_OPTIONS.find((o) => o.value === metric)?.label ?? metric;
}

/**
 * Unit for an outcome, taken from the first child that reported one.
 *
 * Absent is normal, not an error: a type-only ISQ quantity (`temperature :
 * ThermodynamicTemperatureValue`) carries a dimension but no explicit unit
 * symbol, so there is nothing honest to print.
 */
export function outcomeUnit(
  children: ChildDescriptor[],
  name: string,
): string | undefined {
  for (const c of children) {
    const unit = outcomeReading(c, name)?.unit;
    if (unit) return unit;
  }
  return undefined;
}

function unitSuffixed(name: string, unit: string | undefined): string {
  return unit ? `${name} (${unit})` : name;
}

/**
 * Resolve a metric id to a per-child numeric extractor.
 *
 * Unknown ids fall back to `firstMargin`, preserving the prior behaviour for
 * the two built-ins; an `outcome:` id reads that outcome.
 */
export function extractorFor(metric: SweepMetricId): (child: ChildDescriptor) => number {
  const name = outcomeNameFromMetricId(metric);
  if (name !== null) return (child) => outcomeValue(child, name);
  return metric === 'fail_count' ? failCount : firstMargin;
}

// ── Status / verdict rollups for tables ─────────────────────────────

/**
 * Aggregate a child's verdicts into a single rollup kind: priority
 * error > fail > inconclusive > pass. Returns `null` when no verdicts
 * exist yet (renders as "—" in the table).
 */
export function rollupVerdict(child: ChildDescriptor): VerdictKind | null {
  if (child.verdicts.length === 0) return null;
  let rank = 0;
  let best: VerdictKind = 'pass';
  const order: Record<VerdictKind, number> = { pass: 1, inconclusive: 2, fail: 3, error: 4 };
  for (const v of child.verdicts) {
    const r = order[v.verdict];
    if (r > rank) {
      rank = r;
      best = v.verdict;
    }
  }
  return best;
}

// ── Viridis-lite colour scale for heatmap ───────────────────────────

/**
 * 9-stop viridis-lite ramp. Approximates matplotlib's viridis so we keep
 * the perceptual ordering without a dependency. The viewer interpolates
 * between adjacent stops in `colourForNormalised`.
 */
const VIRIDIS_STOPS: [number, number, number][] = [
  [68, 1, 84],
  [72, 40, 120],
  [62, 74, 137],
  [49, 104, 142],
  [38, 130, 142],
  [31, 158, 137],
  [53, 183, 121],
  [109, 205, 89],
  [180, 222, 44],
];

function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

/**
 * Sample the viridis ramp at a 0..1 position. `NaN` / out-of-range values
 * return a transparent sentinel so the heatmap can visually distinguish
 * "no data" from "data == minimum".
 */
export function colourForNormalised(t: number): string {
  if (!Number.isFinite(t)) return 'transparent';
  const clamped = Math.max(0, Math.min(1, t));
  const scaled = clamped * (VIRIDIS_STOPS.length - 1);
  const lo = Math.floor(scaled);
  const hi = Math.min(VIRIDIS_STOPS.length - 1, lo + 1);
  const frac = scaled - lo;
  const [r0, g0, b0] = VIRIDIS_STOPS[lo];
  const [r1, g1, b1] = VIRIDIS_STOPS[hi];
  const r = Math.round(lerp(r0, r1, frac));
  const g = Math.round(lerp(g0, g1, frac));
  const b = Math.round(lerp(b0, b1, frac));
  return `rgb(${r}, ${g}, ${b})`;
}
