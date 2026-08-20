/**
 * SobolBarChart — grouped bar chart of first-order S_i and total-order
 * S_Ti indices (R7.4).
 *
 * Each parameter gets a pair of bars side-by-side:
 *
 *   ▉▉▉ (blue)  — S_i  (first-order, main effect)
 *   ▉▉▉▉ (amber)— S_Ti (total-order, main + interactions)
 *
 * Bar height = variance contribution ∈ [0, 1]. Parameters with large
 * S_Ti but small S_i have strong interaction effects (classic Sobol
 * reading). The chart rounds to two decimals and labels each bar so
 * the user can read exact values without hovering.
 */

import { useMemo } from 'react';
import type { SensitivityResult } from '@/engine/types';

export interface SobolBarChartProps {
  results: SensitivityResult[];
  width?: number;
  height?: number;
}

export function SobolBarChart({
  results,
  width = 560,
  height = 360,
}: SobolBarChartProps) {
  const layout = useMemo(() => buildLayout(results, width, height), [
    results,
    width,
    height,
  ]);

  if (results.length === 0) {
    return (
      <div
        data-testid="sobol-bar-empty"
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
        No Sobol results yet.
      </div>
    );
  }

  return (
    <svg
      data-testid="sobol-bar-chart"
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label="Sobol S_i vs S_Ti bar chart"
    >
      {/* Axis line */}
      <line
        x1={layout.pad.left}
        x2={width - layout.pad.right}
        y1={height - layout.pad.bottom}
        y2={height - layout.pad.bottom}
        stroke="var(--outline-variant)"
        strokeWidth={1}
      />
      <line
        x1={layout.pad.left}
        x2={layout.pad.left}
        y1={layout.pad.top}
        y2={height - layout.pad.bottom}
        stroke="var(--outline-variant)"
        strokeWidth={1}
      />
      {/* y-axis label */}
      <text
        x={12}
        y={(height - layout.pad.top - layout.pad.bottom) / 2 + layout.pad.top}
        textAnchor="middle"
        fill="var(--outline)"
        fontSize={11}
        transform={`rotate(-90 12 ${
          (height - layout.pad.top - layout.pad.bottom) / 2 + layout.pad.top
        })`}
      >
        Variance contribution
      </text>
      {/* y axis ticks (0 / yMax) */}
      <text
        x={layout.pad.left - 6}
        y={height - layout.pad.bottom}
        fill="var(--outline)"
        fontSize={10}
        textAnchor="end"
      >
        0
      </text>
      <text
        x={layout.pad.left - 6}
        y={layout.pad.top + 4}
        fill="var(--outline)"
        fontSize={10}
        textAnchor="end"
      >
        {layout.yMax.toFixed(2)}
      </text>

      {/* Bars */}
      {layout.groups.map((group) => (
        <g
          key={group.name}
          data-testid={`sobol-bar-group-${group.name}`}
          transform={`translate(${group.x} 0)`}
        >
          {/* S_i (first-order) */}
          <rect
            data-testid={`sobol-bar-s1-${group.name}`}
            x={0}
            y={group.s1Top}
            width={layout.barWidth}
            height={Math.max(0, height - layout.pad.bottom - group.s1Top)}
            fill="var(--primary)"
          />
          <text
            x={layout.barWidth / 2}
            y={group.s1Top - 3}
            fill="var(--on-surface)"
            fontSize={9}
            textAnchor="middle"
          >
            {group.s1.toFixed(2)}
          </text>

          {/* S_Ti (total-order) */}
          <rect
            data-testid={`sobol-bar-st-${group.name}`}
            x={layout.barWidth + 4}
            y={group.stTop}
            width={layout.barWidth}
            height={Math.max(0, height - layout.pad.bottom - group.stTop)}
            fill="var(--chart-series-2)"
          />
          <text
            x={layout.barWidth + 4 + layout.barWidth / 2}
            y={group.stTop - 3}
            fill="var(--on-surface)"
            fontSize={9}
            textAnchor="middle"
          >
            {group.st.toFixed(2)}
          </text>

          {/* x-axis label */}
          <text
            x={layout.barWidth + 2}
            y={height - layout.pad.bottom + 14}
            fill="var(--outline)"
            fontSize={11}
            textAnchor="middle"
          >
            {group.name}
          </text>
        </g>
      ))}

      {/* Legend */}
      <g transform={`translate(${width - layout.pad.right - 140} ${layout.pad.top - 4})`}>
        <rect width={10} height={10} fill="var(--primary)" />
        <text x={14} y={9} fill="var(--on-surface)" fontSize={11}>
          S_i
        </text>
        <rect x={50} width={10} height={10} fill="var(--chart-series-2)" />
        <text x={64} y={9} fill="var(--on-surface)" fontSize={11}>
          S_Ti
        </text>
      </g>
    </svg>
  );
}

function buildLayout(
  results: SensitivityResult[],
  width: number,
  height: number,
) {
  const pad = { top: 22, right: 20, bottom: 36, left: 48 };
  const innerW = Math.max(1, width - pad.left - pad.right);
  const innerH = Math.max(1, height - pad.top - pad.bottom);

  let yMax = 0;
  for (const r of results) {
    if (typeof r.s1 === 'number' && Number.isFinite(r.s1)) {
      yMax = Math.max(yMax, r.s1);
    }
    if (typeof r.st === 'number' && Number.isFinite(r.st)) {
      yMax = Math.max(yMax, r.st);
    }
  }
  yMax = yMax > 0 ? Math.max(1, yMax * 1.1) : 1;

  const groupCount = results.length;
  const groupWidth = innerW / Math.max(1, groupCount);
  const barWidth = Math.max(8, (groupWidth - 12) / 2);

  const groups = results.map((r, i) => {
    const s1 = typeof r.s1 === 'number' && Number.isFinite(r.s1) ? r.s1 : 0;
    const st = typeof r.st === 'number' && Number.isFinite(r.st) ? r.st : 0;
    return {
      name: r.name,
      s1,
      st,
      x: pad.left + i * groupWidth + 6,
      s1Top: height - pad.bottom - (s1 / yMax) * innerH,
      stTop: height - pad.bottom - (st / yMax) * innerH,
    };
  });

  return { pad, yMax, groups, barWidth };
}
