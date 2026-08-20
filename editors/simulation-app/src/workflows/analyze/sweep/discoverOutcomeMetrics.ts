/**
 * discoverOutcomeMetrics — enumerate a model's OUTCOMES the same way
 * `discoverSweepParameters` enumerates its knobs.
 *
 * The sweep used to learn its inputs and its outputs from two different
 * places, and only one of them was the model:
 *
 *   inputs   `findElements(uri, 'AttributeUsage')`  — static, no run needed
 *   outputs  `metricRegistry.list()`                — a client-side, push-only
 *                                                     catalogue written ONLY by
 *                                                     PlotsTab / WaveformCard
 *                                                     while rendering a LIVE
 *                                                     session
 *
 * So you could frame the study's inputs before running anything, but not its
 * outputs: the Configure modal said "No metrics registered yet — run the model
 * once so its variables register", which is backwards for a workflow whose
 * entire job is deciding what to measure BEFORE launching.
 *
 * The asymmetry was never necessary. The model already distinguishes the two
 * roles — a state variable or algebraic output is declared `out`, a knob is
 * not — and the same query that finds the knobs carries that marking. The
 * registry stays as a SUPPLEMENT for things only a run can reveal (derived
 * series, injected signals); this is the part that can be known up front.
 *
 * Deliberately NOT a global default change: this adds a producer, it does not
 * replace the registry.
 */

import { queryModel } from '@/shared/api/model';
import type { MetricDescriptor } from '@/shared/metrics/types';

/** Row shape of the `elements` projection — carries `props`, unlike `summary`. */
interface ElementRow {
  id: string;
  name: string | null;
  kind: string;
  props?: Record<string, unknown> | null;
}

/**
 * True when an attribute is declared `out` — the model's own marking for
 * "this is a result", covering ODE/discrete state variables and algebraic
 * outputs alike.
 *
 * `direction` is absent on ordinary attributes and `"in"` on inputs; only
 * `"out"` (and `"inout"`, which is written as well as read) is an outcome.
 */
function isOutcome(row: ElementRow): boolean {
  const direction = row.props?.direction;
  return direction === 'out' || direction === 'inout';
}

/**
 * Outcome metrics for every loaded URI.
 *
 * Queried per FILE uri, not `__workspace__`: the workspace graph is
 * library-overlaid, so a workspace-scoped query returns thousands of ISQ unit
 * and quantity attributes alongside the model's own. `discoverSweepParameters`
 * relies on the same scoping.
 */
export async function discoverOutcomeMetrics(
  uris: readonly string[],
): Promise<MetricDescriptor[]> {
  if (uris.length === 0) return [];
  const seen = new Set<string>();
  const out: MetricDescriptor[] = [];

  for (const uri of uris) {
    let rows: ElementRow[];
    try {
      const result = await queryModel<ElementRow[]>(uri, {
        filter: { type: 'kind', kinds: ['AttributeUsage'] },
        projection: 'elements',
        limit: 1000,
      });
      rows = result.rows;
    } catch {
      // A uri that cannot be queried contributes nothing; the others still
      // produce a list. Matches `discoverSweepParameters`.
      continue;
    }

    for (const row of rows) {
      const name = row.name ?? '';
      if (!name || !isOutcome(row)) continue;
      // Dedupe by NAME, like the parameter side: the backend's time-series
      // surface is name-keyed, so two same-named attributes are one series.
      if (seen.has(name)) continue;
      seen.add(name);
      out.push({
        id: name,
        name,
        source: 'variable',
        expression: name,
        // `aggregator` is deliberately left unset. A sweep usually wants the
        // final value, but `MetricAggregator` has no `last` and inventing one
        // here would widen a shared type on this feature's behalf. Discovery
        // reports WHAT can be measured; how to reduce it stays the consumer's
        // call, which is what the optional field already means.
        unit: typeof row.props?.unit === 'string' ? row.props.unit : undefined,
      });
    }
  }

  out.sort((a, b) => a.name.localeCompare(b.name));
  return out;
}
