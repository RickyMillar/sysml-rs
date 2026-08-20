/**
 * useMonteCarloConfig — local UI state for the MonteCarloWorkflow config
 * panel.
 *
 * Owns four pieces of user intent:
 *
 *   1. Which parameters to sample (per-name selection).
 *   2. The Distribution attached to each selected parameter.
 *   3. `sampleCount` — how many child runs to fan out (default 1000,
 *      capped at `MAX_SAMPLE_COUNT` for memory sanity).
 *   4. `seed` — optional integer for reproducibility. `null` means
 *      "let the backend pick one".
 *
 * Design mirrors `useVerifyConfig`: a single `useState`-backed hook
 * with a flat action surface. Deliberately not a Zustand store — no
 * other view observes this, and "Run Monte Carlo" is a one-shot
 * imperative call driven from this panel.
 *
 * Test coverage: `__tests__/useMonteCarloConfig.test.ts`.
 */

import { useCallback, useMemo, useState } from 'react';
import {
  defaultDistribution,
  isDistributionValid,
  type Distribution,
  type DistributionKind,
  type DistributionMap,
} from './sampleDistribution';

// ── Limits / defaults ───────────────────────────────────────────────

/**
 * Hard ceiling on sample count. The previous ceiling (10k) matched the
 * conceptual ask but ignored `quota_for(SessionKind::Orchestrator) = 20`
 * in `crates/tooling/sysml-service/src/execution.rs` — which is the
 * actual backend bucket each child occupies. When the requested count
 * exceeds the cap, `batch.create` rolls back and the run surfaces a
 * "bucket full" error. Until the backend gains a pooled executor (run
 * N children sequentially under one slot) we ship an honest max that
 * matches the bucket and a smaller default so the default action works
 * against a freshly-started backend. Track BUG 25 for the real fix.
 */
export const MAX_SAMPLE_COUNT = 20;
export const DEFAULT_SAMPLE_COUNT = 10;

// ── Public state shape ──────────────────────────────────────────────

export interface MonteCarloConfigState {
  /** Name → Distribution for every selected parameter, in insertion order. */
  distributions: DistributionMap;
  /** Number of child runs to fan out. Clamped to `[1, MAX_SAMPLE_COUNT]`. */
  sampleCount: number;
  /** Optional seed. `null` → backend picks. */
  seed: number | null;

  // ─ Parameter selection ─
  /** Add a parameter name with its default Distribution (`normal(0,1)`). */
  addParameter: (name: string, kind?: DistributionKind) => void;
  /** Remove a parameter by name (no-op if not present). */
  removeParameter: (name: string) => void;
  /** Bulk-sync selection: ensures `names` are selected; drops anything extra. */
  setParameters: (names: readonly string[]) => void;
  /** True when `name` is currently selected. */
  hasParameter: (name: string) => boolean;

  // ─ Distribution edits ─
  /** Swap the distribution kind for an existing parameter (resets params to defaults). */
  setDistributionKind: (name: string, kind: DistributionKind) => void;
  /** Replace the full Distribution for a parameter. */
  setDistribution: (name: string, dist: Distribution) => void;

  // ─ Sample count / seed ─
  setSampleCount: (n: number) => void;
  setSeed: (seed: number | null) => void;

  // ─ Derived ─
  /** List of selected parameter names in insertion order. */
  parameterNames: readonly string[];
  /** Count of selected parameters. */
  parameterCount: number;
  /** True when at least one parameter is selected and every distribution validates. */
  isValid: boolean;
  /** True when at least one parameter is selected. */
  hasParameters: boolean;
  /** Map of param → validity (for per-row error flags in the editor). */
  validityByName: Readonly<Record<string, boolean>>;
}

export interface UseMonteCarloConfigOptions {
  /** Seed the initial parameters + distributions (useful for tests / drilldown). */
  initialDistributions?: DistributionMap;
  /** Initial sample count (clamped). */
  initialSampleCount?: number;
  /** Initial seed. */
  initialSeed?: number | null;
}

// ── Hook ────────────────────────────────────────────────────────────

