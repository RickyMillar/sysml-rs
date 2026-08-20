/**
 * MonteCarloHistogramViewer — R5.7.
 *
 * Renders one histogram per outcome metric harvested from a Monte Carlo
 * batch. Summary stats (mean, σ, p5, p50, p95) are shown above each
 * chart, and three inline config controls let the user adjust bin
 * count, toggle log-scale on the count axis, and enable a KDE overlay.
 *
 * This is a "kit-friendly" viewer — the component itself is a React
 * node (not a Layer 1 `ResultViewer` registration) because the outcome
 * extraction is caller-supplied. The `'mc-histogram'` kind is added to
 * the viewer-kit union so other sites can still `kind: 'mc-histogram'`
 * against it.
 *
 * SVG is rendered inline (no uPlot dependency) — histograms are small
 * enough that handwritten SVG keeps the bundle lean and the test
 * surface predictable.
 */

import { useId, useMemo, useState } from 'react';
import type { CSSProperties } from 'react';
import type { ChildDescriptor } from '../../workflows/analyze/montecarlo/passRateHelpers';
import {
  buildHistogram,
  kde,
  type HistogramResult,
} from '../../workflows/analyze/montecarlo/histogramHelpers';

/** Outcome metric (id + numeric extractor) for one chart. */
export interface MonteCarloOutcome {
  /** Stable id — used for data-testid and the metric axis label. */
  id: string;
  /** Human label shown as the panel heading. Defaults to `id`. */
  label?: string;
  /** Extract the numeric outcome from a completed child. Return `null`
   *  or `undefined` to skip a child (e.g. pending / failed). */
  extract: (child: ChildDescriptor) => number | null | undefined;
  /** Optional unit string appended to stat labels. */
  unit?: string;
}

export interface MonteCarloHistogramViewerProps {
  /** Iteration records. Incomplete children are tolerated — filtered
   *  out via `extract` returning null. */
  children: ChildDescriptor[];
  /** One or more outcome metrics to bin. */
  outcomes: MonteCarloOutcome[];
  /** Default bin count for the slider (clamped 5–100). Defaults to 20. */
  defaultBinCount?: number;
  /** Optional className hook. */
  className?: string;
  /** Test id passthrough. */
  testId?: string;
}

/** Single-outcome convenience — used by the simplified prop shape the
 *  task spec describes (`{ outcomeExtractor, binCount }`). Kept as a
 *  separate exported surface so CC's sweep viewers can mount it. */
export interface MonteCarloHistogramViewerSingleProps {
  children: ChildDescriptor[];
  outcomeExtractor: (child: ChildDescriptor) => number | null | undefined;
  label?: string;
  unit?: string;
  binCount?: number;
  className?: string;
  testId?: string;
}

const BIN_MIN = 5;
const BIN_MAX = 100;

const ROOT_STYLE: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 16,
  color: 'var(--on-surface)',
  fontSize: 12,
};

const PANEL_STYLE: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 8,
  padding: '12px 14px',
  borderRadius: 8,
  border: '1px solid color-mix(in srgb, var(--outline-variant) 30%, transparent)',
  background: 'color-mix(in srgb, var(--surface-container) 60%, transparent)',
};

const CONTROL_BAR_STYLE: CSSProperties = {
  display: 'flex',
  alignItems: 'center',
  gap: 16,
  flexWrap: 'wrap',
  padding: '8px 12px',
  borderRadius: 8,
  background: 'color-mix(in srgb, var(--outline-variant) 8%, transparent)',
  border: '1px solid color-mix(in srgb, var(--outline-variant) 20%, transparent)',
};

const STATS_ROW_STYLE: CSSProperties = {
  display: 'flex',
  gap: 16,
  flexWrap: 'wrap',
  fontSize: 11,
  opacity: 0.85,
  fontFamily: 'ui-monospace, "JetBrains Mono", monospace',
};

function fmtStat(n: number, unit?: string): string {
  if (!Number.isFinite(n)) return '—';
  const abs = Math.abs(n);
  // Tight formatting: 3 sig figs for large magnitudes, 4 decimals for small.
  const s = abs >= 1000 || abs < 0.01 ? n.toExponential(3) : n.toFixed(4);
  return unit ? `${s} ${unit}` : s;
}

interface HistogramChartProps {
  result: HistogramResult;
  values: number[];
  binCount: number;
  logScale: boolean;
  kdeOverlay: boolean;
  width: number;
  height: number;
  testId?: string;
}

