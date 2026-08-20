/**
 * MorrisScatter — μ*-vs-σ scatter plot for Morris Elementary Effects
 * (R7.4).
 *
 * Each parameter becomes one dot whose x = μ* (mean absolute EE =
 * importance) and y = σ (stddev of EE = nonlinearity / interaction
 * indicator). The "important nonlinear" parameters live in the upper
 * right — that's the standard Morris interpretation (Campolongo 2007).
 *
 * Pure SVG so the component trees cheaply in vitest + requires zero
 * charting deps. Axes auto-scale to the data; dots carry a small
 * readable label the user can correlate back to the config panel.
 */

import { useMemo } from 'react';
import type { SensitivityResult } from '@/engine/types';

export interface MorrisScatterProps {
  /** Per-parameter Morris results. Must have `mu` + `sigma` populated. */
  results: SensitivityResult[];
  /** Total viewport width — defaults to 560. */
  width?: number;
  /** Total viewport height — defaults to 360. */
  height?: number;
}

export function MorrisScatter({
  results,
  width = 560,
  height = 360,
}: MorrisScatterProps) {
  const plot = useMemo(() => buildLayout(results, width, height), [
    results,
    width,
    height,
  ]);

  if (results.length === 0) {
    return (
      <div
        data-testid="morris-scatter-empty"
        style={{
          width,
          height,
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          color: 'var(--outline)',
          fontSize: 12,
        }}
      >
        No Morris results yet.
      </div>
    );
  }

  return (
    <svg
      data-testid="morris-scatter"
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label="Morris μ-vs-σ scatter plot"
    >
      {/* Axes */}
      <line
        x1={plot.pad.left}
        x2={width - plot.pad.right}
        y1={height - plot.pad.bottom}
        y2={height - plot.pad.bottom}
        stroke="var(--outline-variant)"
        strokeWidth={1}
      />
      <line
        x1={plot.pad.left}
        x2={plot.pad.left}
        y1={plot.pad.top}
        y2={height - plot.pad.bottom}
        stroke="var(--outline-variant)"
        strokeWidth={1}
      />
      {/* Axis labels */}
      <text
        x={(width - plot.pad.left - plot.pad.right) / 2 + plot.pad.left}
        y={height - 8}
        textAnchor="middle"
        fill="var(--outline)"
        fontSize={11}
      >
        μ* (importance)
      </text>
      <text
        x={12}
        y={(height - plot.pad.top - plot.pad.bottom) / 2 + plot.pad.top}
        textAnchor="middle"
        fill="var(--outline)"
        fontSize={11}
        transform={`rotate(-90 12 ${
          (height - plot.pad.top - plot.pad.bottom) / 2 + plot.pad.top
        })`}
      >
        σ (nonlinearity / interactions)
      </text>
      {/* x tick endpoints */}
      <text
        x={plot.pad.left}
        y={height - plot.pad.bottom + 12}
        fill="var(--outline)"
        fontSize={10}
        textAnchor="start"
      >
        {formatNumber(plot.xMin)}
      </text>
      <text
        x={width - plot.pad.right}
        y={height - plot.pad.bottom + 12}
        fill="var(--outline)"
        fontSize={10}
        textAnchor="end"
      >
        {formatNumber(plot.xMax)}
      </text>
      {/* y tick endpoints */}
      <text
        x={plot.pad.left - 6}
        y={height - plot.pad.bottom}
        fill="var(--outline)"
        fontSize={10}
        textAnchor="end"
      >
        {formatNumber(plot.yMin)}
      </text>
      <text
        x={plot.pad.left - 6}
        y={plot.pad.top + 4}
        fill="var(--outline)"
        fontSize={10}
        textAnchor="end"
      >
        {formatNumber(plot.yMax)}
      </text>

      {/* Dots */}
      {plot.points.map((p) => (
        <g
          key={p.name}
          data-testid={`morris-scatter-point-${p.name}`}
          transform={`translate(${p.cx} ${p.cy})`}
        >
          <circle
            r={5}
            fill="var(--primary)"
            stroke="var(--on-primary)"
            strokeWidth={1}
          />
          <text
            x={8}
            y={4}
            fill="var(--on-surface)"
            fontSize={11}
            style={{ pointerEvents: 'none' }}
          >
            {p.name}
          </text>
        </g>
      ))}
    </svg>
  );
}

function buildLayout(
  results: SensitivityResult[],
  width: number,
  height: number,
) {
  const pad = { top: 18, right: 20, bottom: 36, left: 48 };
  const innerW = Math.max(1, width - pad.left - pad.right);
  const innerH = Math.max(1, height - pad.top - pad.bottom);

  let xMax = 0;
  let yMax = 0;
  for (const r of results) {
    if (typeof r.mu === 'number' && Number.isFinite(r.mu)) {
      xMax = Math.max(xMax, r.mu);
    }
    if (typeof r.sigma === 'number' && Number.isFinite(r.sigma)) {
      yMax = Math.max(yMax, r.sigma);
    }
  }
  // Pad the ranges slightly so dots on the extremes aren't against the
  // axis line.
  const xMin = 0;
  const yMin = 0;
  xMax = xMax > 0 ? xMax * 1.1 : 1;
  yMax = yMax > 0 ? yMax * 1.1 : 1;

  const points = results
    .filter(
      (r) =>
        typeof r.mu === 'number' &&
        Number.isFinite(r.mu) &&
        typeof r.sigma === 'number' &&
        Number.isFinite(r.sigma),
    )
    .map((r) => {
      const cx = pad.left + (r.mu! / xMax) * innerW;
      const cy = height - pad.bottom - (r.sigma! / yMax) * innerH;
      return { name: r.name, cx, cy };
    });

  return { pad, xMin, xMax, yMin, yMax, points };
}

function formatNumber(n: number): string {
  if (!Number.isFinite(n)) return '—';
  const abs = Math.abs(n);
  if (abs === 0) return '0';
  if (abs < 0.01 || abs >= 1000) return n.toExponential(2);
  return n.toFixed(abs < 1 ? 3 : 2);
}
