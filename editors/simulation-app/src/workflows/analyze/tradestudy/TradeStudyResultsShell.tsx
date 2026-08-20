/**
 * TradeStudyResultsShell — mount point for the R5.11 results viewer.
 *
 * Intentionally a THIN shell. Agent HH (R5.11) fills in the concrete
 * results rendering (ranked table + Pareto front + promote-to-compare
 * button) in parallel. All of HH's work consumes the single
 * `result: TradeStudyRunResult | null` prop below — keep the prop shape
 * stable so the parallel work does not break.
 *
 * The shell renders three states today:
 *
 *   1. `error`     — terminal error from the runner.
 *   2. `isRunning` — spinner + progress line.
 *   3. Otherwise the empty instruction card (no result yet), or a terse
 *      placeholder summary once a result is in hand.
 */

import type { TradeStudyRunResult, TradeStudyRunnerProgress } from './useTradeStudyRunner';

export interface TradeStudyResultsShellProps {
  /**
   * The trade-study result to render. Populated by the R5.10 runner once
   * a run completes; null while the config panel is being set up or no
   * run has fired yet. HH's viewer takes over in R5.11.
   */
  result: TradeStudyRunResult | null;
  /** True while a trade-study run is in flight. */
  isRunning?: boolean;
  /** Optional fan-out progress — rendered as "N/M complete". */
  progress?: TradeStudyRunnerProgress | null;
  /** Optional error message from the most recent run. */
  error?: string | null;
}

export function TradeStudyResultsShell({
  result,
  isRunning = false,
  progress = null,
  error = null,
}: TradeStudyResultsShellProps) {
  if (error) {
    return (
      <div
        data-testid="tradestudy-results-error"
        className="flex flex-col items-center justify-center h-full w-full gap-2"
        style={{ color: 'var(--error)' }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 32, opacity: 0.85 }}
        >
          error
        </span>
        <span style={{ fontSize: 13, fontWeight: 600 }}>Trade study failed</span>
        <span style={{ fontSize: 11, maxWidth: 360, textAlign: 'center' }}>
          {error}
        </span>
      </div>
    );
  }

  if (isRunning) {
    return (
      <div
        data-testid="tradestudy-results-running"
        className="flex flex-col items-center justify-center h-full w-full gap-2"
        style={{ color: 'var(--outline)' }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 32, animation: 'spin 1s linear infinite' }}
        >
          progress_activity
        </span>
        <span style={{ fontSize: 13 }}>Running trade study…</span>
        {progress && progress.total > 0 && (
          <span
            className="mono-text"
            data-testid="tradestudy-results-progress"
            style={{ fontSize: 11 }}
          >
            {progress.completed} / {progress.total}
            {progress.label ? ` · ${progress.label}` : ''}
          </span>
        )}
      </div>
    );
  }

  if (!result) {
    return (
      <div
        data-testid="tradestudy-results-empty"
        className="flex flex-col items-center justify-center h-full w-full gap-2"
        style={{ color: 'var(--outline)' }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 32, opacity: 0.75 }}
        >
          balance
        </span>
        <span style={{ fontSize: 13, fontWeight: 500 }}>
          Configure alternatives and run
        </span>
        <span
          style={{
            fontSize: 11,
            maxWidth: 360,
            textAlign: 'center',
            color: 'var(--outline)',
          }}
        >
          The ranked results table, Pareto front, and promote-to-compare
          controls will appear here once a run completes.
        </span>
      </div>
    );
  }

  // Placeholder — R5.11 viewer mounts here. For now we render a minimal
  // summary so integrators can confirm the wiring round-trips.
  return (
    <div
      data-testid="tradestudy-results-placeholder"
      className="flex flex-col h-full w-full overflow-auto p-6"
      style={{ color: 'var(--on-surface)' }}
    >
      <div
        style={{
          fontSize: 12,
          fontWeight: 600,
          color: 'var(--outline)',
          letterSpacing: '0.04em',
          textTransform: 'uppercase',
          marginBottom: 8,
        }}
      >
        Results ({result.rows.length})
      </div>
      <div
        data-testid="tradestudy-results-best"
        className="mono-text"
        style={{ fontSize: 12, color: 'var(--on-surface-variant)', marginBottom: 8 }}
      >
        Best: {result.bestLabel ?? '—'}
        {result.bestScore !== null ? ` · ${result.bestScore.toFixed(4)}` : ''}
      </div>
      <ul
        data-testid="tradestudy-results-rows"
        style={{
          listStyle: 'none',
          margin: 0,
          padding: 0,
          display: 'flex',
          flexDirection: 'column',
          gap: 4,
        }}
      >
        {result.rows.map((row) => (
          <li
            key={`${row.index}-${row.label}`}
            data-testid={`tradestudy-result-row-${row.index}`}
            className="mono-text"
            style={{
              fontSize: 12,
              color: 'var(--on-surface-variant)',
              display: 'flex',
              gap: 8,
            }}
          >
            <span style={{ width: 24, color: 'var(--outline)' }}>#{row.index + 1}</span>
            <span style={{ flex: 1 }}>{row.label}</span>
            <span>
              {Number.isFinite(row.score) ? row.score.toFixed(4) : '—'}
              {row.error ? ` · ${row.error}` : ''}
            </span>
          </li>
        ))}
      </ul>
      <div
        style={{
          marginTop: 16,
          fontSize: 11,
          color: 'var(--outline)',
          maxWidth: 520,
        }}
      >
        Ranked table, Pareto front, and promote-to-compare land in R5.11.
      </div>
    </div>
  );
}
