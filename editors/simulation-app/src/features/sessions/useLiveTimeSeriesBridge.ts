/**
 * useLiveTimeSeriesBridge — drains the streaming `sessionLiveStore`
 * scalar map into the `useTimeSeriesStore` ring buffer so the
 * PlotsTab / KpisTab keep working.
 *
 * Runs alongside `useSessionStream`. Disabled when `VITE_STREAM_V1`
 * is unset — in that mode the Run workflow has no live data source.
 *
 * UX closeout #4 / #17 (big-model freeze): on a small/medium model
 * (below `BIG_MODEL_VAR_THRESHOLD` scalar vars) every variable is
 * buffered, exactly as before — zero behaviour change. Above the
 * threshold (espresso-production-cell-scale, ~14k vars), buffering
 * everything every tick means `TimeSeriesBuffer.append` walks every
 * ring on every tick just for bookkeeping, whether or not anyone is
 * plotting those variables. Above the threshold we only buffer
 * variables the user has actually shown interest in — pinned,
 * plotted, or promoted from the tree — all of which already funnel
 * into `usePlotSelectionStore` (see `SessionTreeV2.handleTogglePin`
 * and `promoteToPlots.ts`), so this reuses that existing selection
 * set rather than inventing a second one. Once a variable is tracked
 * it stays tracked for the rest of the session — deselecting a plot
 * shouldn't blank its already-recorded history.
 *
 * Known trade-off (flagged, not fixed here): KPI definitions
 * (`KpisTab`) don't register into `usePlotSelectionStore` today, so a
 * KPI added on a big model for a variable that was never plotted/
 * pinned won't start collecting data until it also gets plotted or
 * pinned. Extending KPI definitions to auto-register was considered
 * but rejected for this pass — `usePlotSelectionStore`'s selection
 * list is also what `PlotsTab` auto-merges into "Plot 1" (see its
 * `promotedSelection` effect), so writing into it from `KpisTab`
 * would leak KPI-only variables into the default plot. That needs a
 * separate interest registry to do safely, out of scope here.
 */
import { useEffect, useRef } from 'react';
import { useSessionLiveStore } from './sessionLiveStore';
import { isStreamV1Enabled } from './useSessionStream';
import { useTimeSeriesStore } from '@/shared/data/useTimeSeriesStore';
import { usePlotSelectionStore } from '@/features/results/usePlotSelectionStore';

/**
 * Scalar-variable count above which the bridge switches from
 * "buffer everything" to "buffer only what's selected". Deliberately
 * far above hybrid-scale (~40 vars) and any existing test fixture, and
 * far below espresso-production-cell-scale (~14k vars) — mirrors the
 * DiagramView `NODE_CAP=250` precedent (Bucket 3.11) for the same
 * class of problem.
 */
export const BIG_MODEL_VAR_THRESHOLD = 500;

/**
 * Decide which scalar values this tick actually pushes into the ring
 * buffer. Pure + exported so the selectivity logic is unit-testable
 * without spinning up the subscription plumbing.
 *
 * `trackedVars` is mutated in place (grown, never shrunk) so callers
 * can hold a persistent ref across ticks — the "once tracked, always
 * tracked for the session" rule.
 */
export function selectValuesToBuffer(
  scalarVars: Record<string, number>,
  sessionId: string | null,
  trackedVars: Set<string>,
): Record<string, number> {
  if (Object.keys(scalarVars).length <= BIG_MODEL_VAR_THRESHOLD) {
    return scalarVars;
  }
  for (const name of usePlotSelectionStore.getState().getSelected(sessionId)) {
    trackedVars.add(name);
  }
  if (trackedVars.size === 0) return {};
  const out: Record<string, number> = {};
  for (const name of trackedVars) {
    const v = scalarVars[name];
    if (v !== undefined) out[name] = v;
  }
  return out;
}

export function useLiveTimeSeriesBridge(): void {
  const lastTickRef = useRef<number | null>(null);
  const trackedVarsRef = useRef<Set<string>>(new Set());

  useEffect(() => {
    if (!isStreamV1Enabled()) return;

    // Reset the ring buffer whenever the active session changes.
    const unsubSession = useSessionLiveStore.subscribe((s, prev) => {
      if (s.sessionId !== prev.sessionId) {
        useTimeSeriesStore.getState().reset();
        lastTickRef.current = null;
        trackedVarsRef.current = new Set();
      }
    });

    // Push one point into the ring buffer per new tick.
    const unsubSnap = useSessionLiveStore.subscribe((s, prev) => {
      const snap = s.snapshot;
      if (!snap) return;
      if (snap === prev.snapshot) return;
      // Tick regression = session was reset in place. Drop the buffer
      // so the chart doesn't render a tick-0 sample appended to tick-51's
      // history, and re-arm `lastTickRef` so the dedupe check below
      // doesn't swallow the entire post-reset run.
      if (lastTickRef.current !== null && snap.tick < lastTickRef.current) {
        useTimeSeriesStore.getState().reset();
        lastTickRef.current = null;
        trackedVarsRef.current = new Set();
      }
      if (lastTickRef.current !== null && snap.tick <= lastTickRef.current) {
        return;
      }
      const values = selectValuesToBuffer(snap.scalar_vars, s.sessionId, trackedVarsRef.current);
      useTimeSeriesStore.getState().pushPoint(snap.time_ms, values);
      lastTickRef.current = snap.tick;
    });

    return () => {
      unsubSession();
      unsubSnap();
    };
  }, []);
}
