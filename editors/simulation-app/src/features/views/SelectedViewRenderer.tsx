/**
 * SelectedViewRenderer — top-level bridge from `selectedViewId` → renderer.
 *
 * Bucket 5-followup-3 (2026-05-05): hoisted out of `ViewsPanel` so a single,
 * always-mounted listener drives the diagram regardless of which surface
 * initiated the selection.
 *
 * 3.16 (2026-06-27): the ViewModel became the single dispatch source — graph
 * views render straight from it in `SvgCanvas`.
 *
 * 3.12 (2026-06-28): the ViewModel now also carries the scoped non-graph model
 * (`non_graph`: Table/Tree/Geometry). So this component fetches ONE query — the
 * ViewModel (same react-query key as `SvgCanvas`, deduped) — and dispatches:
 *   - `vm.non_graph` present → push it into the store (DiagramHost → TableView /
 *     BrowserView / GeometryView).
 *   - else → clear the non-graph models so DiagramHost falls through to SvgCanvas.
 * The legacy SModel `/render` round-trip is gone: ONE pipeline, ONE fetch for
 * EVERY view family.
 *
 * Renders nothing.
 */
import { useEffect } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useWorkspaceStore } from '@/store/workspace';
import { getViewModel, WORKSPACE_URI } from '@/shared/api/model';
import type { NonGraphModel } from '@/diagram-svg/viewmodel-types';

export function SelectedViewRenderer() {
  const selectedViewId = useWorkspaceStore((s) => s.selectedViewId);
  const setDiagramPayload = useWorkspaceStore((s) => s.setDiagramPayload);

  // The viewmodel command always composes against the merged workspace
  // graph, so address it with the workspace-scope sentinel.
  const uri = WORKSPACE_URI;

  // Single fetch — same key as SvgCanvas's initial fetch (`['viewmodel', id, []]`),
  // so react-query serves one shared result (no extra round-trip for graph views).
  const vmQuery = useQuery({
    queryKey: ['viewmodel', selectedViewId, [] as string[]],
    queryFn: async () => {
      if (!selectedViewId) return null;
      return getViewModel(uri, selectedViewId, []);
    },
    enabled: !!selectedViewId,
    staleTime: 30_000,
  });

  const vm = vmQuery.data as { non_graph?: NonGraphModel | null } | null;
  const nonGraph = vm?.non_graph ?? null;

  useEffect(() => {
    if (!selectedViewId || !vm) return;
    if (nonGraph) {
      // Table / Tree / Geometry: the tagged payload drops straight into the store.
      setDiagramPayload(nonGraph as Parameters<typeof setDiagramPayload>[0]);
    } else {
      // Graph view: clear the non-graph models so DiagramHost renders SvgCanvas.
      setDiagramPayload(null);
    }
  }, [selectedViewId, vm, nonGraph, setDiagramPayload]);

  return null;
}
