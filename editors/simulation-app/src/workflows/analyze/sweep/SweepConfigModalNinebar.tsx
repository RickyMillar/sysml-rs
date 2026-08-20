/**
 * SweepConfigModalNinebar — the "Configure sweep" modal (ninebar Phase 5).
 *
 * Plan §3 Phase 5: "Range editor … = modal (the 'Configure sweep' act)";
 * §0: config is an overlay, never a resident panel. Follows the
 * ConfigureRunModal precedent exactly: registered by id in the modal
 * registry (frame/Cmd-K reachable), every field writes straight to the
 * shared store (`useSweepStudyStore`) — no apply step, closing the modal
 * is enough. The workflow body + left rail read the same store, so the
 * factor summary and Run button are live against edits (audit F9's
 * "range iteration against visible results": re-open this modal — or the
 * rail's factor rows — while the viewers stay mounted behind it).
 *
 * Parameter discovery shares `discoverSweepParameters` + the query key
 * with the legacy body — one implementation, react-query dedupes.
 */

import { useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { registerModal } from '@/shared/overlays/modalStore';
import { FuzzyCombobox, type FuzzyCandidate } from '@/shared/pickers/FuzzyCombobox';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { metricRegistry } from '@/shared/metrics/registry';
import { useMetricRegistryVersion } from '@/shared/metrics/useMetricRegistry';
import type { MetricDescriptor } from '@/shared/metrics/types';
import { discoverSweepParameters } from './discoverSweepParameters';
import { discoverOutcomeMetrics } from './discoverOutcomeMetrics';
import { useSweepStudyStore, expandStudyChildren, MAX_HORIZON_TICKS } from './useSweepStudyStore';
import { formatModelDuration } from './modelDuration';
import { DEFAULT_RANGE_SPEC, type ParameterRangeEntry, type SweepRunMode } from './useSweepConfig';
import { parseNumberList, type SweepParameterCandidate } from './SweepConfig';
import type { RangeSpec } from './cartesianProduct';

export const SWEEP_CONFIG_MODAL_ID = 'analyze-sweep-config';

// ── Field styles (ConfigureRunModal vocabulary) ─────────────────────

const FIELD_LABEL: React.CSSProperties = {
  fontSize: 'var(--text-xs)',
  color: 'var(--text-secondary)',
  letterSpacing: '0.03em',
  textTransform: 'uppercase',
};

const FIELD_HINT: React.CSSProperties = {
  fontSize: 'var(--text-xs)',
  color: 'var(--text-muted)',
};

const INPUT_STYLE: React.CSSProperties = {
  background: 'var(--surface-sunken)',
  color: 'var(--text-primary)',
  border: '1px solid var(--border-default)',
  borderRadius: 'var(--radius-sm)',
  padding: '4px 8px',
  fontSize: 'var(--text-sm)',
};

export function SweepConfigModalNinebar() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);
  const { data: discoveredParameters = [], isLoading } = useQuery<SweepParameterCandidate[]>({
    queryKey: ['sweep-parameters', workspaceRoot, loadedUris],
    queryFn: () => discoverSweepParameters(loadedUris),
    enabled: !!workspaceRoot && loadedUris.length > 0,
  });
  // Outcomes come from the MODEL, on the same terms as the knobs above: an
  // attribute declared `out` is a result. Previously the only producer was
  // `metricRegistry`, written while a live session rendered in Plots — so the
  // study could not name what it measured until after it had run something.
  const { data: discoveredMetrics = [] } = useQuery<MetricDescriptor[]>({
    queryKey: ['sweep-outcome-metrics', workspaceRoot, loadedUris],
    queryFn: () => discoverOutcomeMetrics(loadedUris),
    enabled: !!workspaceRoot && loadedUris.length > 0,
  });

  // Re-render when a run registers new variables, so the metric list is not
  // frozen at first paint.
  const registryVersion = useMetricRegistryVersion();

  const ranges = useSweepStudyStore((s) => s.ranges);
  const addRange = useSweepStudyStore((s) => s.addRange);
  const removeRange = useSweepStudyStore((s) => s.removeRange);
  const updateRange = useSweepStudyStore((s) => s.updateRange);
  const selectedMetricIds = useSweepStudyStore((s) => s.selectedMetricIds);
  const toggleMetric = useSweepStudyStore((s) => s.toggleMetric);
  const runMode = useSweepStudyStore((s) => s.runMode);
  const setRunMode = useSweepStudyStore((s) => s.setRunMode);
  const horizonTicks = useSweepStudyStore((s) => s.horizonTicks);
  const setHorizonTicks = useSweepStudyStore((s) => s.setHorizonTicks);
  const dtMs = useSweepStudyStore((s) => s.dtMs);
  const setDtMs = useSweepStudyStore((s) => s.setDtMs);

  const [pickerQuery, setPickerQuery] = useState('');

  // An `out` attribute is a result, not a knob. It used to appear in BOTH
  // lists — `temperature` was offered as something to sweep, which would have
  // set an initial condition the dynamics overwrite on the first tick.
  const outcomeNames = useMemo(
    () => new Set(discoveredMetrics.map((m) => m.name)),
    [discoveredMetrics],
  );
  const availableParameters = useMemo(
    () => discoveredParameters.filter((p) => !outcomeNames.has(p.label ?? '')),
    [discoveredParameters, outcomeNames],
  );

  const candidates = useMemo<FuzzyCandidate[]>(
    () =>
      availableParameters
        .filter((c) => !ranges.some((r) => r.parameterId === (c.label ?? c.id)))
        .map((c) => ({ value: c.label ?? c.id, detail: c.uri ? shortUri(c.uri) : undefined })),
    [availableParameters, ranges],
  );

  const exactMatch = useMemo(
    () => availableParameters.find((c) => (c.label ?? c.id) === pickerQuery.trim()) ?? null,
    [availableParameters, pickerQuery],
  );

  const addPicked = () => {
    const name = pickerQuery.trim();
    if (!name) return;
    // parameterId is the BARE attribute name — the backend's override
    // surface is name-keyed (`apply_overrides` → `ctx.set(key, …)`), so
    // the legacy picker's `uri::name` ids silently no-op'd (live-caught,
    // 2026-07-15: sweep children all ran baseline physics). Free-form
    // names are still allowed (the combobox suggests, never restricts).
    const entry: ParameterRangeEntry = {
      parameterId: exactMatch?.label ?? name,
      label: exactMatch?.label ?? name,
      spec: { ...DEFAULT_RANGE_SPEC },
    };
    addRange(entry);
    setPickerQuery('');
  };

  const childCount = useMemo(() => {
    if (ranges.length === 0) return 0;
    return expandStudyChildren(ranges).length;
  }, [ranges]);


  // Model-declared outcomes lead; the registry SUPPLEMENTS with anything only
  // a run can reveal (derived series, injected signals). Deduped by id, with
  // the discovered descriptor winning — it carries the model's own unit.
  const metrics = useMemo<MetricDescriptor[]>(() => {
    const byId = new Map<string, MetricDescriptor>();
    for (const m of discoveredMetrics) byId.set(m.id, m);
    for (const m of metricRegistry.list()) if (!byId.has(m.id)) byId.set(m.id, m);
    return Array.from(byId.values());
  }, [discoveredMetrics, registryVersion]);

  return (
    <div data-testid="sweep-config-modal" className="flex flex-col gap-4" style={{ minWidth: 420 }}>
      {/* ── Parameters ── */}
      <div className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Add parameter</span>
        <div className="flex gap-2">
          <div style={{ flex: 1 }}>
            <FuzzyCombobox
              value={pickerQuery}
              onChange={setPickerQuery}
              candidates={candidates}
              placeholder={isLoading ? 'Discovering parameters…' : 'Search attributes…'}
              testId="sweep-modal-parameter-search"
              inputStyle={INPUT_STYLE}
            />
          </div>
          <button
            type="button"
            data-testid="sweep-modal-parameter-add"
            onClick={addPicked}
            disabled={pickerQuery.trim().length === 0}
            style={{
              ...INPUT_STYLE,
              cursor: pickerQuery.trim().length === 0 ? 'not-allowed' : 'pointer',
              color: pickerQuery.trim().length === 0 ? 'var(--text-muted)' : 'var(--accent-fg)',
            }}
          >
            Add
          </button>
        </div>
        <span style={FIELD_HINT}>
          Sweepable attributes discovered from the loaded model. Each parameter gets a range below.
        </span>
      </div>

      {/* ── Ranges ── */}
      <div className="flex flex-col gap-2">
        <span style={FIELD_LABEL}>Ranges</span>
        {ranges.length === 0 ? (
          <span data-testid="sweep-modal-ranges-empty" style={FIELD_HINT}>
            No parameters yet — add one above to define its range.
          </span>
        ) : (
          <ul className="flex flex-col gap-2" style={{ listStyle: 'none', margin: 0, padding: 0 }}>
            {ranges.map((r) => (
              <RangeRow
                key={r.parameterId}
                entry={r}
                onChange={(spec) => updateRange(r.parameterId, spec)}
                onRemove={() => removeRange(r.parameterId)}
              />
            ))}
          </ul>
        )}
      </div>

      {/* ── Outcome metrics ── */}
      <div className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Outcome metrics</span>
        {metrics.length === 0 ? (
          <span data-testid="sweep-modal-metrics-empty" style={FIELD_HINT}>
            This model declares no `out` attributes, so there is nothing to measure beyond the
            built-in fail-count and margin columns.
          </span>
        ) : (
          <div className="flex flex-wrap gap-1.5">
            {metrics.map((m) => {
              const active = selectedMetricIds.includes(m.id);
              return (
                <button
                  key={m.id}
                  type="button"
                  data-testid={`sweep-modal-metric-${m.id}`}
                  data-active={active}
                  onClick={() => toggleMetric(m.id)}
                  style={{
                    padding: '3px 10px',
                    borderRadius: 4,
                    fontSize: 'var(--text-xs)',
                    cursor: 'pointer',
                    background: active ? 'var(--accent-tint)' : 'transparent',
                    color: active ? 'var(--text-primary)' : 'var(--text-muted)',
                    border: `1px solid ${active ? 'var(--accent)' : 'var(--border-hairline)'}`,
                  }}
                >
                  {m.name}
                </button>
              );
            })}
          </div>
        )}
      </div>

      {/* ── Run mode + horizon + combination count ── */}
      <div className="flex items-end gap-3">
        <div className="flex flex-col gap-1.5">
          <span style={FIELD_LABEL}>Run mode</span>
          <div role="radiogroup" aria-label="Sweep run mode" className="flex gap-1">
            <ModePill mode="parallel" current={runMode} onPick={setRunMode} />
            <ModePill mode="sequential" current={runMode} onPick={setRunMode} />
          </div>
        </div>
        <label className="flex flex-col gap-1.5" style={{ width: 130 }}>
          <span style={FIELD_LABEL}>Horizon (ticks)</span>
          <input
            type="number"
            min={1}
            max={MAX_HORIZON_TICKS}
            data-testid="sweep-modal-horizon"
            value={horizonTicks}
            onChange={(e) => setHorizonTicks(Number(e.target.value))}
            style={INPUT_STYLE}
          />
        </label>
        <label className="flex flex-col gap-1.5" style={{ width: 110 }}>
          <span style={FIELD_LABEL}>Step (ms)</span>
          <input
            type="number"
            min={0}
            step="any"
            data-testid="sweep-modal-dt"
            value={dtMs}
            onChange={(e) => setDtMs(Number(e.target.value))}
            style={INPUT_STYLE}
          />
        </label>
        <span
          data-testid="sweep-modal-model-duration"
          className="mono-text"
          style={{ fontSize: 'var(--text-sm)', color: 'var(--text-secondary)' }}
        >
          {formatModelDuration(horizonTicks * dtMs)}
        </span>
        <span
          data-testid="sweep-modal-child-count"
          className="mono-text"
          style={{ marginLeft: 'auto', fontSize: 'var(--text-sm)', color: 'var(--text-secondary)' }}
        >
          {childCount} {childCount === 1 ? 'combination' : 'combinations'}
        </span>
      </div>
      <span style={FIELD_HINT}>
        Each combination runs for the horizon, then its verification cases are evaluated against
        the final state — verdicts feed the table, tornado, and heatmap.
      </span>
      <span style={FIELD_HINT}>
        Ticks × step is the model time each combination covers. A model whose behaviour takes
        minutes needs a coarser step, not just more ticks — outcomes are read where the run
        stops, so a run that stops early reports the model mid-transient.
      </span>
      <span style={FIELD_HINT}>
        Changes apply immediately — close the modal and run from the rail. The viewers stay
        mounted behind this modal, so re-opening it iterates ranges against visible results.
      </span>
    </div>
  );
}

