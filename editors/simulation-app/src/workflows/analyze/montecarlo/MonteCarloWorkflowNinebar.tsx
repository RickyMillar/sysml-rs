/**
 * MonteCarloWorkflowNinebar — the flag-on Monte Carlo surface (ninebar
 * Phase 5).
 *
 * Same recomposition as Sweep: the histogram / pass-rate viewer kit is
 * the full-bleed hero, the distribution editors live in the "Configure
 * Monte Carlo" MODAL (`McConfigModalNinebar`), the left rail carries the
 * study summary + Run (shared `AnalyzeRail` chrome), and the bottom
 * strip is the shared `AnalyzeBatchStrip`. This body also finally WIRES
 * `features/stats` (audit F5 — QQ plot + moments/CI overlays, 0
 * importers since R7.2): `MonteCarloStatsPanel` composes above the
 * histograms on the complete state, fed the same outcome extractors.
 *
 * The legacy two-column body (`montecarlo/MonteCarloWorkflow` flag-off)
 * is untouched.
 */

import { useMemo } from 'react';
import { LeftRailContent, BottomStripContent } from '@/app/slots';
import { useModalStore } from '@/shared/overlays/modalStore';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { MonteCarloStatsPanel } from '@/features/stats';
import type { BatchStatus, ChildDescriptor as EngineChild, ChildStatus } from '@/engine/types';

import { AnalyzeBatchStrip } from '../ninebar/AnalyzeBatchStrip';
import { AnalyzeRail, RailListRow, RailEmptyHint, HeroNotice } from '../ninebar/chrome';
import { useMonteCarloRunner, type MonteCarloChild } from './useMonteCarloRunner';
import { useMonteCarloResults } from './useMonteCarloResults';
import { MonteCarloResultsPanel } from './MonteCarloResultsPanel';
import { buildOutcomesFromChildren, collectConstraintIds } from './buildViewerInputs';
import { useMcStudyStore, mcStudyIsValid } from './useMcStudyStore';
import { MC_CONFIG_MODAL_ID } from './McConfigModalNinebar';
import type { Distribution } from './sampleDistribution';
// Registers the 'analyze-montecarlo-config' modal (module side effect).
import './McConfigModalNinebar';