function HistogramChart(props: HistogramChartProps) {
  const { result, values, logScale, kdeOverlay, width, height, testId } = props;
  const { bins } = result;
  if (bins.length === 0) {
    return (
      <div
        data-testid={testId ? `${testId}-empty` : undefined}
        style={{
          width,
          height,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          opacity: 0.5,
          fontStyle: 'italic',
          border: '1px dashed color-mix(in srgb, var(--outline-variant) 30%, transparent)',
          borderRadius: 6,
        }}
      >
        No samples yet
      </div>
    );
  }

  const pad = { top: 8, right: 12, bottom: 22, left: 32 };
  const plotW = Math.max(10, width - pad.left - pad.right);
  const plotH = Math.max(10, height - pad.top - pad.bottom);

  const maxRaw = bins.reduce((m, b) => (b.count > m ? b.count : m), 0);
  // Log-scale uses `1 + count` so zero-count bins stay at the baseline.
  const scaleY = (c: number) => {
    if (maxRaw === 0) return 0;
    if (logScale) {
      const num = Math.log10(1 + c);
      const den = Math.log10(1 + maxRaw) || 1;
      return (num / den) * plotH;
    }
    return (c / maxRaw) * plotH;
  };

  const xMin = bins[0].lower;
  const xMax = bins[bins.length - 1].upper;
  const scaleX = (x: number) => ((x - xMin) / (xMax - xMin || 1)) * plotW;

  // KDE overlay: evaluate the density on a 64-point grid and scale to
  // the tallest histogram bar so it visually tracks.
  let kdePath = '';
  if (kdeOverlay && values.length > 1) {
    const gridN = 64;
    const grid = new Array(gridN);
    for (let i = 0; i < gridN; i++) grid[i] = xMin + (i / (gridN - 1)) * (xMax - xMin);
    const density = kde(values, grid);
    const maxD = density.reduce((m, v) => (v > m ? v : m), 0) || 1;
    const parts: string[] = [];
    for (let i = 0; i < gridN; i++) {
      const sx = scaleX(grid[i]);
      const sy = plotH - (density[i] / maxD) * plotH;
      parts.push(`${i === 0 ? 'M' : 'L'}${sx.toFixed(2)},${sy.toFixed(2)}`);
    }
    kdePath = parts.join(' ');
  }

  const barColor = 'var(--chart-series-2)';
  const kdeColor = 'var(--chart-series-4)';
  const axisColor = 'color-mix(in srgb, var(--outline-variant) 40%, transparent)';

  return (
    <svg
      role="img"
      aria-label="Outcome histogram"
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      data-testid={testId}
    >
      <g transform={`translate(${pad.left},${pad.top})`}>
        {/* Y-axis baseline */}
        <line x1={0} y1={plotH} x2={plotW} y2={plotH} stroke={axisColor} />
        {/* Bars */}
        {bins.map((b, i) => {
          const x = scaleX(b.lower);
          const x2 = scaleX(b.upper);
          const barW = Math.max(1, x2 - x - 1);
          const barH = scaleY(b.count);
          return (
            <rect
              key={i}
              x={x}
              y={plotH - barH}
              width={barW}
              height={barH}
              fill={barColor}
              fillOpacity={0.65}
              data-testid={testId ? `${testId}-bar-${i}` : undefined}
            />
          );
        })}
        {/* KDE overlay */}
        {kdePath && (
          <path
            d={kdePath}
            fill="none"
            stroke={kdeColor}
            strokeWidth={1.5}
            data-testid={testId ? `${testId}-kde` : undefined}
          />
        )}
        {/* X-axis labels (min/max) */}
        <text x={0} y={plotH + 14} fontSize={10} fill="currentColor" opacity={0.7}>
          {fmtStat(xMin)}
        </text>
        <text x={plotW} y={plotH + 14} fontSize={10} fill="currentColor" opacity={0.7} textAnchor="end">
          {fmtStat(xMax)}
        </text>
        {/* Y-axis max label */}
        <text x={-4} y={10} fontSize={10} fill="currentColor" opacity={0.7} textAnchor="end">
          {logScale ? `log(${maxRaw})` : String(maxRaw)}
        </text>
      </g>
    </svg>
  );
}

interface HistogramPanelProps {
  outcome: MonteCarloOutcome;
  children: ChildDescriptor[];
  binCount: number;
  logScale: boolean;
  kdeOverlay: boolean;
}

