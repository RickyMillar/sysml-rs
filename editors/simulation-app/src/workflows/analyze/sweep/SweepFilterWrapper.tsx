/**
 * SweepFilterWrapper — reconcile-time glue between BB's config, CC's
 * viewers, and this agent's filter/drill/slice features (R5.4 + R5.5).
 *
 * Mount this inside `<SweepResultsShell>` (or wrap it around CC's table
 * viewer). It owns:
 *
 *   1. The post-hoc predicate state + chip rendering.
 *   2. The `sysml.batch.slice` query, gated on the predicate.
 *   3. The drill-into-Run click handler for child rows.
 *
 * What it does NOT own:
 *   - The unfiltered children list (CC's shell owns the stream from the
 *     batch runner — we swap our sliced list in when a filter is active).
 *   - The range editor / pre-run filter (BB's config owns those; pre-run
 *     uses `applyPreRunFilter` directly).
 *
 * This wrapper exists to keep `SweepResultsShell.tsx` itself untouched
 * during reconcile — BB or CC just import and render `<SweepFilterWrapper>`
 * inside their shell, passing the unsliced children + the list of viewer
 * slots as render props.
 */

import { useMemo, useState, type ReactNode } from 'react';
import type { ChildDescriptor, ParamPredicate, SliceFilter } from '@/engine/types';
import { SweepFilterBar } from './SweepFilterBar';
import { useSweepSlice } from './useSweepSlice';
import { useSweepDrill, type SweepDrillApi } from './useSweepDrill';

export interface SweepFilterWrapperRenderProps {
  /** Children currently visible (sliced when a predicate is active; unsliced otherwise). */
  children: ChildDescriptor[];
  /** Drill API — forward `.drill` to the viewer's `onChildSelect` callback. */
  drill: SweepDrillApi;
  /** `true` while the slice query is in-flight. */
  isSlicing: boolean;
  /** Error from the slice query, if any. */
  sliceError: Error | null;
}

export interface SweepFilterWrapperProps {
  /** Stable id for the current batch — required for the post-hoc slice. */
  batchId: string | null;
  /** Full (unsliced) list of children — streams from BB/CC's runner. */
  unsliced: ChildDescriptor[];
  /** Parameter names the filter bar should expose. */
  params: string[];
  /**
   * Render prop. Callers embed CC's viewers (`<SweepTableViewer
   * onChildSelect={drill.drill} …>`) here and thread the visible
   * children through.
   */
  render: (args: SweepFilterWrapperRenderProps) => ReactNode;
}

/**
 * Translate a `ParamPredicate` into the backend's `SliceFilter` shape.
 * Exported for tests + any imperative caller that wants to issue a slice
 * without going through the hook.
 */
export function predicateToSliceFilter(
  predicate: ParamPredicate,
): SliceFilter {
  return { param_predicate: { ...predicate } };
}

export function SweepFilterWrapper(props: SweepFilterWrapperProps) {
  const { batchId, unsliced, params, render } = props;

  const [predicate, setPredicate] = useState<ParamPredicate | null>(null);
  const filter = useMemo<SliceFilter | null>(
    () => (predicate ? predicateToSliceFilter(predicate) : null),
    [predicate],
  );

  const slice = useSweepSlice(batchId, filter);
  const drill = useSweepDrill();

  const visible = predicate && slice.data ? slice.data : unsliced;

  return (
    <div className="sweep-filter-wrapper" data-testid="sweep-filter-wrapper">
      <SweepFilterBar
        title="Slice results"
        params={params}
        predicate={predicate}
        onApply={setPredicate}
        onClear={() => setPredicate(null)}
      />
      {render({
        children: visible,
        drill,
        isSlicing: slice.isFetching,
        sliceError: slice.error ?? null,
      })}
    </div>
  );
}
