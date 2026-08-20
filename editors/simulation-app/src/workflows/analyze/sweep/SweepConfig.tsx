/**
 * SweepConfig — left-side config panel for /analyze/sweep (R5.1).
 *
 * Three stacked sections:
 *
 *   1. Parameter picker — search + add a swept parameter by name. Each
 *      picked entry renders its own range editor (min / max / step or
 *      explicit value list).
 *   2. Outcome metric dropdown — multi-select from MetricRegistry.
 *      Drives what the R5.2 streaming viewer tracks per child.
 *   3. Run mode toggle — sequential vs parallel.
 *
 * Plus a Run button that stays disabled until at least one range
 * expands to a non-empty value list. The button is a pure event emitter
 * — the parent workflow (SweepWorkflow) owns what "run" actually does.
 *
 * This component is UI-only — all selection / run mode state is owned
 * by `useSweepConfig`, passed in via the `config` prop. Parameter
 * candidates are also passed in so this panel stays dumb (matches the
 * VerifyConfig pattern).
 */

import { useMemo, useState } from 'react';
import {
  DEFAULT_RANGE_SPEC,
  type SweepConfigState,
  type SweepRunMode,
} from './useSweepConfig';
import type { RangeSpec } from './cartesianProduct';
import type { MetricDescriptor } from '@/shared/metrics/types';

/**
 * A candidate parameter the user can add to the sweep. Supplied by the
 * parent (SweepWorkflow pulls from workspace introspection in later
 * rounds — R5.1 accepts an empty list gracefully).
 */
export interface SweepParameterCandidate {
  /** Stable element id / qualified name. */
  id: string;
  /** Display label (falls back to `id`). */
  label?: string;
  /** Optional element kind — shown as a subtle annotation. */
  kind?: string;
  /** Source URI of the parameter (for later drill-down). */
  uri?: string;
}

export interface SweepConfigProps {
  /** Parameters the user may add to the sweep. */
  availableParameters: SweepParameterCandidate[];
  /** The config state (from `useSweepConfig`). */
  config: SweepConfigState;
  /** Metrics available in the registry (usually `metricRegistry.list()`). */
  availableMetrics: MetricDescriptor[];
  /** True while a sweep is in flight — disables Run. */
  isRunning?: boolean;
  /** Whether a workspace is actually loaded — gates the panel. */
  hasWorkspace?: boolean;
  /** Loading flag for the parameter list (React Query flag). */
  isLoadingParameters?: boolean;
  /** Click handler for the Run Sweep button. */
  onRun: () => void;
}

