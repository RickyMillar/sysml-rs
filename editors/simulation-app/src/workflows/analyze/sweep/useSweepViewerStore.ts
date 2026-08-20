/**
 * useSweepViewerStore — per-batch "which viewer tab is active" persistence.
 *
 * R5.3 requires the viewer switcher to remember the user's pick across
 * re-renders (tab was "Heatmap" — stay on "Heatmap" when a new polling
 * response lands). We key by `batchId` so each batch's preference is
 * independent; no persistence to disk — this is pure UI state.
 *
 * Defaults:
 *   - If a batch is new (`activeFor` returns undefined), the switcher
 *     falls back to `'sweep-table'`. Pinned here as a constant so the
 *     switcher and any test can agree on the default without duplication.
 */
import { create } from 'zustand';

/** Which viewer tab is live for a given batch. */
export type SweepViewerId =
  | 'sweep-table'
  | 'sweep-tornado'
  | 'sweep-parallel-coords'
  | 'sweep-heatmap';

export const DEFAULT_SWEEP_VIEWER: SweepViewerId = 'sweep-table';

interface SweepViewerStoreState {
  /** batchId -> active viewer id. */
  activeByBatch: Record<string, SweepViewerId>;

  /** Current viewer for a batch, defaulting to `'sweep-table'`. */
  activeFor: (batchId: string) => SweepViewerId;

  /** Persist the user's pick for a batch. */
  setActive: (batchId: string, viewer: SweepViewerId) => void;

  /** Drop state for a batch (on batch close / archive). */
  clearBatch: (batchId: string) => void;
}

export const useSweepViewerStore = create<SweepViewerStoreState>((set, get) => ({
  activeByBatch: {},

  activeFor: (batchId) => get().activeByBatch[batchId] ?? DEFAULT_SWEEP_VIEWER,

  setActive: (batchId, viewer) =>
    set((s) => ({ activeByBatch: { ...s.activeByBatch, [batchId]: viewer } })),

  clearBatch: (batchId) =>
    set((s) => {
      const next = { ...s.activeByBatch };
      delete next[batchId];
      return { activeByBatch: next };
    }),
}));
