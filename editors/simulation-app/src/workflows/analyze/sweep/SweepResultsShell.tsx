/**
 * SweepResultsShell — mount point for R5.2 / R5.3 / R5.5 viewers.
 *
 * This component is intentionally a THIN shell. Round 5 agents fill in
 * the real rendering in parallel:
 *
 *   - CC (R5.2 / R5.3) — streaming table, tornado, parallel-coords, heatmap
 *   - DD (R5.5)        — drill any row back into RunWorkflow
 *
 * All viewers read from `{ batchId, children, status }`. Keep the prop
 * shape stable — changing it breaks the parallel work.
 *
 * Today (R5.1), the shell renders three states:
 *
 *   1. Empty state (no batch yet) → "Configure ranges and click Run
 *      Sweep". This is the first thing the user sees when they land on
 *      `/analyze/sweep`.
 *   2. Running state → spinner + "N of M complete" counter. CC's viewer
 *      replaces this with the streaming table in R5.2.
 *   3. Results-available state → count badge + verdict rollup. CC's
 *      viewer kit mounts below this header.
 */

import type { BatchStatus, ChildDescriptor } from '@/engine/types';

export interface SweepResultsShellProps {
  /**
   * Backend batch id once `sysml.batch.create` has returned; `null`
   * before any run has been kicked off or after a reset.
   */
  batchId: string | null;
  /**
   * Live child descriptors — streamed in by the runner as the backend
   * advances the batch. One entry per cartesian-product point.
   */
  children: ChildDescriptor[];
  /** Aggregate batch lifecycle state. */
  status: BatchStatus;
  /** Optional error message from the most recent run. */
  error?: string | null;
}

export function SweepResultsShell({
  batchId,
  children,
  status,
  error = null,
}: SweepResultsShellProps) {
  if (error || status.kind === 'failed') {
    const reason = error ?? (status.kind === 'failed' ? status.reason : 'Unknown error');
    return (
      <div
        data-testid="sweep-results-error"
        className="flex flex-col items-center justify-center h-full w-full gap-2"
        style={{ color: 'var(--error)' }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 32, opacity: 0.85 }}
        >
          error
        </span>
        <span style={{ fontSize: 13, fontWeight: 600 }}>Sweep failed</span>
        <span style={{ fontSize: 11, maxWidth: 360, textAlign: 'center' }}>
          {reason}
        </span>
      </div>
    );
  }

  // No batch yet — empty state prompting the user to configure and run.
  if (!batchId && status.kind === 'pending' && children.length === 0) {
    return (
      <div
        data-testid="sweep-results-empty"
        className="flex flex-col items-center justify-center h-full w-full gap-2"
        style={{ color: 'var(--outline)' }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 32, opacity: 0.75 }}
        >
          tune
        </span>
        <span style={{ fontSize: 13, fontWeight: 500 }}>
          Configure ranges and click Run Sweep
        </span>
        <span
          style={{
            fontSize: 11,
            maxWidth: 360,
            textAlign: 'center',
            color: 'var(--outline)',
          }}
        >
          Streaming table, tornado, parallel-coords, and heatmap
          viewers will render here once the batch completes.
        </span>
      </div>
    );
  }

  // Running — show progress + a soft spinner.
  if (status.kind === 'running') {
    return (
      <div
        data-testid="sweep-results-running"
        className="flex flex-col items-center justify-center h-full w-full gap-2"
        style={{ color: 'var(--outline)' }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 32, animation: 'spin 1s linear infinite' }}
        >
          progress_activity
        </span>
        <span style={{ fontSize: 13 }}>
          Running sweep — {status.completed} of{' '}
          {status.completed + status.running} complete
        </span>
        <span
          data-testid="sweep-results-count-badge"
          className="mono-text"
          style={{ fontSize: 11, color: 'var(--outline)' }}
        >
          {children.length} {children.length === 1 ? 'child' : 'children'}
        </span>
      </div>
    );
  }

  // Complete (or pending with children already seeded by a fresh batch
  // create) — render the placeholder header; CC mounts the real viewer
  // kit below.
  const statusLabel: string =
    !status || !status.kind
      ? '…'
      : status.kind === 'complete'
        ? 'Complete'
        : status.kind === 'pending'
          ? 'Queued'
          : String((status as { kind: string }).kind);

  return (
    <div
      data-testid="sweep-results-placeholder"
      className="flex flex-col h-full w-full overflow-auto p-6"
      style={{ color: 'var(--on-surface)' }}
    >
      <div
        className="flex items-center gap-3"
        style={{
          fontSize: 12,
          fontWeight: 600,
          color: 'var(--outline)',
          letterSpacing: '0.04em',
          textTransform: 'uppercase',
          marginBottom: 8,
        }}
      >
        <span>Sweep results</span>
        <span
          data-testid="sweep-results-count-badge"
          className="mono-text"
          style={{
            fontSize: 11,
            fontWeight: 500,
            color: 'var(--on-surface-variant)',
            padding: '2px 8px',
            background: 'var(--surface-container)',
            borderRadius: 999,
          }}
        >
          {children.length} {children.length === 1 ? 'child' : 'children'}
        </span>
        <span
          className="mono-text"
          style={{
            fontSize: 11,
            fontWeight: 500,
            color: 'var(--outline)',
            marginLeft: 'auto',
          }}
        >
          {statusLabel}
        </span>
      </div>
      <div
        style={{
          fontSize: 11,
          color: 'var(--outline)',
          maxWidth: 520,
        }}
      >
        Streaming table, tornado, parallel-coords and heatmap viewers
        render here as children complete. Drill-to-Run reveals the
        underlying session.
      </div>
    </div>
  );
}