function HistogramPanel({
  outcome,
  children,
  binCount,
  logScale,
  kdeOverlay,
}: HistogramPanelProps) {
  const values = useMemo(() => {
    const out: number[] = [];
    for (const c of children) {
      const v = outcome.extract(c);
      if (v == null || !Number.isFinite(v)) continue;
      out.push(v as number);
    }
    return out;
  }, [children, outcome]);

  const result = useMemo(() => buildHistogram(values, binCount), [values, binCount]);
  const label = outcome.label ?? outcome.id;

  return (
    <section style={PANEL_STYLE} data-testid={`mc-histogram-panel-${outcome.id}`}>
      <header style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline' }}>
        <h4 style={{ margin: 0, fontSize: 13, fontWeight: 600 }}>{label}</h4>
        <span style={{ fontSize: 11, opacity: 0.7 }}>
          N = {values.length}
        </span>
      </header>
      <div style={STATS_ROW_STYLE} data-testid={`mc-histogram-stats-${outcome.id}`}>
        <span>mean {fmtStat(result.stats.mean, outcome.unit)}</span>
        <span>σ {fmtStat(result.stats.sigma, outcome.unit)}</span>
        <span>p5 {fmtStat(result.stats.p5, outcome.unit)}</span>
        <span>p50 {fmtStat(result.stats.p50, outcome.unit)}</span>
        <span>p95 {fmtStat(result.stats.p95, outcome.unit)}</span>
      </div>
      <HistogramChart
        result={result}
        values={values}
        binCount={binCount}
        logScale={logScale}
        kdeOverlay={kdeOverlay}
        width={480}
        height={160}
        testId={`mc-histogram-chart-${outcome.id}`}
      />
    </section>
  );
}

export function MonteCarloHistogramViewer(props: MonteCarloHistogramViewerProps) {
  const { children, outcomes, defaultBinCount = 20, className, testId } = props;
  const id = useId();
  const [binCount, setBinCount] = useState(() =>
    Math.max(BIN_MIN, Math.min(BIN_MAX, defaultBinCount)),
  );
  const [logScale, setLogScale] = useState(false);
  const [kdeOverlay, setKdeOverlay] = useState(false);

  if (outcomes.length === 0) {
    return (
      <div
        className={className}
        data-testid={testId ?? 'mc-histogram-empty'}
        role="status"
        style={{
          padding: 24,
          border: '1px dashed color-mix(in srgb, var(--outline-variant) 35%, transparent)',
          borderRadius: 8,
          textAlign: 'center',
          fontStyle: 'italic',
          opacity: 0.7,
          color: 'var(--on-surface)',
          fontSize: 12,
        }}
      >
        No outcome metrics configured.
      </div>
    );
  }

  return (
    <div className={className} style={ROOT_STYLE} data-testid={testId ?? 'mc-histogram-viewer'}>
      <div style={CONTROL_BAR_STYLE} role="group" aria-label="Histogram settings">
        <label style={{ display: 'flex', alignItems: 'center', gap: 8 }} htmlFor={`${id}-bins`}>
          <span style={{ fontSize: 11, opacity: 0.75 }}>Bins</span>
          <input
            id={`${id}-bins`}
            data-testid="mc-histogram-bin-slider"
            type="range"
            min={BIN_MIN}
            max={BIN_MAX}
            step={1}
            value={binCount}
            onChange={(e) => setBinCount(Number(e.target.value))}
            style={{ width: 120 }}
            aria-valuemin={BIN_MIN}
            aria-valuemax={BIN_MAX}
            aria-valuenow={binCount}
          />
          <span style={{ fontVariantNumeric: 'tabular-nums', minWidth: 24, textAlign: 'right' }}>
            {binCount}
          </span>
        </label>
        <label style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <input
            type="checkbox"
            data-testid="mc-histogram-log-toggle"
            checked={logScale}
            onChange={(e) => setLogScale(e.target.checked)}
          />
          <span>Log scale</span>
        </label>
        <label style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <input
            type="checkbox"
            data-testid="mc-histogram-kde-toggle"
            checked={kdeOverlay}
            onChange={(e) => setKdeOverlay(e.target.checked)}
          />
          <span>KDE overlay</span>
        </label>
      </div>
      {outcomes.map((o) => (
        <HistogramPanel
          key={o.id}
          outcome={o}
          children={children}
          binCount={binCount}
          logScale={logScale}
          kdeOverlay={kdeOverlay}
        />
      ))}
    </div>
  );
}

/**
 * Single-outcome convenience wrapper. Task spec shape:
 *   `{ children, outcomeExtractor, binCount? }`.
 * Produces one histogram with the supplied extractor.
 */
export function MonteCarloHistogramViewerSingle(
  props: MonteCarloHistogramViewerSingleProps,
) {
  const { children, outcomeExtractor, label = 'outcome', unit, binCount, className, testId } = props;
  return (
    <MonteCarloHistogramViewer
      className={className}
      testId={testId}
      children={children}
      defaultBinCount={binCount}
      outcomes={[{ id: 'outcome', label, unit, extract: outcomeExtractor }]}
    />
  );
}
