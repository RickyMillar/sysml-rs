/**
 * SweepViewerSwitcher — tab strip + active viewer mount (R5.3).
 *
 * BB's `<SweepResultsShell>` mounts this component inside its run panel.
 * We own:
 *   - the four tabs (Table / Tornado / ParallelCoords / Heatmap),
 *   - per-batch active-tab persistence via `useSweepViewerStore`,
 *   - the metric + axis configuration surface,
 *   - dispatch to the matching ResultViewer by `ResultViewerKind`.
 *
 * We do NOT own:
 *   - the batch poller (BB's `useSweepRunner`),
 *   - the drill callback wiring (DD's scope),
 *   - the batch lifecycle (start / cancel / archive).
 *
 * Contract: the caller passes a stable `batchId` (so the switcher can
 * persist the user's tab pick) and an up-to-date `children` array from
 * the poller. `onChildSelect` is optional — when absent the Table rows
 * stay non-interactive.
 */
import { useState } from 'react';
import type { CSSProperties } from 'react';
import {
  type ChildDescriptor,
  type SweepMetricId,
  metricLabelFor,
} from '@/shared/viewers/sweepViewerHelpers';
import { sweepTableViewer, type SweepTableData } from '@/shared/viewers/SweepTableViewer';
import {
  sweepTornadoViewer,
  type SweepTornadoData,
} from '@/shared/viewers/SweepTornadoViewer';
import {
  sweepParallelCoordsViewer,
  type SweepParallelCoordsData,
} from '@/shared/viewers/SweepParallelCoordsViewer';
import {
  sweepHeatmapViewer,
  type SweepHeatmapData,
} from '@/shared/viewers/SweepHeatmapViewer';
import {
  DEFAULT_SWEEP_VIEWER,
  type SweepViewerId,
  useSweepViewerStore,
} from './useSweepViewerStore';

export interface SweepViewerSwitcherProps {
  /** Stable id per batch run — used to key the per-batch tab memory. */
  batchId: string;
  /** Streaming children from `useSweepRunner` — may be incomplete. */
  children: ChildDescriptor[];
  /** Row-click handler for the Table; ignored by other viewers. */
  onChildSelect?: (child: ChildDescriptor) => void;
  /** Render height for each viewer; forwarded via AxesConfig.height. */
  height?: number;
}

interface TabDescriptor {
  id: SweepViewerId;
  label: string;
}

const TABS: TabDescriptor[] = [
  { id: 'sweep-table', label: 'Table' },
  { id: 'sweep-tornado', label: 'Tornado' },
  { id: 'sweep-parallel-coords', label: 'Parallel Coords' },
  { id: 'sweep-heatmap', label: 'Heatmap' },
];

/**
 * The component BB mounts inside `<SweepResultsShell>`. Pure-function at
 * the boundary: all state is either in props or in `useSweepViewerStore`.
 */
export function SweepViewerSwitcher({
  batchId,
  children,
  onChildSelect,
  height = 360,
}: SweepViewerSwitcherProps) {
  const active = useSweepViewerStore((s) => s.activeByBatch[batchId] ?? DEFAULT_SWEEP_VIEWER);
  const setActive = useSweepViewerStore((s) => s.setActive);

  // Metric is local (not persisted) — it's a viewer detail, and all three
  // numeric viewers share the same choice so switching tabs keeps context.
  // Widened past the two verdict built-ins so a measured model outcome is
  // selectable in exactly the same place.
  const [metric, setMetric] = useState<SweepMetricId>('fail_count');

  return (
    <div data-testid="sweep-viewer-switcher" style={rootStyle}>
      <div role="tablist" aria-label="Sweep viewer tabs" style={tabRowStyle}>
        {TABS.map((tab) => {
          const selected = active === tab.id;
          return (
            <button
              key={tab.id}
              type="button"
              role="tab"
              aria-selected={selected}
              data-testid={`sweep-viewer-tab-${tab.id}`}
              onClick={() => setActive(batchId, tab.id)}
              style={tabStyle(selected)}
            >
              {tab.label}
            </button>
          );
        })}
      </div>

      <div style={viewerBodyStyle(height)}>
        {active === 'sweep-table'
          ? sweepTableViewer.render(
              {
                kind: 'sweep-table',
                children,
                config: {
                  metric,
                  metricLabel: metricLabelFor(metric, children),
                  onChildSelect,
                },
              } satisfies SweepTableData,
              { height },
            )
          : null}

        {active === 'sweep-tornado'
          ? sweepTornadoViewer.render(
              {
                kind: 'sweep-tornado',
                children,
                config: { metric, onMetricChange: setMetric },
              } satisfies SweepTornadoData,
              { height },
            )
          : null}

        {active === 'sweep-parallel-coords'
          ? sweepParallelCoordsViewer.render(
              {
                kind: 'sweep-parallel-coords',
                children,
                config: { metric, onMetricChange: setMetric },
              } satisfies SweepParallelCoordsData,
              { height },
            )
          : null}

        {active === 'sweep-heatmap'
          ? sweepHeatmapViewer.render(
              {
                kind: 'sweep-heatmap',
                children,
                config: { metric, onMetricChange: setMetric },
              } satisfies SweepHeatmapData,
              { height },
            )
          : null}
      </div>
    </div>
  );
}

// ── Styles ─────────────────────────────────────────────────────────

const rootStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 8,
  width: '100%',
};

const tabRowStyle: CSSProperties = {
  display: 'flex',
  gap: 2,
  padding: 2,
  borderBottom: '1px solid color-mix(in srgb, var(--outline-variant) 25%, transparent)',
};

function tabStyle(selected: boolean): CSSProperties {
  return {
    padding: '6px 12px',
    fontSize: 12,
    fontWeight: selected ? 600 : 400,
    color: selected ? 'var(--on-surface)' : 'color-mix(in srgb, var(--on-surface) 70%, transparent)',
    background: selected
      ? 'color-mix(in srgb, var(--outline-variant) 18%, transparent)'
      : 'transparent',
    border: 'none',
    borderBottom: selected ? '2px solid var(--accent)' : '2px solid transparent',
    cursor: 'pointer',
    borderRadius: '4px 4px 0 0',
    transition: 'background 120ms ease, color 120ms ease',
  };
}

function viewerBodyStyle(height: number): CSSProperties {
  return {
    flex: 1,
    minHeight: height,
    padding: 12,
    background: 'color-mix(in srgb, var(--outline-variant) 4%, transparent)',
    borderRadius: 8,
  };
}
