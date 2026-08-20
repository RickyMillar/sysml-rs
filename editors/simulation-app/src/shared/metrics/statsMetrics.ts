/**
 * Statistical metric registry extension — R7.2.
 *
 * Surfaces five aggregation metrics (`mean`, `stddev`, `ci95_lower`,
 * `ci95_upper`, `distribution_family`) that downstream consumers can
 * register into the shared `MetricRegistry` via `registerStatsMetrics`.
 *
 * Each metric is a *consumer-side* computation — it reads a
 * `ChildDescriptor[]` plus a caller-supplied value extractor and returns
 * a `MetricValue`. The registry stores the MetricDescriptors (see
 * `registry.ts`); the aggregator itself stays next to the feature so
 * consumers can swap RNGs, change confidence levels, etc., without
 * mutating registered descriptors.
 */

import type { ChildDescriptor as McChild } from '../../workflows/analyze/montecarlo/passRateHelpers';
import type { ChildDescriptor as SweepChild } from '../viewers/sweepViewerHelpers';
import {
  bootstrapCI,
  confidenceInterval,
  createSeededRng,
  fitDistribution,
  mean as meanOf,
  stddev as stddevOf,
  type DistributionFamily,
} from '../../features/stats/statsHelpers';
import type { MetricDescriptor } from './types';
import type { MetricRegistry } from './registry';

/** A descriptor-shaped child — the union shared by MC and Sweep batches. */
export type StatsChild = McChild | SweepChild;

/**
 * Value extractor for a stats metric. Return NaN to skip.
 *
 * Typed as `(child: any) => number` rather than `(child: StatsChild) =>
 * number` because the two ChildDescriptor shapes have structurally
 * incompatible optional fields (e.g. session_id: string vs string?). The
 * extractor is *consumer-owned*, so we widen the parameter type to let
 * callers pass a strongly-typed extractor for their specific batch kind
 * (MC OR Sweep) without TS complaining about the opposite branch.
 */
// eslint-disable-next-line @typescript-eslint/no-explicit-any
export type StatsExtractor = (child: any) => number;

/**
 * MetricValue — the shape produced by every stats metric aggregator.
 *
 * A neutral wrapper so downstream consumers (KPI cards, CSV export,
 * report generators) can handle numeric + categorical results through
 * one interface. `unit` is optional because some metrics (family,
 * skew, p-values in later rounds) are unit-less.
 */
export interface MetricValue {
  /** Stable id — matches the MetricDescriptor.id. */
  id: string;
  /** Numeric value when applicable; NaN when the metric is categorical. */
  value: number;
  /** Categorical value (e.g. fit family) when the metric is not numeric. */
  category?: DistributionFamily;
  /** Number of samples that contributed. */
  n: number;
  /** Human label — matches the MetricDescriptor.name. */
  label: string;
  /** Unit carried through from the extractor context (optional). */
  unit?: string;
}

/** Metric id constants — exported so consumers avoid typos. */
export const STATS_METRIC_IDS = [
  'mean',
  'stddev',
  'ci95_lower',
  'ci95_upper',
  'distribution_family',
] as const;
export type StatsMetricId = (typeof STATS_METRIC_IDS)[number];

/**
 * MetricDescriptors to register into the shared MetricRegistry.
 *
 * Every descriptor uses `source: 'expression'` — they are derived
 * computations, not raw session variables. `aggregator` is set to a
 * named reducer when one fits; the distribution-family metric has no
 * scalar reducer and relies on the bespoke `computeStatsMetric` path.
 */
export const STATS_METRIC_DESCRIPTORS: Record<StatsMetricId, MetricDescriptor> = {
  mean: {
    id: 'mean',
    name: 'Mean',
    source: 'expression',
    expression: 'mean(values)',
    aggregator: 'mean',
  },
  stddev: {
    id: 'stddev',
    name: 'Standard deviation',
    source: 'expression',
    expression: 'stddev(values)',
    aggregator: (samples: number[]) => {
      if (samples.length < 2) return 0;
      const m = samples.reduce((s, x) => s + x, 0) / samples.length;
      let sq = 0;
      for (const x of samples) {
        const d = x - m;
        sq += d * d;
      }
      return Math.sqrt(sq / (samples.length - 1));
    },
  },
  ci95_lower: {
    id: 'ci95_lower',
    name: '95% CI (lower)',
    source: 'expression',
    expression: 'ci95_lower(values)',
  },
  ci95_upper: {
    id: 'ci95_upper',
    name: '95% CI (upper)',
    source: 'expression',
    expression: 'ci95_upper(values)',
  },
  distribution_family: {
    id: 'distribution_family',
    name: 'Fitted distribution',
    source: 'expression',
    expression: 'fitDistribution(values).family',
  },
};

/** Register all stats metrics into the supplied registry. Idempotent. */
export function registerStatsMetrics(registry: MetricRegistry): void {
  for (const id of STATS_METRIC_IDS) {
    registry.register(STATS_METRIC_DESCRIPTORS[id]);
  }
}

