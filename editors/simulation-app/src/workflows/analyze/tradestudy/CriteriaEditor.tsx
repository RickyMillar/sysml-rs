/**
 * CriteriaEditor — multi-select picker over the MetricRegistry + per-metric
 * objective (Min/Max) + optional weight field.
 *
 * Each already-added criterion shows up as a row with:
 *   - metric name
 *   - Min/Max objective toggle (segmented pill pair)
 *   - numeric weight input (raw; the runner normalises before submit)
 *   - remove button
 *
 * A trailing "Add criterion" select lets the user pick any metric that
 * isn't already selected. Matches the look of VerifyConfig's suite
 * selector.
 *
 * UI-only — state lives in `useTradeStudyConfig`.
 */

import { useMemo } from 'react';
import type { MetricDescriptor } from '@/shared/metrics/types';
import type {
  TradeStudyConfigState,
  CriterionConfig,
  TradeStudyObjective,
} from './useTradeStudyConfig';

export interface CriteriaEditorProps {
  config: TradeStudyConfigState;
  /** Full registry snapshot — passed in so the picker can show names
   *  even when the consumer has no other view on the registry. */
  metrics: readonly MetricDescriptor[];
}

export function CriteriaEditor({ config, metrics }: CriteriaEditorProps) {
  const {
    criteria,
    normalizedWeights,
    weights,
    addCriterion,
    removeCriterion,
    setObjective,
    setWeight,
    resetWeights,
  } = config;

  // Metrics NOT yet selected — the add-picker's options.
  const available = useMemo(() => {
    const chosen = new Set(criteria.map((c) => c.metricId));
    return metrics.filter((m) => !chosen.has(m.id));
  }, [criteria, metrics]);

  return (
    <section
      data-testid="tradestudy-config-criteria"
      className="flex flex-col gap-2 px-3 py-3"
      style={{ borderBottom: '1px solid var(--outline-variant)' }}
    >
      <div className="flex items-center gap-2">
        <span
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: 'var(--outline)',
            letterSpacing: '0.04em',
            textTransform: 'uppercase',
          }}
        >
          Criteria
        </span>
        <span
          className="mono-text"
          style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          data-testid="tradestudy-criteria-count"
        >
          {criteria.length}
        </span>
        {criteria.length > 0 && (
          <button
            type="button"
            data-testid="tradestudy-reset-weights"
            onClick={resetWeights}
            style={{
              background: 'transparent',
              border: 'none',
              color: 'var(--primary)',
              fontSize: 11,
              cursor: 'pointer',
              padding: '2px 4px',
            }}
          >
            Reset weights
          </button>
        )}
      </div>

      {criteria.length === 0 ? (
        <div
          data-testid="tradestudy-criteria-empty"
          style={{ fontSize: 11, color: 'var(--outline)', lineHeight: 1.4 }}
        >
          Pick at least one metric. Min/Max defaults follow the TradeStudies
          stdlib conventions (cost/latency/error/penalty → Min; otherwise Max).
        </div>
      ) : (
        <ul
          data-testid="tradestudy-criteria-list"
          style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 4 }}
        >
          {criteria.map((c, i) => {
            const metric = metrics.find((m) => m.id === c.metricId);
            return (
              <CriterionRow
                key={c.metricId}
                criterion={c}
                name={metric?.name ?? c.metricId}
                unit={metric?.unit}
                rawWeight={weights[i] ?? 0}
                normWeight={normalizedWeights[i] ?? 0}
                onSetObjective={(obj) => setObjective(c.metricId, obj)}
                onSetWeight={(w) => setWeight(c.metricId, w)}
                onRemove={() => removeCriterion(c.metricId)}
              />
            );
          })}
        </ul>
      )}

      {/* Picker for adding a new criterion */}
      <div className="flex items-center gap-2">
        <select
          data-testid="tradestudy-add-criterion"
          value=""
          disabled={available.length === 0}
          onChange={(e) => {
            const id = e.target.value;
            if (!id) return;
            const m = metrics.find((x) => x.id === id);
            addCriterion(id, m?.name);
            // Reset the select to its placeholder.
            e.target.value = '';
          }}
          style={{
            flex: 1,
            height: 26,
            padding: '0 6px',
            background: 'var(--surface-container)',
            color: 'var(--on-surface)',
            border: '1px solid var(--outline-variant)',
            borderRadius: 6,
            fontSize: 12,
          }}
        >
          <option value="">
            {available.length === 0 ? 'All metrics added' : 'Add criterion…'}
          </option>
          {available.map((m) => (
            <option key={m.id} value={m.id}>
              {m.name}{m.unit ? ` (${m.unit})` : ''}
            </option>
          ))}
        </select>
      </div>
    </section>
  );
}

