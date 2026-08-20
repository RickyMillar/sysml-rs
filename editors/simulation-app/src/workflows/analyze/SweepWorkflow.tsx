/**
 * SweepWorkflow — route /analyze/sweep (R5.1).
 *
 * Two-column layout:
 *
 *   ┌───────────────────────┬──────────────────────────────┐
 *   │ SweepConfig           │ SweepResultsShell            │
 *   │  • parameter picker   │  (filled by R5.2 / R5.3)     │
 *   │  • range editor       │                              │
 *   │  • outcome metric     │                              │
 *   │  • run-mode toggle    │                              │
 *   │  • Run Sweep          │                              │
 *   └───────────────────────┴──────────────────────────────┘
 *
 * R5.1 scope: the CONFIG-side Sweep UI + run trigger + results shell.
 * Agents CC and DD own the streaming viewers and drill receiver in
 * R5.2+ — their work mounts inside `<SweepResultsShell>` without
 * touching this file.
 */

import { useCallback, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { SweepConfig, type SweepParameterCandidate } from './sweep/SweepConfig';
import { SweepResultsShell } from './sweep/SweepResultsShell';
import { useSweepConfig, type ParameterRangeEntry } from './sweep/useSweepConfig';
import { useSweepRunner } from './sweep/useSweepRunner';
import { generateChildrenParams } from './sweep/useSweepConfig';
import { discoverSweepParameters } from './sweep/discoverSweepParameters';
import { SweepWorkflowNinebar } from './sweep/SweepWorkflowNinebar';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { useMetricRegistryVersion } from '@/shared/metrics/useMetricRegistry';
import { metricRegistry } from '@/shared/metrics/registry';
import { EmbeddedDiagram } from '@/components/diagram/EmbeddedDiagram';
import { isFlagEnabled } from '@/featureFlags';
import type { MetricDescriptor } from '@/shared/metrics/types';

/**
 * Route entry for /analyze/sweep. Under the (default-on) `ninebar` flag
 * the surface is the re-composed viewer-hero body (`SweepWorkflowNinebar`);
 * flag-off keeps the legacy two-column body verbatim (deleted in Phase 8
 * per F17). Parameter discovery lives in `sweep/discoverSweepParameters`
 * (hoisted so the flag-on config modal shares it — one implementation,
 * one query key).
 */
export function SweepWorkflow() {
  if (isFlagEnabled('ninebar')) return <SweepWorkflowNinebar />;
  return <SweepWorkflowLegacy />;
}

function SweepWorkflowLegacy() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);
  const { data: availableParameters = [], isLoading: isLoadingTargets } =
    useQuery<SweepParameterCandidate[]>({
      queryKey: ['sweep-parameters', workspaceRoot, loadedUris],
      queryFn: () => discoverSweepParameters(loadedUris),
      enabled: !!workspaceRoot && loadedUris.length > 0,
    });

  // MetricRegistry is a mutable catalogue outside React state. It used to be
  // snapshotted through a `useMemo` keyed on a `useState(0)` that nothing ever
  // updated, so a metric registered after first paint never appeared. Subscribe
  // to the registry's version instead.
  const registryVersion = useMetricRegistryVersion();
  const availableMetrics = useMemo<MetricDescriptor[]>(
    () => metricRegistry.list(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [registryVersion, availableParameters],
  );

  const config = useSweepConfig();
  const runner = useSweepRunner();

  const [error, setError] = useState<string | null>(null);

  const handleRun = useCallback(async () => {
    setError(null);
    if (config.ranges.length === 0) return;
    const childrenParams = generateChildrenParams(config.ranges);
    if (childrenParams.length === 0) {
      setError('Every range expanded to zero values. Adjust min/max/step or the values list.');
      return;
    }
    // Pick a target URI — first loaded file is the R5.1 heuristic; the
    // user picks explicitly once workspace-level target selection lands
    // in a later round.
    const uri = loadedUris[0];
    if (!uri) {
      setError('Load a workspace before running a sweep.');
      return;
    }
    await runner.start({
      uri,
      childrenParams,
      runMode: config.runMode,
      label: `Sweep of ${config.ranges.length} ${config.ranges.length === 1 ? 'parameter' : 'parameters'}`,
    });
  }, [config.ranges, config.runMode, loadedUris, runner]);

  const isRunning = runner.status.kind === 'running';

  return (
    <div
      data-testid="sweep-workflow"
      className="flex flex-row h-full w-full overflow-hidden"
    >
      <SweepConfig
        availableParameters={availableParameters}
        availableMetrics={availableMetrics}
        config={config}
        isRunning={isRunning}
        hasWorkspace={!!workspaceRoot}
        isLoadingParameters={isLoadingTargets}
        onRun={handleRun}
      />
      <main
        data-testid="sweep-results"
        className="flex-1 overflow-hidden"
        style={{ background: 'var(--surface)' }}
      >
        <SweepResultsShell
          batchId={runner.batchId}
          children={runner.children}
          status={runner.status}
          error={error ?? (runner.error ? runner.error.message : null)}
        />
      </main>
      {/* Phase 6 — diagram on every workflow tab. */}
      <EmbeddedDiagram label="Model" />
    </div>
  );
}

// Re-export types so parallel agents (CC/DD) can import from the
// workflow namespace without reaching into the `sweep/` subdirectory.
export type { ParameterRangeEntry };
