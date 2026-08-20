/**
 * discoverSweepParameters — enumerate sweepable parameters
 * (`AttributeUsage` elements) across every loaded URI.
 *
 * Hoisted verbatim out of `SweepWorkflow.tsx` (ninebar Phase 5) so the
 * flag-on "Configure sweep" modal and the legacy flag-off body share ONE
 * discovery implementation + react-query key. Mirrors `discoverParameters`
 * in `MonteCarloWorkflow` — they fetch the same thing. See
 * docs/test-checklist-2026-04-20.md BUG 18 for why this replaced the
 * prior `useRunTargets` source, which surfaced StateDefinitions and
 * AnalysisCaseUsages (not sweepable attributes).
 */

import { findElements } from '@/shared/api/model';
import type { SweepParameterCandidate } from './SweepConfig';

export async function discoverSweepParameters(
  uris: readonly string[],
): Promise<SweepParameterCandidate[]> {
  if (uris.length === 0) return [];
  const seen = new Set<string>();
  const out: SweepParameterCandidate[] = [];
  for (const uri of uris) {
    let elements: Awaited<ReturnType<typeof findElements>>;
    try {
      elements = await findElements(uri, 'AttributeUsage');
    } catch {
      continue;
    }
    for (const el of elements) {
      const name = el.name ?? '';
      if (!name) continue;
      // Dedupe by NAME: the backend override surface is name-keyed
      // (`apply_overrides` sets context variables by bare name), so two
      // same-named attributes are ONE overridable knob — offering both
      // as separate rows implied a per-file distinction that doesn't
      // exist (and the `uri::name` id, if used as an override key,
      // silently no-ops — live-caught 2026-07-15).
      if (seen.has(name)) continue;
      seen.add(name);
      out.push({ id: `${uri}::${name}`, label: name, kind: 'AttributeUsage', uri });
    }
  }
  out.sort((a, b) => (a.label ?? a.id).localeCompare(b.label ?? b.id));
  return out;
}
