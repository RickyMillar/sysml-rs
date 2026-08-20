/**
 * SweepTornadoViewer — sensitivity ranking across swept parameters (R5.3).
 *
 * For each swept parameter we compute `computeSensitivity` — the range of
 * the selected outcome metric across that parameter's observed values.
 * Parameters are then ranked by absolute range and drawn as horizontal
 * bars (classic tornado diagram): widest-range parameter at the top.
 *
 * The bar is split around a centre line at the grand mean so the direction
 * of sensitivity is readable at a glance:
 *   - left half  = metric values BELOW the cross-parameter mean,
 *   - right half = metric values ABOVE.
 *
 * Streaming behaviour: the viewer accepts any subset of children. Pending /
 * running children contribute NaN metrics and are skipped — bars grow as
 * data arrives. Single-value parameters render with zero-width bars and
 * are sorted to the bottom so they don't crowd the top.
 */
import { useMemo } from 'react';
import type { CSSProperties } from 'react';
import type { AxesConfig, ResultData, ResultViewer } from './types';
import {
  type ChildDescriptor,
  type SweepMetricId,
  metricOptionsFor,
  collectParamNames,
  computeSensitivity,
  extractorFor,
} from './sweepViewerHelpers';

// ── Payload ────────────────────────────────────────────────────────

export interface SweepTornadoConfig {
  metric?: SweepMetricId;
  /** Called when the user picks a new metric from the dropdown. */
  onMetricChange?: (metric: SweepMetricId) => void;
}

export interface SweepTornadoData extends ResultData {
  kind: 'sweep-tornado';
  children: ChildDescriptor[];
  config?: SweepTornadoConfig;
}

// ── Component ──────────────────────────────────────────────────────

interface SweepTornadoProps {
  data: SweepTornadoData;
  axes: AxesConfig;
}

function SweepTornado({ data, axes }: SweepTornadoProps) {
  const metric = data.config?.metric ?? 'fail_count';
  const onMetricChange = data.config?.onMetricChange;
  const extractor = useMemo(() => extractorFor(metric), [metric]);
  // Built-ins plus whatever outcomes the children have reported so far.
  const metricOptions = useMemo(() => metricOptionsFor(data.children), [data.children]);
  const paramNames = useMemo(() => collectParamNames(data.children), [data.children]);

  const rows = useMemo(() => {
    const ranked = paramNames.map((name) => ({
      name,
      stat: computeSensitivity(data.children, name, extractor),
    }));
    // Descending range; multi-sample params above single-value ones.
    ranked.sort((a, b) => {
      if (a.stat.samples < 2 && b.stat.samples >= 2) return 1;
      if (b.stat.samples < 2 && a.stat.samples >= 2) return -1;
      return b.stat.range - a.stat.range;
    });
    return ranked;
  }, [paramNames, data.children, extractor]);

  // Grand range across all rows so bars share a common x-scale.
  const maxRange = useMemo(() => {
    let m = 0;
    for (const r of rows) if (r.stat.range > m) m = r.stat.range;
    return m;
  }, [rows]);

  if (data.children.length === 0 || paramNames.length === 0) {
    return <EmptyState axes={axes} />;
  }

  const hasSignal = maxRange > 0;

  return (
    <div
      data-testid="sweep-tornado"
      style={{
        height: axes.height,
        width: axes.width ?? '100%',
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        color: 'var(--on-surface)',
        fontSize: 12,
      }}
    >
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'center',
          marginBottom: 4,
        }}
      >
        <span style={{ opacity: 0.7 }}>Parameter sensitivity</span>
        <MetricPicker metric={metric} options={metricOptions} onChange={onMetricChange} />
      </div>
      {!hasSignal ? (
        <div
          data-testid="sweep-tornado-no-signal"
          style={{
            padding: '12px 16px',
            borderRadius: 8,
            border: '1px dashed color-mix(in srgb, var(--outline-variant) 35%, transparent)',
            opacity: 0.7,
            fontStyle: 'italic',
          }}
        >
          Waiting for sweep points to produce a metric spread…
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {rows.map((row) => (
            <TornadoBar
              key={row.name}
              name={row.name}
              low={row.stat.low}
              high={row.stat.high}
              range={row.stat.range}
              samples={row.stat.samples}
              scale={maxRange}
            />
          ))}
        </div>
      )}
    </div>
  );
}

