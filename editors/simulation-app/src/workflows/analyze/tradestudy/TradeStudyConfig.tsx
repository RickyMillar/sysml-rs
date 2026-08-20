/**
 * TradeStudyConfig — left-side config panel for /analyze/trade-study.
 *
 * Composes the three editing surfaces that make up a trade-study setup:
 *
 *   1. AlternativesEditor — N labelled alternatives, each with an
 *      `overrides: Record<string, Value>` map.
 *   2. CriteriaEditor     — multi-select over the MetricRegistry, plus
 *      per-metric objective (Min/Max) and per-metric weight.
 *   3. A "Run Trade Study" button in the footer. The button is a pure
 *      event-emitter; `TradeStudyWorkflow` owns what "run" actually does.
 *
 * State is owned by `useTradeStudyConfig` and passed in via the `config`
 * prop (matches the pattern used by `VerifyConfig`). Metric descriptors
 * are likewise passed in so the component stays dumb and testable.
 *
 * Validation: the Run button stays disabled until the config has at
 * least two alternatives AND at least one criterion. The reason for the
 * disabled state is surfaced under the footer summary so the user
 * doesn't have to guess.
 */

import { AlternativesEditor } from './AlternativesEditor';
import { CriteriaEditor } from './CriteriaEditor';
import type { TradeStudyConfigState } from './useTradeStudyConfig';
import type { MetricDescriptor } from '@/shared/metrics/types';

export interface TradeStudyConfigProps {
  /** The config state (from `useTradeStudyConfig`). */
  config: TradeStudyConfigState;
  /** Available metrics for the criterion picker. */
  metrics: readonly MetricDescriptor[];
  /** True while a trade study run is in flight — disables Run. */
  isRunning?: boolean;
  /** Whether a workspace is loaded — gates the panel. */
  hasWorkspace?: boolean;
  /** Click handler for the Run Trade Study button. */
  onRun: () => void;
}

export function TradeStudyConfig({
  config,
  metrics,
  isRunning = false,
  hasWorkspace = true,
  onRun,
}: TradeStudyConfigProps) {
  const { alternatives, criteria, validation } = config;
  const canRun = validation.canRun && !isRunning && hasWorkspace;

  return (
    <aside
      data-testid="tradestudy-config"
      className="flex flex-col shrink-0 h-full overflow-hidden"
      style={{
        width: 340,
        borderRight: '1px solid var(--outline-variant)',
        background: 'var(--surface-container-low)',
      }}
    >
      {/* Header */}
      <div
        className="flex items-center gap-2 px-3 shrink-0"
        style={{
          height: 36,
          borderBottom: '1px solid var(--outline-variant)',
        }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 16, color: 'var(--primary)' }}
        >
          balance
        </span>
        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--on-surface)' }}>
          Trade Study
        </span>
        <span
          className="mono-text"
          style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          data-testid="tradestudy-config-summary"
        >
          {alternatives.length} alt · {criteria.length} crit
        </span>
      </div>

      {/* Scrollable editor column */}
      <div className="flex-1 overflow-y-auto">
        <AlternativesEditor config={config} />
        <CriteriaEditor config={config} metrics={metrics} />
      </div>

      {/* Run button + validation note */}
      <section
        className="flex flex-col gap-2 px-3 py-3 shrink-0"
        style={{
          borderTop: '1px solid var(--outline-variant)',
          background: 'var(--surface-container)',
        }}
      >
        <div
          data-testid="tradestudy-run-summary"
          style={{ fontSize: 11, color: 'var(--outline)', lineHeight: 1.4 }}
        >
          {!hasWorkspace ? (
            <span>Load a workspace to run a trade study.</span>
          ) : validation.canRun ? (
            <span>
              {alternatives.length} alternatives · {criteria.length}{' '}
              {criteria.length === 1 ? 'criterion' : 'criteria'} · weights
              normalised on submit
            </span>
          ) : (
            <span data-testid="tradestudy-run-reason">{validation.reason}</span>
          )}
        </div>
        <button
          type="button"
          data-testid="tradestudy-run"
          disabled={!canRun}
          onClick={onRun}
          style={{
            height: 32,
            background: canRun ? 'var(--primary)' : 'var(--surface-container-high)',
            color: canRun ? 'var(--on-primary)' : 'var(--outline)',
            border: 'none',
            borderRadius: 6,
            fontSize: 12,
            fontWeight: 600,
            cursor: canRun ? 'pointer' : 'not-allowed',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 6,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
            play_arrow
          </span>
          {isRunning ? 'Running…' : 'Run Trade Study'}
        </button>
      </section>
    </aside>
  );
}