export function SweepConfig({
  availableParameters,
  config,
  availableMetrics,
  isRunning = false,
  hasWorkspace = true,
  isLoadingParameters = false,
  onRun,
}: SweepConfigProps) {
  const {
    ranges,
    selectedMetricIds,
    runMode,
    addRange,
    removeRange,
    updateRange,
    toggleMetric,
    setRunMode,
    hasRuns,
    childCount,
  } = config;

  return (
    <aside
      data-testid="sweep-config"
      className="flex flex-col shrink-0 h-full overflow-hidden"
      style={{
        width: 360,
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
          tune
        </span>
        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--on-surface)' }}>
          Parameter Sweep
        </span>
        <span
          className="mono-text"
          style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          data-testid="sweep-config-summary"
        >
          {childCount} runs
        </span>
      </div>

      {/* Parameter picker + range editors */}
      <section
        data-testid="sweep-config-parameters"
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
          selectedIds={new Set(ranges.map((r) => r.parameterId))}
          onPick={(candidate) =>
            addRange({
              parameterId: candidate.id,
              label: candidate.label,
              spec: { ...DEFAULT_RANGE_SPEC },
            })
          }
          hasWorkspace={hasWorkspace}
          isLoading={isLoadingParameters}
        />

        {ranges.length === 0 ? (
          <p
            data-testid="sweep-ranges-empty"
            style={{
              margin: 0,
              fontSize: 11,
              color: 'var(--outline)',
              fontStyle: 'italic',
            }}
          >
            Pick a parameter above to define a range.
          </p>
        ) : (
          <ul
            data-testid="sweep-range-list"
            style={{ listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: 8 }}
          >
            {ranges.map((entry) => (
              <RangeEditor
                key={entry.parameterId}
                parameterId={entry.parameterId}
                label={entry.label ?? entry.parameterId}
                spec={entry.spec}
                onSpecChange={(spec) => updateRange(entry.parameterId, spec)}
                onRemove={() => removeRange(entry.parameterId)}
              />
            ))}
          </ul>
        )}
      </section>

      {/* Outcome metric selector */}
      <section
        data-testid="sweep-config-metrics"
        className="flex flex-col gap-2 px-3 py-3"
        style={{ borderBottom: '1px solid var(--outline-variant)' }}
      >
        <div className="flex items-baseline">
          <span
            style={{
              fontSize: 11,
              fontWeight: 600,
              color: 'var(--outline)',
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
            }}
          >
            Outcome metrics
          </span>
          <span
            className="mono-text"
            style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          >
            {selectedMetricIds.length} selected
          </span>
        </div>
        <MetricDropdown
          available={availableMetrics}
          selectedIds={new Set(selectedMetricIds)}
          onToggle={toggleMetric}
        />
        {selectedMetricIds.length === 0 && (
          <p
            data-testid="sweep-metrics-empty"
            style={{
              margin: 0,
              fontSize: 11,
              color: 'var(--outline)',
              fontStyle: 'italic',
            }}
          >
            No metrics selected — the viewer will list every registered
            variable by default.
          </p>
        )}
      </section>

      {/* Run mode toggle */}
      <section
        data-testid="sweep-config-run-mode"
        className="flex flex-col gap-1 px-3 py-3"
        style={{ borderBottom: '1px solid var(--outline-variant)' }}
      >
        <span
          style={{
            fontSize: 11,
            fontWeight: 600,
            color: 'var(--outline)',
            letterSpacing: '0.04em',
            textTransform: 'uppercase',
          }}
        >
          Run mode
        </span>
        <div role="radiogroup" aria-label="Run mode" className="flex gap-1">
          <ModePill
            label="Parallel"
            active={runMode === 'parallel'}
            onClick={() => setRunMode('parallel')}
            testId="sweep-run-mode-parallel"
          />
          <ModePill
            label="Sequential"
            active={runMode === 'sequential'}
            onClick={() => setRunMode('sequential')}
            testId="sweep-run-mode-sequential"
          />
        </div>
      </section>

      {/* Run button + running summary */}
      <section
        className="flex flex-col gap-2 px-3 py-3 shrink-0"
        style={{
          borderTop: '1px solid var(--outline-variant)',
          background: 'var(--surface-container)',
          marginTop: 'auto',
        }}
      >
        <div
          data-testid="sweep-running-summary"
          style={{ fontSize: 11, color: 'var(--outline)', lineHeight: 1.4 }}
        >
          {childCount} {childCount === 1 ? 'child' : 'children'} ·{' '}
          <span style={{ color: 'var(--on-surface-variant)' }}>
            {runMode === 'parallel' ? 'parallel' : 'sequential'}
          </span>
          {selectedMetricIds.length > 0 && (
            <>
              {' '}· tracking{' '}
              <span style={{ color: 'var(--on-surface-variant)' }}>
                {selectedMetricIds.length}{' '}
                {selectedMetricIds.length === 1 ? 'metric' : 'metrics'}
              </span>
            </>
          )}
        </div>
        <button
          type="button"
          data-testid="sweep-run"
          disabled={!hasRuns || isRunning || !hasWorkspace}
          onClick={onRun}
          style={{
            height: 32,
            background:
              hasRuns && !isRunning && hasWorkspace
                ? 'var(--primary)'
                : 'var(--surface-container-high)',
            color:
              hasRuns && !isRunning && hasWorkspace
                ? 'var(--on-primary)'
                : 'var(--outline)',
            border: 'none',
            borderRadius: 6,
            fontSize: 12,
            fontWeight: 600,
            cursor:
              hasRuns && !isRunning && hasWorkspace ? 'pointer' : 'not-allowed',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 6,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
            play_arrow
          </span>
          {isRunning ? 'Running…' : 'Run Sweep'}
        </button>
      </section>
    </aside>
  );
}

// ── Sub-components ──────────────────────────────────────────────────

function ParameterPicker({
  available,
  selectedIds,
  onPick,
  hasWorkspace,
  isLoading,
}: {
  available: SweepParameterCandidate[];
  selectedIds: Set<string>;
  onPick: (candidate: SweepParameterCandidate) => void;
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

  // Allow direct entry when there are no candidates or when the query
  // doesn't match one — pressing Enter adds the raw query string as a
  // parameter id (the backend resolves it at batch-create time).
  const handleFreeEntrySubmit = () => {
    const q = query.trim();
    if (!q) return;
    if (selectedIds.has(q)) return;
    onPick({ id: q, label: q });
    setQuery('');
  };

  return (
    <div className="flex flex-col gap-1">
      <input
        type="text"
        data-testid="sweep-parameter-search"
        placeholder={hasWorkspace ? 'Search or type a parameter name…' : 'Load a workspace first'}
        value={query}
        disabled={!hasWorkspace}
        onChange={(e) => setQuery(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault();
            handleFreeEntrySubmit();
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
          data-testid="sweep-parameter-suggestions"
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
                data-testid={`sweep-parameter-option-${c.id}`}
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
                  display: 'flex',
                  alignItems: 'center',
                  gap: 6,
                }}
              >
                <span className="truncate" style={{ flex: 1 }}>
                  {c.label ?? c.id}
                </span>
                {c.kind && (
                  <span
                    className="mono-text"
                    style={{ fontSize: 10, color: 'var(--outline)' }}
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
          data-testid="sweep-parameters-loading"
          style={{ fontSize: 11, color: 'var(--outline)' }}
        >
          Discovering parameters…
        </span>
      )}
    </div>
  );
}

function RangeEditor({
  parameterId,
  label,
  spec,
  onSpecChange,
  onRemove,
}: {
  parameterId: string;
  label: string;
  spec: RangeSpec;
  onSpecChange: (spec: RangeSpec) => void;
  onRemove: () => void;
}) {
  return (
    <li
      data-testid={`sweep-range-row-${parameterId}`}
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
          data-testid={`sweep-range-remove-${parameterId}`}
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
        <KindPill
          label="Grid"
          active={spec.kind === 'grid'}
          onClick={() =>
            onSpecChange(
              spec.kind === 'grid'
                ? spec
                : { kind: 'grid', min: 0, max: 1, step: 0.25 },
            )
          }
          testId={`sweep-range-kind-grid-${parameterId}`}
        />
        <KindPill
          label="List"
          active={spec.kind === 'list'}
          onClick={() =>
            onSpecChange(
              spec.kind === 'list' ? spec : { kind: 'list', values: [0, 0.5, 1] },
            )
          }
          testId={`sweep-range-kind-list-${parameterId}`}
        />
      </div>

      {spec.kind === 'grid' ? (
        <div className="flex gap-1">
          <NumField
            label="min"
            value={spec.min}
            onChange={(min) => onSpecChange({ ...spec, min })}
            testId={`sweep-range-min-${parameterId}`}
          />
          <NumField
            label="max"
            value={spec.max}
            onChange={(max) => onSpecChange({ ...spec, max })}
            testId={`sweep-range-max-${parameterId}`}
          />
          <NumField
            label="step"
            value={spec.step}
            onChange={(step) => onSpecChange({ ...spec, step })}
            testId={`sweep-range-step-${parameterId}`}
          />
        </div>
      ) : (
        <input
          type="text"
          data-testid={`sweep-range-values-${parameterId}`}
          value={spec.values.join(', ')}
          onChange={(e) => {
            const parsed = parseNumberList(e.target.value);
            onSpecChange({ kind: 'list', values: parsed });
          }}
          style={{
            height: 26,
            padding: '0 8px',
            background: 'var(--surface)',
            color: 'var(--on-surface)',
            border: '1px solid var(--outline-variant)',
            borderRadius: 4,
            fontSize: 12,
          }}
          placeholder="comma-separated values, e.g. 1, 2, 3"
        />
      )}
    </li>
  );
}

function KindPill({
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
        flex: 1,
        height: 22,
        background: active ? 'var(--primary-container)' : 'transparent',
        color: active ? 'var(--on-primary-container)' : 'var(--outline)',
        border: `1px solid ${active ? 'var(--primary)' : 'var(--outline-variant)'}`,
        borderRadius: 4,
        fontSize: 11,
        fontWeight: active ? 600 : 500,
        cursor: 'pointer',
      }}
    >
      {label}
    </button>
  );
}

function NumField({
  label,
  value,
  onChange,
  testId,
}: {
  label: string;
  value: number;
  onChange: (n: number) => void;
  testId: string;
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

function MetricDropdown({
  available,
  selectedIds,
  onToggle,
}: {
  available: MetricDescriptor[];
  selectedIds: Set<string>;
  onToggle: (id: string) => void;
}) {
  if (available.length === 0) {
    return (
      <p
        data-testid="sweep-metrics-none-registered"
        style={{
          margin: 0,
          fontSize: 11,
          color: 'var(--outline)',
          fontStyle: 'italic',
        }}
      >
        No metrics registered yet — run a child session to populate the
        registry, or wait for constraint / calc sources (R5.3).
      </p>
    );
  }
  return (
    <select
      data-testid="sweep-metric-select"
      multiple
      size={Math.min(6, Math.max(3, available.length))}
      value={Array.from(selectedIds)}
      onChange={(e) => {
        // Because this is a <select multiple>, the browser gives us the
        // full selected list per change — we reconcile by diffing.
        const nextIds = new Set<string>(
          Array.from(e.currentTarget.selectedOptions).map((o) => o.value),
        );
        // Toggle only the symmetric difference so `onToggle` callers
        // stay one-id-at-a-time (matches the hook contract).
        for (const id of nextIds) {
          if (!selectedIds.has(id)) onToggle(id);
        }
        for (const id of selectedIds) {
          if (!nextIds.has(id)) onToggle(id);
        }
      }}
      style={{
        padding: 4,
        background: 'var(--surface-container)',
        color: 'var(--on-surface)',
        border: '1px solid var(--outline-variant)',
        borderRadius: 6,
        fontSize: 12,
      }}
    >
      {available.map((m) => (
        <option key={m.id} value={m.id}>
          {m.name}
          {m.unit ? ` [${m.unit}]` : ''}
        </option>
      ))}
    </select>
  );
}

function ModePill({
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
        flex: 1,
        height: 26,
        background: active ? 'var(--primary-container)' : 'transparent',
        color: active ? 'var(--on-primary-container)' : 'var(--outline)',
        border: `1px solid ${active ? 'var(--primary)' : 'var(--outline-variant)'}`,
        borderRadius: 6,
        fontSize: 11,
        fontWeight: active ? 600 : 500,
        cursor: 'pointer',
      }}
    >
      {label}
    </button>
  );
}

// ── Pure helpers ────────────────────────────────────────────────────

/**
 * Parse a comma- or whitespace-separated list of numbers. Drops
 * non-numeric tokens silently so the user can edit in place without
 * the input jumping to zero mid-type.
 */
function parseNumberList(raw: string): number[] {
  return raw
    .split(/[\s,]+/)
    .map((tok) => tok.trim())
    .filter((tok) => tok.length > 0)
    .map((tok) => Number(tok))
    .filter((n) => Number.isFinite(n));
}

export { parseNumberList };
// Re-export for ergonomic consumption from the workflow.
export type { SweepRunMode };
