/**
 * useSweepStudyStore — shared sweep configuration for the flag-on
 * ninebar Analyze surface (Phase 5).
 *
 * `useSweepConfig` (the legacy hook) is deliberately component-local —
 * fine when config and results share one two-column body. The ninebar
 * recomposition splits the config act across three hosts that render in
 * different subtrees (the "Configure sweep" MODAL, the left-rail factor
 * summary, and the workflow body that owns the runner), so the study
 * definition graduates to a store — the same reasoning that gave
 * ConfigureRunModal store-backed fields ("closing the modal is enough",
 * no apply step, no prop threading through the modal registry).
 *
 * Pure range math is NOT re-implemented: expansion/cartesian logic stays
 * in `cartesianProduct.ts` / `generateChildrenParams` (one home).
 */

import { create } from 'zustand';
import type { ParameterRangeEntry, SweepRunMode } from './useSweepConfig';
import { DEFAULT_RUN_MODE, generateChildrenParams } from './useSweepConfig';
import type { RangeSpec, SweepPoint } from './cartesianProduct';

/** Backend bulk-step cap (MAX_BULK_STEP_TICKS) — clamp the horizon to it. */
export const MAX_HORIZON_TICKS = 20_000;
export const DEFAULT_HORIZON_TICKS = 1_000;

/**
 * Simulation time step, ms. Model time covered by a child is
 * `horizonTicks * dtMs`, so this is half of what decides whether a study sees
 * a model's behaviour at all — 20,000 ticks is 20 s at the 1 ms default and
 * 2,000 s at 100 ms, and only one of those reaches the end of a slow thermal
 * transient.
 */
export const DEFAULT_DT_MS = 1;

/**
 * Where the last batch id is remembered.
 *
 * A sweep's RESULTS lived only in `useSweepRunner`'s React state, inside a
 * component the router unmounts the moment you leave `/analyze/sweep`. Coming
 * back gave you an empty workflow — while the batch, its children, their
 * outcomes and their traces were all still sitting on the backend, which keeps
 * batches for the life of the process.
 *
 * The `batch_id` was the only key to any of it, and it died with the
 * component. There is no `batch.list` command, so a lost id makes a live batch
 * permanently unreachable. Persisting the id is what makes leaving the page
 * survivable; `localStorage` (not module state) so a reload works too.
 */
const LAST_BATCH_ID_KEY = 'sysml.sweep.lastBatchId';

export function readLastBatchId(): string | null {
  if (typeof window === 'undefined') return null;
  try {
    const raw = window.localStorage.getItem(LAST_BATCH_ID_KEY);
    return raw && raw.length > 0 ? raw : null;
  } catch {
    return null;
  }
}

export function writeLastBatchId(batchId: string | null): void {
  if (typeof window === 'undefined') return;
  try {
    if (batchId == null || batchId.length === 0) {
      window.localStorage.removeItem(LAST_BATCH_ID_KEY);
    } else {
      window.localStorage.setItem(LAST_BATCH_ID_KEY, batchId);
    }
  } catch {
    // Storage full / blocked — the run still works for this tab's lifetime.
  }
}

interface SweepStudyState {
  /** One entry per swept parameter, in user-ordered sequence. */
  ranges: ParameterRangeEntry[];
  /** Selected outcome metric ids (from MetricRegistry). */
  selectedMetricIds: string[];
  /** Sequential vs parallel child execution. */
  runMode: SweepRunMode;
  /**
   * Ticks each child runs before verification + stop — the study
   * horizon. Model time = horizonTicks × dtMs.
   */
  horizonTicks: number;
  /** Simulation time step in ms applied to every child. */
  dtMs: number;

  addRange: (entry: ParameterRangeEntry) => void;
  removeRange: (parameterId: string) => void;
  updateRange: (parameterId: string, spec: RangeSpec) => void;
  clearRanges: () => void;
  toggleMetric: (metricId: string) => void;
  setRunMode: (mode: SweepRunMode) => void;
  setHorizonTicks: (ticks: number) => void;
  setDtMs: (dtMs: number) => void;
}

/**
 * Where the study definition is remembered.
 *
 * The batch id above brings the RESULTS back; this brings back the study that
 * produced them. Restoring one without the other is worse than restoring
 * neither: the left rail would report "0 factors, 0 combinations" beside a
 * full table of evaluated results, which is a surface contradicting itself.
 *
 * Same hand-rolled `localStorage` shape as `workspaceRoot` — this codebase
 * does not use zustand's persist middleware.
 */
const STUDY_KEY = 'sysml.sweep.study';

