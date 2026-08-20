/**
 * SensitivityWorkflow — route /analyze/sensitivity (R7.4).
 *
 * Two-column layout identical in skeleton to Sweep / Monte Carlo /
 * Trade Study:
 *
 *   ┌─────────────────┬──────────────────────────────────────┐
 *   │ SensitivityConfig │ SensitivityResultsShell            │
 *   │  • method       │  • μ-vs-σ scatter (Morris)          │
 *   │  • parameters   │  • grouped bar chart (Sobol)        │
 *   │  • r/p or N     │  • ranked tornado summary           │
 *   │  • metric       │                                    │
 *   │  • Run          │                                    │
 *   └─────────────────┴──────────────────────────────────────┘
 */

import { useCallback, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { findElements } from '@/shared/api/model';
import { metricRegistry } from '@/shared/metrics/registry';
import type { MetricDescriptor } from '@/shared/metrics/types';
import {
  SensitivityConfig,
  type SensitivityParameterCandidate,
} from './SensitivityConfig';
import { SensitivityResultsShell } from './SensitivityResultsShell';
import { SensitivityWorkflowNinebar } from './SensitivityWorkflowNinebar';
import { useSensitivityConfig } from './useSensitivityConfig';
import { useSensitivityRunner } from './useSensitivityRunner';
import { isFlagEnabled } from '@/featureFlags';

/**
 * Discover sensitivity-sweepable parameters — every `AttributeUsage`
 * across loaded URIs. Mirrors Sweep + MC so all three analyze routes
 * share the same parameter catalogue. See BUG 18 writeup.
 */
async function discoverSensitivityParameters(
  uris: readonly string[],
): Promise<SensitivityParameterCandidate[]> {
  if (uris.length === 0) return [];
  const seen = new Set<string>();
  const out: SensitivityParameterCandidate[] = [];
  for (const uri of uris) {
    let elements: Awaited<ReturnType<typeof findElements>>;
    try {
      elements = await findElements(uri, 'AttributeUsage');
    } catch {
      continue;
    }
    for (const el of elements) {
      const name = el.name ?? '';
      if (!name) continue;
      const id = `${uri}::${name}`;
      if (seen.has(id)) continue;
      seen.add(id);
      out.push({ id, label: name, kind: 'AttributeUsage', uri });
    }
  }
  out.sort((a, b) => (a.label ?? a.id).localeCompare(b.label ?? b.id));
  return out;
}

/**
 * Route entry for /analyze/sensitivity. Under the (default-on) `ninebar`
 * flag the surface is the re-composed viewer-hero body
 * (`SensitivityWorkflowNinebar`); flag-off keeps the legacy two-column
 * body verbatim (deleted in Phase 8 per F17).
 */
export function SensitivityWorkflow() {
  if (isFlagEnabled('ninebar')) return <SensitivityWorkflowNinebar />;
  return <SensitivityWorkflowLegacy />;
}

function SensitivityWorkflowLegacy() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);
  const { data: availableParameters = [], isLoading: isLoadingTargets } =
    useQuery<SensitivityParameterCandidate[]>({
      queryKey: ['sensitivity-parameters', workspaceRoot, loadedUris],
      queryFn: () => discoverSensitivityParameters(loadedUris),
      enabled: !!workspaceRoot && loadedUris.length > 0,
    });

  const [metricTick] = useState(0);
  const availableMetrics = useMemo<MetricDescriptor[]>(
    () => metricRegistry.list(),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [metricTick, availableParameters],
  );

  const config = useSensitivityConfig();
  const runner = useSensitivityRunner();

  const [localError, setLocalError] = useState<string | null>(null);

  const handleRun = useCallback(async () => {
    setLocalError(null);
    const uri = loadedUris[0];
    if (!uri) {
      setLocalError('Load a workspace before running sensitivity analysis.');
      return;
    }
    if (!config.isValid) {
      setLocalError('Fix the configuration before running.');
      return;
    }
    await runner.start({
      uri,
      method: config.method,
      params: config.paramRanges,
      r: config.morrisR,
      p: config.morrisP,
      n: config.sobolN,
      seed: config.seed,
      outputMetric: config.outputMetric,
      label: `${config.method === 'morris' ? 'Morris' : 'Sobol'} analysis`,
    });
  }, [
    loadedUris,
    config.isValid,
    config.method,
    config.paramRanges,
    config.morrisR,
    config.morrisP,
    config.sobolN,
    config.seed,
    config.outputMetric,
    runner,
  ]);

  const isRunning =
    runner.state === 'creating' ||
    runner.state === 'running' ||
    runner.state === 'analyzing';

  return (
    <div
      data-testid="sensitivity-workflow"
      className="flex flex-row h-full w-full overflow-hidden"
    >
      <SensitivityConfig
        availableParameters={availableParameters}
        config={config}
        availableMetrics={availableMetrics}
        isRunning={isRunning}
        hasWorkspace={!!workspaceRoot}
        isLoadingParameters={isLoadingTargets}
        onRun={handleRun}
      />
      <main
        data-testid="sensitivity-results"
        className="flex-1 overflow-hidden"
        style={{ background: 'var(--surface)' }}
      >
        <SensitivityResultsShell
          batchId={runner.batchId}
          children={runner.children}
          state={runner.state}
          results={runner.results}
          error={localError ?? runner.error}
        />
      </main>
    </div>
  );
}
