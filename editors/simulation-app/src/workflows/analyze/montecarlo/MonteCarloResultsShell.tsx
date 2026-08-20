/**
 * MonteCarloResultsShell — mount point for R5.7 viewers (histograms,
 * percentile bands, pass-rate dashboard, CSV export).
 *
 * This component is intentionally a THIN shell. Round 5 agents FF/FF2
 * will fill in the concrete viewer rendering in parallel — they read
 * the `{ batchId, children }` prop pair below. Keep the prop shape
 * stable; changing it breaks the parallel work.
 *
 * Today (R5.6), the shell renders these states:
 *
 *   1. Empty — no batch kicked off yet; instructs the user to configure
 *      distributions on the left and click Run.
 *   2. Creating — the backend has been called but no batchId came back.
 *   3. Running — we have a batchId and a `completed / total` progress
 *      line. Children may stream in as the backend reports them.
 *   4. Error — the batch failed to create or reported an error.
 *   5. Complete — terse summary (batchId + child count); viewers land
 *      in R5.7 behind this.
 */

import type { MonteCarloChild } from './useMonteCarloRunner';
import { MonteCarloResultsPanel } from './MonteCarloResultsPanel';
import { useMonteCarloResults } from './useMonteCarloResults';
import { buildOutcomesFromChildren, collectConstraintIds } from './buildViewerInputs';

export interface MonteCarloResultsShellProps {
  /** Batch id returned by `sysml.batch.create`; null until creation completes. */
  batchId: string | null;
  /** Child sessions surfaced by `sysml.batch.status`. Empty until running. */
  children: MonteCarloChild[];
  /** Creating / running / complete / error state, surfaced by the runner. */
  state?: 'idle' | 'creating' | 'running' | 'complete' | 'error';
  /** Live completed count while running. */
  completed?: number;
  /** Total child count planned. */
  total?: number;
  /** Error message if the batch failed. */
  error?: string | null;
}

export function MonteCarloResultsShell({
  batchId,
  children,
  state = 'idle',
  completed = 0,
  total = 0,
  error = null,
}: MonteCarloResultsShellProps) {
  if (state === 'error' || error) {
    return (
      <EmptyPanel
        testId="montecarlo-results-error"
        icon="error"
        iconColor="var(--error)"
        title="Monte Carlo batch failed"
        hint={error ?? 'Unknown error'}
      />
    );
  }

  if (state === 'creating') {
    return (
      <EmptyPanel
        testId="montecarlo-results-creating"
        icon="progress_activity"
        title="Creating batch…"
        hint="Shipping sampled parameters to the backend."
        spinning
      />
    );
  }

  if (state === 'running') {
    const pct = total > 0 ? Math.round((completed / total) * 100) : 0;
    return (
      <div
        data-testid="montecarlo-results-running"
        className="flex flex-col items-center justify-center h-full w-full gap-3"
        style={{ color: 'var(--outline)' }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 32, animation: 'spin 1s linear infinite' }}
        >
          progress_activity
        </span>
        <span style={{ fontSize: 13 }}>Running Monte Carlo…</span>
        <span
          className="mono-text"
          data-testid="montecarlo-results-progress"
          style={{ fontSize: 11 }}
        >
          {completed} / {total} ({pct}%)
        </span>
        {batchId && (
          <span
            className="mono-text"
            data-testid="montecarlo-results-batchid"
            style={{ fontSize: 10 }}
          >
            batch: {batchId}
          </span>
        )}
      </div>
    );
  }

  if (state === 'complete') {
    return <CompleteState batchId={batchId} streamChildren={children} />;
  }

  // state === 'idle'
  return (
    <EmptyPanel
      testId="montecarlo-results-empty"
      icon="casino"
      title="Configure and Run Monte Carlo"
      hint="Pick parameters, choose a distribution, set the sample count, then click Run. Results will appear here."
    />
  );
}

