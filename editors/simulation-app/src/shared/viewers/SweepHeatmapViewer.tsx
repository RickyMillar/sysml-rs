/**
 * SweepHeatmapViewer — 2D sweep grid with viridis colour scale (R5.3).
 *
 * Applies when exactly two parameters are swept. The user picks which
 * two params map to x / y and which metric fills each cell; we build a
 * sorted-unique grid via `buildHeatmapGrid` and render an SVG rectangle
 * per cell. NaN cells (no data yet, or a pending / failed child) render
 * with a hatch pattern so streaming progress is visible.
 *
 * If the swept parameter space is not exactly 2-dimensional (zero, one,
 * or 3+), we render an informative empty state rather than a silent
 * no-op — the user needs to know why the heatmap tab is empty.
 */
import { useEffect, useMemo, useState } from 'react';
import type { CSSProperties, ReactElement } from 'react';
import type { AxesConfig, ResultData, ResultViewer } from './types';
import {
  type ChildDescriptor,
  type SweepMetricId,
  metricOptionsFor,
  buildHeatmapGrid,
  collectParamNames,
  colourForNormalised,
  extractorFor,
} from './sweepViewerHelpers';

// ── Payload ────────────────────────────────────────────────────────

export interface SweepHeatmapConfig {
  /** Preferred x-axis parameter. If absent, we pick `paramNames[0]`. */
  xParam?: string;
  /** Preferred y-axis parameter. If absent, we pick `paramNames[1]`. */
  yParam?: string;
  metric?: SweepMetricId;
  onMetricChange?: (m: SweepMetricId) => void;
  onAxisChange?: (next: { xParam: string; yParam: string }) => void;
}

export interface SweepHeatmapData extends ResultData {
  kind: 'sweep-heatmap';
  children: ChildDescriptor[];
  config?: SweepHeatmapConfig;
}

// ── Component ──────────────────────────────────────────────────────

interface SweepHeatmapProps {
  data: SweepHeatmapData;
  axes: AxesConfig;
}

function SweepHeatmap({ data, axes }: SweepHeatmapProps) {
  const { children, config } = data;
  const metric = config?.metric ?? 'fail_count';
  const extractor = useMemo(() => extractorFor(metric), [metric]);
  // Built-ins plus whatever outcomes the children have reported so far.
  const metricOptions = useMemo(() => metricOptionsFor(children), [children]);
  const paramNames = useMemo(() => collectParamNames(children), [children]);

  // Local picks so the viewer works standalone; config overrides take
  // precedence when the parent owns the state.
  const [localX, setLocalX] = useState<string | null>(null);
  const [localY, setLocalY] = useState<string | null>(null);

  const xParam = config?.xParam ?? localX ?? paramNames[0] ?? null;
  const yParam = config?.yParam ?? localY ?? paramNames[1] ?? null;

  // Reset local picks when the param set changes shape — avoids dangling
  // references to params that have since been removed.
  useEffect(() => {
    if (localX && !paramNames.includes(localX)) setLocalX(null);
    if (localY && !paramNames.includes(localY)) setLocalY(null);
  }, [paramNames, localX, localY]);

  // All hooks must run unconditionally — feed null/degenerate inputs when
  // the viewer is in an empty state so the hook order stays stable across
  // streaming renders.
  const gridReady = Boolean(xParam && yParam && xParam !== yParam);
  const grid = useMemo(
    () =>
      gridReady
        ? buildHeatmapGrid(children, xParam as string, yParam as string, extractor)
        : { x: [], y: [], values: [] as number[][] },
    [gridReady, children, xParam, yParam, extractor],
  );

  // Metric range for viridis normalisation. NaN-safe.
  const { vmin, vmax } = useMemo(() => {
    let mn = Number.POSITIVE_INFINITY;
    let mx = Number.NEGATIVE_INFINITY;
    for (const row of grid.values) {
      for (const v of row) {
        if (!Number.isFinite(v)) continue;
        if (v < mn) mn = v;
        if (v > mx) mx = v;
      }
    }
    return { vmin: mn, vmax: mx };
  }, [grid.values]);

  if (children.length === 0) return <EmptyState axes={axes} message="No sweep points yet" />;
  if (paramNames.length < 2) {
    return <EmptyState axes={axes} message="Heatmap needs at least two swept parameters" />;
  }
  if (!xParam || !yParam || xParam === yParam) {
    return <EmptyState axes={axes} message="Pick two distinct parameters to plot" />;
  }

  const hasSignal = Number.isFinite(vmin) && Number.isFinite(vmax);

  return (
    <div
      data-testid="sweep-heatmap"
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
      <div style={{ display: 'flex', gap: 12, justifyContent: 'space-between', flexWrap: 'wrap' }}>
        <div style={{ display: 'flex', gap: 8 }}>
          <AxisPicker
            label="X"
            testId="sweep-heatmap-x"
            value={xParam}
            options={paramNames}
            onChange={(v) => {
              if (config?.onAxisChange) config.onAxisChange({ xParam: v, yParam });
              else setLocalX(v);
            }}
          />
          <AxisPicker
            label="Y"
            testId="sweep-heatmap-y"
            value={yParam}
            options={paramNames}
            onChange={(v) => {
              if (config?.onAxisChange) config.onAxisChange({ xParam, yParam: v });
              else setLocalY(v);
            }}
          />
        </div>
        <MetricPicker metric={metric} options={metricOptions} onChange={config?.onMetricChange} />
      </div>

      {!hasSignal ? (
        <div
          data-testid="sweep-heatmap-waiting"
          style={{
            padding: '12px 16px',
            borderRadius: 8,
            border: '1px dashed color-mix(in srgb, var(--outline-variant) 35%, transparent)',
            opacity: 0.7,
            fontStyle: 'italic',
          }}
        >
          Waiting for cells to settle…
        </div>
      ) : (
        <HeatmapGridView
          grid={grid}
          xLabel={xParam}
          yLabel={yParam}
          vmin={vmin}
          vmax={vmax}
        />
      )}
    </div>
  );
}