/** The persisted half of the store — definition only, never derived counts. */
interface PersistedStudy {
  ranges: ParameterRangeEntry[];
  selectedMetricIds: string[];
  runMode: SweepRunMode;
  horizonTicks: number;
  dtMs: number;
}

function readStudy(): Partial<PersistedStudy> {
  if (typeof window === 'undefined') return {};
  try {
    const raw = window.localStorage.getItem(STUDY_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw) as unknown;
    // Anything unrecognisable is discarded rather than trusted: a shape from
    // an older build must not resurrect as a half-valid study.
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) return {};
    const p = parsed as Partial<PersistedStudy>;
    return {
      ranges: Array.isArray(p.ranges) ? p.ranges : undefined,
      selectedMetricIds: Array.isArray(p.selectedMetricIds) ? p.selectedMetricIds : undefined,
      runMode: p.runMode === 'sequential' || p.runMode === 'parallel' ? p.runMode : undefined,
      horizonTicks: typeof p.horizonTicks === 'number' ? p.horizonTicks : undefined,
      dtMs: typeof p.dtMs === 'number' ? p.dtMs : undefined,
    };
  } catch {
    return {};
  }
}

function writeStudy(study: PersistedStudy): void {
  if (typeof window === 'undefined') return;
  try {
    window.localStorage.setItem(
      STUDY_KEY,
      JSON.stringify({
        ranges: study.ranges,
        selectedMetricIds: study.selectedMetricIds,
        runMode: study.runMode,
        horizonTicks: study.horizonTicks,
        dtMs: study.dtMs,
      } satisfies PersistedStudy),
    );
  } catch {
    // Storage full / blocked — the study still works for this tab's lifetime.
  }
}

const restored = readStudy();

export const useSweepStudyStore = create<SweepStudyState>((set) => ({
  ranges: restored.ranges ?? [],
  selectedMetricIds: restored.selectedMetricIds ?? [],
  runMode: restored.runMode ?? DEFAULT_RUN_MODE,
  horizonTicks: restored.horizonTicks ?? DEFAULT_HORIZON_TICKS,
  dtMs: restored.dtMs ?? DEFAULT_DT_MS,

  addRange: (entry) =>
    set((s) =>
      // Idempotent by parameterId — re-picking keeps the edited spec
      // (matches `useSweepConfig.addRange`).
      s.ranges.some((e) => e.parameterId === entry.parameterId)
        ? s
        : { ranges: [...s.ranges, entry] },
    ),
  removeRange: (parameterId) =>
    set((s) => ({ ranges: s.ranges.filter((e) => e.parameterId !== parameterId) })),
  updateRange: (parameterId, spec) =>
    set((s) => ({
      ranges: s.ranges.map((e) => (e.parameterId === parameterId ? { ...e, spec } : e)),
    })),
  clearRanges: () => set({ ranges: [] }),
  toggleMetric: (metricId) =>
    set((s) => ({
      selectedMetricIds: s.selectedMetricIds.includes(metricId)
        ? s.selectedMetricIds.filter((m) => m !== metricId)
        : [...s.selectedMetricIds, metricId],
    })),
  setRunMode: (runMode) => set({ runMode }),
  setHorizonTicks: (ticks) =>
    set({
      horizonTicks: Number.isFinite(ticks)
        ? Math.min(MAX_HORIZON_TICKS, Math.max(1, Math.round(ticks)))
        : DEFAULT_HORIZON_TICKS,
    }),
  setDtMs: (dtMs) =>
    set({
      // No upper clamp: a coarse step is a legitimate choice for a slow
      // model, and the solver's own stability is the real constraint. A
      // non-positive step is not, so it falls back rather than dividing by
      // zero downstream.
      dtMs: Number.isFinite(dtMs) && dtMs > 0 ? dtMs : DEFAULT_DT_MS,
    }),
}));

/**
 * Expand the store's ranges into per-child parameter assignments.
 * Selector-boundary derivation (no second source of truth in the store);
 * callers memo on `ranges` identity.
 */
export function expandStudyChildren(ranges: readonly ParameterRangeEntry[]): SweepPoint[] {
  return generateChildrenParams(ranges);
}

// Persist on ANY change rather than inside each setter: a setter added later
// cannot forget to do it, which is how half-persisted stores drift.
useSweepStudyStore.subscribe((s) => {
  writeStudy({
    ranges: s.ranges,
    selectedMetricIds: s.selectedMetricIds,
    runMode: s.runMode,
    horizonTicks: s.horizonTicks,
    dtMs: s.dtMs,
  });
});
