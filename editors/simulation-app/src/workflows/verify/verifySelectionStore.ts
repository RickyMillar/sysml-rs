/**
 * verifySelectionStore — the matrix's selected-verdict echo (design 1a).
 *
 * The Verify right rail is dead — the case document is the one detail
 * surface. What survives is a single-click SELECTION on the matrix
 * ("chip click selects · double-click opens the case"): this tiny store is
 * the channel the matrix writes and reads to paint the amber selection echo
 * on the chosen cell. (Historically it also fed the now-deleted right-rail
 * verdict-detail context.)
 *
 * Deliberately verify-scoped and session-agnostic — a verdict already
 * carries its own `evidence` (session + tick + element), so there is no
 * sessionId to key on here.
 */

import { create } from 'zustand';
import type { Verdict } from '@/engine/types';

interface VerifySelectionState {
  /** The verdict whose detail the right rail should show, or null. */
  selectedVerdict: Verdict | null;
  /** Set the selection (matrix cell / history marker click). */
  select: (verdict: Verdict) => void;
  /** Clear the selection (e.g. on a fresh run or workflow unmount). */
  clear: () => void;
}

export const useVerifySelectionStore = create<VerifySelectionState>((set) => ({
  selectedVerdict: null,
  select: (verdict) => set({ selectedVerdict: verdict }),
  clear: () => set({ selectedVerdict: null }),
}));
