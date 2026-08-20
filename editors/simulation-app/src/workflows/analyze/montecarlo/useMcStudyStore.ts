/**
 * useMcStudyStore — shared Monte Carlo study configuration for the
 * flag-on ninebar Analyze surface (Phase 5).
 *
 * Same rationale as `useSweepStudyStore`: the "Configure Monte Carlo"
 * MODAL, the left-rail distribution summary, and the workflow body (the
 * runner owner) render in different subtrees, so the study definition
 * lives in a store (ConfigureRunModal precedent — closing the modal is
 * enough). Distribution math/validation is NOT re-implemented — it stays
 * in `sampleDistribution.ts` (one home): `defaultDistribution`,
 * `isDistributionValid`, `generateChildrenParams`.
 */

import { create } from 'zustand';
import {
  defaultDistribution,
  isDistributionValid,
  type Distribution,
  type DistributionKind,
  type DistributionMap,
} from './sampleDistribution';

export const MC_MAX_SAMPLE_COUNT = 20;
export const MC_DEFAULT_SAMPLE_COUNT = 10;

function clampSampleCount(n: number): number {
  if (!Number.isFinite(n)) return MC_DEFAULT_SAMPLE_COUNT;
  return Math.min(MC_MAX_SAMPLE_COUNT, Math.max(1, Math.round(n)));
}

interface McStudyState {
  /** Name → Distribution for every selected parameter, insertion order. */
  distributions: DistributionMap;
  sampleCount: number;
  /** Optional seed. `null` → backend picks. */
  seed: number | null;

  addParameter: (name: string, kind?: DistributionKind) => void;
  removeParameter: (name: string) => void;
  setDistributionKind: (name: string, kind: DistributionKind) => void;
  setDistribution: (name: string, dist: Distribution) => void;
  setSampleCount: (n: number) => void;
  setSeed: (seed: number | null) => void;
}

export const useMcStudyStore = create<McStudyState>((set) => ({
  distributions: {},
  sampleCount: MC_DEFAULT_SAMPLE_COUNT,
  seed: null,

  addParameter: (name, kind = 'normal') =>
    set((s) =>
      s.distributions[name]
        ? s
        : { distributions: { ...s.distributions, [name]: defaultDistribution(kind) } },
    ),
  removeParameter: (name) =>
    set((s) => {
      if (!s.distributions[name]) return s;
      const next = { ...s.distributions } as Record<string, Distribution>;
      delete next[name];
      return { distributions: next };
    }),
  setDistributionKind: (name, kind) =>
    set((s) =>
      s.distributions[name]
        ? { distributions: { ...s.distributions, [name]: defaultDistribution(kind) } }
        : s,
    ),
  setDistribution: (name, dist) =>
    set((s) =>
      s.distributions[name] ? { distributions: { ...s.distributions, [name]: dist } } : s,
    ),
  setSampleCount: (n) => set({ sampleCount: clampSampleCount(n) }),
  setSeed: (seed) => set({ seed }),
}));

/** Selector-boundary derivations (no second source of truth). */
export function mcStudyIsValid(distributions: DistributionMap): boolean {
  const names = Object.keys(distributions);
  return names.length > 0 && names.every((n) => isDistributionValid(distributions[n]!));
}

export function mcValidityByName(distributions: DistributionMap): Record<string, boolean> {
  return Object.fromEntries(
    Object.entries(distributions).map(([n, d]) => [n, isDistributionValid(d)]),
  );
}
