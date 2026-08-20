/**
 * SweepParallelCoordsViewer — classic parallel-coordinates plot (R5.3).
 *
 * One vertical axis per parameter + one for the outcome metric. Each
 * ChildDescriptor contributes one polyline crossing every axis at its
 * normalised value. Failing-verdict rollups are highlighted in red so
 * fail clusters pop visually across hundreds of lines.
 *
 * Brushing: the user drags on an axis to set `[min, max]` in that axis's
 * data space. Any line whose axis value falls outside the brush is
 * dimmed, but never removed — brushing is a filter, not a delete. Clearing
 * the brush (click outside the band) restores full opacity.
 *
 * Axis normalisation goes through `normaliseAxisValues` so the viewer does
 * not care about raw units. Empty / single-value / NaN axes render as a
 * centre-only stripe (the normaliser returns 0.5 uniformly) — this keeps
 * streaming mounts stable while data arrives.
 */
import { useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import type { AxesConfig, ResultData, ResultViewer } from './types';
import {
  type ChildDescriptor,
  type SweepMetricId,
  metricOptionsFor,
  metricLabelFor,
  collectParamNames,
  extractorFor,
  normaliseAxisValues,
  rollupVerdict,
  toNumber,
} from './sweepViewerHelpers';

// ── Payload ────────────────────────────────────────────────────────

export interface SweepParallelCoordsConfig {
  metric?: SweepMetricId;
  onMetricChange?: (m: SweepMetricId) => void;
}

export interface SweepParallelCoordsData extends ResultData {
  kind: 'sweep-parallel-coords';
  children: ChildDescriptor[];
  config?: SweepParallelCoordsConfig;
}

// ── Brush state ────────────────────────────────────────────────────

interface Brush {
  axis: string;
  /** Values in the axis's data space (not normalised). */
  lo: number;
  hi: number;
}

// ── Component ──────────────────────────────────────────────────────

const METRIC_AXIS = '__metric__';
const CANVAS_W = 600;
const CANVAS_H = 280;
const MARGIN = { top: 24, right: 32, bottom: 28, left: 32 };

interface SweepParallelCoordsProps {
  data: SweepParallelCoordsData;
  axes: AxesConfig;
}

function SweepParallelCoords({ data, axes }: SweepParallelCoordsProps) {
  const { children, config } = data;
  const metric = config?.metric ?? 'fail_count';
  const metricLabel = metricLabelFor(metric, children);
  const extractor = useMemo(() => extractorFor(metric), [metric]);
  // Built-ins plus whatever outcomes the children have reported so far.
  const metricOptions = useMemo(() => metricOptionsFor(children), [children]);
  const paramNames = useMemo(() => collectParamNames(children), [children]);
  const axisNames = useMemo(() => [...paramNames, METRIC_AXIS], [paramNames]);

  // Per-axis normalisers.
  const normalisers = useMemo(() => {
    const map = new Map<string, ReturnType<typeof normaliseAxisValues>>();
    for (const axis of axisNames) {
      const values = children.map((c) =>
        axis === METRIC_AXIS ? extractor(c) : toNumber(c.params[axis]),
      );
      map.set(axis, normaliseAxisValues(values));
    }
    return map;
  }, [axisNames, children, extractor]);

  const [brush, setBrush] = useState<Brush | null>(null);

  if (children.length === 0 || axisNames.length <= 1) {
    return <EmptyState axes={axes} />;
  }

  const innerW = CANVAS_W - MARGIN.left - MARGIN.right;
  const innerH = CANVAS_H - MARGIN.top - MARGIN.bottom;
  const step = axisNames.length > 1 ? innerW / (axisNames.length - 1) : 0;

  const axisX = (i: number) => MARGIN.left + i * step;

  const polylineFor = (child: ChildDescriptor): { points: string; withinBrush: boolean } => {
    const parts: string[] = [];
    let within = true;
    axisNames.forEach((axis, i) => {
      const raw =
        axis === METRIC_AXIS ? extractor(child) : toNumber(child.params[axis]);
      const norm = normalisers.get(axis)!;
      const y = MARGIN.top + (1 - norm.normalise(raw)) * innerH;
      parts.push(`${axisX(i)},${y}`);
      if (brush && brush.axis === axis && Number.isFinite(raw)) {
        if (raw < brush.lo || raw > brush.hi) within = false;
      }
    });
    return { points: parts.join(' '), withinBrush: within };
  };

  const failingChildIds = new Set(
    children.filter((c) => rollupVerdict(c) === 'fail').map((c) => c.session_id),
  );

  return (
    <div
      data-testid="sweep-parallel-coords"
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
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
        <span style={{ opacity: 0.7 }}>
          {brush ? (
            <>
              Brush on <strong>{brush.axis === METRIC_AXIS ? metricLabel : brush.axis}</strong>:{' '}
              {brush.lo.toFixed(2)} … {brush.hi.toFixed(2)}{' '}
              <button
                type="button"
                data-testid="sweep-pc-clear-brush"
                onClick={() => setBrush(null)}
                style={clearButtonStyle}
              >
                clear
              </button>
            </>
          ) : (
            'Drag on an axis to brush'
          )}
        </span>
        <MetricPicker metric={metric} options={metricOptions} onChange={config?.onMetricChange} />
      </div>
      <svg
        viewBox={`0 0 ${CANVAS_W} ${CANVAS_H}`}
        preserveAspectRatio="xMidYMid meet"
        style={{ width: '100%', height: axes.height ?? CANVAS_H }}
        data-testid="sweep-pc-svg"
        role="img"
        aria-label="Parallel coordinates plot"
      >
        {/* Axes */}
        {axisNames.map((axis, i) => {
          const norm = normalisers.get(axis)!;
          const x = axisX(i);
          return (
            <g key={axis} data-testid={`sweep-pc-axis-${axis}`}>
              <line
                x1={x}
                x2={x}
                y1={MARGIN.top}
                y2={MARGIN.top + innerH}
                stroke="currentColor"
                strokeOpacity={0.25}
                strokeWidth={1}
              />
              <text
                x={x}
                y={MARGIN.top - 8}
                textAnchor="middle"
                fontSize={11}
                fill="currentColor"
                opacity={0.8}
              >
                {axis === METRIC_AXIS ? metricLabel : axis}
              </text>
              <text
                x={x}
                y={MARGIN.top + innerH + 14}
                textAnchor="middle"
                fontSize={10}
                fill="currentColor"
                opacity={0.5}
              >
                {Number.isFinite(norm.min) ? formatShort(norm.min) : '—'}
              </text>
              <text
                x={x}
                y={MARGIN.top - 20}
                textAnchor="middle"
                fontSize={10}
                fill="currentColor"
                opacity={0.5}
              >
                {Number.isFinite(norm.max) ? formatShort(norm.max) : '—'}
              </text>
              <AxisBrushHandle
                axis={axis}
                x={x}
                y={MARGIN.top}
                height={innerH}
                normaliser={norm}
                onBrush={setBrush}
                active={brush?.axis === axis ? brush : null}
              />
            </g>
          );
        })}
        {/* Polylines */}
        {children.map((child) => {
          const { points, withinBrush } = polylineFor(child);
          const fail = failingChildIds.has(child.session_id);
          const stroke = fail ? 'var(--verdict-fail)' : 'var(--chart-series-2)';
          const opacity = withinBrush ? (fail ? 0.9 : 0.55) : 0.08;
          return (
            <polyline
              key={child.session_id}
              points={points}
              fill="none"
              stroke={stroke}
              strokeOpacity={opacity}
              strokeWidth={fail ? 1.6 : 1.1}
              data-testid={`sweep-pc-line-${child.index}`}
              data-within-brush={withinBrush}
              data-fail={fail}
            />
          );
        })}
      </svg>
    </div>
  );
}

// ── Axis brush handle ──────────────────────────────────────────────

interface AxisBrushHandleProps {
  axis: string;
  x: number;
  y: number;
  height: number;
  normaliser: ReturnType<typeof normaliseAxisValues>;
  onBrush: (brush: Brush | null) => void;
  active: Brush | null;
}

function AxisBrushHandle({ axis, x, y, height, normaliser, onBrush, active }: AxisBrushHandleProps) {
  const [drag, setDrag] = useState<{ startY: number; curY: number } | null>(null);

  const pxToValue = (py: number): number => {
    const { min, max } = normaliser;
    if (!Number.isFinite(min) || !Number.isFinite(max) || min === max) return min;
    const t = 1 - (py - y) / height;
    return min + Math.max(0, Math.min(1, t)) * (max - min);
  };

  const onDown = (e: React.PointerEvent<SVGRectElement>) => {
    (e.target as Element).setPointerCapture?.(e.pointerId);
    const pt = (e.target as SVGGraphicsElement).ownerSVGElement?.createSVGPoint();
    let localY = e.clientY;
    if (pt) {
      pt.x = e.clientX;
      pt.y = e.clientY;
      const screen = (e.target as SVGGraphicsElement).ownerSVGElement?.getScreenCTM();
      if (screen) {
        const inv = pt.matrixTransform(screen.inverse());
        localY = inv.y;
      }
    }
    setDrag({ startY: localY, curY: localY });
  };

  const onMove = (e: React.PointerEvent<SVGRectElement>) => {
    if (!drag) return;
    const pt = (e.target as SVGGraphicsElement).ownerSVGElement?.createSVGPoint();
    let localY = e.clientY;
    if (pt) {
      pt.x = e.clientX;
      pt.y = e.clientY;
      const screen = (e.target as SVGGraphicsElement).ownerSVGElement?.getScreenCTM();
      if (screen) {
        const inv = pt.matrixTransform(screen.inverse());
        localY = inv.y;
      }
    }
    setDrag({ startY: drag.startY, curY: localY });
  };

  const onUp = () => {
    if (!drag) return;
    const a = pxToValue(drag.startY);
    const b = pxToValue(drag.curY);
    if (Math.abs(drag.curY - drag.startY) < 4) {
      onBrush(null);
    } else {
      onBrush({ axis, lo: Math.min(a, b), hi: Math.max(a, b) });
    }
    setDrag(null);
  };

  // Visible band when brushed.
  let bandY = y;
  let bandH = 0;
  if (active && normaliser.min !== normaliser.max) {
    const tHi = normaliser.normalise(active.hi);
    const tLo = normaliser.normalise(active.lo);
    const yHi = y + (1 - tHi) * height;
    const yLo = y + (1 - tLo) * height;
    bandY = Math.min(yHi, yLo);
    bandH = Math.abs(yHi - yLo);
  }

  return (
    <g>
      <rect
        data-testid={`sweep-pc-brush-${axis}`}
        x={x - 10}
        y={y}
        width={20}
        height={height}
        fill="transparent"
        style={{ cursor: 'ns-resize', touchAction: 'none' }}
        onPointerDown={onDown}
        onPointerMove={onMove}
        onPointerUp={onUp}
        onPointerCancel={onUp}
      />
      {active ? (
        <rect
          x={x - 6}
          y={bandY}
          width={12}
          height={bandH}
          fill="var(--accent)"
          fillOpacity={0.2}
          stroke="var(--accent)"
          strokeOpacity={0.5}
          pointerEvents="none"
        />
      ) : null}
    </g>
  );
}

// ── Metric picker ──────────────────────────────────────────────────

function MetricPicker({
  metric,
  options,
  onChange,
}: {
  metric: SweepMetricId;
  /** Selectable metrics for THIS batch — built-ins plus measured outcomes. */
  options: { value: SweepMetricId; label: string }[];
  onChange?: (next: SweepMetricId) => void;
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
        data-testid="sweep-pc-metric"
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

function formatShort(v: number): string {
  if (!Number.isFinite(v)) return '—';
  if (Number.isInteger(v)) return String(v);
  if (Math.abs(v) >= 1000) return v.toExponential(1);
  return v.toFixed(2);
}

const clearButtonStyle: CSSProperties = {
  marginLeft: 6,
  background: 'transparent',
  border: 'none',
  color: 'var(--on-surface)',
  textDecoration: 'underline',
  cursor: 'pointer',
  padding: 0,
  fontSize: 11,
};

// ── Empty state ────────────────────────────────────────────────────

function EmptyState({ axes }: { axes: AxesConfig }) {
  return (
    <div
      data-testid="sweep-parallel-coords-empty"
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
      }}
    >
      Sweep at least one parameter to populate the axes
    </div>
  );
}

// ── Canonical viewer export ────────────────────────────────────────

export const sweepParallelCoordsViewer: ResultViewer<SweepParallelCoordsData> = {
  id: 'sweep-parallel-coords-default',
  kind: 'sweep-parallel-coords',
  accepts: (data): data is SweepParallelCoordsData => data.kind === 'sweep-parallel-coords',
  render: (data, axes) => <SweepParallelCoords data={data} axes={axes} />,
};
