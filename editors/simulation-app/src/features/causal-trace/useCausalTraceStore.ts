/**
 * useCausalTraceStore — client-side root selection for the causal trace
 * panel (R7.1).
 *
 * A minimal Zustand store that holds the currently-selected "why did this
 * happen?" root. Other panels (pass/fail grid, breakpoints list) call
 * `setRoot(...)` when the user clicks a failing verdict or a triggered
 * breakpoint; the CausalTracePanel subscribes and queries the backend.
 *
 * Keeping this a separate store (instead of a slice on `useSessionStore`)
 * means R7.1 can ship without touching the shared session store — other
 * agents working there in parallel don't have to reconcile.
 */

import { create } from 'zustand';
import type { CausalTraceRoot } from './useCausationTrace';

interface CausalTraceStoreState {
  /**
   * Currently-selected root for the trace panel. `null` when nothing has
   * been selected; the panel shows its empty state.
   */
  root: CausalTraceRoot | null;
  /**
   * Monotonic counter bumped on every `setRoot` call (even when the next
   * root is structurally identical to the current one). Lets the panel
   * imperatively re-focus / scroll-into-view when the user re-clicks the
   * same verdict.
   */
  refocusTick: number;
  /** Replace the root. Emits a refocus tick. */
  setRoot: (root: CausalTraceRoot | null) => void;
  /** Clear the root. */
  clear: () => void;
}

export const useCausalTraceStore = create<CausalTraceStoreState>((set) => ({
  root: null,
  refocusTick: 0,
  setRoot: (root) =>
    set((state) => ({ root, refocusTick: state.refocusTick + 1 })),
  clear: () => set({ root: null }),
}));

/**
 * Ergonomic helper for consumers (pass/fail grid cells, breakpoint hit
 * rows) that don't want to import the whole hook. Equivalent to calling
 * `useCausalTraceStore.getState().setRoot(...)`.
 */
export function setCausalTraceRoot(root: CausalTraceRoot | null): void {
  useCausalTraceStore.getState().setRoot(root);
}