// ── Range row ───────────────────────────────────────────────────────

function RangeRow({
  entry,
  onChange,
  onRemove,
}: {
  entry: ParameterRangeEntry;
  onChange: (spec: RangeSpec) => void;
  onRemove: () => void;
}) {
  const spec = entry.spec;
  return (
    <li
      data-testid={`sweep-modal-range-${entry.parameterId}`}
      className="flex flex-col gap-1.5"
      style={{
        border: '1px solid var(--border-hairline)',
        borderRadius: 'var(--radius-sm)',
        padding: '8px 10px',
      }}
    >
      <div className="flex items-center gap-2">
        <span className="mono-text truncate" style={{ fontSize: 'var(--text-sm)', color: 'var(--text-primary)' }}>
          {entry.label ?? entry.parameterId}
        </span>
        <div className="flex gap-1" style={{ marginLeft: 'auto' }}>
          <SpecKindPill
            label="grid"
            active={spec.kind === 'grid'}
            testId={`sweep-modal-range-kind-grid-${entry.parameterId}`}
            onClick={() => {
              if (spec.kind !== 'grid') onChange({ kind: 'grid', min: 0, max: 1, step: 0.25 });
            }}
          />
          <SpecKindPill
            label="values"
            active={spec.kind === 'list'}
            testId={`sweep-modal-range-kind-list-${entry.parameterId}`}
            onClick={() => {
              if (spec.kind !== 'list') onChange({ kind: 'list', values: [] });
            }}
          />
          <button
            type="button"
            aria-label={`Remove ${entry.label ?? entry.parameterId}`}
            data-testid={`sweep-modal-range-remove-${entry.parameterId}`}
            onClick={onRemove}
            className="material-symbols-outlined"
            style={{ fontSize: 15, color: 'var(--text-muted)', background: 'none', border: 'none', cursor: 'pointer', padding: 2 }}
          >
            close
          </button>
        </div>
      </div>
      {spec.kind === 'grid' ? (
        <div className="flex items-center gap-2">
          <NumField label="min" value={spec.min} testId={`sweep-modal-range-min-${entry.parameterId}`} onCommit={(v) => onChange({ ...spec, min: v })} />
          <NumField label="max" value={spec.max} testId={`sweep-modal-range-max-${entry.parameterId}`} onCommit={(v) => onChange({ ...spec, max: v })} />
          <NumField label="step" value={spec.step} testId={`sweep-modal-range-step-${entry.parameterId}`} onCommit={(v) => onChange({ ...spec, step: v })} />
        </div>
      ) : (
        <label className="flex flex-col gap-1">
          <input
            data-testid={`sweep-modal-range-values-${entry.parameterId}`}
            defaultValue={spec.values.join(', ')}
            placeholder="e.g. 0.01, 0.02, 0.05"
            onBlur={(e) => onChange({ kind: 'list', values: parseNumberList(e.target.value) })}
            style={INPUT_STYLE}
          />
          <span style={FIELD_HINT}>Comma/space-separated numbers; applied on blur.</span>
        </label>
      )}
    </li>
  );
}