// ── Complete state ──────────────────────────────────────────────────

/**
 * Rendered when the runner flips to `state === 'complete'`. Pulls
 * per-child descriptors (including verdicts) via `sysml.batch.results`,
 * derives the viewer inputs (outcomes + constraintIds), and mounts
 * `MonteCarloResultsPanel` — which itself composes
 * `PassRateDashboard`, `MonteCarloHistogramViewer`, and the CSV
 * download button.
 *
 * `streamChildren` is the list surfaced by the live polling loop.
 * Today the backend doesn't attach per-child output metrics on the
 * results response, so histograms fall back to sampled input params —
 * still useful for validating the sampler, and forward-compatible:
 * once a `metrics` field lands on `ChildDescriptor` the outcome
 * builder picks them up automatically.
 */
function CompleteState({
  batchId,
  streamChildren,
}: {
  batchId: string | null;
  streamChildren: MonteCarloChild[];
}) {
  const { children: results, isLoading, isError, error } =
    useMonteCarloResults(batchId, true);

  if (!batchId) {
    return (
      <EmptyPanel
        testId="montecarlo-results-complete-nobatch"
        icon="warning"
        title="Batch missing"
        hint="Completed batches must have an id — reload and try again."
      />
    );
  }

  if (isLoading) {
    return (
      <EmptyPanel
        testId="montecarlo-results-complete-loading"
        icon="progress_activity"
        title="Loading results…"
        hint="Fetching per-iteration data from the backend."
        spinning
      />
    );
  }

  if (isError) {
    return (
      <EmptyPanel
        testId="montecarlo-results-complete-error"
        icon="error"
        iconColor="var(--error)"
        title="Could not load results"
        hint={error?.message ?? 'sysml.batch.results returned an error.'}
      />
    );
  }

  // Prefer the result set (carries verdicts + params); fall back to the
  // streaming view if the backend returned no rows for some reason.
  const effective = results.length > 0 ? results : [];
  if (effective.length === 0 && streamChildren.length > 0) {
    // Rare: completion signal arrived before results landed — surface a
    // terse confirmation so the user isn't staring at a blank pane.
    return (
      <EmptyPanel
        testId="montecarlo-results-complete-empty"
        icon="inventory_2"
        title={`Monte Carlo complete (${streamChildren.length} runs)`}
        hint={`batch: ${batchId}`}
      />
    );
  }

  const outcomes = buildOutcomesFromChildren(effective);
  const constraintIds = collectConstraintIds(effective);

  return (
    <div
      data-testid="montecarlo-results-complete"
      className="flex flex-col h-full w-full overflow-auto p-6"
      style={{ color: 'var(--on-surface)' }}
    >
      <MonteCarloResultsPanel
        batchId={batchId}
        children={effective}
        outcomes={outcomes}
        constraintIds={constraintIds}
      />
    </div>
  );
}

// ── Sub-components ──────────────────────────────────────────────────

function EmptyPanel({
  testId,
  icon,
  iconColor,
  title,
  hint,
  spinning = false,
}: {
  testId: string;
  icon: string;
  iconColor?: string;
  title: string;
  hint: string;
  spinning?: boolean;
}) {
  return (
    <div
      data-testid={testId}
      className="flex flex-col items-center justify-center h-full w-full gap-2"
      style={{ color: iconColor ?? 'var(--outline)' }}
    >
      <span
        className="material-symbols-outlined"
        style={{
          fontSize: 32,
          opacity: 0.85,
          animation: spinning ? 'spin 1s linear infinite' : undefined,
        }}
      >
        {icon}
      </span>
      <span style={{ fontSize: 13, fontWeight: 600 }}>{title}</span>
      <span
        style={{
          fontSize: 11,
          maxWidth: 360,
          textAlign: 'center',
          color: 'var(--outline)',
        }}
      >
        {hint}
      </span>
    </div>
  );
}
