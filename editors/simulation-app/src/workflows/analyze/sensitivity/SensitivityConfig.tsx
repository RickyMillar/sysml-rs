/**
 * SensitivityConfig — left-side config panel for /analyze/sensitivity
 * (R7.4).
 *
 * Stacked sections:
 *   1. Method picker — Morris / Sobol.
 *   2. Parameter multi-select with [min, max] range editor.
 *   3. Method-specific sampler knobs (r / p for Morris, N for Sobol).
 *   4. Output metric dropdown (from MetricRegistry + known verdict
 *      pseudo-metrics).
 *   5. Seed + Run button.
 *
 * The component is UI-only — selection / validation state lives on
 * `useSensitivityConfig`. Parameter candidates come from the parent
 * (SensitivityWorkflow pulls run targets from the workspace).
 */

import { useMemo, useState } from 'react';
import type { MetricDescriptor } from '@/shared/metrics/types';
import type { SensitivityMethod } from '@/engine/types';
import type { SensitivityConfigState } from './useSensitivityConfig';

export interface SensitivityParameterCandidate {
  id: string;
  label?: string;
  kind?: string;
  uri?: string;
}

export interface SensitivityConfigProps {
  availableParameters: SensitivityParameterCandidate[];
  config: SensitivityConfigState;
  availableMetrics: MetricDescriptor[];
  isRunning?: boolean;
  hasWorkspace?: boolean;
  isLoadingParameters?: boolean;
  onRun: () => void;
}

