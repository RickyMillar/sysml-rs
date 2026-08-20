/**
 * Workflow actor identity — WHO signs attestations and other workflow
 * events. Deliberately an explicit, user-set value (steward ruling
 * 2026-07-16): never derived from the OS username, never defaulted —
 * the backend hard-rejects blank actors, and the UI must collect the
 * identity before the first workflow write. Persisted locally (it's a
 * per-user tool preference, not model state).
 */

import { create } from 'zustand';

const STORAGE_KEY = 'sysml.workflow.actor';

function loadActor(): string | null {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    return raw && raw.trim() !== '' ? raw : null;
  } catch {
    return null;
  }
}

interface WorkflowActorState {
  actor: string | null;
  setActor: (actor: string) => void;
}

export const useWorkflowActorStore = create<WorkflowActorState>((set) => ({
  actor: typeof window === 'undefined' ? null : loadActor(),
  setActor: (actor) => {
    const trimmed = actor.trim();
    if (trimmed === '') return;
    try {
      window.localStorage.setItem(STORAGE_KEY, trimmed);
    } catch {
      // Persistence is best-effort; the in-memory value still applies.
    }
    set({ actor: trimmed });
  },
}));
