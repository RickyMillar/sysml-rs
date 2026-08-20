/**
 * MetricDescriptor — Layer 1 primitive (extensibility plan EP1.7).
 *
 * A MetricDescriptor is the atomic unit of "a thing that can be charted,
 * tabulated, swept, or verified". Today it is wired into the Waveform
 * card's variable selection; tomorrow it will be consumed by Sweep and
 * Monte Carlo workflows, KPI cards, and verification cases without
 * touching any of the panel code.
 *
 * Why a registry (not ad-hoc arrays):
 *   - Multiple producers can register metrics (session variables,
 *     constraint expressions, calc-def outputs, user-authored expressions)
 *     without each consumer needing to know the sources.
 *   - Consumers filter by predicate (`source === 'variable'`,
 *     `domain === 'thermal'`, etc.) and get a stable typed surface.
 *   - Round 1 only auto-registers raw session variables so the current
 *     Plots UX is preserved bit-for-bit; later rounds expand the
 *     catalogue without re-opening the picker or chart code.
 *
 */

/** Source from which a metric is materialised. */
export type MetricSource = 'variable' | 'expression' | 'constraint';

/**
 * Aggregator reducing a sample list to a single scalar. Used by Sweep /
 * Monte Carlo / KPI consumers — the registry itself just stores the
 * declaration; consumers apply it.
 *
 * `first_crossing` is domain-specific (first sample crossing a threshold
 * the caller supplies) and needs caller-provided context; the named
 * aggregators are pure reductions.
 */
export type MetricAggregator =
  | 'mean'
  | 'max'
  | 'p95'
  | 'first_crossing'
  | ((samples: number[]) => number);

/**
 * A declarative description of a metric. The descriptor is pure data —
 * resolution of the samples (for `source: 'variable'` that's a direct
 * timeSeries lookup; for `expression` it's evaluating the AST) is the
 * consumer's responsibility.
 */
export interface MetricDescriptor {
  /** Stable identity. For variables this is the variable name; for
   *  expressions/constraints the element id or a derived hash. Consumers
   *  use this as the React key and the persisted selection key. */
  id: string;

  /** Human-facing label (e.g. "trip_time", "T_busbar"). */
  name: string;

  /** Where the samples come from. */
  source: MetricSource;

  /**
   * For `source: 'variable'` this is just the variable name; for
   * `expression` / `constraint` it's the expression text (a SysML-v2
   * expression, parsed by callers).
   */
  expression: string;

  /** Default aggregator used when reducing to a scalar (KPI, sweep). */
  aggregator?: MetricAggregator;

  /** Display unit when known (from ISQ inference or model authoring). */
  unit?: string;

  /**
   * Physics domain classification (electrical/thermal/etc.) — used by the
   * picker grouping + the colour on the chart. Falls back to a heuristic
   * classifier (classifyVariableDomain) at the consumer when absent.
   */
  domain?: string;
}