export function SensitivityConfig({
  availableParameters,
  config,
  availableMetrics,
  isRunning = false,
  hasWorkspace = true,
  isLoadingParameters = false,
  onRun,
}: SensitivityConfigProps) {
  const {
    method,
    setMethod,
    ranges,
    addRange,
    removeRange,
    updateRange,
    morrisR,
    morrisP,
    sobolN,
    setMorrisR,
    setMorrisP,
    setSobolN,
    outputMetric,
    setOutputMetric,
    seed,
    setSeed,
    isValid,
    childCount,
  } = config;

  const selectedIds = useMemo(
    () => new Set(ranges.map((r) => r.parameterId)),
    [ranges],
  );

  return (
    <aside
      data-testid="sensitivity-config"
      className="flex flex-col shrink-0 h-full overflow-hidden"
      style={{
        width: 360,
        borderRight: '1px solid var(--outline-variant)',
        background: 'var(--surface-container-low)',
      }}
    >
      <div
        className="flex items-center gap-2 px-3 shrink-0"
        style={{ height: 36, borderBottom: '1px solid var(--outline-variant)' }}
      >
        <span
          className="material-symbols-outlined"
          style={{ fontSize: 16, color: 'var(--primary)' }}
        >
          analytics
        </span>
        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--on-surface)' }}>
          Sensitivity Analysis
        </span>
        <span
          className="mono-text"
          style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          data-testid="sensitivity-config-summary"
        >
          {childCount} runs
        </span>
      </div>

      {/* Method picker */}
      <section
        data-testid="sensitivity-config-method"
        className="flex flex-col gap-2 px-3 py-3"
        style={{ borderBottom: '1px solid var(--outline-variant)' }}
      >
        <label
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: 'var(--outline)',
            letterSpacing: '0.04em',
            textTransform: 'uppercase',
          }}
        >
          Method
        </label>
        <div role="radiogroup" aria-label="Method" className="flex gap-1">
          <MethodPill
            label="Morris"
            hint="cheaper · screening"
            active={method === 'morris'}
            onClick={() => setMethod('morris')}
            testId="sensitivity-method-morris"
          />
          <MethodPill
            label="Sobol"
            hint="full variance"
            active={method === 'sobol'}
            onClick={() => setMethod('sobol')}
            testId="sensitivity-method-sobol"
          />
        </div>
        <p
          style={{ margin: 0, fontSize: 10, color: 'var(--outline)', lineHeight: 1.4 }}
        >
          {method === 'morris'
            ? 'Generates r trajectories (one perturbation at a time). Cheap screening.'
            : 'Generates N·(d+2) runs using Saltelli A/B/C matrices. Higher cost, richer decomposition.'}
        </p>
      </section>

      {/* Parameter picker */}
      <section
        data-testid="sensitivity-config-parameters"
        className="flex flex-col gap-2 px-3 py-3"
        style={{ borderBottom: '1px solid var(--outline-variant)' }}
      >
        <div className="flex items-baseline">
          <label
            style={{
              fontSize: 11,
              fontWeight: 600,
              color: 'var(--outline)',
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
            }}
          >
            Parameters
          </label>
          <span
            className="mono-text"
            style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          >
            {ranges.length} selected
          </span>
        </div>

        <ParameterPicker
          available={availableParameters}
          selectedIds={selectedIds}
          onPick={(c) =>
            addRange({
              parameterId: c.id,
              label: c.label ?? c.id,
              min: 0,
              max: 1,
            })
          }
          hasWorkspace={hasWorkspace}
          isLoading={isLoadingParameters}
        />

        {ranges.length === 0 ? (
          <p
            data-testid="sensitivity-ranges-empty"
            style={{
              margin: 0,
              fontSize: 11,
              color: 'var(--outline)',
              fontStyle: 'italic',
            }}
          >
            Pick a parameter above to set its sampling range.
          </p>
        ) : (
          <ul
            data-testid="sensitivity-range-list"
            style={{
              listStyle: 'none',
              margin: 0,
              padding: 0,
              display: 'flex',
              flexDirection: 'column',
              gap: 8,
            }}
          >
            {ranges.map((entry) => (
              <RangeRow
                key={entry.parameterId}
                parameterId={entry.parameterId}
                label={entry.label ?? entry.parameterId}
                min={entry.min}
                max={entry.max}
                onChange={(patch) => updateRange(entry.parameterId, patch)}
                onRemove={() => removeRange(entry.parameterId)}
              />
            ))}
          </ul>
        )}
      </section>

      {/* Method-specific knobs */}
      <section
        data-testid="sensitivity-config-sampler"
        className="flex flex-col gap-2 px-3 py-3"
        style={{ borderBottom: '1px solid var(--outline-variant)' }}
      >
        <label
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: 'var(--outline)',
            letterSpacing: '0.04em',
            textTransform: 'uppercase',
          }}
        >
          Sampler
        </label>
        {method === 'morris' ? (
          <div className="flex gap-2">
            <NumField
              label="r (trajectories)"
              value={morrisR}
              min={1}
              onChange={setMorrisR}
              testId="sensitivity-morris-r"
            />
            <NumField
              label="p (levels)"
              value={morrisP}
              min={2}
              onChange={setMorrisP}
              testId="sensitivity-morris-p"
            />
          </div>
        ) : (
          <NumField
            label="N (base samples)"
            value={sobolN}
            min={1}
            onChange={setSobolN}
            testId="sensitivity-sobol-n"
          />
        )}
      </section>

      {/* Output metric + seed */}
      <section
        data-testid="sensitivity-config-output"
        className="flex flex-col gap-2 px-3 py-3"
        style={{ borderBottom: '1px solid var(--outline-variant)' }}
      >
        <label
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: 'var(--outline)',
            letterSpacing: '0.04em',
            textTransform: 'uppercase',
          }}
        >
          Output metric
        </label>
        <OutputMetricPicker
          available={availableMetrics}
          value={outputMetric}
          onChange={setOutputMetric}
        />
        <label
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: 'var(--outline)',
            letterSpacing: '0.04em',
            textTransform: 'uppercase',
            marginTop: 6,
          }}
        >
          Seed
        </label>
        <NumField
          label="seed"
          value={seed}
          onChange={setSeed}
          testId="sensitivity-seed"
          min={0}
        />
      </section>

      {/* Run button */}
      <section
        className="flex flex-col gap-2 px-3 py-3 shrink-0"
        style={{
          borderTop: '1px solid var(--outline-variant)',
          background: 'var(--surface-container)',
          marginTop: 'auto',
        }}
      >
        <div
          data-testid="sensitivity-running-summary"
          style={{ fontSize: 11, color: 'var(--outline)', lineHeight: 1.4 }}
        >
          {childCount} runs · {method} · {ranges.length}{' '}
          {ranges.length === 1 ? 'parameter' : 'parameters'}
        </div>
        <button
          type="button"
          data-testid="sensitivity-run"
          disabled={!isValid || isRunning || !hasWorkspace}
          onClick={onRun}
          style={{
            height: 32,
            background:
              isValid && !isRunning && hasWorkspace
                ? 'var(--primary)'
                : 'var(--surface-container-high)',
            color:
              isValid && !isRunning && hasWorkspace
                ? 'var(--on-primary)'
                : 'var(--outline)',
            border: 'none',
            borderRadius: 6,
            fontSize: 12,
            fontWeight: 600,
            cursor: isValid && !isRunning && hasWorkspace ? 'pointer' : 'not-allowed',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 6,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
            play_arrow
          </span>
          {isRunning ? 'Running…' : 'Run Analysis'}
        </button>
      </section>
    </aside>
  );
}

