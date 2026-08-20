/**
 * SweepWorkflowNinebar — the flag-on Sweep surface (ninebar Phase 5).
 *
 * "Analyze, re-composed": the result viewer kit (Table / Tornado /
 * Parallel Coords / Heatmap — built in R5.3 but never mounted; the
 * legacy shell still renders placeholders) is finally the full-bleed
 * primary surface. Configuration lives in the "Configure sweep" MODAL
 * (`SweepConfigModalNinebar`), the left rail carries the crib-sheet 3a
 * factor summary (shared `AnalyzeRail` chrome: factors →
 * combinations/evaluated/failing footer + quota headroom) + Run, and
 * the bottom strip is the live batch lifecycle (`AnalyzeBatchStrip`).
 * Drilling a table row rides the existing `useSweepDrill` URL handshake
 * into /run (the receiver pushes the investigation-trail hop). A stats
 * toggle mounts `SweepStatsPanel` (`features/stats`, audit F5) above
 * the viewers.
 *
 * The legacy two-column body (`SweepWorkflow` flag-off) is untouched.
 */

import { useMemo, useState } from 'react';
import { LeftRailContent, BottomStripContent } from '@/app/slots';
import { useModalStore } from '@/shared/overlays/modalStore';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { SweepStatsPanel } from '@/features/stats';
import {
  extractorFor,
  metricOptionsFor,
  outcomeMetricId,
  type ChildDescriptor as ViewerChildDescriptor,
} from '@/shared/viewers/sweepViewerHelpers';
import type { ChildDescriptor } from '@/engine/types';

import { AnalyzeBatchStrip } from '../ninebar/AnalyzeBatchStrip';
import { AnalyzeRail, RailListRow, RailEmptyHint, HeroNotice } from '../ninebar/chrome';
import { SweepViewerSwitcher } from './SweepViewerSwitcher';
import { SweepFilterWrapper } from './SweepFilterWrapper';
import { useSweepRunner, type ChildTruncation } from './useSweepRunner';
import { useSweepStudyStore, expandStudyChildren } from './useSweepStudyStore';
import { SWEEP_CONFIG_MODAL_ID } from './SweepConfigModalNinebar';
import { expandRange } from './cartesianProduct';
import type { ParameterRangeEntry } from './useSweepConfig';
// Registers the 'analyze-sweep-config' modal (module side effect).
import './SweepConfigModalNinebar';