// ── Metric picker ──────────────────────────────────────────────────

interface MetricPickerProps {
  metric: SweepMetricId;
  /** Selectable metrics for THIS batch — built-ins plus measured outcomes. */
  options: { value: SweepMetricId; label: string }[];
  onChange?: (next: SweepMetricId) => void;
}

function MetricPicker({ metric, options, onChange }: MetricPickerProps) {
  const disabled = !onChange;
  return (
    <label
      style={{
        display: 'inline-flex',
        alignItems: 'center',
        gap: 6,
        opacity: disabled ? 0.5 : 1,
      }}
    >
      <span style={{ fontSize: 11 }}>Metric</span>
      <select
        data-testid="sweep-tornado-metric"
        value={metric}
        disabled={disabled}
        onChange={(e) => onChange?.(e.target.value as SweepMetricId)}
        style={{
          fontSize: 11,
          padding: '2px 6px',
          borderRadius: 4,
          background: 'var(--surface)',
          color: 'var(--on-surface)',
          border: '1px solid color-mix(in srgb, var(--outline-variant) 30%, transparent)',
        }}
      >
        {options.map((opt) => (
          <option key={opt.value} value={opt.value}>
            {opt.label}
          </option>
        ))}
      </select>
    </label>
  );
}

// ── Bar ────────────────────────────────────────────────────────────

interface TornadoBarProps {
  name: string;
  low: number;
  high: number;
  range: number;
  samples: number;
  scale: number;
}

function TornadoBar({ name, low, high, range, samples, scale }: TornadoBarProps) {
  const pct = scale > 0 ? (range / scale) * 100 : 0;
  const single = samples < 2;
  const barStyle: CSSProperties = {
    height: 14,
    width: `${pct}%`,
    borderRadius: 4,
    background: single
      ? 'color-mix(in srgb, var(--outline-variant) 35%, transparent)'
      : 'linear-gradient(90deg, var(--chart-series-2) 0%, var(--chart-series-3) 100%)',
    transition: 'width 240ms ease-out',
  };

  return (
    <div
      data-testid={`sweep-tornado-bar-${name}`}
      data-samples={samples}
      style={{ display: 'grid', gridTemplateColumns: '140px 1fr 120px', alignItems: 'center', gap: 8 }}
    >
      <span style={{ fontWeight: 500, whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis' }}>
        {name}
      </span>
      <div
        role="img"
        aria-label={`${name} range ${formatNum(range)} across ${samples} sample${samples === 1 ? '' : 's'}`}
        style={{
          height: 14,
          width: '100%',
          background: 'color-mix(in srgb, var(--outline-variant) 12%, transparent)',
          borderRadius: 4,
          overflow: 'hidden',
        }}
      >
        <div style={barStyle} />
      </div>
      <span style={{ fontVariantNumeric: 'tabular-nums', fontSize: 11, opacity: 0.85 }}>
        {Number.isFinite(low) && Number.isFinite(high)
          ? `${formatNum(low)} … ${formatNum(high)}`
          : '—'}
      </span>
    </div>
  );
}

function formatNum(v: number): string {
  if (!Number.isFinite(v)) return '—';
  if (Number.isInteger(v)) return String(v);
  return v.toFixed(3);
}

// ── Empty state ────────────────────────────────────────────────────

function EmptyState({ axes }: { axes: AxesConfig }) {
  return (
    <div
      data-testid="sweep-tornado-empty"
      role="status"
      style={{
        height: axes.height ?? 200,
        width: axes.width ?? '100%',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        color: 'var(--on-surface)',
        opacity: 0.6,
        fontSize: 13,
        fontStyle: 'italic',
        border: '1px dashed color-mix(in srgb, var(--outline-variant) 35%, transparent)',
        borderRadius: 8,
        padding: 16,
      }}
    >
      Run a sweep with at least one parameter to see sensitivity
    </div>
  );
}

// ── Canonical viewer export ────────────────────────────────────────

export const sweepTornadoViewer: ResultViewer<SweepTornadoData> = {
  id: 'sweep-tornado-default',
  kind: 'sweep-tornado',
  accepts: (data): data is SweepTornadoData => data.kind === 'sweep-tornado',
  render: (data, axes) => <SweepTornado data={data} axes={axes} />,
};
