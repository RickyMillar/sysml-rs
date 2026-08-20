/**
 * Runnable target discovery — pure normalization from raw backend responses
 * into grouped RunTargetGroup[].
 *
 * This module has no side effects and no React dependencies.
 */

import type { RunTargetSummary, RunTargetGroup, RunTargetKind } from './types';

/** Raw element shape returned by `findElements` / `sysml.query`
 *  (`summary` projection — carries the ownership path). */
export interface RawElement {
  id: string;
  name: string | null;
  kind: string;
  qualified_name?: string | null;
}

/** The owning scope's qualified path — `Pkg::Sub` for `Pkg::Sub::Case`.
 *  `null` for root-namespace elements (no `::`) or unnamed chains. */
export function ownerPathOf(qualifiedName: string | null | undefined): string | null {
  if (!qualifiedName) return null;
  const cut = qualifiedName.lastIndexOf('::');
  return cut > 0 ? qualifiedName.slice(0, cut) : null;
}

/** One structural group of items sharing an owning scope. */
export interface OwnerPathGroup<T> {
  /** The shared owning scope's qualified path; `null` = ungrouped
   *  (root-namespace or unnamed-chain items). */
  ownerPath: string | null;
  items: T[];
}

/**
 * Group items by the owning scope of their qualified name — the
 * structural "compliance suite" grouping (cases declared together in
 * one package/suite share a group). Groups are sorted by owner path;
 * the `null` (ungrouped) bucket, when present, comes LAST and renders
 * without a band header. Item order within a group is preserved.
 */
export function groupByOwnerPath<T>(
  items: readonly T[],
  qualifiedNameOf: (item: T) => string | null | undefined,
): Array<OwnerPathGroup<T>> {
  const byPath = new Map<string | null, T[]>();
  for (const item of items) {
    const path = ownerPathOf(qualifiedNameOf(item));
    const list = byPath.get(path) ?? [];
    list.push(item);
    byPath.set(path, list);
  }
  const named = [...byPath.entries()]
    .filter((e): e is [string, T[]] => e[0] !== null)
    .sort((a, b) => a[0].localeCompare(b[0]))
    .map(([ownerPath, groupItems]) => ({ ownerPath, items: groupItems }));
  const ungrouped = byPath.get(null);
  return ungrouped ? [...named, { ownerPath: null, items: ungrouped }] : named;
}

/** Map from backend element kind → RunTargetKind. */
const KIND_MAP: Record<string, RunTargetKind> = {
  // Simulation entry points (state machine defs + exhibit/usage)
  StateDefinition: 'simulation',
  StateUsage: 'simulation',
  ExhibitStateUsage: 'simulation',
  // Analysis cases
  AnalysisCaseUsage: 'analysisCases',
  AnalysisCaseDefinition: 'analysisCases',
  // Verification
  VerificationCaseUsage: 'verificationSuites',
  VerificationCaseDefinition: 'verificationSuites',
};

/** Group metadata keyed by RunTargetKind. */
const GROUP_META: Record<RunTargetKind, { label: string; icon: string; order: number }> = {
  simulation: { label: 'Simulations', icon: 'memory', order: 0 },
  analysisCases: { label: 'Analysis Cases', icon: 'analytics', order: 1 },
  verificationSuites: { label: 'Verification Suites', icon: 'verified', order: 2 },
};

/**
 * Normalize raw backend element lists into grouped RunTargetGroup[].
 *
 * @param simulations - Elements from findElements(uri, 'StateDefinition') etc.
 * @param analysisCases - AnalysisCaseUsage / AnalysisCaseDefinition elements.
 * @param verificationCases - VerificationCaseUsage / VerificationCaseDefinition elements.
 * @param uri - The source model URI (attached to every target).
 */
export function normalizeRunTargets(
  simulations: RawElement[],
  analysisCases: RawElement[],
  verificationCases: RawElement[],
  uri: string,
): RunTargetGroup[] {
  const groups = new Map<RunTargetKind, RunTargetSummary[]>();

  function addTargets(elements: RawElement[], kind: RunTargetKind) {
    for (const el of elements) {
      if (!el.name) continue; // Skip anonymous elements
      const list = groups.get(kind) ?? [];
      // Deduplicate by id
      if (list.some((t) => t.id === el.id)) continue;
      list.push({
        id: el.id,
        name: el.name,
        kind,
        uri,
        qualifiedName: el.qualified_name ?? null,
        ownerPath: ownerPathOf(el.qualified_name),
        metadata: {
          elementKind: el.kind,
        },
      });
      groups.set(kind, list);
    }
  }

  addTargets(simulations, 'simulation');
  addTargets(analysisCases, 'analysisCases');
  addTargets(verificationCases, 'verificationSuites');

  // Build sorted groups
  const result: RunTargetGroup[] = [];
  for (const [kind, targets] of groups) {
    if (targets.length === 0) continue;
    const meta = GROUP_META[kind];
    targets.sort((a, b) => (a.name ?? '').localeCompare(b.name ?? ''));
    result.push({
      label: meta.label,
      kind,
      icon: meta.icon,
      targets,
    });
  }

  result.sort((a, b) => GROUP_META[a.kind].order - GROUP_META[b.kind].order);
  return result;
}

export { KIND_MAP };
