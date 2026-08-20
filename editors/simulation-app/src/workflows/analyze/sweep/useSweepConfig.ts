/**
 * useSweepConfig — local UI state for the SweepWorkflow config panel.
 *
 * Owns the three pieces of user intent that make up a sweep:
 *
 *   1. A list of parameter ranges: one entry per swept parameter, each
 *      carrying a `RangeSpec` (either a `{min, max, step}` grid or an
 *      explicit `values[]` list).
 *   2. Selected outcome metric ids (what to chart / track across sweep
 *      points — sourced from MetricRegistry by the UI).
 *   3. Run mode: sequential vs parallel (affects how the batch is
 *      created; parallel is the default and fans out all children in
 *      one `sysml.batch.create` call).
 *
 * Pure local state — lives for the lifetime of the SweepWorkflow page
 * and nothing else. Deliberately NOT a Zustand store: no other surface
 * needs to observe this, and "Run Sweep" is a one-shot imperative call
 * driven by this panel.
 *
 * Consumers: SweepConfig (UI), SweepWorkflow (derived summary + run
 * button wiring). Tested in `__tests__/useSweepConfig.test.ts`.
 */

import { useCallback, useMemo, useState } from 'react';
import {
  cartesianProduct,
  expandRange,
  type ParameterRanges,
  type RangeSpec,
  type SweepPoint,
} from './cartesianProduct';

// ── Range entries ────────────────────────────────────────────────────

/**
 * One swept parameter as the UI tracks it: the element id of the
 * parameter in the model (stable across reloads), an optional human
 * label, and the user-edited `RangeSpec`.
 */
export interface ParameterRangeEntry {
  /** Element id / qualified name of the parameter. */
  parameterId: string;
  /** Display label (falls back to `parameterId` when absent). */
  label?: string;
  /** The actual range spec the user edits. */
  spec: RangeSpec;
}

/** Default spec seeded when the user picks a new parameter. */
export const DEFAULT_RANGE_SPEC: RangeSpec = { kind: 'grid', min: 0, max: 1, step: 0.25 };

// ── Run mode ─────────────────────────────────────────────────────────

/**
 * Whether to fan out all children in one batch (`parallel`) or run them
 * one at a time (`sequential`). The backend accepts both modes via the
 * same `sysml.batch.create` payload — the `run_mode` field tells the
 * scheduler whether to concurrency-limit to 1.
 */
export type SweepRunMode = 'sequential' | 'parallel';
export const DEFAULT_RUN_MODE: SweepRunMode = 'parallel';

// ── Hook surface ─────────────────────────────────────────────────────

export interface SweepConfigState {
  /** One entry per swept parameter, in user-ordered sequence. */
  ranges: ParameterRangeEntry[];
  /** Selected outcome metric ids (from MetricRegistry). */
  selectedMetricIds: string[];
  /** Sequential vs parallel child execution. */
  runMode: SweepRunMode;

  // range actions
  /** Add a parameter range (no-op if `parameterId` is already present). */
  addRange: (entry: ParameterRangeEntry) => void;
  /** Remove a parameter by id. */
  removeRange: (parameterId: string) => void;
  /** Replace the spec for an existing range (no-op if not present). */
  updateRange: (parameterId: string, spec: RangeSpec) => void;
  /** Remove every range. */
  clearRanges: () => void;

  // metric actions
  /** Toggle inclusion of a metric id. */
  toggleMetric: (metricId: string) => void;
  /** Replace the selected metric list wholesale. */
  setSelectedMetrics: (ids: readonly string[]) => void;

  // run mode
  setRunMode: (mode: SweepRunMode) => void;

  // derived
  /** True when at least one range has a valid (non-empty) value list. */
  hasRuns: boolean;
  /** Total number of child runs (product of all range lengths). */
  childCount: number;
  /** Expanded children params list (one entry per cartesian point). */
  children: SweepPoint[];
}

export interface UseSweepConfigOptions {
  initialRanges?: readonly ParameterRangeEntry[];
  initialMetrics?: readonly string[];
  initialRunMode?: SweepRunMode;
}