// ── Sub-components ─────────────────────────────────────────────────

function MethodPill({
  label,
  hint,
  active,
  onClick,
  testId,
}: {
  label: string;
  hint: string;
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
        flex: 1,
        height: 38,
        background: active ? 'var(--primary-container)' : 'transparent',
        color: active ? 'var(--on-primary-container)' : 'var(--outline)',
        border: `1px solid ${active ? 'var(--primary)' : 'var(--outline-variant)'}`,
        borderRadius: 6,
        fontSize: 11,
        fontWeight: active ? 600 : 500,
        cursor: 'pointer',
        display: 'flex',
        flexDirection: 'column',
        justifyContent: 'center',
        alignItems: 'center',
        gap: 2,
      }}
    >
      <span style={{ fontSize: 12 }}>{label}</span>
      <span style={{ fontSize: 9, opacity: 0.8 }}>{hint}</span>
    </button>
  );
}

function ParameterPicker({
  available,
  selectedIds,
  onPick,
  hasWorkspace,
  isLoading,
}: {
  available: SensitivityParameterCandidate[];
  selectedIds: Set<string>;
  onPick: (c: SensitivityParameterCandidate) => void;
  hasWorkspace: boolean;
  isLoading: boolean;
}) {
  const [query, setQuery] = useState('');
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const notPicked = available.filter((c) => !selectedIds.has(c.id));
    if (!q) return notPicked.slice(0, 20);
    return notPicked
      .filter((c) => {
        const label = (c.label ?? c.id).toLowerCase();
        return label.includes(q) || c.id.toLowerCase().includes(q);
      })
      .slice(0, 20);
  }, [available, selectedIds, query]);

  const handleFreeEntry = () => {
    const q = query.trim();
    if (!q || selectedIds.has(q)) return;
    onPick({ id: q, label: q });
    setQuery('');
  };

  return (
    <div className="flex flex-col gap-1">
      <input
        type="text"
        data-testid="sensitivity-parameter-search"
        placeholder={
          hasWorkspace ? 'Search or type a parameter name…' : 'Load a workspace first'
        }
        value={query}
        disabled={!hasWorkspace}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            handleFreeEntry();
          }
        }}
        style={{
          height: 28,
          padding: '0 8px',
          background: 'var(--surface-container)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 6,
          fontSize: 12,
        }}
      />
      {filtered.length > 0 && (
        <ul
          data-testid="sensitivity-parameter-suggestions"
          style={{
            listStyle: 'none',
            margin: 0,
            padding: 2,
            maxHeight: 140,
            overflowY: 'auto',
            background: 'var(--surface-container)',
            border: '1px solid var(--outline-variant)',
            borderRadius: 6,
          }}
        >
          {filtered.map((c) => (
            <li key={c.id}>
              <button
                type="button"
                data-testid={`sensitivity-parameter-option-${c.id}`}
                onClick={() => {
                  onPick(c);
                  setQuery('');
                }}
                style={{
                  width: '100%',
                  background: 'transparent',
                  color: 'var(--on-surface)',
                  border: 'none',
                  padding: '4px 6px',
                  fontSize: 12,
                  textAlign: 'left',
                  cursor: 'pointer',
                }}
              >
                <span className="truncate">{c.label ?? c.id}</span>
                {c.kind && (
                  <span
                    className="mono-text"
                    style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 6 }}
                  >
                    {c.kind}
                  </span>
                )}
              </button>
            </li>
          ))}
        </ul>
      )}
      {isLoading && (
        <span
          data-testid="sensitivity-parameters-loading"
          style={{ fontSize: 11, color: 'var(--outline)' }}
        >
          Discovering parameters…
        </span>
      )}
    </div>
  );
}

