/**
 * useCompareStore — Zustand slice for R4.2 CompareWorkflow local state.
 *
 * This is the new multi-session compare store. It is intentionally
 * separate from the legacy single-pair `compareBaseline` slot on the
 * session store, so the two can coexist during the migration.
 *
 * Holds:
 *   - `pickedSessionIds` — 2..6 session IDs chosen in the picker
 *   - `sharedTick` — the current playhead tick (0-based, clamped to max)
 *   - `isPlaying` — whether auto-advance is running
 *   - `layout` — overlay | side-by-side
 *   - `activeModeId` — which `CompareMode` (ensemble / golden / two-design)
 *     is currently focused in the right-rail config slot
 *   - `pickedVariables` — optional manual variable filter. `null` means
 *     "auto" (selectors pick top-N by cross-session variance).
 *
 * No remote data lives here — waveforms / diagrams read from the session
 * detail + time-series stores as usual.
 */

import { create } from 'zustand';

export type CompareLayout = 'overlay' | 'side-by-side';

export interface CompareStoreState {
  // ── Picked sessions ─────────────────────────────────────────────
  pickedSessionIds: string[];
  setPickedSessionIds: (ids: string[]) => void;
  togglePickedSession: (id: string) => void;

  // ── Shared playhead ─────────────────────────────────────────────
  sharedTick: number;
  setSharedTick: (tick: number) => void;
  isPlaying: boolean;
  setIsPlaying: (playing: boolean) => void;

  // ── Layout ──────────────────────────────────────────────────────
  /**
   * User-chosen layout, or `null` for "auto" (picker count drives the
   * default: ≤3 → overlay, ≥4 → side-by-side).
   */
  layout: CompareLayout | null;
  setLayout: (layout: CompareLayout | null) => void;

  // ── Mode ────────────────────────────────────────────────────────
  activeModeId: string | null;
  setActiveModeId: (id: string | null) => void;

  // ── Variable picker ─────────────────────────────────────────────
  /** Null = auto-pick (top-N by cross-session variance). */
  pickedVariables: string[] | null;
  setPickedVariables: (vars: string[] | null) => void;

  // ── Golden mode (Phase 6, plan row 24) ──────────────────────────
  /** Archived (golden-pinned) run picked as the reference, by archive
   *  entry id. Null = none picked yet. */
  goldenArchiveId: string | null;
  setGoldenArchiveId: (id: string | null) => void;
  /** Relative tolerance for the golden comparison (fraction, default
   *  0.05 = 5% — matches modes/golden DEFAULT_TOLERANCE). */
  goldenToleranceRel: number;
  setGoldenToleranceRel: (v: number) => void;
}

/** Clamp the picked set to the 2..6 window, trimming from the tail. */
export function clampPicks(ids: string[]): string[] {
  const deduped: string[] = [];
  const seen = new Set<string>();
  for (const id of ids) {
    if (seen.has(id)) continue;
    seen.add(id);
    deduped.push(id);
    if (deduped.length >= 6) break;
  }
  return deduped;
}

export const useCompareStore = create<CompareStoreState>((set) => ({
  pickedSessionIds: [],
  setPickedSessionIds: (ids) => set({ pickedSessionIds: clampPicks(ids) }),
  togglePickedSession: (id) =>
    set((s) => {
      const has = s.pickedSessionIds.includes(id);
      const next = has
        ? s.pickedSessionIds.filter((x) => x !== id)
        : clampPicks([...s.pickedSessionIds, id]);
      return { pickedSessionIds: next };
    }),

  sharedTick: 0,
  setSharedTick: (tick) => {
    const t = Math.max(0, Math.floor(Number.isFinite(tick) ? tick : 0));
    set({ sharedTick: t });
  },
  isPlaying: false,
  setIsPlaying: (playing) => set({ isPlaying: playing }),

  layout: null,
  setLayout: (layout) => set({ layout }),

  activeModeId: null,
  setActiveModeId: (id) => set({ activeModeId: id }),

  pickedVariables: null,
  setPickedVariables: (vars) => set({ pickedVariables: vars }),

  goldenArchiveId: null,
  setGoldenArchiveId: (id) => set({ goldenArchiveId: id }),
  goldenToleranceRel: 0.05,
  setGoldenToleranceRel: (v) =>
    set({ goldenToleranceRel: Number.isFinite(v) && v >= 0 ? v : 0.05 }),
}));

/**
 * Resolve the effective layout from the user choice + pick count.
 * `null` on the state = auto: overlay for ≤3 picks, side-by-side for ≥4.
 */
export function resolveLayout(
  userChoice: CompareLayout | null,
  pickCount: number,
): CompareLayout {
  if (userChoice) return userChoice;
  return pickCount >= 4 ? 'side-by-side' : 'overlay';
}