export function useSweepConfig(
  opts: UseSweepConfigOptions = {},
): SweepConfigState {
  const {
    initialRanges = [],
    initialMetrics = [],
    initialRunMode = DEFAULT_RUN_MODE,
  } = opts;

  const [ranges, setRanges] = useState<ParameterRangeEntry[]>(() => [...initialRanges]);
  const [selectedMetricIds, setSelectedMetricIds] = useState<string[]>(() => [...initialMetrics]);
  const [runMode, setRunMode] = useState<SweepRunMode>(initialRunMode);

  const addRange = useCallback((entry: ParameterRangeEntry) => {
    setRanges((prev) => {
      // Idempotent by parameterId — if the user re-picks the same
      // parameter, keep their existing spec rather than clobbering it
      // with the default. This matches the "additive" selection UX of
      // the Verify case picker.
      if (prev.some((e) => e.parameterId === entry.parameterId)) return prev;
      return [...prev, entry];
    });
  }, []);

  const removeRange = useCallback((parameterId: string) => {
    setRanges((prev) => prev.filter((e) => e.parameterId !== parameterId));
  }, []);

  const updateRange = useCallback((parameterId: string, spec: RangeSpec) => {
    setRanges((prev) =>
      prev.map((e) => (e.parameterId === parameterId ? { ...e, spec } : e)),
    );
  }, []);

  const clearRanges = useCallback(() => {
    setRanges([]);
  }, []);

  const toggleMetric = useCallback((metricId: string) => {
    setSelectedMetricIds((prev) =>
      prev.includes(metricId) ? prev.filter((m) => m !== metricId) : [...prev, metricId],
    );
  }, []);

  const setSelectedMetrics = useCallback((ids: readonly string[]) => {
    setSelectedMetricIds([...ids]);
  }, []);

  // Materialise ranges → children every render. The work is O(∏|Ai|)
  // but the UI caps child count at 10_000 and the typical sweep is
  // well under 1_000, so recomputing every render is cheaper than
  // tracking a cached memo that has to be invalidated on every spec
  // edit.
  const { children, hasRuns, childCount } = useMemo(() => {
    const paramRanges: ParameterRanges = {};
    for (const r of ranges) {
      paramRanges[r.parameterId] = expandRange(r.spec);
    }
    const pts = cartesianProduct(paramRanges);
    // `hasRuns` excludes the vacuous `[{}]` product (no ranges edited
    // yet). A sweep with no ranges is indistinguishable from "no sweep"
    // so the Run button stays disabled until the user adds at least
    // one parameter.
    const hasAny = ranges.length > 0 && pts.length > 0;
    return { children: pts, hasRuns: hasAny, childCount: hasAny ? pts.length : 0 };
  }, [ranges]);

  return {
    ranges,
    selectedMetricIds,
    runMode,
    addRange,
    removeRange,
    updateRange,
    clearRanges,
    toggleMetric,
    setSelectedMetrics,
    setRunMode,
    hasRuns,
    childCount,
    children,
  };
}

// ── Pure helpers (exported for tests) ────────────────────────────────

/**
 * Build the `children_params` payload expected by `sysml.batch.create`
 * from a set of parameter range entries. Thin wrapper around
 * `cartesianProduct` + `expandRange` — exposed so tests can assert the
 * cartesian expansion without mounting the hook.
 *
 *   generateChildrenParams([
 *     { parameterId: 'a', spec: { kind: 'list', values: [1, 2] } },
 *     { parameterId: 'b', spec: { kind: 'list', values: [10, 20] } },
 *   ])
 *   → [
 *       { a: 1, b: 10 }, { a: 1, b: 20 },
 *       { a: 2, b: 10 }, { a: 2, b: 20 },
 *     ]
 *
 * Returns `[]` when `entries` is empty (no vacuous `[{}]` — the UI
 * treats "no ranges" as "nothing to sweep" and disables Run).
 */
export function generateChildrenParams(
  entries: readonly ParameterRangeEntry[],
): SweepPoint[] {
  if (entries.length === 0) return [];
  const paramRanges: ParameterRanges = {};
  for (const r of entries) {
    paramRanges[r.parameterId] = expandRange(r.spec);
  }
  const pts = cartesianProduct(paramRanges);
  // The cartesian helper returns `[{}]` for an all-empty map, which we
  // already guarded above. Any zero-length range collapses to `[]` —
  // pass that through unchanged.
  return pts;
}
