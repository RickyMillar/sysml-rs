/**
 * usePlotSelectionStore — per-session plot variable selection.
 *
 * Kept separate from `useSessionStore` so independent agents can evolve
 * session-level state without colliding with plot UX. Selection is
 * keyed by `(activeSessionId, variable_name)`; switching to a different
 * session yields an independent selection set (defaulting to empty).
 *
 * Default state is empty so the user explicitly opts-in to the variables
 * they want to chart — surfaces the picker as the obvious next action.
 */

import { create } from 'zustand';

interface PlotSelectionState {
  /** sessionId -> set of selected variable names. */
  selectionsBySession: Record<string, string[]>;

  /** Returns the (deduplicated, stable) selection list for a session. */
  getSelected: (sessionId: string | null) => string[];

  /** Replace the entire selection list for a session. */
  setSelected: (sessionId: string, names: string[]) => void;

  /** Toggle a single variable's selection in a session. */
  toggleSelected: (sessionId: string, name: string) => void;

  /** Add multiple variables at once (idempotent — duplicates removed). */
  addMany: (sessionId: string, names: string[]) => void;

  /** Remove all selected variables for a session. */
  clear: (sessionId: string) => void;

  /** Drop selection state for a session entirely (e.g. on session delete). */
  resetSession: (sessionId: string) => void;
}

export const usePlotSelectionStore = create<PlotSelectionState>((set, get) => ({
  selectionsBySession: {},

  getSelected: (sessionId) => {
    if (!sessionId) return [];
    return get().selectionsBySession[sessionId] ?? [];
  },

  setSelected: (sessionId, names) =>
    set((s) => ({
      selectionsBySession: {
        ...s.selectionsBySession,
        [sessionId]: Array.from(new Set(names)),
      },
    })),

  toggleSelected: (sessionId, name) =>
    set((s) => {
      const current = s.selectionsBySession[sessionId] ?? [];
      const next = current.includes(name)
        ? current.filter((n) => n !== name)
        : [...current, name];
      return {
        selectionsBySession: {
          ...s.selectionsBySession,
          [sessionId]: next,
        },
      };
    }),

  addMany: (sessionId, names) =>
    set((s) => {
      const current = s.selectionsBySession[sessionId] ?? [];
      const merged = Array.from(new Set([...current, ...names]));
      return {
        selectionsBySession: {
          ...s.selectionsBySession,
          [sessionId]: merged,
        },
      };
    }),

  clear: (sessionId) =>
    set((s) => ({
      selectionsBySession: {
        ...s.selectionsBySession,
        [sessionId]: [],
      },
    })),

  resetSession: (sessionId) =>
    set((s) => {
      const next = { ...s.selectionsBySession };
      delete next[sessionId];
      return { selectionsBySession: next };
    }),
}));

/** Domain classification used by the picker grouping. */
export type VariableDomain =
  | 'electrical'
  | 'thermal'
  | 'mechanical'
  | 'protection'
  | 'signal'
  | 'other';

/**
 * Heuristic classifier — mirrors the colour-guess in PlotsTab but
 * extends to all six domains. Pure function so the picker can re-use it.
 */
export function classifyVariableDomain(name: string): VariableDomain {
  const lower = name.toLowerCase();
  if (
    lower.includes('current') ||
    lower.includes('voltage') ||
    lower.startsWith('v_') ||
    lower.startsWith('i_') ||
    lower.includes('power') ||
    lower.includes('volt') ||
    lower.includes('amp')
  ) {
    return 'electrical';
  }
  if (
    lower.startsWith('t_') ||
    lower.includes('temp') ||
    lower.includes('thermal') ||
    lower.includes('heat')
  ) {
    return 'thermal';
  }
  if (
    lower.includes('trip') ||
    lower.includes('protect') ||
    lower.includes('fault') ||
    lower.includes('alarm')
  ) {
    return 'protection';
  }
  if (
    lower.includes('vel') ||
    lower.includes('accel') ||
    lower.includes('pos') ||
    lower.includes('force') ||
    lower.includes('torque') ||
    lower.includes('mass')
  ) {
    return 'mechanical';
  }
  if (
    lower.includes('signal') ||
    lower.startsWith('sig_') ||
    lower.includes('cmd') ||
    lower.includes('enable')
  ) {
    return 'signal';
  }
  return 'other';
}

export const DOMAIN_LABELS: Record<VariableDomain, string> = {
  electrical: 'Electrical',
  thermal: 'Thermal',
  mechanical: 'Mechanical',
  protection: 'Protection',
  signal: 'Signal',
  other: 'Other',
};

export const DOMAIN_ORDER: VariableDomain[] = [
  'electrical',
  'thermal',
  'mechanical',
  'protection',
  'signal',
  'other',
];
