/**
 * requirementsSelectionStore — the argument-free channel between the
 * requirements table (grid + document modes write the selection) and
 * the links rail context (reads it). Same F15 pattern as Verify's
 * `verifySelectionStore`: rail `render()` takes no args, the panel
 * reads its own store.
 */

import { create } from 'zustand';
import type { RequirementRow } from '@/features/requirements/types';

interface RequirementsSelectionState {
  selectedRow: RequirementRow | null;
  setSelectedRow: (row: RequirementRow | null) => void;
}

export const useRequirementsSelectionStore = create<RequirementsSelectionState>(
  (set) => ({
    selectedRow: null,
    setSelectedRow: (selectedRow) => set({ selectedRow }),
  }),
);
