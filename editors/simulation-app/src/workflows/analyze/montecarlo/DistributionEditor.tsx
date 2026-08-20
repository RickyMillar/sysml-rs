/**
 * DistributionEditor — per-parameter Distribution editor used inside
 * `MonteCarloConfig`. One component handles every kind; the body is a
 * `switch` on `dist.kind`:
 *
 *   - normal:      mean, σ
 *   - uniform:     min, max
 *   - triangular:  min, mode, max
 *   - custom-cdf:  textarea parsed on-edit into `CdfPoint[]`
 *
 * Stateless / dumb: the parent owns the Distribution and the
 * remove-parameter action. Value changes fire `onChange(next)`;
 * kind changes fire `onKindChange(next)` so the parent can reset the
 * distribution to the kind's defaults (via `setDistributionKind`).
 */

import { useMemo } from 'react';
import {
  parseCustomCdfPoints,
  type CustomCdfDist,
  type Distribution,
  type DistributionKind,
  type NormalDist,
  type TriangularDist,
  type UniformDist,
} from './sampleDistribution';

export interface DistributionEditorProps {
  /** The parameter name this editor is bound to (used for testids + header). */
  paramName: string;
  /** The current distribution (source of truth owned by the parent). */
  distribution: Distribution;
  /** True when the parent thinks this distribution is currently valid. */
  isValid: boolean;
  /** Fires when the distribution kind flips (parent resets fields to defaults). */
  onKindChange: (kind: DistributionKind) => void;
  /** Fires on any field edit — parent stores the next Distribution verbatim. */
  onChange: (next: Distribution) => void;
  /** Remove this parameter from the selection. */
  onRemove: () => void;
}

const KIND_LABELS: Record<DistributionKind, string> = {
  normal: 'Normal',
  uniform: 'Uniform',
  triangular: 'Triangular',
  'custom-cdf': 'Custom CDF',
};

const KIND_ORDER: readonly DistributionKind[] = [
  'normal',
  'uniform',
  'triangular',
  'custom-cdf',
];

export function DistributionEditor({
  paramName,
  distribution,
  isValid,
  onKindChange,
  onChange,
  onRemove,
}: DistributionEditorProps) {
  return (
    <div
      data-testid={`mc-dist-editor-${paramName}`}
      data-kind={distribution.kind}
      data-valid={isValid}
      className="flex flex-col gap-2 px-3 py-2"
      style={{
        borderBottom: '1px solid var(--outline-variant)',
        background: isValid ? 'transparent' : 'var(--error-container, rgba(220,60,60,0.08))',
      }}
    >
      {/* Header: name + kind picker + remove */}
      <div className="flex items-center gap-2">
        <span
          data-testid={`mc-param-name-${paramName}`}
          className="mono-text flex-1 truncate"
          style={{ fontSize: 12, fontWeight: 600, color: 'var(--on-surface)' }}
        >
          {paramName}
        </span>
        <select
          data-testid={`mc-kind-select-${paramName}`}
          value={distribution.kind}
          onChange={(e) => onKindChange(e.target.value as DistributionKind)}
          style={{
            height: 24,
            padding: '0 6px',
            background: 'var(--surface-container)',
            color: 'var(--on-surface)',
            border: '1px solid var(--outline-variant)',
            borderRadius: 4,
            fontSize: 11,
          }}
        >
          {KIND_ORDER.map((k) => (
            <option key={k} value={k}>
              {KIND_LABELS[k]}
            </option>
          ))}
        </select>
        <button
          type="button"
          data-testid={`mc-remove-param-${paramName}`}
          onClick={onRemove}
          aria-label={`Remove parameter ${paramName}`}
          style={{
            width: 22,
            height: 22,
            background: 'transparent',
            border: 'none',
            color: 'var(--outline)',
            cursor: 'pointer',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
          }}
        >
          <span className="material-symbols-outlined" style={{ fontSize: 16 }}>
            close
          </span>
        </button>
      </div>

      {/* Body dispatches on kind. */}
      {distribution.kind === 'normal' && (
        <NormalEditor
          paramName={paramName}
          dist={distribution}
          onChange={onChange}
        />
      )}
      {distribution.kind === 'uniform' && (
        <UniformEditor
          paramName={paramName}
          dist={distribution}
          onChange={onChange}
        />
      )}
      {distribution.kind === 'triangular' && (
        <TriangularEditor
          paramName={paramName}
          dist={distribution}
          onChange={onChange}
        />
      )}
      {distribution.kind === 'custom-cdf' && (
        <CustomCdfEditor
          paramName={paramName}
          dist={distribution}
          onChange={onChange}
        />
      )}
    </div>
  );
}

// ── Sub-editors ─────────────────────────────────────────────────────

