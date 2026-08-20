/**
 * TradeStudyWorkflowNinebar — the flag-on Trade Study surface (ninebar
 * Phase 5).
 *
 * Recomposition: the results shell (ranked table + best-score summary)
 * is the full-bleed hero; alternatives/criteria/weights editing lives in
 * a "Configure trade study" MODAL that REUSES the existing props-driven
 * `AlternativesEditor` + `CriteriaEditor` verbatim. Unlike Sweep/MC the
 * modal is rendered DIRECTLY by this body (not the id registry): the
 * editors are threaded the `useTradeStudyConfig` hook state, whose
 * action surface is far richer than a flat store — mirroring it into
 * zustand would duplicate the state machine (the registry's only extra
 * power, Cmd-K opening, isn't worth that drift risk). The rail carries
 * the alternatives summary + Run; the strip shows completed/total
 * progress (this runner reports no per-child descriptors).
 *
 * The legacy two-column body (`TradeStudyWorkflow` flag-off) is
 * untouched.
 */

import { useCallback, useMemo, useState } from 'react';
import { LeftRailContent, BottomStripContent } from '@/app/slots';
import { Modal } from '@/shared/overlays/Modal';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import type { BatchStatus } from '@/engine/types';

import { AnalyzeBatchStrip } from '../ninebar/AnalyzeBatchStrip';
import { AnalyzeRail, RailListRow, RailEmptyHint, HeroNotice } from '../ninebar/chrome';
import { TradeStudyResultsShell } from './TradeStudyResultsShell';
import { AlternativesEditor } from './AlternativesEditor';
import { CriteriaEditor } from './CriteriaEditor';
import { useTradeStudyConfig } from './useTradeStudyConfig';
import { useTradeStudyRunner } from './useTradeStudyRunner';
import { useCandidateMetrics } from './useCandidateMetrics';

export function TradeStudyWorkflowNinebar() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);

  const config = useTradeStudyConfig();
  const runner = useTradeStudyRunner();
  const metrics = useCandidateMetrics(loadedUris);

  const [configOpen, setConfigOpen] = useState(false);
  const isRunning = runner.state === 'running';

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

  const stripStatus = useMemo<BatchStatus>(() => {
    switch (runner.state) {
      case 'running':
        return {
          kind: 'running',
          running: Math.max(0, (runner.progress?.total ?? 0) - (runner.progress?.completed ?? 0)),
          completed: runner.progress?.completed ?? 0,
        };
      case 'complete':
        return { kind: 'complete' };
      case 'error':
        return { kind: 'failed', reason: runner.error?.message ?? 'Trade study failed' };
      default:
        return { kind: 'pending' };
    }
  }, [runner.state, runner.progress, runner.error]);

  const hasResult = runner.result !== null;

  return (
    <div
      data-testid="tradestudy-workflow-ninebar"
      className="flex flex-col h-full w-full min-h-0"
      style={{ background: 'var(--surface-canvas)', color: 'var(--text-primary)' }}
    >
      <LeftRailContent>
        <AnalyzeRail
          icon="balance"
          title="Trade Study"
          headerCount={`${config.alternatives.length} ${config.alternatives.length === 1 ? 'alternative' : 'alternatives'}`}
          sectionTitle="Alternatives"
          onConfigure={() => setConfigOpen(true)}
          summary={[
            { label: 'alternatives', value: String(config.alternatives.length) },
            { label: 'criteria', value: String(config.criteria.length) },
            { label: 'evaluated', value: String(runner.progress?.completed ?? 0) },
          ]}
          plannedChildren={config.alternatives.length}
          plannedNoun="alternatives"
          runLabel="Run Trade Study"
          canRun={!!workspaceRoot && config.validation.canRun}
          isRunning={isRunning}
          onRun={handleRun}
          testIdPrefix="tradestudy"
        >
          {config.alternatives.length === 0 ? (
            <RailEmptyHint>
              No alternatives yet — Configure adds design alternatives and scoring criteria.
            </RailEmptyHint>
          ) : (
            <ul data-testid="tradestudy-rail-alternatives" style={{ listStyle: 'none', margin: 0, padding: '4px 0' }}>
              {config.alternatives.map((alt) => (
                <li key={alt.id}>
                  <RailListRow
                    testId={`tradestudy-rail-alternative-${alt.id}`}
                    name={alt.label}
                    detail={`${Object.keys(alt.overrides).length} overrides`}
                    onClick={() => setConfigOpen(true)}
                  />
                </li>
              ))}
            </ul>
          )}
        </AnalyzeRail>
      </LeftRailContent>

      <div className="flex-1 min-h-0 overflow-auto" data-testid="tradestudy-hero">
        {!hasResult && !isRunning && !runner.error ? (
          <HeroNotice
            testId="tradestudy-hero-empty"
            icon="balance"
            title="Define a trade study"
            detail="Add design alternatives (parameter overrides) and weighted criteria in the Configure modal, then run — the ranked results render here."
            action={{ label: 'Configure trade study', testId: 'tradestudy-hero-configure', onClick: () => setConfigOpen(true) }}
          />
        ) : (
          <TradeStudyResultsShell
            result={runner.result}
            isRunning={isRunning}
            progress={runner.progress}
            error={runner.error ? runner.error.message : null}
          />
        )}
      </div>

      <BottomStripContent>
        <AnalyzeBatchStrip
          methodLabel="Trade Study"
          status={stripStatus}
          children={[]}
          progress={runner.progress ?? null}
        />
      </BottomStripContent>

      {/* Config modal — direct render (see file doc for why not the registry). */}
      <Modal open={configOpen} onClose={() => setConfigOpen(false)} title="Configure trade study">
        <div data-testid="tradestudy-config-modal" className="flex flex-col gap-4" style={{ minWidth: 440 }}>
          <AlternativesEditor config={config} />
          <CriteriaEditor config={config} metrics={metrics} />
          {!config.validation.canRun && config.validation.reason && (
            <span data-testid="tradestudy-modal-validation" style={{ fontSize: 'var(--text-xs)', color: 'var(--severity-warning)' }}>
              {config.validation.reason}
            </span>
          )}
          <span style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>
            Changes apply immediately — close the modal and run from the rail.
          </span>
        </div>
      </Modal>
    </div>
  );
}