// ── Grid renderer ──────────────────────────────────────────────────

interface HeatmapGridViewProps {
  grid: ReturnType<typeof buildHeatmapGrid>;
  xLabel: string;
  yLabel: string;
  vmin: number;
  vmax: number;
}

const CELL = 36;
const PAD = { top: 28, left: 72, right: 60, bottom: 44 };

function HeatmapGridView({ grid, xLabel, yLabel, vmin, vmax }: HeatmapGridViewProps) {
  const width = PAD.left + grid.x.length * CELL + PAD.right;
  const height = PAD.top + grid.y.length * CELL + PAD.bottom;

  const span = vmax - vmin;
  const normalise = (v: number) => (span === 0 ? 0.5 : (v - vmin) / span);

  return (
    <div style={{ width: '100%', overflow: 'auto' }}>
      <svg
        width={width}
        height={height}
        data-testid="sweep-heatmap-svg"
        role="img"
        aria-label={`Heatmap of ${yLabel} × ${xLabel}`}
      >
        <defs>
          <pattern id="sweep-heatmap-hatch" patternUnits="userSpaceOnUse" width="6" height="6">
            <rect width="6" height="6" fill="color-mix(in srgb, var(--outline-variant) 10%, transparent)" />
            <path d="M0,6 L6,0" stroke="currentColor" strokeOpacity="0.25" strokeWidth="1" />
          </pattern>
        </defs>
        {/* X labels */}
        {grid.x.map((xv, xi) => (
          <text
            key={xi}
            x={PAD.left + xi * CELL + CELL / 2}
            y={PAD.top - 8}
            textAnchor="middle"
            fontSize={10}
            fill="currentColor"
            opacity={0.8}
          >
            {formatTick(xv)}
          </text>
        ))}
        <text
          x={PAD.left + (grid.x.length * CELL) / 2}
          y={height - 10}
          textAnchor="middle"
          fontSize={11}
          fill="currentColor"
        >
          {xLabel}
        </text>
        {/* Y labels */}
        {grid.y.map((yv, yi) => (
          <text
            key={yi}
            x={PAD.left - 8}
            y={PAD.top + yi * CELL + CELL / 2 + 4}
            textAnchor="end"
            fontSize={10}
            fill="currentColor"
            opacity={0.8}
          >
            {formatTick(yv)}
          </text>
        ))}
        <text
          x={14}
          y={PAD.top + (grid.y.length * CELL) / 2}
          textAnchor="middle"
          fontSize={11}
          fill="currentColor"
          transform={`rotate(-90 14 ${PAD.top + (grid.y.length * CELL) / 2})`}
        >
          {yLabel}
        </text>
        {/* Cells */}
        {grid.y.map((_, yi) =>
          grid.x.map((__, xi) => {
            const v = grid.values[yi][xi];
            const cellX = PAD.left + xi * CELL;
            const cellY = PAD.top + yi * CELL;
            const finite = Number.isFinite(v);
            const fill = finite ? colourForNormalised(normalise(v)) : 'url(#sweep-heatmap-hatch)';
            return (
              <g key={`${yi}-${xi}`} data-testid={`sweep-heatmap-cell-${xi}-${yi}`}>
                <rect
                  x={cellX}
                  y={cellY}
                  width={CELL}
                  height={CELL}
                  fill={fill}
                  stroke="color-mix(in srgb, var(--outline-variant) 40%, transparent)"
                  strokeWidth={0.5}
                />
                {finite ? (
                  <text
                    x={cellX + CELL / 2}
                    y={cellY + CELL / 2 + 4}
                    textAnchor="middle"
                    fontSize={10}
                    fill="#fff"
                    style={{ pointerEvents: 'none' }}
                  >
                    {formatCell(v)}
                  </text>
                ) : null}
              </g>
            );
          }),
        )}
        {/* Legend */}
        <Legend
          x={PAD.left + grid.x.length * CELL + 12}
          y={PAD.top}
          height={Math.max(60, grid.y.length * CELL)}
          vmin={vmin}
          vmax={vmax}
        />
      </svg>
    </div>
  );
}

