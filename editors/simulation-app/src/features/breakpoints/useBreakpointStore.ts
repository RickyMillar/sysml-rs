/**
 * Breakpoint client-side store (R2.3).
 *
 * A Zustand-ish store that owns the local view of the user's breakpoints.
 * Lives client-side so the panel UI and the diagram overlay can share a
 * single source of truth without either consumer having to poll the
 * backend separately.
 *
 * ── Why a local store, not react-query? ───────────────────────────────
 * Breakpoints are primarily user-authored — the user adds/clears them
 * from the panel and the session reports hits. A local store keeps the
 * UI optimistic: add / clear feels instantaneous and the backend round-
 * trip happens in the background. When the backend assigns an id we
 * upsert it back into the entry.
 *
 * ── Extensibility hooks (Round 4+) ────────────────────────────────────
 * `BreakpointLocal` carries four optional fields reserved for later
 * rounds, all `undefined` today. They are stored so the same panel row
 * doesn't need to be rewritten when those features land; the backend
 * ignores them for now.
 *
 *   condition?   — R4: expression evaluated on hit
 *   hitCount?    — R4: fire every Nth time
 *   logMessage?  — R4: logpoint (log instead of pause)
 *   enabled?     — future: soft-disable without removing the entry
 *
 * The store also tracks which breakpoint was last hit (so the panel can
 * flash the row amber) and a pending-add flag (so the Add dialog can
 * render a spinner while the backend call is in flight).
 */

import { create } from 'zustand';
import type { Breakpoint, BreakpointId } from '@/engine/types';

// ── Types ────────────────────────────────────────────────────────────

/**
 * Local wrapper around the backend `Breakpoint` shape.
 *
 * The backend today accepts only the `breakpoint` field; everything else
 * is UI-only. Keeping them on the same record (rather than a parallel
 * map) keeps row rendering + extensibility trivial — the panel reads a
 * single object to draw itself.
 */
export interface BreakpointLocal {
  /** Client id. Seeded to a local value; replaced with backend id after `set`. */
  id: BreakpointId;
  /** The backend-shaped breakpoint (kind + target + kind-specific fields). */
  breakpoint: Breakpoint;
  /** R4+: condition expression evaluated when the location is hit. */
  condition?: string;
  /** R4+: fire only every Nth hit (1-based; `undefined` = every hit). */
  hitCount?: number;
  /** R4+: logpoint — log a message instead of pausing. */
  logMessage?: string;
  /** Future: soft-disable (keep record but stop arming). Defaults to true. */
  enabled?: boolean;
}

/** UI-side snapshot of a recent breakpoint-hit event. */
export interface BreakpointHit {
  id: BreakpointId;
  /** Monotonic timestamp (Date.now) so the panel can fade the flash out. */
  hitAtMs: number;
  /** Echo of the backend context payload (target, tick, ...). Free-form. */
  context?: Record<string, unknown>;
}

interface BreakpointStoreState {
  // ── Persisted ─────────────────────────────────────────────────────
  breakpoints: BreakpointLocal[];
  /** Most recently hit breakpoint (for row flash + toast). `null` when none. */
  lastHit: BreakpointHit | null;
  /** True while an Add dialog submission is in-flight. */
  isAdding: boolean;

  // ── Actions ───────────────────────────────────────────────────────
  /** Add a breakpoint locally. Returns the local id the caller can use
   *  while awaiting the backend response. */
  addLocal: (entry: Omit<BreakpointLocal, 'id'> & { id?: BreakpointId }) => BreakpointId;

  /** Replace a local id with the backend-assigned one (post `set`). */
  reconcileId: (localId: BreakpointId, backendId: BreakpointId) => void;

  /** Remove by id (either local or backend). No-op when absent. */
  remove: (id: BreakpointId) => void;

  /** Clear every breakpoint from local state. */
  clearAll: () => void;

  /** Record a hit. The panel reads `lastHit` and flashes the matching row. */
  recordHit: (hit: BreakpointHit) => void;

  /** Toggle the `isAdding` spinner flag. */
  setAdding: (flag: boolean) => void;

  /** Replace the entire breakpoint list (used by `listBreakpoints` sync). */
  replaceAll: (entries: BreakpointLocal[]) => void;

  /**
   * Merge a single local breakpoint's future-feature fields in place.
   * R4 agents will call this from their Edit dialog without having to
   * reach into the store's internals.
   */
  patch: (id: BreakpointId, patch: Partial<BreakpointLocal>) => void;
}

// ── Helpers ──────────────────────────────────────────────────────────

/** Generate a stable local id. Prefixed `local-` so it never collides
 *  with a backend id (which is an opaque UUID / snowflake). */
export function makeLocalId(): BreakpointId {
  // Prefer crypto.randomUUID when available (modern browsers + node 19+).
  const g = globalThis as unknown as { crypto?: { randomUUID?: () => string } };
  if (g.crypto?.randomUUID) return `local-${g.crypto.randomUUID()}`;
  // Fallback: timestamp + random. Collision is acceptable for local ids.
  return `local-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`;
}