export function MonteCarloWorkflowNinebar() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);

  const distributions = useMcStudyStore((s) => s.distributions);
  const sampleCount = useMcStudyStore((s) => s.sampleCount);
  const seed = useMcStudyStore((s) => s.seed);
  const openModal = useModalStore((s) => s.openModal);

  const runner = useMonteCarloRunner();
  const isRunning = runner.state === 'creating' || runner.state === 'running';
  const isTerminal = runner.state === 'complete' || runner.state === 'error';

  // Results (with verdicts) are fetched once the batch settles — the
  // same query the legacy complete-state uses.
  const results = useMonteCarloResults(runner.batchId, runner.state === 'complete');

  const isValid = mcStudyIsValid(distributions);
  const names = Object.keys(distributions);

  const handleRun = () => {
    if (!isValid || loadedUris.length === 0) return;
    void runner.run({
      workspaceRoot,
      uri: loadedUris[0]!,
      distributions,
      count: sampleCount,
      seed,
    });
  };

  // Strip children: live poll shape while running, verdict-carrying
  // results once complete (so failing/promote read the verdict ladder).
  const stripChildren = useMemo<EngineChild[]>(() => {
    if (runner.state === 'complete' && results.children.length > 0) {
      return results.children.map((c) => ({
        session_id: c.session_id ?? null,
        index: c.index,
        params: c.params ?? {},
        // passRateHelpers' status vocabulary also has 'cancelled', which
        // the engine union lacks (debt note) — a cancelled child is
        // terminal-without-results, closest to 'failed' for the counts.
        status: normaliseStatus(c.status),
        verdicts: c.verdicts,
      }));
    }
    return runner.children.map(toEngineChild);
  }, [runner.state, runner.children, results.children]);

  const stripStatus = useMemo<BatchStatus>(() => {
    switch (runner.state) {
      case 'running':
        return {
          kind: 'running',
          running: Math.max(0, runner.total - runner.completed),
          completed: runner.completed,
        };
      case 'complete':
        return { kind: 'complete' };
      case 'error':
        return { kind: 'failed', reason: runner.error ?? 'Monte Carlo batch failed' };
      case 'creating':
      case 'idle':
      default:
        return { kind: 'pending' };
    }
  }, [runner.state, runner.total, runner.completed, runner.error]);

  const outcomes = useMemo(
    () => (results.children.length > 0 ? buildOutcomesFromChildren(results.children) : []),
    [results.children],
  );
  const constraintIds = useMemo(
    () => (results.children.length > 0 ? collectConstraintIds(results.children) : []),
    [results.children],
  );

  const failing = stripChildren.filter((c) => (c.verdicts ?? []).some((v) => v.verdict === 'fail')).length;

  return (
    <div
      data-testid="montecarlo-workflow-ninebar"
      className="flex flex-col h-full w-full min-h-0"
      style={{ background: 'var(--surface-canvas)', color: 'var(--text-primary)' }}
    >
      <LeftRailContent>
        <AnalyzeRail
          icon="casino"
          title="Monte Carlo"
          headerCount={`${names.length} ${names.length === 1 ? 'parameter' : 'parameters'}`}
          sectionTitle="Distributions"
          onConfigure={() => openModal(MC_CONFIG_MODAL_ID)}
          summary={[
            { label: 'samples', value: String(sampleCount) },
            { label: 'seed', value: seed === null ? 'auto' : String(seed) },
            { label: 'evaluated', value: String(stripChildren.filter((c) => c.status === 'complete').length) },
            { label: 'failing', value: String(failing), tone: failing > 0 ? 'fail' : undefined },
          ]}
          plannedChildren={sampleCount}
          plannedNoun="samples"
          runLabel="Run Monte Carlo"
          canRun={!!workspaceRoot && isValid}
          isRunning={isRunning}
          onRun={handleRun}
          testIdPrefix="mc"
        >
          {names.length === 0 ? (
            <RailEmptyHint>
              No parameters yet — Configure adds parameters and their sampling distributions.
            </RailEmptyHint>
          ) : (
            <ul data-testid="mc-rail-distributions" style={{ listStyle: 'none', margin: 0, padding: '4px 0' }}>
              {names.map((name) => (
                <li key={name}>
                  <RailListRow
                    testId={`mc-rail-distribution-${name}`}
                    name={name}
                    detail={summariseDistribution(distributions[name]!)}
                    onClick={() => openModal(MC_CONFIG_MODAL_ID)}
                  />
                </li>
              ))}
            </ul>
          )}
        </AnalyzeRail>
      </LeftRailContent>

      <div className="flex-1 min-h-0 overflow-auto" data-testid="mc-hero">
        {runner.state === 'error' ? (
          <HeroNotice
            testId="mc-hero-error"
            icon="error"
            tone="error"
            title="Monte Carlo batch failed"
            detail={runner.error ?? 'Unknown error'}
          />
        ) : runner.state === 'idle' ? (
          <HeroNotice
            testId="mc-hero-empty"
            icon="casino"
            title="Define a Monte Carlo study"
            detail="Pick parameters and sampling distributions in the Configure modal, then run — histograms, pass rates, and distribution statistics render here."
            action={{ label: 'Configure Monte Carlo', testId: 'mc-hero-configure', onClick: () => openModal(MC_CONFIG_MODAL_ID) }}
          />
        ) : isRunning ? (
          <HeroNotice
            testId="mc-hero-running"
            icon="casino"
            title={runner.state === 'creating' ? 'Creating batch…' : `Sampling — ${runner.completed} of ${runner.total}`}
            detail="Children stream in as the batch advances; results and statistics render when the batch settles."
          />
        ) : results.isLoading ? (
          <HeroNotice testId="mc-hero-loading" icon="casino" title="Loading results…" detail="Fetching per-iteration data from the backend." />
        ) : results.isError ? (
          <HeroNotice
            testId="mc-hero-results-error"
            icon="error"
            tone="error"
            title="Could not load results"
            detail={results.error?.message ?? 'sysml.batch.results returned an error.'}
          />
        ) : runner.batchId && results.children.length > 0 ? (
          <div className="flex flex-col gap-3 p-3" data-testid="mc-hero-complete">
            {/* features/stats wired (audit F5): moments + CI + QQ per outcome. */}
            <MonteCarloStatsPanel
              children={results.children}
              outcomes={outcomes}
              testId="mc-stats-panel"
            />
            <MonteCarloResultsPanel
              batchId={runner.batchId}
              children={results.children}
              outcomes={outcomes}
              constraintIds={constraintIds}
            />
          </div>
        ) : isTerminal ? (
          <HeroNotice
            testId="mc-hero-no-results"
            icon="inventory_2"
            title="Batch complete — no per-iteration results returned"
            detail={runner.batchId ? `batch: ${runner.batchId}` : 'The batch id was lost; re-run the study.'}
          />
        ) : (
          <HeroNotice testId="mc-hero-pending" icon="casino" title="Batch pending" detail="Waiting for the backend to accept the study." />
        )}
      </div>

      <BottomStripContent>
        <AnalyzeBatchStrip methodLabel="Monte Carlo" status={stripStatus} children={stripChildren} />
      </BottomStripContent>
    </div>
  );
}

/** Poll-shape child → engine shape for the strip counts. */
function toEngineChild(c: MonteCarloChild, index: number): EngineChild {
  return {
    session_id: c.id || null,
    index,
    params: c.params ?? {},
    status: normaliseStatus(c.status),
  };
}

function normaliseStatus(raw: string | null | undefined): ChildStatus {
  switch (raw) {
    case 'running':
      return 'running';
    case 'complete':
      return 'complete';
    case 'failed':
    case 'error':
    case 'cancelled':
      return 'failed';
    default:
      return 'pending';
  }
}

export function summariseDistribution(dist: Distribution): string {
  switch (dist.kind) {
    case 'normal':
      return `N(${dist.mean}, ${dist.sigma})`;
    case 'uniform':
      return `U(${dist.min}, ${dist.max})`;
    case 'triangular':
      return `T(${dist.min}, ${dist.mode}, ${dist.max})`;
    case 'custom-cdf':
      return `CDF · ${dist.points.length} pts`;
  }
}
