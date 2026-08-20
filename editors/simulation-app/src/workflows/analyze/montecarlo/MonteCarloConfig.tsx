/**
 * MonteCarloConfig — left-side config panel for /analyze/montecarlo.
 *
 * Three sections, top-to-bottom:
 *
 *   1. Parameter multi-select — pulls model-level parameter names from
 *      the workspace tree (AttributeUsage / attribute-like elements)
 *      and lets the user toggle each into the sampling set.
 *   2. Per-parameter DistributionEditor — one row per selected
 *      parameter, with the kind picker + kind-specific fields +
 *      inline validation.
 *   3. Sample count + seed + Run button.
 *
 * Dumb component: all state lives in `config` (`useMonteCarloConfig`).
 * The parent workflow (`MonteCarloWorkflow`) provides `availableParameters`
 * and the `onRun` handler so this file is easy to render in isolation.
 */

import { useMemo, useState } from 'react';
import {
  DistributionEditor,
} from './DistributionEditor';
import {
  MAX_SAMPLE_COUNT,
  type MonteCarloConfigState,
} from './useMonteCarloConfig';

/**
 * A candidate parameter name surfaced from the model. We only need the
 * display label and a stable key for checkbox state — the runner
 * ships the plain name to the backend as the override key.
 */
export interface McParameterCandidate {
  /** Stable key (same as `name` today; kept distinct in case we key by qname later). */
  id: string;
  /** The name that's passed as the override key (matches backend overrides). */
  name: string;
  /** Optional qualifier text ("MyPart :: voltage") used purely for display. */
  qualifier?: string;
}

export interface MonteCarloConfigProps {
  /** Candidate parameters the user may sample — driven by the workspace. */
  availableParameters: readonly McParameterCandidate[];
  /** Config state from `useMonteCarloConfig`. */
  config: MonteCarloConfigState;
  /** True while a batch run is in flight — disables Run. */
  isRunning?: boolean;
  /** Whether a workspace is actually loaded — gates the panel. */
  hasWorkspace?: boolean;
  /** Loading flag for the parameter list. */
  isLoadingParameters?: boolean;
  /** Click handler for the Run Monte Carlo button. */
  onRun: () => void;
}