function NumField({
  label,
  value,
  testId,
  onCommit,
}: {
  label: string;
  value: number;
  testId: string;
  onCommit: (v: number) => void;
}) {
  return (
    <label className="flex items-center gap-1.5" style={{ flex: 1, minWidth: 0 }}>
      <span className="mono-text" style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)' }}>{label}</span>
      <input
        type="number"
        data-testid={testId}
        defaultValue={value}
        onBlur={(e) => {
          const n = Number(e.target.value);
          if (Number.isFinite(n)) onCommit(n);
        }}
        style={{ ...INPUT_STYLE, width: '100%', minWidth: 0 }}
      />
    </label>
  );
}

function SpecKindPill({
  label,
  active,
  testId,
  onClick,
}: {
  label: string;
  active: boolean;
  testId: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      data-testid={testId}
      data-active={active}
      onClick={onClick}
      style={{
        padding: '2px 8px',
        borderRadius: 4,
        fontSize: 'var(--text-xs)',
        cursor: 'pointer',
        background: active ? 'var(--accent-tint)' : 'transparent',
        color: active ? 'var(--text-primary)' : 'var(--text-muted)',
        border: `1px solid ${active ? 'var(--accent)' : 'var(--border-hairline)'}`,
      }}
    >
      {label}
    </button>
  );
}

function ModePill({
  mode,
  current,
  onPick,
}: {
  mode: SweepRunMode;
  current: SweepRunMode;
  onPick: (m: SweepRunMode) => void;
}) {
  const active = mode === current;
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      data-testid={`sweep-modal-mode-${mode}`}
      onClick={() => onPick(mode)}
      style={{
        padding: '3px 10px',
        borderRadius: 4,
        fontSize: 'var(--text-xs)',
        cursor: 'pointer',
        background: active ? 'var(--accent-tint)' : 'transparent',
        color: active ? 'var(--text-primary)' : 'var(--text-muted)',
        border: `1px solid ${active ? 'var(--accent)' : 'var(--border-hairline)'}`,
      }}
    >
      {mode}
    </button>
  );
}

function shortUri(uri: string): string {
  const idx = uri.lastIndexOf('/');
  return idx >= 0 ? uri.slice(idx + 1) : uri;
}

registerModal({
  id: SWEEP_CONFIG_MODAL_ID,
  title: 'Configure sweep',
  component: SweepConfigModalNinebar,
});
