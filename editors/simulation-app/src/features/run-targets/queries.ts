/**
 * Runnable model target discovery — React Query hooks.
 *
 * Uses `sysml.stats` + `sysml.query` to discover runnable elements:
 *   - Simulations: StateDefinition elements
 *   - Analysis Cases: AnalysisCaseUsage / AnalysisCaseDefinition
 *   - Verification Suites: VerificationCaseUsage / VerificationCaseDefinition
 */

import { useQuery } from '@tanstack/react-query';
import { findElements, getStats } from '@/shared/api/model';
import { normalizeRunTargets } from './normalize';
import type { RunTargetGroup } from './types';
import type { RawElement } from './normalize';

/** Workspace stats shape (element kind -> count). */
type StatsMap = Record<string, number>;

/**
 * Fetch run targets for a single loaded URI.
 *
 * Queries are conditional on stats — we only fire `findElements` calls
 * when the corresponding kind has a non-zero count.
 */
async function discoverTargetsForUri(
  uri: string,
  stats: StatsMap,
): Promise<RunTargetGroup[]> {
  const promises: {
    simulations: Promise<RawElement[]>;
    analysisCases: Promise<RawElement[]>;
    verificationCases: Promise<RawElement[]>;
  } = {
    simulations: Promise.resolve([]),
    analysisCases: Promise.resolve([]),
    verificationCases: Promise.resolve([]),
  };

  // Simulations: StateDefinition entries
  const smDefCount = (stats.StateDefinition ?? 0);
  if (smDefCount > 0) {
    promises.simulations = findElements(uri, 'StateDefinition');
  }


  // Analysis + verification cases — always queried unconditionally (P4 fix).
  // The stats-count gate was unsafe: on workspaces where
  // `sysml.workspace.info` stats don't account for subpackage files
  // (e.g. `Verification/IEC_Compliance.sysml` on espresso-production-cell),
  // the gate suppressed the findElements calls for files that actually
  // had cases, and the Verify picker reported "No verification cases".
  // Always-query matches the pattern Monte Carlo / Trade Study use for
  // AttributeUsage discovery; empty-file findElements calls are cheap
  // and every URI is queried in parallel.
  promises.analysisCases = Promise.all([
    findElements(uri, 'AnalysisCaseUsage').catch(() => []),
    findElements(uri, 'AnalysisCaseDefinition').catch(() => []),
  ]).then(([usages, defs]) => [...usages, ...defs]);

  promises.verificationCases = Promise.all([
    findElements(uri, 'VerificationCaseUsage').catch(() => []),
    findElements(uri, 'VerificationCaseDefinition').catch(() => []),
  ]).then(([usages, defs]) => [...usages, ...defs]);

  const [simulations, analysisCases, verificationCases] = await Promise.all([
    promises.simulations,
    promises.analysisCases,
    promises.verificationCases,
  ]);

  return normalizeRunTargets(simulations, analysisCases, verificationCases, uri);
}

/**
 * Merge groups from multiple URIs. Concatenates targets within each
 * kind group and deduplicates by target id.
 */
function mergeGroups(allGroups: RunTargetGroup[][]): RunTargetGroup[] {
  const merged = new Map<string, RunTargetGroup>();
  for (const groups of allGroups) {
    for (const group of groups) {
      const existing = merged.get(group.kind);
      if (existing) {
        // Merge targets, deduplicating by id
        const ids = new Set(existing.targets.map((t) => t.id));
        for (const t of group.targets) {
          if (!ids.has(t.id)) {
            existing.targets.push(t);
            ids.add(t.id);
          }
        }
        existing.targets.sort((a, b) => (a.name ?? '').localeCompare(b.name ?? ''));
      } else {
        merged.set(group.kind, { ...group, targets: [...group.targets] });
      }
    }
  }
  return Array.from(merged.values());
}

/**
 * React Query hook: discovers run targets across all loaded workspace URIs.
 *
 * @param workspaceRoot - The workspace root path. Pass null to disable the query.
 * @param loadedUris - Array of currently loaded model URIs.
 */
export function useRunTargets(
  workspaceRoot: string | null,
  loadedUris: string[],
) {
  return useQuery<RunTargetGroup[]>({
    queryKey: ['runTargets', workspaceRoot, loadedUris],
    queryFn: async () => {
      if (!workspaceRoot || loadedUris.length === 0) return [];

      // Step 1: Fetch stats for every loaded URI in parallel
      const statsEntries = await Promise.all(
        loadedUris.map(async (uri) => {
          try {
            const raw = await getStats(uri);
            // Backend wraps kind counts under elements_by_kind
            const stats = (raw.elements_by_kind ?? raw) as StatsMap;
            return { uri, stats };
          } catch {
            return { uri, stats: {} as StatsMap };
          }
        }),
      );

      // Step 2: Discover targets per-URI in parallel
      const perUriGroups = await Promise.all(
        statsEntries.map(({ uri, stats }) => discoverTargetsForUri(uri, stats)),
      );

      // Step 3: Merge across all URIs
      return mergeGroups(perUriGroups);
    },
    enabled: !!workspaceRoot && loadedUris.length > 0,
    staleTime: 30_000, // 30s — targets don't change unless the model is reloaded
  });
}
