/**
 * useInvestigationTrail — multi-hop breadcrumb store for cross-workflow
 * drill navigation (ninebar Phase 1, audit F15).
 *
 * Replaces the single-hop `drilledFrom` field that used to live on
 * `useSessionStore` (`features/sessions/store.ts`). A single slot can only
 * represent "you came from Verify once" — a second drill silently
 * clobbered the first, and there was no way to represent a chain like
 * Verify -> Run -> Analyze -> Run. This store keeps the whole chain as an
 * ordered list of hops plus a cursor into it, so "back" doesn't destroy
 * the hops ahead of the cursor until something new is actually pushed
 * (browser-history semantics — see `push`).
 *
 * ## URL contract
 *
 * The wire format between workflows is UNCHANGED from the old `drilledFrom`
 * handshake — this store only changes how the receiving side represents
 * what it read. The sender (`workflows/verify/useDrillFromVerdict.ts`,
 * `workflows/analyze/sweep/useSweepDrill.ts`) still only ever builds a URL
 * of the form:
 *
 *   /run?session=<session_id>&tick=<n>&element=<qualified_name>
 *
 * where:
 *   - `session` (required)  — the source session id (`fromSessionId`)
 *   - `tick`    (optional)  — the evidence tick (`tick`)
 *   - `element` (optional)  — the evidence element's qualified name
 *                             (`elementId`); omitted when the verdict/row
 *                             carries no element reference
 *
 * `workflows/run/useDrillReceiver.ts` reads those three params on mount
 * and — instead of calling the old `setDrilledFrom` — calls `push()` with
 * a hop built from them. No new query params were introduced; only the
 * consumer-side representation changed from "one slot" to "a stack".
 *
 * ## Hop shape
 *
 * `origin` names the workflow the hop was pushed FROM ('verify' | 'run' |
 * 'analyze'), not the destination — the user always lands on RunWorkflow
 * today, so the origin is the only ambiguous part worth recording. `label`
 * is a precomputed human-readable string for the breadcrumb UI (e.g.
 * "Verify · tick 482") so `DrilledFromBanner` doesn't need per-origin
 * formatting logic.
 *
 * ## Clearing
 *
 * The trail is cleared whenever the active session changes, mirroring the
 * old "drilledFrom cleared when active session changes" rule. This is
 * wired directly in `useSessionStore.setActiveSession` /
 * `useSessionStore.resetViewState` (`features/sessions/store.ts`) via a
 * call to `useInvestigationTrail.getState().clear()`, rather than a
 * cross-store subscription: the session store already owns "session
 * changed -> reset dependent view state" for `focusPath` /
 * `selectedScope` / `focusedActionPath`, so hooking in there keeps all of
 * that reset logic in one place and avoids a subscribe-on-import side
 * effect running before any component has mounted.
 */

import { create } from 'zustand';

/** Workflow a hop was drilled FROM. */
export type InvestigationOrigin = 'verify' | 'run' | 'analyze';

export interface InvestigationHop {
  /** Workflow the user drilled FROM (see file doc). */
  origin: InvestigationOrigin;
  /** Session id the hop's evidence belongs to. May be `''` if unknown. */
  fromSessionId: string;
  /** Evidence tick, when the hop carries one (e.g. a Verify verdict). */
  tick?: number;
  /** Qualified name / element id the hop points at, when known. */
  elementId?: string;
  /** Precomputed breadcrumb label, e.g. "Verify · tick 482". */
  label: string;
}

interface InvestigationTrailState {
  /**
   * Full recorded history, oldest first. May contain hops PAST `cursor`
   * — those are "forward" hops left over from a `popTo` call that
   * haven't been discarded yet. They're dropped the next time `push` is
   * called (see `push`).
   */
  hops: InvestigationHop[];
  /**
   * Index into `hops` of the hop currently in view. `-1` means the trail
   * is empty / nothing is in view (root scope, banner hidden).
   */
  cursor: number;

  /**
   * Append a new hop. If `cursor` sits behind the end of `hops` (the user
   * popped back and then drilled somewhere new), the forward hops are
   * dropped first — standard browser-history semantics: a fresh
   * navigation replaces whatever "redo" branch was ahead of you.
   */
  push: (hop: InvestigationHop) => void;

  /**
   * Move the cursor to `index` without discarding any hops. Used by
   * "back" (`popTo(cursor - 1)`, see `DrilledFromBanner`) and by clicking
   * an earlier breadcrumb segment directly. Clamped to
   * `[-1, hops.length - 1]`.
   */
  popTo: (index: number) => void;

  /** Drop the entire trail (e.g. on session switch — see file doc). */
  clear: () => void;
}

export const useInvestigationTrail = create<InvestigationTrailState>((set) => ({
  hops: [],
  cursor: -1,

  push: (hop) =>
    set((s) => {
      const kept = s.hops.slice(0, s.cursor + 1);
      const hops = [...kept, hop];
      return { hops, cursor: hops.length - 1 };
    }),

  popTo: (index) =>
    set((s) => ({
      cursor: Math.max(-1, Math.min(index, s.hops.length - 1)),
    })),

  clear: () => set({ hops: [], cursor: -1 }),
}));

/**
 * Convenience selector — the hop currently in view, or `null` when the
 * trail is empty. Not stored as a derived field to avoid a second source
 * of truth; compute it at the selector boundary instead.
 */
export function selectCurrentHop(
  s: Pick<InvestigationTrailState, 'hops' | 'cursor'>,
): InvestigationHop | null {
  return s.cursor >= 0 && s.cursor < s.hops.length ? s.hops[s.cursor] : null;
}