// ── CriterionRow ────────────────────────────────────────────────────

function CriterionRow({
  criterion,
  name,
  unit,
  rawWeight,
  normWeight,
  onSetObjective,
  onSetWeight,
  onRemove,
}: {
  criterion: CriterionConfig;
  name: string;
  unit?: string;
  rawWeight: number;
  normWeight: number;
  onSetObjective: (obj: TradeStudyObjective) => void;
  onSetWeight: (w: number) => void;
  onRemove: () => void;
}) {
  return (
    <li
      data-testid={`tradestudy-criterion-${criterion.metricId}`}
      style={{
        background: 'var(--surface-container)',
        border: '1px solid var(--outline-variant)',
        borderRadius: 6,
        padding: 6,
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
      }}
    >
      <div className="flex items-center gap-2">
        <div className="flex-1 min-w-0">
          <div
            className="truncate"
            style={{ fontSize: 12, color: 'var(--on-surface)' }}
          >
            {name}
          </div>
          {unit && (
            <div
              className="truncate mono-text"
              style={{ fontSize: 10, color: 'var(--outline)' }}
            >
              {unit}
            </div>
          )}
        </div>
        <button
          type="button"
          aria-label="Remove criterion"
          data-testid={`tradestudy-criterion-remove-${criterion.metricId}`}
          onClick={onRemove}
          style={{
            background: 'transparent',
            border: 'none',
            cursor: 'pointer',
            color: 'var(--outline)',
            display: 'flex',
            alignItems: 'center',
            padding: 2,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
            close
          </span>
        </button>
      </div>

      <div className="flex items-center gap-1">
        <div
          role="radiogroup"
          aria-label={`${name} objective`}
          className="flex"
          style={{
            borderRadius: 5,
            overflow: 'hidden',
            border: '1px solid var(--outline-variant)',
          }}
        >
          <ObjectivePill
            label="Min"
            active={criterion.objective === 'min'}
            onClick={() => onSetObjective('min')}
            testId={`tradestudy-objective-min-${criterion.metricId}`}
          />
          <ObjectivePill
            label="Max"
            active={criterion.objective === 'max'}
            onClick={() => onSetObjective('max')}
            testId={`tradestudy-objective-max-${criterion.metricId}`}
          />
        </div>
        <div className="flex items-center gap-1" style={{ marginLeft: 'auto' }}>
          <span
            className="mono-text"
            style={{ fontSize: 10, color: 'var(--outline)' }}
          >
            w
          </span>
          <input
            type="number"
            step="0.1"
            min="0"
            data-testid={`tradestudy-weight-${criterion.metricId}`}
            value={rawWeight}
            onChange={(e) => {
              const n = parseFloat(e.target.value);
              onSetWeight(Number.isFinite(n) ? n : 0);
            }}
            style={{
              width: 54,
              height: 22,
              padding: '0 4px',
              background: 'var(--surface-container-high)',
              color: 'var(--on-surface)',
              border: '1px solid var(--outline-variant)',
              borderRadius: 4,
              fontSize: 11,
              fontFamily: 'monospace',
              textAlign: 'right',
            }}
          />
          <span
            className="mono-text"
            data-testid={`tradestudy-weight-norm-${criterion.metricId}`}
            title="Normalised weight (sums to 1 on submit)"
            style={{ fontSize: 10, color: 'var(--outline)' }}
          >
            → {normWeight.toFixed(2)}
          </span>
        </div>
      </div>
    </li>
  );
}

function ObjectivePill({
  label,
  active,
  onClick,
  testId,
}: {
  label: string;
  active: boolean;
  onClick: () => void;
  testId: string;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      data-testid={testId}
      data-active={active}
      onClick={onClick}
      style={{
        width: 42,
        height: 22,
        background: active ? 'var(--primary-container)' : 'transparent',
        color: active ? 'var(--on-primary-container)' : 'var(--outline)',
        border: 'none',
        fontSize: 11,
        fontWeight: active ? 600 : 500,
        cursor: 'pointer',
      }}
    >
      {label}
    </button>
  );
}