/** Human-readable symbol for a `CompareOp`. Exported so the dialog,
 *  the row, and tooltips stay in lock-step. */
export function compareOpSymbol(op: import('@/engine/types').CompareOp): string {
  switch (op) {
    case 'lt': return '<';
    case 'le': return '≤';
    case 'gt': return '>';
    case 'ge': return '≥';
    case 'eq': return '=';
    case 'ne': return '≠';
  }
}

/** A compact label for a breakpoint row. Uses the explicit `label` when
 *  set, falls back to a kind-dependent descriptor. Exported for tests. */
export function breakpointLabel(bp: Breakpoint): string {
  if (bp.label) return bp.label;
  switch (bp.kind) {
    case 'state-entry':
    case 'transition-fire':
    case 'action-invoke':
    case 'constraint-violation':
      return bp.target;
    case 'threshold-crossing': {
      const arrow =
        bp.direction === 'rising' ? '↑' :
        bp.direction === 'falling' ? '↓' :
        '⇅';
      const debounce =
        typeof bp.debounce_ticks === 'number' && bp.debounce_ticks > 0
          ? ` (debounce ${bp.debounce_ticks}t)`
          : '';
      return `when \`${bp.variable}\` crosses ${arrow} ${bp.threshold}${debounce}`;
    }
    case 'conditional': {
      const scope = bp.target ? `${bp.target}.` : '';
      return `when \`${scope}${bp.variable}\` ${compareOpSymbol(bp.op)} ${bp.value}`;
    }
  }
}

/** Material-symbols icon name for a breakpoint kind. Used by rows and by
 *  the Add dialog picker. */
export function breakpointIcon(kind: Breakpoint['kind']): string {
  switch (kind) {
    case 'state-entry': return 'circle';
    case 'transition-fire': return 'arrow_forward';
    case 'action-invoke': return 'play_arrow';
    case 'constraint-violation': return 'rule';
    case 'threshold-crossing': return 'show_chart';
    case 'conditional': return 'function';
  }
}

/** Extract element targets (state / transition / action / constraint) that
 *  a breakpoint is attached to. Pure — used by the overlay to map breakpoints
 *  onto diagram node ids. */
export function breakpointTargetNode(bp: Breakpoint): string | null {
  switch (bp.kind) {
    case 'state-entry':
    case 'transition-fire':
    case 'action-invoke':
    case 'constraint-violation':
      return bp.target;
    case 'threshold-crossing':
      // Variable-based bp's don't map to a node. If the backend later
      // resolves the owning element we can surface it here.
      return null;
    case 'conditional':
      // Conditional breakpoints are element-scoped — light up the diagram
      // node that owns the watched variable.
      return bp.target;
  }
}

// ── Store ────────────────────────────────────────────────────────────

export const useBreakpointStore = create<BreakpointStoreState>((set) => ({
  breakpoints: [],
  lastHit: null,
  isAdding: false,

  addLocal: (entry) => {
    const id = entry.id ?? makeLocalId();
    const record: BreakpointLocal = {
      enabled: true,
      ...entry,
      id,
    };
    set((state) => ({ breakpoints: [...state.breakpoints, record] }));
    return id;
  },

  reconcileId: (localId, backendId) =>
    set((state) => ({
      breakpoints: state.breakpoints.map((bp) =>
        bp.id === localId
          ? { ...bp, id: backendId, breakpoint: { ...bp.breakpoint, id: backendId } }
          : bp,
      ),
    })),

  remove: (id) =>
    set((state) => ({
      breakpoints: state.breakpoints.filter((bp) => bp.id !== id),
      // Don't touch lastHit — the flash naturally fades out once the row
      // is gone. If the removed row was the hit one we clear that to avoid
      // a dangling reference.
      lastHit: state.lastHit?.id === id ? null : state.lastHit,
    })),

  clearAll: () => set({ breakpoints: [], lastHit: null }),

  recordHit: (hit) => set({ lastHit: hit }),

  setAdding: (flag) => set({ isAdding: flag }),

  replaceAll: (entries) => set({ breakpoints: entries }),

  patch: (id, patch) =>
    set((state) => ({
      breakpoints: state.breakpoints.map((bp) =>
        bp.id === id ? { ...bp, ...patch } : bp,
      ),
    })),
}));

// ── Selectors (reusable, stable) ─────────────────────────────────────

/** Count of currently-armed breakpoints (enabled !== false). */
export const selectArmedCount = (s: BreakpointStoreState): number =>
  s.breakpoints.filter((bp) => bp.enabled !== false).length;

/** Every node target that has at least one armed breakpoint. Used by the
 *  diagram overlay to render markers. */
export const selectTargetNodes = (s: BreakpointStoreState): string[] => {
  const out = new Set<string>();
  for (const bp of s.breakpoints) {
    if (bp.enabled === false) continue;
    const node = breakpointTargetNode(bp.breakpoint);
    if (node) out.add(node);
  }
  return Array.from(out);
};