export function SweepWorkflowNinebar() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);

  const ranges = useSweepStudyStore((s) => s.ranges);
  const runMode = useSweepStudyStore((s) => s.runMode);
  const horizonTicks = useSweepStudyStore((s) => s.horizonTicks);
  const selectedMetricIds = useSweepStudyStore((s) => s.selectedMetricIds);
  const dtMs = useSweepStudyStore((s) => s.dtMs);
  const openModal = useModalStore((s) => s.openModal);

  const runner = useSweepRunner();
  const isRunning = runner.status.kind === 'running';
  const [showStats, setShowStats] = useState(false);

  const studyChildren = useMemo(() => expandStudyChildren(ranges), [ranges]);
  const childCount = ranges.length > 0 ? studyChildren.length : 0;

  const handleRun = async () => {
    if (childCount === 0) return;
    const uri = loadedUris[0];
    if (!uri) return;
    await runner.start({
      uri,
      childrenParams: studyChildren,
      runMode,
      horizonTicks,
      dtMs,
      // The outcomes the user picked in Configure. These were stored and
      // then dropped on the floor: selecting `temperature` changed nothing
      // about what the run measured or what the viewers could show.
      outcomes: selectedMetricIds,
      label: `Sweep of ${ranges.length} ${ranges.length === 1 ? 'parameter' : 'parameters'}`,
    });
  };

  const paramNames = useMemo(() => {
    const first = runner.children[0]?.params;
    if (first && Object.keys(first).length > 0) return Object.keys(first);
    return ranges.map((r) => r.parameterId);
  }, [runner.children, ranges]);

  const hasBatch = runner.batchId !== null || runner.children.length > 0;
  const evaluated = runner.progress.complete;
  const failing = countFailing(runner.children);

  // Stats offers the same metric menu as the numeric viewers: the two
  // verdict built-ins plus every outcome the children actually reported.
  const statsMetrics = useMemo(
    () =>
      metricOptionsFor(toViewerChildren(runner.children)).map((opt) => ({
        id: opt.value,
        label: opt.label,
        extract: extractorFor(opt.value),
      })),
    [runner.children],
  );

  return (
    <div
      data-testid="sweep-workflow-ninebar"
      className="flex flex-col h-full w-full min-h-0"
      style={{ background: 'var(--surface-canvas)', color: 'var(--text-primary)' }}
    >
      <LeftRailContent>
        <AnalyzeRail
          icon="tune"
          title="Sweep"
          headerCount={`${ranges.length} ${ranges.length === 1 ? 'factor' : 'factors'}`}
          sectionTitle="Factors"
          onConfigure={() => openModal(SWEEP_CONFIG_MODAL_ID)}
          summary={[
            { label: 'combinations', value: String(childCount) },
            { label: 'evaluated', value: String(evaluated) },
            { label: 'failing', value: String(failing), tone: failing > 0 ? 'fail' : undefined },
          ]}
          plannedChildren={childCount}
          runLabel="Run Sweep"
          canRun={!!workspaceRoot && childCount > 0}
          isRunning={isRunning}
          onRun={() => void handleRun()}
          testIdPrefix="sweep"
        >
          {ranges.length === 0 ? (
            <RailEmptyHint>No factors yet — Configure adds parameters and ranges.</RailEmptyHint>
          ) : (
            <ul data-testid="sweep-rail-factors" style={{ listStyle: 'none', margin: 0, padding: '4px 0' }}>
              {ranges.map((r) => (
                <li key={r.parameterId}>
                  <RailListRow
                    testId={`sweep-rail-factor-${r.parameterId}`}
                    name={r.label ?? r.parameterId}
                    detail={summariseSpec(r)}
                    onClick={() => openModal(SWEEP_CONFIG_MODAL_ID)}
                  />
                </li>
              ))}
            </ul>
          )}
        </AnalyzeRail>
      </LeftRailContent>

      <div className="flex-1 min-h-0 overflow-auto" data-testid="sweep-hero">
        {runner.error || runner.status.kind === 'failed' ? (
          <HeroNotice
            testId="sweep-hero-error"
            icon="error"
            tone="error"
            title="Sweep failed"
            detail={runner.error?.message ?? (runner.status.kind === 'failed' ? runner.status.reason : 'Unknown error')}
          />
        ) : !hasBatch ? (
          <HeroNotice
            testId="sweep-hero-empty"
            icon="tune"
            title="Define a sweep"
            detail="Pick parameters and ranges in the Configure sweep modal, then run — the streaming table, tornado, parallel-coords, and heatmap render here."
            action={{ label: 'Configure sweep', testId: 'sweep-hero-configure', onClick: () => openModal(SWEEP_CONFIG_MODAL_ID) }}
          />
        ) : isRunning && runner.children.length === 0 ? (
          <HeroNotice
            testId="sweep-hero-running"
            icon="tune"
            title="Running sweep…"
            detail="Children stream into the viewers as the batch advances."
          />
        ) : (
          <div className="flex flex-col p-3" style={{ minHeight: '100%' }}>
            {runner.truncations.length > 0 && (
              <TruncationNotice
                truncations={runner.truncations}
                total={runner.children.length}
              />
            )}
            <div className="flex items-center" style={{ marginBottom: 4 }}>
              <button
                type="button"
                data-testid="sweep-stats-toggle"
                data-active={showStats}
                onClick={() => setShowStats((v) => !v)}
                style={{
                  marginLeft: 'auto',
                  padding: '2px 10px',
                  borderRadius: 4,
                  fontSize: 11,
                  cursor: 'pointer',
                  background: showStats ? 'var(--accent-tint)' : 'transparent',
                  color: showStats ? 'var(--text-primary)' : 'var(--text-muted)',
                  border: `1px solid ${showStats ? 'var(--accent)' : 'var(--border-hairline)'}`,
                }}
              >
                Stats
              </button>
            </div>
            {showStats && (
              <div data-testid="sweep-stats-panel" style={{ marginBottom: 8 }}>
                {/* features/stats wired (audit F5): per-(parameter × metric)
                    moments/CI overlays over the same streaming children. */}
                <SweepStatsPanel
                  children={toViewerChildren(runner.children)}
                  metrics={statsMetrics}
                />
              </div>
            )}
            <SweepFilterWrapper
              batchId={runner.batchId}
              unsliced={runner.children}
              params={paramNames}
              render={({ children: visible, drill }) => (
                <SweepViewerSwitcher
                  batchId={runner.batchId ?? 'pending'}
                  children={toViewerChildren(visible)}
                  onChildSelect={drill.drill}
                  height={520}
                />
              )}
            />
          </div>
        )}
      </div>

      <BottomStripContent>
        <AnalyzeBatchStrip methodLabel="Sweep" status={runner.status} children={runner.children} />
      </BottomStripContent>
    </div>
  );
}

