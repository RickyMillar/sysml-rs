/**
 * SensitivityWorkflowNinebar — the flag-on Sensitivity surface (ninebar
 * Phase 5).
 *
 * Recomposition: the Morris μ-vs-σ scatter / Sobol bar / ranked tornado
 * shell is the full-bleed hero; method + parameter-range + sampler
 * configuration lives in a "Configure sensitivity" MODAL rendered
 * directly by this body (same rationale as Trade Study: the config hook
 * state is threaded as props, not mirrored into a store). The rail
 * carries the parameter summary + Run; the strip is the shared
 * `AnalyzeBatchStrip` over the runner's streaming children.
 *
 * The legacy two-column body (`sensitivity/SensitivityWorkflow`
 * flag-off) is untouched.
 */

import { useCallback, useMemo, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { LeftRailContent, BottomStripContent } from '@/app/slots';
import { Modal } from '@/shared/overlays/Modal';
import { FuzzyCombobox, type FuzzyCandidate } from '@/shared/pickers/FuzzyCombobox';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { useWorkspaceUris } from '@/features/packages/queries';
import { findElements } from '@/shared/api/model';
import { metricRegistry } from '@/shared/metrics/registry';
import type { BatchStatus, SensitivityMethod } from '@/engine/types';

import { AnalyzeBatchStrip } from '../ninebar/AnalyzeBatchStrip';
import { AnalyzeRail, RailListRow, RailEmptyHint, HeroNotice } from '../ninebar/chrome';
import { SensitivityResultsShell } from './SensitivityResultsShell';
import type { SensitivityParameterCandidate } from './SensitivityConfig';
import {
  useSensitivityConfig,
  DEFAULT_RANGE,
  type SensitivityConfigState,
  type SensitivityRangeEntry,
} from './useSensitivityConfig';
import { useSensitivityRunner } from './useSensitivityRunner';

/** Same discovery + query key as the legacy body (shared cache). */
async function discoverSensitivityParameters(
  uris: readonly string[],
): Promise<SensitivityParameterCandidate[]> {
  if (uris.length === 0) return [];
  const seen = new Set<string>();
  const out: SensitivityParameterCandidate[] = [];
  for (const uri of uris) {
    let elements: Awaited<ReturnType<typeof findElements>>;
    try {
      elements = await findElements(uri, 'AttributeUsage');
    } catch {
      continue;
    }
    for (const el of elements) {
      const name = el.name ?? '';
      if (!name) continue;
      const id = `${uri}::${name}`;
      if (seen.has(id)) continue;
      seen.add(id);
      out.push({ id, label: name, kind: 'AttributeUsage', uri });
    }
  }
  out.sort((a, b) => (a.label ?? a.id).localeCompare(b.label ?? b.id));
  return out;
}

export function SensitivityWorkflowNinebar() {
  const workspaceRoot = useWorkspaceUIStore((s) => s.workspaceRoot);
  const { data: wsData } = useWorkspaceUris(workspaceRoot);
  const loadedUris = useMemo(() => wsData?.uris ?? [], [wsData?.uris]);
  const { data: availableParameters = [], isLoading } = useQuery<SensitivityParameterCandidate[]>({
    queryKey: ['sensitivity-parameters', workspaceRoot, loadedUris],
    queryFn: () => discoverSensitivityParameters(loadedUris),
    enabled: !!workspaceRoot && loadedUris.length > 0,
  });

  const config = useSensitivityConfig();
  const runner = useSensitivityRunner();
  const [configOpen, setConfigOpen] = useState(false);
  const [localError, setLocalError] = useState<string | null>(null);

  const isRunning =
    runner.state === 'creating' || runner.state === 'running' || runner.state === 'analyzing';

  const handleRun = useCallback(async () => {
    setLocalError(null);
    const uri = loadedUris[0];
    if (!uri) {
      setLocalError('Load a workspace before running sensitivity analysis.');
      return;
    }
    if (!config.isValid) {
      setLocalError('Fix the configuration before running.');
      return;
    }
    await runner.start({
      uri,
      method: config.method,
      params: config.paramRanges,
      r: config.morrisR,
      p: config.morrisP,
      n: config.sobolN,
      seed: config.seed,
      outputMetric: config.outputMetric,
      label: `${config.method === 'morris' ? 'Morris' : 'Sobol'} analysis`,
    });
  }, [loadedUris, config, runner]);

  const stripStatus = useMemo<BatchStatus>(() => {
    switch (runner.state) {
      case 'running':
      case 'analyzing': {
        const completed = runner.children.filter((c) => c.status === 'complete').length;
        return { kind: 'running', running: Math.max(0, runner.children.length - completed), completed };
      }
      case 'complete':
        return { kind: 'complete' };
      case 'error':
        return { kind: 'failed', reason: runner.error ?? 'Sensitivity analysis failed' };
      default:
        return { kind: 'pending' };
    }
  }, [runner.state, runner.children, runner.error]);

  const failing = runner.children.filter((c) => (c.verdicts ?? []).some((v) => v.verdict === 'fail')).length;

  return (
    <div
      data-testid="sensitivity-workflow-ninebar"
      className="flex flex-col h-full w-full min-h-0"
      style={{ background: 'var(--surface-canvas)', color: 'var(--text-primary)' }}
    >
      <LeftRailContent>
        <AnalyzeRail
          icon="analytics"
          title="Sensitivity"
          headerCount={`${config.ranges.length} ${config.ranges.length === 1 ? 'parameter' : 'parameters'}`}
          sectionTitle="Parameters"
          onConfigure={() => setConfigOpen(true)}
          summary={[
            { label: 'method', value: config.method === 'morris' ? 'Morris' : 'Sobol' },
            { label: 'runs', value: String(config.childCount) },
            { label: 'evaluated', value: String(runner.children.filter((c) => c.status === 'complete').length) },
            { label: 'failing', value: String(failing), tone: failing > 0 ? 'fail' : undefined },
          ]}
          plannedChildren={config.childCount}
          plannedNoun="runs"
          runLabel="Run Analysis"
          canRun={!!workspaceRoot && config.isValid}
          isRunning={isRunning}
          onRun={() => void handleRun()}
          testIdPrefix="sensitivity"
        >
          {config.ranges.length === 0 ? (
            <RailEmptyHint>
              No parameters yet — Configure picks parameters, their ranges, and the sampler.
            </RailEmptyHint>
          ) : (
            <ul data-testid="sensitivity-rail-parameters" style={{ listStyle: 'none', margin: 0, padding: '4px 0' }}>
              {config.ranges.map((r) => (
                <li key={r.parameterId}>
                  <RailListRow
                    testId={`sensitivity-rail-parameter-${r.parameterId}`}
                    name={r.label ?? r.parameterId}
                    detail={`${r.min} – ${r.max}`}
                    onClick={() => setConfigOpen(true)}
                  />
                </li>
              ))}
            </ul>
          )}
        </AnalyzeRail>
      </LeftRailContent>

      <div className="flex-1 min-h-0 overflow-auto" data-testid="sensitivity-hero">
        {runner.state === 'idle' && !localError ? (
          <HeroNotice
            testId="sensitivity-hero-empty"
            icon="analytics"
            title="Define a sensitivity analysis"
            detail="Pick parameters and ranges, choose Morris screening or Sobol indices in the Configure modal, then run — the μ-σ scatter, index bars, and ranked tornado render here."
            action={{ label: 'Configure sensitivity', testId: 'sensitivity-hero-configure', onClick: () => setConfigOpen(true) }}
          />
        ) : (
          <SensitivityResultsShell
            batchId={runner.batchId}
            children={runner.children}
            state={runner.state}
            results={runner.results}
            error={localError ?? runner.error}
          />
        )}
      </div>

      <BottomStripContent>
        <AnalyzeBatchStrip methodLabel="Sensitivity" status={stripStatus} children={runner.children} />
      </BottomStripContent>

      {/* Config modal — direct render (hook state threaded as props). */}
      <Modal open={configOpen} onClose={() => setConfigOpen(false)} title="Configure sensitivity">
        <SensitivityConfigModalBody
          config={config}
          availableParameters={availableParameters}
          isLoadingParameters={isLoading}
        />
      </Modal>
    </div>
  );
}

// ── Modal body ──────────────────────────────────────────────────────

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

function SensitivityConfigModalBody({
  config,
  availableParameters,
  isLoadingParameters,
}: {
  config: SensitivityConfigState;
  availableParameters: SensitivityParameterCandidate[];
  isLoadingParameters: boolean;
}) {
  const [pickerQuery, setPickerQuery] = useState('');

  const candidates = useMemo<FuzzyCandidate[]>(
    () =>
      availableParameters
        .filter((c) => !config.ranges.some((r) => r.parameterId === c.id))
        .map((c) => ({ value: c.label ?? c.id, detail: c.uri ? c.uri.split('/').pop() : undefined })),
    [availableParameters, config.ranges],
  );

  const addPicked = () => {
    const name = pickerQuery.trim();
    if (!name) return;
    // BARE name — the backend override surface is name-keyed; `uri::name`
    // ids silently no-op (see SweepConfigModalNinebar, live-caught).
    config.addRange({
      parameterId: name,
      label: name,
      ...DEFAULT_RANGE,
    });
    setPickerQuery('');
  };

  const metricCandidates = useMemo<FuzzyCandidate[]>(() => {
    const fromRegistry = metricRegistry.list().map((m) => ({ value: m.name, detail: m.source }));
    const fromModel = availableParameters.map((c) => ({ value: c.label ?? c.id, detail: 'attribute' }));
    const seen = new Set<string>();
    return [...fromRegistry, ...fromModel].filter((c) => {
      if (seen.has(c.value)) return false;
      seen.add(c.value);
      return true;
    });
  }, [availableParameters]);

  return (
    <div data-testid="sensitivity-config-modal" className="flex flex-col gap-4" style={{ minWidth: 420 }}>
      {/* ── Method ── */}
      <div className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Method</span>
        <div role="radiogroup" aria-label="Sensitivity method" className="flex gap-1">
          <MethodPill method="morris" label="Morris screening" current={config.method} onPick={config.setMethod} />
          <MethodPill method="sobol" label="Sobol indices" current={config.method} onPick={config.setMethod} />
        </div>
      </div>

      {/* ── Parameters ── */}
      <div className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Add parameter</span>
        <div className="flex gap-2">
          <div style={{ flex: 1 }}>
            <FuzzyCombobox
              value={pickerQuery}
              onChange={setPickerQuery}
              candidates={candidates}
              placeholder={isLoadingParameters ? 'Discovering parameters…' : 'Search attributes…'}
              testId="sensitivity-modal-parameter-search"
              inputStyle={INPUT_STYLE}
            />
          </div>
          <button
            type="button"
            data-testid="sensitivity-modal-parameter-add"
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
      </div>

      {/* ── Ranges (continuum min/max — no grid step) ── */}
      <div className="flex flex-col gap-2">
        <span style={FIELD_LABEL}>Ranges</span>
        {config.ranges.length === 0 ? (
          <span data-testid="sensitivity-modal-ranges-empty" style={FIELD_HINT}>
            No parameters yet — sensitivity samples each range as a continuum.
          </span>
        ) : (
          <ul className="flex flex-col gap-2" style={{ listStyle: 'none', margin: 0, padding: 0 }}>
            {config.ranges.map((r) => (
              <SensitivityRangeRow key={r.parameterId} entry={r} config={config} />
            ))}
          </ul>
        )}
      </div>

      {/* ── Sampler knobs ── */}
      <div className="flex items-end gap-3">
        {config.method === 'morris' ? (
          <>
            <NumInput label="trajectories (r)" testId="sensitivity-modal-morris-r" value={config.morrisR} onCommit={config.setMorrisR} />
            <NumInput label="levels (p)" testId="sensitivity-modal-morris-p" value={config.morrisP} onCommit={config.setMorrisP} />
          </>
        ) : (
          <NumInput label="base samples (n)" testId="sensitivity-modal-sobol-n" value={config.sobolN} onCommit={config.setSobolN} />
        )}
        <NumInput label="seed" testId="sensitivity-modal-seed" value={config.seed} onCommit={config.setSeed} />
        <span className="mono-text" data-testid="sensitivity-modal-run-count" style={{ marginLeft: 'auto', fontSize: 'var(--text-sm)', color: 'var(--text-secondary)' }}>
          {config.childCount} runs
        </span>
      </div>

      {/* ── Output metric ── */}
      <div className="flex flex-col gap-1.5">
        <span style={FIELD_LABEL}>Output metric</span>
        <FuzzyCombobox
          value={config.outputMetric}
          onChange={config.setOutputMetric}
          candidates={metricCandidates}
          placeholder="Metric to score each run on…"
          testId="sensitivity-modal-metric"
          inputStyle={INPUT_STYLE}
        />
        <span style={FIELD_HINT}>
          Each completed run is scored on this metric; sensitivity indices rank the parameters by
          their influence on it.
        </span>
      </div>

      <span style={FIELD_HINT}>Changes apply immediately — close the modal and run from the rail.</span>
    </div>
  );
}

function SensitivityRangeRow({
  entry,
  config,
}: {
  entry: SensitivityRangeEntry;
  config: SensitivityConfigState;
}) {
  return (
    <li
      data-testid={`sensitivity-modal-range-${entry.parameterId}`}
      className="flex items-center gap-2"
      style={{
        border: '1px solid var(--border-hairline)',
        borderRadius: 'var(--radius-sm)',
        padding: '6px 10px',
      }}
    >
      <span className="mono-text truncate" style={{ fontSize: 'var(--text-sm)', flex: 1, minWidth: 0 }}>
        {entry.label ?? entry.parameterId}
      </span>
      <NumInput
        label="min"
        testId={`sensitivity-modal-range-min-${entry.parameterId}`}
        value={entry.min}
        onCommit={(v) => config.updateRange(entry.parameterId, { min: v })}
        narrow
      />
      <NumInput
        label="max"
        testId={`sensitivity-modal-range-max-${entry.parameterId}`}
        value={entry.max}
        onCommit={(v) => config.updateRange(entry.parameterId, { max: v })}
        narrow
      />
      <button
        type="button"
        aria-label={`Remove ${entry.label ?? entry.parameterId}`}
        data-testid={`sensitivity-modal-range-remove-${entry.parameterId}`}
        onClick={() => config.removeRange(entry.parameterId)}
        className="material-symbols-outlined"
        style={{ fontSize: 15, color: 'var(--text-muted)', background: 'none', border: 'none', cursor: 'pointer', padding: 2 }}
      >
        close
      </button>
    </li>
  );
}

function NumInput({
  label,
  testId,
  value,
  onCommit,
  narrow = false,
}: {
  label: string;
  testId: string;
  value: number;
  onCommit: (v: number) => void;
  narrow?: boolean;
}) {
  return (
    <label className="flex items-center gap-1.5" style={{ width: narrow ? 110 : 140 }}>
      <span className="mono-text" style={{ fontSize: 'var(--text-xs)', color: 'var(--text-muted)', whiteSpace: 'nowrap' }}>{label}</span>
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

function MethodPill({
  method,
  label,
  current,
  onPick,
}: {
  method: SensitivityMethod;
  label: string;
  current: SensitivityMethod;
  onPick: (m: SensitivityMethod) => void;
}) {
  const active = method === current;
  return (
    <button
      type="button"
      role="radio"
      aria-checked={active}
      data-testid={`sensitivity-modal-method-${method}`}
      onClick={() => onPick(method)}
      style={{
        flex: 1,
        padding: '4px 10px',
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
