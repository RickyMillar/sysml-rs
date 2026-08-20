/**
 * CompareMode — the plug-in contract consumed by R4.3 (Agent X).
 *
 * The compare workflow exposes a right-rail "mode config" slot. Each
 * mode (ensemble / golden / two-design / whatever else lands later)
 * registers a `CompareMode` and the workflow picks one active mode at
 * a time via `useCompareStore.activeModeId`.
 *
 * This contract is intentionally minimal:
 *   - The workflow owns sharedTick, picked sessions, layout.
 *   - The mode owns its own config widgets and (optionally) a main-area
 *     render override. Everything the mode needs from the workflow is
 *     threaded through `CompareContext` so modes stay stateless w.r.t.
 *     the shell.
 *
 * IMPORTANT: the interface shape below is LOCKED for R4.3 integration.
 * Changes here require coordination with Agent X.
 */

import type { ReactNode } from 'react';
import { createContext, useContext } from 'react';
import type { CompareLayout } from './useCompareStore';

/**
 * Read-only snapshot of the compare shell state, passed to each mode's
 * render hooks so they stay pure functions of (state) -> ReactNode.
 */
export interface CompareContext {
  /** Current playhead tick (0-based, clamped to max across picks). */
  sharedTick: number;
  /** Scrubber setter — modes can jump the playhead from their own UI. */
  setSharedTick: (t: number) => void;
  /** Session IDs currently in the picker (2..6). */
  pickedSessionIds: string[];
  /** Effective layout (user choice, with auto resolution already applied). */
  layout: CompareLayout;
}

/**
 * A compare mode plug-in.
 *
 * `configRender` is rendered inside the right sidebar "mode config" slot.
 * `mainRender` (optional) is rendered in the center if the mode wants to
 * override the default chart grid. When omitted, the default compare
 * main area (overlay / side-by-side) is used.
 */
export interface CompareMode {
  /** Stable machine id (also the URL param value). */
  id: string;
  /** Human-readable label shown in the mode switcher chip. */
  label: string;
  /** One-line description surfaced as hover/tooltip on the mode chip. */
  description: string;
  /** Right-rail config widgets (always rendered when mode is active). */
  configRender: (ctx: CompareContext) => ReactNode;
  /** Optional center-panel override. */
  mainRender?: (ctx: CompareContext) => ReactNode;
}

/**
 * React context the shell populates with the live `CompareContext` so
 * deeply nested mode widgets don't have to prop-drill. Modes that
 * prefer the prop form can just read the arg of `configRender`.
 */
const CompareCtx = createContext<CompareContext | null>(null);
CompareCtx.displayName = 'CompareModeContext';

export const CompareModeProvider = CompareCtx.Provider;

/**
 * Hook form — for mode widgets that live below `configRender`'s top
 * frame (e.g. a deeply-nested slider component). Throws if called
 * outside the provider so accidental misuse is loud.
 */
export function useCompareMode(): CompareContext {
  const ctx = useContext(CompareCtx);
  if (!ctx) {
    throw new Error('useCompareMode must be used inside <CompareModeProvider>');
  }
  return ctx;
}

/**
 * Default built-in modes used when R4.3 has not yet shipped. A no-op
 * placeholder that renders a "No compare modes registered" hint so the
 * slot is visible but unobtrusive. Agent X replaces this by registering
 * real modes via `registerCompareMode` (see `CompareWorkflow.tsx`).
 */
export const PLACEHOLDER_MODE: CompareMode = {
  id: 'placeholder',
  label: 'No mode',
  description: 'No compare modes registered — R4.3 will plug ensemble / golden / two-design modes here.',
  configRender: () => null,
};