/**
 * Say so when children stopped before the horizon they were given.
 *
 * Every outcome is read where the run stopped, so a truncated child reports
 * the model mid-transient while still showing `Complete` in the table. That
 * reads as a finished result and is not one — which is exactly the shape of
 * failure this surface exists to prevent.
 */
function TruncationNotice({
  truncations,
  total,
}: {
  truncations: readonly ChildTruncation[];
  total: number;
}) {
  const worst = truncations.reduce((a, b) =>
    a.advancedTicks / a.requestedTicks <= b.advancedTicks / b.requestedTicks ? a : b,
  );
  const pct = Math.round((worst.advancedTicks / worst.requestedTicks) * 100);
  return (
    <div
      data-testid="sweep-truncation-notice"
      style={{
        marginBottom: 8,
        padding: '8px 12px',
        borderRadius: 6,
        fontSize: 12,
        border: '1px solid var(--severity-warning, var(--border-hairline))',
        background: 'color-mix(in srgb, var(--severity-warning, gray) 10%, transparent)',
        color: 'var(--text-primary)',
      }}
    >
      <strong>
        {truncations.length} of {total} {truncations.length === 1 ? 'run' : 'runs'} stopped early.
      </strong>{' '}
      The shortest covered {worst.advancedTicks.toLocaleString()} of{' '}
      {worst.requestedTicks.toLocaleString()} ticks ({pct}%)
      {worst.timeMs != null ? `, reaching ${(worst.timeMs / 1000).toLocaleString()} s of model time` : ''}
      . Outcomes below are read where each run stopped, not at rest — raise the step size in
      Configure so the horizon reaches the behaviour you are measuring.
    </div>
  );
}

function countFailing(children: readonly ChildDescriptor[]): number {
  return children.filter((c) => (c.verdicts ?? []).some((v) => v.verdict === 'fail')).length;
}

/**
 * Adapt the engine's `ChildDescriptor` (nullable `session_id`, optional
 * `verdicts`) to the viewer kit's stricter shape. The two shapes never
 * met before this recomposition (the legacy shell rendered placeholders,
 * not viewers) — flagged to the debt ledger as a duplicate contract to
 * collapse. `''` keeps un-materialised children non-drillable
 * (`canDrillChild` requires a non-empty id).
 */
function toViewerChildren(children: readonly ChildDescriptor[]): ViewerChildDescriptor[] {
  return children.map((c) => ({
    ...c,
    session_id: c.session_id ?? '',
    verdicts: c.verdicts ?? [],
  }));
}

function summariseSpec(entry: ParameterRangeEntry): string {
  const spec = entry.spec;
  if (spec.kind === 'grid') {
    const n = expandRange(spec).length;
    return `${spec.min} → ${spec.max} · ${n}`;
  }
  return `${spec.values.length} values`;
}