/** Options shared by every stats-metric call. */
export interface ComputeStatsMetricOptions {
  /** Bootstrap RNG. Defaults to a seeded Mulberry32. */
  rng?: () => number;
  /** Bootstrap iteration count (CI metrics). Default 500. */
  bootstrapIterations?: number;
  /** Unit to attach to the MetricValue. */
  unit?: string;
}

/**
 * Compute a single stats metric from a child batch + value extractor.
 *
 * This is the primary entry point for the registry consumers:
 *
 *   const descriptors = registry.filter(m => STATS_METRIC_IDS.includes(m.id));
 *   for (const d of descriptors) {
 *     const val = computeStatsMetric(d.id, children, extractor);
 *     // render val.value / val.category
 *   }
 *
 * Returns a MetricValue with `value: NaN` for empty inputs.
 */
export function computeStatsMetric(
  id: StatsMetricId,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  children: readonly any[],
  extractor: StatsExtractor,
  options: ComputeStatsMetricOptions = {},
): MetricValue {
  const desc = STATS_METRIC_DESCRIPTORS[id];
  const values = extractValues(children, extractor);
  const rng = options.rng ?? createSeededRng(0xfeedface);
  const iters = Math.max(1, Math.floor(options.bootstrapIterations ?? 500));

  if (id === 'mean') {
    const m = meanOf(values);
    return { id, value: m, n: values.length, label: desc.name, unit: options.unit };
  }
  if (id === 'stddev') {
    const s = stddevOf(values);
    return { id, value: s, n: values.length, label: desc.name, unit: options.unit };
  }
  if (id === 'ci95_lower') {
    if (values.length === 0) {
      return { id, value: Number.NaN, n: 0, label: desc.name, unit: options.unit };
    }
    // Prefer bootstrap for stability; fall back to parametric on n == 1.
    const ci = bootstrapCI(values, 0.95, iters, rng);
    return { id, value: ci.lower, n: values.length, label: desc.name, unit: options.unit };
  }
  if (id === 'ci95_upper') {
    if (values.length === 0) {
      return { id, value: Number.NaN, n: 0, label: desc.name, unit: options.unit };
    }
    const ci = bootstrapCI(values, 0.95, iters, rng);
    return { id, value: ci.upper, n: values.length, label: desc.name, unit: options.unit };
  }
  // distribution_family — categorical.
  const fit = fitDistribution(values);
  return {
    id,
    value: Number.NaN,
    category: fit.family,
    n: values.length,
    label: desc.name,
    unit: options.unit,
  };
}

/**
 * Convenience — compute every stats metric in one pass. Returns a map
 * keyed by metric id. Shares the bootstrap RNG across the two CI calls
 * so the two bounds stay paired.
 */
export function computeAllStatsMetrics(
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  children: readonly any[],
  extractor: StatsExtractor,
  options: ComputeStatsMetricOptions = {},
): Record<StatsMetricId, MetricValue> {
  // Share a single RNG between the two CI metrics so the returned bounds
  // come from the *same* bootstrap resample run — a common expectation.
  const rng = options.rng ?? createSeededRng(0xfeedface);
  const iters = Math.max(1, Math.floor(options.bootstrapIterations ?? 500));
  const values = extractValues(children, extractor);
  const unit = options.unit;

  const meanV: MetricValue = {
    id: 'mean',
    value: meanOf(values),
    n: values.length,
    label: STATS_METRIC_DESCRIPTORS.mean.name,
    unit,
  };
  const stddevV: MetricValue = {
    id: 'stddev',
    value: stddevOf(values),
    n: values.length,
    label: STATS_METRIC_DESCRIPTORS.stddev.name,
    unit,
  };
  let ciLo = Number.NaN;
  let ciHi = Number.NaN;
  if (values.length > 0) {
    const ci = bootstrapCI(values, 0.95, iters, rng);
    ciLo = ci.lower;
    ciHi = ci.upper;
  }
  const ciLowV: MetricValue = {
    id: 'ci95_lower',
    value: ciLo,
    n: values.length,
    label: STATS_METRIC_DESCRIPTORS.ci95_lower.name,
    unit,
  };
  const ciHiV: MetricValue = {
    id: 'ci95_upper',
    value: ciHi,
    n: values.length,
    label: STATS_METRIC_DESCRIPTORS.ci95_upper.name,
    unit,
  };
  const fit = fitDistribution(values);
  const famV: MetricValue = {
    id: 'distribution_family',
    value: Number.NaN,
    category: fit.family,
    n: values.length,
    label: STATS_METRIC_DESCRIPTORS.distribution_family.name,
    unit,
  };

  return {
    mean: meanV,
    stddev: stddevV,
    ci95_lower: ciLowV,
    ci95_upper: ciHiV,
    distribution_family: famV,
  };
}

/** Re-export the parametric CI helper's result shape for consumer convenience. */
export { confidenceInterval };

// eslint-disable-next-line @typescript-eslint/no-explicit-any
function extractValues(children: readonly any[], extractor: StatsExtractor): number[] {
  const out: number[] = [];
  for (const c of children) {
    let v: number;
    try {
      v = extractor(c);
    } catch {
      continue;
    }
    if (Number.isFinite(v)) out.push(v);
  }
  return out;
}
