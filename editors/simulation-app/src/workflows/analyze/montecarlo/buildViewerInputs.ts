/**
 * Derive viewer inputs (histogram outcomes + pass-rate constraint ids)
 * from a set of Monte Carlo `ChildDescriptor`s.
 *
 * - Outcomes: one histogram per numeric-valued metric/param key that
 *   appears in at least one completed child. Today the backend doesn't
 *   attach per-child output metrics, so these are almost always the
 *   sampled input params — still worth charting so the user can
 *   validate that the sampler produced the intended distribution.
 *   When a `metrics` field starts landing on `ChildDescriptor` the same
 *   helper picks it up transparently (metrics take precedence over
 *   params on key collisions).
 *
 * - Constraint ids: union of every verdict id across all children,
 *   sorted alphabetically so the pass-rate dashboard row order is
 *   stable across re-renders.
 */

import type { ChildDescriptor } from './passRateHelpers';
import type { MonteCarloOutcome } from '../../../shared/viewers/MonteCarloHistogramViewer';
import type { Value } from '../../../engine/types';

function isNumeric(v: Value | undefined): v is number {
  return typeof v === 'number' && Number.isFinite(v);
}

/** Resolve `id` the same way `passRateHelpers.verdictId` does. */
function verdictLabel(
  v: { id?: string; metadata?: Record<string, unknown> },
  idx: number,
): string {
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
 * Collect the union of numeric-valued metric + param keys across all
 * terminal children. Each key becomes one `MonteCarloOutcome` with an
 * extractor that reads `metrics[k] ?? params[k]` (metrics first so a
 * future backend change doesn't drop the more-informative source).
 */
export function buildOutcomesFromChildren(
  children: ChildDescriptor[],
): MonteCarloOutcome[] {
  const keys = new Set<string>();
  const source = new Map<string, 'metric' | 'param'>();

  for (const c of children) {
    if (c.metrics) {
      for (const [k, v] of Object.entries(c.metrics)) {
        if (isNumeric(v)) {
          keys.add(k);
          source.set(k, 'metric');
        }
      }
    }
    if (c.params) {
      for (const [k, v] of Object.entries(c.params)) {
        if (isNumeric(v) && !source.has(k)) {
          keys.add(k);
          source.set(k, 'param');
        }
      }
    }
  }

  const sorted = Array.from(keys).sort();
  return sorted.map((k) => ({
    id: k,
    label: source.get(k) === 'param' ? `${k} (input)` : k,
    extract: (child: ChildDescriptor): number | null => {
      const m = child.metrics?.[k];
      if (isNumeric(m)) return m;
      const p = child.params?.[k];
      if (isNumeric(p)) return p;
      return null;
    },
  }));
}

/**
 * Collect every distinct verdict id across all children, sorted.
 * Empty array when no children have verdicts — downstream helpers
 * treat that as "pass-rate not yet applicable" and short-circuit.
 */
export function collectConstraintIds(children: ChildDescriptor[]): string[] {
  const ids = new Set<string>();
  for (const c of children) {
    const verdicts = c.verdicts ?? [];
    for (let i = 0; i < verdicts.length; i++) {
      ids.add(verdictLabel(verdicts[i], i));
    }
  }
  return Array.from(ids).sort();
}
