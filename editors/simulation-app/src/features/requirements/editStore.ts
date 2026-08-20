/**
 * Requirement edit state — the pending/confirmed/failed cell machinery
 * (workbench design §7.5). ONE edit in flight is BINDING: spans are only
 * valid against the last parse, so while `pendingKey` is set, edit entry
 * everywhere else is refused (`beginEdit` returns false).
 *
 * "Confirmed" has no stored state — quiet is the confirmation: the
 * pending badge clears when the commit lands and the refetched row shows
 * the value. `failed` persists per-cell until the user re-enters edit.
 */

import { create } from 'zustand';

/** One key per editable cell: `${elementId}:${field}`. Create rows key on
 *  the parent: `${parentId}:create`. */
export const cellKey = (elementId: string, field: string): string =>
  `${elementId}:${field}`;

interface RequirementEditState {
  /** Cell with an open (uncommitted) editor — at most one. */
  editingKey: string | null;
  /** Cell whose commit is in flight — at most one, gates everything. */
  pendingKey: string | null;
  /** Last failed commit; cleared by re-entering edit on that cell. */
  failed: { key: string; message: string } | null;
  /** Enter edit mode. Returns false (refused) while a commit is pending. */
  beginEdit: (key: string) => boolean;
  cancelEdit: () => void;
  /** Move the open editor into the in-flight state. */
  markPending: (key: string) => void;
  /** Commit landed — clear pending (the refetched row is the confirmation). */
  markConfirmed: (key: string) => void;
  markFailed: (key: string, message: string) => void;
}

export const useRequirementEditStore = create<RequirementEditState>((set, get) => ({
  editingKey: null,
  pendingKey: null,
  failed: null,
  beginEdit: (key) => {
    if (get().pendingKey !== null) return false;
    set((state) => ({
      editingKey: key,
      // Re-entering edit on a failed cell clears its error.
      failed: state.failed?.key === key ? null : state.failed,
    }));
    return true;
  },
  cancelEdit: () => set({ editingKey: null }),
  markPending: (key) => set({ editingKey: null, pendingKey: key, failed: null }),
  markConfirmed: (key) =>
    set((state) => (state.pendingKey === key ? { pendingKey: null } : {})),
  markFailed: (key, message) =>
    set((state) => ({
      failed: { key, message },
      ...(state.pendingKey === key ? { pendingKey: null } : {}),
    })),
}));