export function useMonteCarloConfig(
  opts: UseMonteCarloConfigOptions = {},
): MonteCarloConfigState {
  const {
    initialDistributions = {},
    initialSampleCount = DEFAULT_SAMPLE_COUNT,
    initialSeed = null,
  } = opts;

  const [distributions, setDistributions] = useState<DistributionMap>(
    () => ({ ...initialDistributions }),
  );
  const [sampleCount, setSampleCountRaw] = useState<number>(() =>
    clampSampleCount(initialSampleCount),
  );
  const [seed, setSeed] = useState<number | null>(initialSeed);

  // ─ Parameter selection ─

  const addParameter = useCallback(
    (name: string, kind: DistributionKind = 'normal') => {
      if (!name) return;
      setDistributions((prev) => {
        if (name in prev) return prev;
        return { ...prev, [name]: defaultDistribution(kind) };
      });
    },
    [],
  );

  const removeParameter = useCallback((name: string) => {
    setDistributions((prev) => {
      if (!(name in prev)) return prev;
      const next = { ...prev };
      delete next[name];
      return next;
    });
  }, []);

  const setParameters = useCallback((names: readonly string[]) => {
    setDistributions((prev) => {
      const want = new Set(names);
      const next: Record<string, Distribution> = {};
      // Keep existing entries that are still selected, in the order given.
      for (const n of names) {
        next[n] = prev[n] ?? defaultDistribution('normal');
      }
      // Drop anything not in `want` — implicit by starting fresh.
      // Bail on no-op so callers relying on identity don't re-render.
      if (
        Object.keys(prev).length === names.length &&
        names.every((n) => n in prev) &&
        Object.keys(prev).every((n) => want.has(n))
      ) {
        return prev;
      }
      return next;
    });
  }, []);

  // ─ Distribution edits ─

  const setDistributionKind = useCallback(
    (name: string, kind: DistributionKind) => {
      setDistributions((prev) => {
        if (!(name in prev)) return prev;
        if (prev[name].kind === kind) return prev;
        return { ...prev, [name]: defaultDistribution(kind) };
      });
    },
    [],
  );

  const setDistribution = useCallback((name: string, dist: Distribution) => {
    setDistributions((prev) => {
      if (!(name in prev)) return prev;
      return { ...prev, [name]: dist };
    });
  }, []);

  const setSampleCount = useCallback((n: number) => {
    setSampleCountRaw(clampSampleCount(n));
  }, []);

  // ─ Derived ─

  const parameterNames = useMemo(() => Object.keys(distributions), [distributions]);
  const parameterCount = parameterNames.length;
  const hasParameters = parameterCount > 0;

  const validityByName = useMemo<Readonly<Record<string, boolean>>>(() => {
    const out: Record<string, boolean> = {};
    for (const name of parameterNames) {
      out[name] = isDistributionValid(distributions[name]);
    }
    return out;
  }, [distributions, parameterNames]);

  const isValid = useMemo(
    () =>
      hasParameters &&
      sampleCount >= 1 &&
      sampleCount <= MAX_SAMPLE_COUNT &&
      parameterNames.every((n) => validityByName[n]),
    [hasParameters, sampleCount, parameterNames, validityByName],
  );

  const hasParameter = useCallback(
    (name: string) => name in distributions,
    [distributions],
  );

  return {
    distributions,
    sampleCount,
    seed,
    addParameter,
    removeParameter,
    setParameters,
    hasParameter,
    setDistributionKind,
    setDistribution,
    setSampleCount,
    setSeed,
    parameterNames,
    parameterCount,
    isValid,
    hasParameters,
    validityByName,
  };
}

// ── Helpers ─────────────────────────────────────────────────────────

/** Clamp to `[1, MAX_SAMPLE_COUNT]` and coerce to an integer. */
export function clampSampleCount(n: number): number {
  if (!Number.isFinite(n)) return DEFAULT_SAMPLE_COUNT;
  const i = Math.floor(n);
  if (i < 1) return 1;
  if (i > MAX_SAMPLE_COUNT) return MAX_SAMPLE_COUNT;
  return i;
}