export function MonteCarloConfig({
  availableParameters,
  config,
  isRunning = false,
  hasWorkspace = true,
  isLoadingParameters = false,
  onRun,
}: MonteCarloConfigProps) {
  const {
    distributions,
    sampleCount,
    seed,
    parameterNames,
    parameterCount,
    isValid,
    validityByName,
    addParameter,
    removeParameter,
    setDistributionKind,
    setDistribution,
    setSampleCount,
    setSeed,
    hasParameter,
  } = config;

  const [paramFilter, setParamFilter] = useState('');

  const visibleCandidates = useMemo(() => {
    const q = paramFilter.trim().toLowerCase();
    if (!q) return availableParameters;
    return availableParameters.filter(
      (p) =>
        p.name.toLowerCase().includes(q) ||
        (p.qualifier ?? '').toLowerCase().includes(q),
    );
  }, [availableParameters, paramFilter]);

  const runDisabled = !isValid || isRunning;

  return (
    <aside
      data-testid="montecarlo-config"
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
          casino
        </span>
        <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--on-surface)' }}>
          Monte Carlo
        </span>
        <span
          className="mono-text"
          style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          data-testid="montecarlo-config-summary"
        >
          {parameterCount} param{parameterCount === 1 ? '' : 's'} · {sampleCount} samples
        </span>
      </div>

      {/* Parameter picker */}
      <section
        data-testid="montecarlo-config-params"
        className="flex flex-col shrink-0"
        style={{ borderBottom: '1px solid var(--outline-variant)' }}
      >
        <div className="flex items-center gap-2 px-3 py-2">
          <span
            style={{
              fontSize: 11,
              fontWeight: 600,
              color: 'var(--outline)',
              letterSpacing: '0.04em',
              textTransform: 'uppercase',
            }}
          >
            Parameters
          </span>
          <span
            className="mono-text"
            style={{ fontSize: 10, color: 'var(--outline)', marginLeft: 'auto' }}
          >
            {parameterCount}/{availableParameters.length}
          </span>
        </div>
        <div className="px-3 pb-2">
          <input
            type="text"
            data-testid="montecarlo-param-filter"
            placeholder="Filter parameters…"
            value={paramFilter}
            onChange={(e) => setParamFilter(e.target.value)}
            style={{
              width: '100%',
              height: 26,
              padding: '0 8px',
              background: 'var(--surface-container)',
              color: 'var(--on-surface)',
              border: '1px solid var(--outline-variant)',
              borderRadius: 4,
              fontSize: 11,
            }}
          />
        </div>

        <div
          data-testid="montecarlo-param-list"
          style={{
            maxHeight: 180,
            overflowY: 'auto',
            borderTop: '1px solid var(--outline-variant)',
          }}
        >
          {!hasWorkspace ? (
            <EmptyRow
              icon="folder_open"
              title="No workspace loaded"
              hint="Load a workspace to list parameters."
              testId="montecarlo-params-no-workspace"
            />
          ) : isLoadingParameters ? (
            <EmptyRow
              icon="progress_activity"
              title="Scanning parameters…"
              hint="Reading attribute usages from the model."
              spinning
              testId="montecarlo-params-loading"
            />
          ) : availableParameters.length === 0 ? (
            <EmptyRow
              icon="search_off"
              title="No parameters found"
              hint="Add an AttributeUsage (e.g. `attribute voltage = 12;`) to the model."
              testId="montecarlo-params-empty"
            />
          ) : (
            <ul style={{ listStyle: 'none', margin: 0, padding: '4px 0' }}>
              {visibleCandidates.map((p) => (
                <ParamRow
                  key={p.id}
                  candidate={p}
                  checked={hasParameter(p.name)}
                  onToggle={() =>
                    hasParameter(p.name) ? removeParameter(p.name) : addParameter(p.name)
                  }
                />
              ))}
            </ul>
          )}
        </div>
      </section>

      {/* Distribution editors (one per selected parameter) */}
      <section
        data-testid="montecarlo-config-distributions"
        className="flex flex-col flex-1 overflow-y-auto"
      >
        <div
          className="flex items-center gap-2 px-3 py-2 shrink-0"
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
            Distributions
          </span>
        </div>
        {parameterCount === 0 ? (
          <EmptyRow
            icon="tune"
            title="Pick parameters above"
            hint="Each selected parameter gets its own distribution editor."
            testId="montecarlo-distributions-empty"
          />
        ) : (
          parameterNames.map((name) => (
            <DistributionEditor
              key={name}
              paramName={name}
              distribution={distributions[name]}
              isValid={!!validityByName[name]}
              onKindChange={(k) => setDistributionKind(name, k)}
              onChange={(d) => setDistribution(name, d)}
              onRemove={() => removeParameter(name)}
            />
          ))
        )}
      </section>

      {/* Sample count + seed + Run */}
      <section
        className="flex flex-col gap-2 px-3 py-3 shrink-0"
        style={{
          borderTop: '1px solid var(--outline-variant)',
          background: 'var(--surface-container)',
        }}
      >
        <div className="grid grid-cols-2 gap-2">
          <label className="flex flex-col gap-1">
            <span
              style={{
                fontSize: 10,
                color: 'var(--outline)',
                textTransform: 'uppercase',
                letterSpacing: '0.04em',
              }}
            >
              Samples (max {MAX_SAMPLE_COUNT})
            </span>
            <input
              type="number"
              data-testid="montecarlo-sample-count"
              value={sampleCount}
              min={1}
              max={MAX_SAMPLE_COUNT}
              step={1}
              onChange={(e) => setSampleCount(Number(e.target.value))}
              style={{
                height: 26,
                padding: '0 6px',
                background: 'var(--surface-container-low)',
                color: 'var(--on-surface)',
                border: '1px solid var(--outline-variant)',
                borderRadius: 4,
                fontSize: 11,
              }}
            />
          </label>
          <label className="flex flex-col gap-1">
            <span
              style={{
                fontSize: 10,
                color: 'var(--outline)',
                textTransform: 'uppercase',
                letterSpacing: '0.04em',
              }}
            >
              Seed (blank = auto)
            </span>
            <input
              type="number"
              data-testid="montecarlo-seed"
              value={seed ?? ''}
              step={1}
              onChange={(e) => {
                const raw = e.target.value;
                if (raw.trim() === '') {
                  setSeed(null);
                  return;
                }
                const n = Number(raw);
                setSeed(Number.isFinite(n) ? Math.floor(n) : null);
              }}
              style={{
                height: 26,
                padding: '0 6px',
                background: 'var(--surface-container-low)',
                color: 'var(--on-surface)',
                border: '1px solid var(--outline-variant)',
                borderRadius: 4,
                fontSize: 11,
              }}
            />
          </label>
        </div>

        <button
          type="button"
          data-testid="montecarlo-run"
          disabled={runDisabled}
          onClick={onRun}
          style={{
            height: 32,
            background: runDisabled ? 'var(--surface-container-high)' : 'var(--primary)',
            color: runDisabled ? 'var(--outline)' : 'var(--on-primary)',
            border: 'none',
            borderRadius: 6,
            fontSize: 12,
            fontWeight: 600,
            cursor: runDisabled ? 'not-allowed' : 'pointer',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 6,
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
            play_arrow
          </span>
          {isRunning ? 'Running…' : 'Run Monte Carlo'}
        </button>
      </section>
    </aside>
  );
}

// ── Sub-components ──────────────────────────────────────────────────

function ParamRow({
  candidate,
  checked,
  onToggle,
}: {
  candidate: McParameterCandidate;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <li>
      <label
        data-testid={`montecarlo-param-row-${candidate.id}`}
        data-checked={checked}
        className="flex items-center gap-2 px-3 py-1.5"
        style={{
          cursor: 'pointer',
          fontSize: 12,
          color: 'var(--on-surface)',
        }}
      >
        <input
          type="checkbox"
          checked={checked}
          onChange={onToggle}
          data-testid={`montecarlo-param-checkbox-${candidate.id}`}
          style={{ cursor: 'pointer' }}
        />
        <div className="flex-1 min-w-0">
          <div className="truncate mono-text" style={{ fontSize: 12 }}>
            {candidate.name}
          </div>
          {candidate.qualifier && (
            <div
              className="truncate"
              style={{ fontSize: 10, color: 'var(--outline)' }}
            >
              {candidate.qualifier}
            </div>
          )}
        </div>
      </label>
    </li>
  );
}

function EmptyRow({
  icon,
  title,
  hint,
  testId,
  spinning = false,
}: {
  icon: string;
  title: string;
  hint: string;
  testId: string;
  spinning?: boolean;
}) {
  return (
    <div
      data-testid={testId}
      className="flex flex-col items-center justify-center gap-2 px-4 py-6"
      style={{ color: 'var(--outline)' }}
    >
      <span
        className="material-symbols-outlined"
        style={{
          fontSize: 24,
          opacity: 0.8,
          animation: spinning ? 'spin 1s linear infinite' : undefined,
        }}
      >
        {icon}
      </span>
      <span style={{ fontSize: 12, fontWeight: 500 }}>{title}</span>
      <span
        style={{ fontSize: 11, maxWidth: 260, textAlign: 'center' }}
      >
        {hint}
      </span>
    </div>
  );
}