function NormalEditor({
  paramName,
  dist,
  onChange,
}: {
  paramName: string;
  dist: NormalDist;
  onChange: (next: Distribution) => void;
}) {
  return (
    <div className="grid grid-cols-2 gap-2">
      <NumberField
        testId={`mc-normal-mean-${paramName}`}
        label="mean"
        value={dist.mean}
        onChange={(v) => onChange({ ...dist, mean: v })}
      />
      <NumberField
        testId={`mc-normal-sigma-${paramName}`}
        label="σ"
        value={dist.sigma}
        min={0}
        onChange={(v) => onChange({ ...dist, sigma: v })}
      />
    </div>
  );
}

function UniformEditor({
  paramName,
  dist,
  onChange,
}: {
  paramName: string;
  dist: UniformDist;
  onChange: (next: Distribution) => void;
}) {
  return (
    <div className="grid grid-cols-2 gap-2">
      <NumberField
        testId={`mc-uniform-min-${paramName}`}
        label="min"
        value={dist.min}
        onChange={(v) => onChange({ ...dist, min: v })}
      />
      <NumberField
        testId={`mc-uniform-max-${paramName}`}
        label="max"
        value={dist.max}
        onChange={(v) => onChange({ ...dist, max: v })}
      />
    </div>
  );
}

function TriangularEditor({
  paramName,
  dist,
  onChange,
}: {
  paramName: string;
  dist: TriangularDist;
  onChange: (next: Distribution) => void;
}) {
  return (
    <div className="grid grid-cols-3 gap-2">
      <NumberField
        testId={`mc-tri-min-${paramName}`}
        label="min"
        value={dist.min}
        onChange={(v) => onChange({ ...dist, min: v })}
      />
      <NumberField
        testId={`mc-tri-mode-${paramName}`}
        label="mode"
        value={dist.mode}
        onChange={(v) => onChange({ ...dist, mode: v })}
      />
      <NumberField
        testId={`mc-tri-max-${paramName}`}
        label="max"
        value={dist.max}
        onChange={(v) => onChange({ ...dist, max: v })}
      />
    </div>
  );
}

function CustomCdfEditor({
  paramName,
  dist,
  onChange,
}: {
  paramName: string;
  dist: CustomCdfDist;
  onChange: (next: Distribution) => void;
}) {
  // Parse errors surface inline; valid parses propagate the new `points`.
  const parseResult = useMemo(() => {
    try {
      return { ok: true as const, points: parseCustomCdfPoints(dist.raw) };
    } catch (err) {
      return {
        ok: false as const,
        error: err instanceof Error ? err.message : String(err),
      };
    }
  }, [dist.raw]);

  return (
    <div className="flex flex-col gap-1">
      <label
        style={{
          fontSize: 10,
          color: 'var(--outline)',
          textTransform: 'uppercase',
          letterSpacing: '0.04em',
        }}
      >
        CDF points — one "x, cdf" per line
      </label>
      <textarea
        data-testid={`mc-custom-cdf-${paramName}`}
        value={dist.raw}
        onChange={(e) => {
          const raw = e.target.value;
          // Parse on change; if it fails, keep the old `points` so the
          // distribution stays internally consistent until the user fixes it.
          try {
            const points = parseCustomCdfPoints(raw);
            onChange({ ...dist, raw, points });
          } catch {
            onChange({ ...dist, raw });
          }
        }}
        rows={4}
        style={{
          fontFamily: 'ui-monospace, SFMono-Regular, Menlo, monospace',
          fontSize: 11,
          padding: 6,
          background: 'var(--surface-container)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          resize: 'vertical',
        }}
      />
      {parseResult.ok ? (
        <span
          data-testid={`mc-custom-cdf-status-${paramName}`}
          style={{ fontSize: 10, color: 'var(--outline)' }}
        >
          {parseResult.points.length} point{parseResult.points.length === 1 ? '' : 's'}
        </span>
      ) : (
        <span
          data-testid={`mc-custom-cdf-error-${paramName}`}
          style={{ fontSize: 10, color: 'var(--error)' }}
        >
          {parseResult.error}
        </span>
      )}
    </div>
  );
}

function NumberField({
  label,
  value,
  min,
  onChange,
  testId,
}: {
  label: string;
  value: number;
  min?: number;
  onChange: (next: number) => void;
  testId: string;
}) {
  return (
    <label className="flex flex-col gap-1">
      <span
        style={{
          fontSize: 10,
          color: 'var(--outline)',
          textTransform: 'uppercase',
          letterSpacing: '0.04em',
        }}
      >
        {label}
      </span>
      <input
        type="number"
        data-testid={testId}
        value={Number.isFinite(value) ? value : ''}
        min={min}
        step="any"
        onChange={(e) => {
          const next = Number(e.target.value);
          // Passing NaN keeps the field editable but flips validity off.
          onChange(Number.isNaN(next) ? Number.NaN : next);
        }}
        style={{
          height: 24,
          padding: '0 6px',
          background: 'var(--surface-container)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          fontSize: 11,
        }}
      />
    </label>
  );
}
