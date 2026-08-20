/**
 * TradeStudyWorkflow — route /analyze/trade-study (R5.10).
 *
 * Two-column workflow page:
 *
 *   ┌─────────────────────┬──────────────────────────────┐
 *   │ TradeStudyConfig    │ TradeStudyResultsShell       │
 *   │  • Alternatives     │  (filled by R5.11 viewer)    │
 *   │  • Criteria         │                              │
 *   │  • Weights          │                              │
 *   │  • Run button       │                              │
 *   └─────────────────────┴──────────────────────────────┘
 *
 * R5.10 wires the config panel + the runner end of the pipeline. The
 * results viewer (R5.11 / HH) and the promote-to-compare hand-off land
 * separately — this file's only responsibility for those is to keep the
 * `TradeStudyResultsShell` prop shape stable so the parallel work does
 * not break.
 *
 * Backend path: `useTradeStudyRunner` prefers `sysml.batch.create` when
 * available and falls back to the legacy `sysml.trade_study` command
 * otherwise. See that hook's header for the full decision log.
 */

import { useCallback } from 'react';
import { TradeStudyConfig } from './tradestudy/TradeStudyConfig';
import { TradeStudyResultsShell } from './tradestudy/TradeStudyResultsShell';
import { TradeStudyWorkflowNinebar } from './tradestudy/TradeStudyWorkflowNinebar';
import { useTradeStudyConfig } from './tradestudy/useTradeStudyConfig';
import { useTradeStudyRunner } from './tradestudy/useTradeStudyRunner';
import { useCandidateMetrics } from './tradestudy/useCandidateMetrics';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { isFlagEnabled } from '@/featureFlags';

/**
 * Route entry for /analyze/trade-study. Under the (default-on) `ninebar`
 * flag the surface is the re-composed viewer-hero body
 * (`TradeStudyWorkflowNinebar`); flag-off keeps the legacy two-column
 * body verbatim (deleted in Phase 8 per F17).
 */
export function TradeStudyWorkflow() {
  if (isFlagEnabled('ninebar')) return <TradeStudyWorkflowNinebar />;
  return <TradeStudyWorkflowLegacy />;
}

function TradeStudyWorkflowLegacy() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = wsData?.uris ?? [];

  const config = useTradeStudyConfig();
  const runner = useTradeStudyRunner();

  // Discover candidate criterion metrics: union of model AttributeUsage
  // names and any already-registered MetricDescriptors from the live
  // session (PlotsTab / expression producers). See the hook for
  // why we don't rely on the registry alone.
  const metrics = useCandidateMetrics(loadedUris);

  const handleRun = useCallback(() => {
    if (!config.validation.canRun) return;
    // Trade study runs workspace-wide; still gate on a loaded workspace.
    if (loadedUris.length === 0) return;
    void runner.run({
      alternatives: config.alternatives,
      criteria: config.criteria,
      weights: config.normalizedWeights,
    });
  }, [config.alternatives, config.criteria, config.normalizedWeights, config.validation.canRun, loadedUris, runner]);

  return (
    <div
      data-testid="tradestudy-workflow"
      className="flex flex-row h-full w-full overflow-hidden"
    >
      <TradeStudyConfig
        config={config}
        metrics={metrics}
        isRunning={runner.state === 'running'}
        hasWorkspace={!!workspaceRoot}
        onRun={handleRun}
      />
      <main
        data-testid="tradestudy-results"
        className="flex-1 overflow-hidden"
        style={{ background: 'var(--surface)' }}
      >
        <TradeStudyResultsShell
          result={runner.result}
          isRunning={runner.state === 'running'}
          progress={runner.progress}
          error={runner.error ? runner.error.message : null}
        />
      </main>
    </div>
  );
}