function Legend({
  x,
  y,
  height,
  vmin,
  vmax,
}: {
  x: number;
  y: number;
  height: number;
  vmin: number;
  vmax: number;
}) {
  const stops = 12;
  const cells: ReactElement[] = [];
  for (let i = 0; i < stops; i++) {
    const t = i / (stops - 1);
    cells.push(
      <rect
        key={i}
        x={x}
        y={y + (1 - t) * (height - height / stops)}
        width={14}
        height={height / stops + 1}
        fill={colourForNormalised(t)}
      />,
    );
  }
  return (
    <g data-testid="sweep-heatmap-legend">
      {cells}
      <text x={x + 18} y={y + 6} fontSize={10} fill="currentColor" opacity={0.75}>
        {formatTick(vmax)}
      </text>
      <text x={x + 18} y={y + height - 2} fontSize={10} fill="currentColor" opacity={0.75}>
        {formatTick(vmin)}
      </text>
    </g>
  );
}

// ── Pickers ────────────────────────────────────────────────────────

interface AxisPickerProps {
  label: string;
  testId: string;
  value: string;
  options: string[];
  onChange: (next: string) => void;
}

function AxisPicker({ label, testId, value, options, onChange }: AxisPickerProps) {
  return (
    <label style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
      <span style={{ fontSize: 11 }}>{label}</span>
      <select
        data-testid={testId}
        value={value}
        onChange={(e) => onChange(e.target.value)}
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
          <option key={opt} value={opt}>
            {opt}
          </option>
        ))}
      </select>
    </label>
  );
}

function MetricPicker({
  metric,
  options,
  onChange,
}: {
  metric: SweepMetricId;
  /** Selectable metrics for THIS batch — built-ins plus measured outcomes. */
  options: { value: SweepMetricId; label: string }[];
  onChange?: (m: SweepMetricId) => void;
}) {
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
        data-testid="sweep-heatmap-metric"
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

function formatTick(v: number): string {
  if (!Number.isFinite(v)) return '—';
  if (Number.isInteger(v)) return String(v);
  if (Math.abs(v) >= 1000 || (Math.abs(v) > 0 && Math.abs(v) < 0.01)) return v.toExponential(1);
  return v.toFixed(2);
}

function formatCell(v: number): string {
  if (!Number.isFinite(v)) return '';
  if (Number.isInteger(v)) return String(v);
  return v.toFixed(2);
}

// ── Empty state ────────────────────────────────────────────────────

function EmptyState({ axes, message }: { axes: AxesConfig; message: string }) {
  return (
    <div
      data-testid="sweep-heatmap-empty"
      role="status"
      style={{
        height: axes.height ?? 260,
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
        textAlign: 'center' as CSSProperties['textAlign'],
      }}
    >
      {message}
    </div>
  );
}

// ── Canonical viewer export ────────────────────────────────────────

export const sweepHeatmapViewer: ResultViewer<SweepHeatmapData> = {
  id: 'sweep-heatmap-default',
  kind: 'sweep-heatmap',
  accepts: (data): data is SweepHeatmapData => data.kind === 'sweep-heatmap',
  render: (data, axes) => <SweepHeatmap data={data} axes={axes} />,
};
