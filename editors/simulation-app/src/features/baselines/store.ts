/**
 * Selected-baseline UI state — which baseline suspect flags are computed
 * against. Session-scoped (not persisted): the in-memory backend store
 * means baseline names are only meaningful within a server lifetime.
 */

import { create } from 'zustand';

interface BaselineUIState {
  /** Selected baseline name, or null when none picked / none exist. */
  selected: string | null;
  setSelected: (name: string | null) => void;
}

export const useBaselineStore = create<BaselineUIState>((set) => ({
  selected: null,
  setSelected: (name) => set({ selected: name }),
}));
