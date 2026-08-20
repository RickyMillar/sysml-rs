/**
 * useSensitivityConfig — local UI state for the Sensitivity workflow
 * (R7.4).
 *
 * Owns the four pieces of user intent for a Morris / Sobol run:
 *
 *   1. method — `'morris' | 'sobol'`.
 *   2. ranges — one [`ParamRange`] per picked parameter (min / max).
 *      The UI uses the same RangeEditor pattern as SweepConfig but
 *      collapses each range to just its bounds (no grid step / list —
 *      sensitivity samples in a continuum, not a grid).
 *   3. per-method sampler config:
 *        - Morris: `r` trajectories + `p` levels.
 *        - Sobol:  `n` base samples.
 *   4. output metric — key used by `extract_child_metric` on the
 *      backend to score each completed child.
 *
 * Pure local state, no Zustand — same scoping rationale as
 * `useSweepConfig`.
 */

import { useCallback, useMemo, useState } from 'react';
import type { ParamRange, SensitivityMethod } from '@/engine/types';

// ── Defaults ────────────────────────────────────────────────────────

/** Default ParamRange seeded when the user adds a new parameter. */
export const DEFAULT_RANGE: Omit<ParamRange, 'name'> = { min: 0, max: 1 };

/** Default Morris levels — 4 is the textbook choice (Δ = 2/3). */
export const DEFAULT_MORRIS_P = 4;
/** Default Morris trajectories. 10 is enough for a screening study. */
export const DEFAULT_MORRIS_R = 10;
/** Default Sobol base sample size. 64 keeps the pure-UI preview cheap
 *  while still giving useful index estimates. */
export const DEFAULT_SOBOL_N = 64;

// ── Range entry ─────────────────────────────────────────────────────

/**
 * One sensitivity parameter as the UI tracks it: element / qualified
 * name (stable id), display label, and the editable `[min, max]` range.
 */
export interface SensitivityRangeEntry {
  parameterId: string;
  label?: string;
  min: number;
  max: number;
}

// ── Hook surface ────────────────────────────────────────────────────

export interface SensitivityConfigState {
  method: SensitivityMethod;
  ranges: SensitivityRangeEntry[];
  morrisR: number;
  morrisP: number;
  sobolN: number;
  outputMetric: string;
  seed: number;

  // method
  setMethod: (m: SensitivityMethod) => void;
  // ranges
  addRange: (entry: SensitivityRangeEntry) => void;
  removeRange: (parameterId: string) => void;
  updateRange: (
    parameterId: string,
    patch: Partial<Omit<SensitivityRangeEntry, 'parameterId'>>,
  ) => void;
  clearRanges: () => void;
  // sampler knobs
  setMorrisR: (r: number) => void;
  setMorrisP: (p: number) => void;
  setSobolN: (n: number) => void;
  // metric + seed
  setOutputMetric: (m: string) => void;
  setSeed: (s: number) => void;

  // derived
  /** True when there's at least one range and sampler config is sane. */
  isValid: boolean;
  /** Expected number of child runs. */
  childCount: number;
  /** Parameter ranges as the shape the sampler / backend expect. */
  paramRanges: ParamRange[];
}

export interface UseSensitivityConfigOptions {
  initialMethod?: SensitivityMethod;
  initialRanges?: readonly SensitivityRangeEntry[];
  initialMorrisR?: number;
  initialMorrisP?: number;
  initialSobolN?: number;
  initialOutputMetric?: string;
  initialSeed?: number;
}

export function useSensitivityConfig(
  opts: UseSensitivityConfigOptions = {},
): SensitivityConfigState {
  const [method, setMethod] = useState<SensitivityMethod>(
    opts.initialMethod ?? 'morris',
  );
  const [ranges, setRanges] = useState<SensitivityRangeEntry[]>(
    () => [...(opts.initialRanges ?? [])],
  );
  const [morrisR, setMorrisR] = useState<number>(
    opts.initialMorrisR ?? DEFAULT_MORRIS_R,
  );
  const [morrisP, setMorrisP] = useState<number>(
    opts.initialMorrisP ?? DEFAULT_MORRIS_P,
  );
  const [sobolN, setSobolN] = useState<number>(
    opts.initialSobolN ?? DEFAULT_SOBOL_N,
  );
  const [outputMetric, setOutputMetric] = useState<string>(
    opts.initialOutputMetric ?? '',
  );
  const [seed, setSeed] = useState<number>(opts.initialSeed ?? 42);

  const addRange = useCallback((entry: SensitivityRangeEntry) => {
    setRanges((prev) => {
      if (prev.some((r) => r.parameterId === entry.parameterId)) return prev;
      return [...prev, entry];
    });
  }, []);

  const removeRange = useCallback((parameterId: string) => {
    setRanges((prev) => prev.filter((r) => r.parameterId !== parameterId));
  }, []);

  const updateRange = useCallback(
    (
      parameterId: string,
      patch: Partial<Omit<SensitivityRangeEntry, 'parameterId'>>,
    ) => {
      setRanges((prev) =>
        prev.map((r) => (r.parameterId === parameterId ? { ...r, ...patch } : r)),
      );
    },
    [],
  );

  const clearRanges = useCallback(() => setRanges([]), []);

  const paramRanges = useMemo<ParamRange[]>(
    () =>
      ranges.map((r) => ({
        // Use label (human) as the `name` when available — it matches
        // the AttributeUsage name used by the backend override applier.
        name: r.label ?? r.parameterId,
        min: r.min,
        max: r.max,
      })),
    [ranges],
  );

  const childCount = useMemo(() => {
    const d = ranges.length;
    if (d === 0) return 0;
    if (method === 'morris') return morrisR * (d + 1);
    return sobolN * (d + 2);
  }, [ranges.length, method, morrisR, sobolN]);

  const isValid = useMemo(() => {
    if (ranges.length === 0) return false;
    if (!outputMetric) return false;
    for (const r of ranges) {
      if (!Number.isFinite(r.min) || !Number.isFinite(r.max)) return false;
      if (r.max <= r.min) return false;
    }
    if (method === 'morris') {
      if (morrisR < 1 || morrisP < 2) return false;
    } else if (sobolN < 1) {
      return false;
    }
    return true;
  }, [ranges, outputMetric, method, morrisR, morrisP, sobolN]);

  return {
    method,
    ranges,
    morrisR,
    morrisP,
    sobolN,
    outputMetric,
    seed,
    setMethod,
    addRange,
    removeRange,
    updateRange,
    clearRanges,
    setMorrisR,
    setMorrisP,
    setSobolN,
    setOutputMetric,
    setSeed,
    isValid,
    childCount,
    paramRanges,
  };
}
