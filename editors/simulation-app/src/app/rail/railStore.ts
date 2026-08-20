/**
 * useRightRailStore — the slide-in right-rail host state (ninebar Phase 1).
 *
 * ARCHITECTURAL RULE (F15, decided 2026-07-14): the rail context API takes
 * NO sessionId. Contexts are active-session only by construction — they
 * consume active-session hooks (`useVar`, `useTick`, `useStreamPhase`, …
 * from `sessionLiveStore`), never a session id threaded through this
 * store. Rail pinning therefore *follows the active session*; it does
 * NOT freeze to the session it was pinned under — if the active session
 * changes, a pinned context re-renders against the new session's live
 * data rather than showing stale data from the session it was opened
 * against. Re-keying by sessionId (so a pin genuinely freezes to one
 * session) is deferred to Phase 6 (Compare), the first surface that
 * genuinely needs two independent live mirrors — do not add a sessionId
 * parameter to `open`/`pin` ahead of that consumer (plan v1.3 audit F15).
 *
 * Slot rules (audit F3 — at most two stacked contexts, one pinned + one
 * transient/selection-driven):
 *   - Empty by default on a fresh load: `pinned` and `transient` both
 *     start `null`. There are only ever two fields, so "max two stacked"
 *     is enforced structurally, not by a length check.
 *   - `open(contextId)` sets the transient slot, replacing whatever was
 *     transient before (a new selection swaps the inspector, it never
 *     stacks a third context). Opening the context that is already
 *     pinned is a no-op — it is already visible.
 *   - `pin(contextId)` promotes a context into the pinned slot,
 *     replacing any previously pinned context. If `contextId` is the
 *     current transient, the transient slot is cleared as part of the
 *     same update (the context *moved* into the pinned slot, it isn't
 *     duplicated into both) — this is the normal "pin the open context"
 *     flow driven by the rail header's pin toggle. If `contextId` is not
 *     the current transient, it becomes pinned outright and the
 *     transient slot is left untouched.
 *   - `unpin()` clears the pinned slot only; the transient slot (if any)
 *     is unaffected.
 *   - `close()` clears the transient slot only; a pinned context stays
 *     open until explicitly unpinned — closing the selection-driven
 *     panel must never silently drop a pinned one.
 */
import { create } from 'zustand';

interface RightRailState {
  /** The pinned context id, or `null` when nothing is pinned. */
  pinned: string | null;
  /** The transient (selection-driven) context id, or `null`. */
  transient: string | null;

  /** Open `contextId` as the transient context, replacing any previous transient. */
  open: (contextId: string) => void;
  /** Close the transient context. The pinned context (if any) is untouched. */
  close: () => void;
  /** Promote `contextId` into the pinned slot, replacing any previous pin. */
  pin: (contextId: string) => void;
  /** Clear the pinned slot. The transient context (if any) is untouched. */
  unpin: () => void;
}

export const useRightRailStore = create<RightRailState>((set, get) => ({
  pinned: null,
  transient: null,

  open: (contextId) => {
    if (get().pinned === contextId) return; // already visible, pinned
    set({ transient: contextId });
  },

  close: () => set({ transient: null }),

  pin: (contextId) => {
    const { transient } = get();
    set({
      pinned: contextId,
      transient: transient === contextId ? null : transient,
    });
  },

  unpin: () => set({ pinned: null }),
}));