function RangeRow({
  parameterId,
  label,
  min,
  max,
  onChange,
  onRemove,
}: {
  parameterId: string;
  label: string;
  min: number;
  max: number;
  onChange: (patch: { min?: number; max?: number }) => void;
  onRemove: () => void;
}) {
  return (
    <li
      data-testid={`sensitivity-range-row-${parameterId}`}
      style={{
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
        padding: 8,
        background: 'var(--surface-container)',
        border: '1px solid var(--outline-variant)',
        borderRadius: 6,
      }}
    >
      <div className="flex items-center gap-2">
        <span
          className="truncate"
          style={{ fontSize: 12, fontWeight: 500, color: 'var(--on-surface)', flex: 1 }}
        >
          {label}
        </span>
        <button
          type="button"
          data-testid={`sensitivity-range-remove-${parameterId}`}
          onClick={onRemove}
          aria-label={`Remove ${label}`}
          style={{
            background: 'transparent',
            border: 'none',
            color: 'var(--outline)',
            cursor: 'pointer',
            padding: 2,
            display: 'flex',
            alignItems: 'center',
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
            close
          </span>
        </button>
      </div>
      <div className="flex gap-1">
        <NumField
          label="min"
          value={min}
          onChange={(v) => onChange({ min: v })}
          testId={`sensitivity-range-min-${parameterId}`}
        />
        <NumField
          label="max"
          value={max}
          onChange={(v) => onChange({ max: v })}
          testId={`sensitivity-range-max-${parameterId}`}
        />
      </div>
    </li>
  );
}

function NumField({
  label,
  value,
  onChange,
  testId,
  min,
}: {
  label: string;
  value: number;
  onChange: (v: number) => void;
  testId: string;
  min?: number;
}) {
  return (
    <label
      className="flex flex-col"
      style={{ flex: 1, fontSize: 10, color: 'var(--outline)' }}
    >
      <span>{label}</span>
      <input
        type="number"
        data-testid={testId}
        value={Number.isFinite(value) ? value : ''}
        min={min}
        onChange={(e) => {
          const n = Number(e.target.value);
          onChange(Number.isFinite(n) ? n : 0);
        }}
        style={{
          height: 24,
          padding: '0 6px',
          background: 'var(--surface)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          fontSize: 12,
        }}
      />
    </label>
  );
}

function OutputMetricPicker({
  available,
  value,
  onChange,
}: {
  available: MetricDescriptor[];
  value: string;
  onChange: (v: string) => void;
}) {
  // Verdict-derived pseudo-metrics are always available because the
  // backend's `extract_child_metric` handles them directly.
  const verdictMetrics: { id: string; name: string }[] = [
    { id: 'fail_count', name: 'Fail count' },
    { id: 'pass_count', name: 'Pass count' },
    { id: 'verdict_numeric', name: 'Verdict numeric (pass=1, fail=0)' },
  ];
  const combined = [
    ...verdictMetrics,
    ...available.map((m) => ({
      id: m.id,
      name: `${m.name}${m.unit ? ` [${m.unit}]` : ''}`,
    })),
  ];

  return (
    <select
      data-testid="sensitivity-output-metric"
      value={value}
      onChange={(e) => onChange(e.currentTarget.value)}
      style={{
        height: 28,
        padding: '0 6px',
        background: 'var(--surface-container)',
        color: 'var(--on-surface)',
        border: '1px solid var(--outline-variant)',
        borderRadius: 6,
        fontSize: 12,
      }}
    >
      <option value="">Pick a metric…</option>
      {combined.map((m) => (
        <option key={m.id} value={m.id}>
          {m.name}
        </option>
      ))}
    </select>
  );
}
