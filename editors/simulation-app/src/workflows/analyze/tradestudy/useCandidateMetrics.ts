/**
 * useCandidateMetrics — discover candidate criterion metrics for the
 * Trade Study picker.
 *
 * Trade Study criteria are conceptually OUTPUT metrics (you optimise
 * for them), but SysML v2 doesn't distinguish inputs vs outputs at
 * the AttributeUsage level — both are just named attributes of a
 * part. The stdlib TradeStudies::ObjectiveUsage pattern exists for
 * formal criterion declaration, but models in the examples workspace
 * rarely use it, and the user's explicit ask (sprint note B15) said
 * the zero-state "Add criterion" dropdown has to surface SOMETHING
 * useful without waiting for spec-perfect modelling.
 *
 * This hook unions two sources:
 *   1. Already-registered metrics from the MetricRegistry (variables
 *      that a live session has published via PlotsTab's sync).
 *   2. Every AttributeUsage name across the loaded workspace files —
 *      the same discovery pattern Monte Carlo uses for parameters.
 *
 * The result is the authoritative metric pool for the Criteria
 * dropdown. `defaultObjectiveForMetric` then picks Min/Max based on
 * the name heuristic (cost/latency/error/penalty → Min; else Max).
 *
 * NOTE: future work (Option A in the B15 plan) could pull the formal
 * list via a dedicated `sysml.tradestudy.list_objectives` backend
 * command — this hook stays compatible; the command's output would
 * simply be a third union source.
 */

import { useQuery } from '@tanstack/react-query';
import { findElements } from '@/shared/api/model';
import type { MetricDescriptor } from '@/shared/metrics/types';
import { metricRegistry } from '@/shared/metrics/registry';

async function discoverAttributeUsages(
  uris: readonly string[],
): Promise<MetricDescriptor[]> {
  if (uris.length === 0) return [];

  const seen = new Set<string>();
  const out: MetricDescriptor[] = [];
  for (const uri of uris) {
    let elements: Awaited<ReturnType<typeof findElements>>;
    try {
      elements = await findElements(uri, 'AttributeUsage');
    } catch {
      // Single-file failures shouldn't nuke the whole pool — the
      // remaining URIs may still produce useful candidates.
      continue;
    }
    for (const el of elements) {
      const name = el.name ?? '';
      if (!name) continue;
      if (seen.has(name)) continue;
      seen.add(name);
      out.push({
        id: name,
        name,
        source: 'variable', // same source bucket as session-variable metrics
        expression: name,
      });
    }
  }
  out.sort((a, b) => a.name.localeCompare(b.name));
  return out;
}

/**
 * Merge two MetricDescriptor lists, keyed by id. Later entries win on
 * id collision so registered (in-session) metadata takes precedence
 * over discovered skeletons.
 */
function mergeMetrics(
  a: readonly MetricDescriptor[],
  b: readonly MetricDescriptor[],
): MetricDescriptor[] {
  const byId = new Map<string, MetricDescriptor>();
  for (const m of a) byId.set(m.id, m);
  for (const m of b) byId.set(m.id, m);
  return Array.from(byId.values()).sort((x, y) => x.name.localeCompare(y.name));
}

export function useCandidateMetrics(
  loadedUris: readonly string[],
): MetricDescriptor[] {
  const { data: discovered = [] } = useQuery<MetricDescriptor[]>({
    queryKey: ['tradestudy-candidate-metrics', loadedUris],
    queryFn: () => discoverAttributeUsages(loadedUris),
    enabled: loadedUris.length > 0,
    staleTime: 60_000,
  });

  const registered = metricRegistry.list();
  return mergeMetrics(discovered, registered);
}
