/**
 * QQPlot — small Q-Q scatter rendered inline as SVG.
 *
 * Used by `StatsOverlay` to surface the goodness-of-fit visually. Pure
 * render — the numeric work happens in `statsHelpers.qqPoints`. Kept
 * tiny (no axes overlay gymnastics, no chart library) because the plot
 * is an accompaniment, not the headline.
 *
 * `prefers-reduced-motion` is honoured: we never animate the stroke or
 * points. The only visual transition in the stats feature family is
 * the opacity fade on the distribution chip, itself disabled when the
 * user has reduced-motion set.
 */

import { useMemo } from 'react';
import type { CSSProperties } from 'react';
import { qqPoints, normalQuantile, fitDistribution } from './statsHelpers';

export interface QQPlotProps {
  /** Sample values to plot. */
  values: number[];
  /**
   * Reference distribution's inverse CDF. Defaults to the standard
   * normal quantile; `StatsOverlay` can pass the fitted distribution's
   * inverse-CDF to test the fit visually.
   */
  inverseCdf?: (p: number) => number;
  /** Canvas size. Default 160×120 — sized to tuck beside the stats row. */
  width?: number;
  height?: number;
  /** Label shown as the SVG aria-label. */
  label?: string;
  /** Test id passthrough for the RTL layer. */
  testId?: string;
}

const POINT_COLOR = 'var(--chart-series-2)';
const REFERENCE_COLOR = 'var(--chart-label)';
const AXIS_COLOR = 'var(--chart-axis)';

const ROOT_STYLE: CSSProperties = {
  display: 'inline-block',
  color: 'var(--text-primary)',
};

/**
 * Build an inverse CDF that standardises against the fitted distribution.
 * When `inverseCdf` is omitted we fall back to the fitted family: normal
 * uses z-quantile, lognormal uses exp(normalQuantile · σ + μ), uniform
 * uses a linear span.
 */
function defaultInverseCdf(values: number[]): (p: number) => number {
  const fit = fitDistribution(values);
  if (fit.family === 'lognormal') {
    const mu = fit.params.mu ?? 0;
    const sigma = fit.params.sigma ?? 1;
    return (p) => Math.exp(mu + sigma * normalQuantile(p));
  }
  if (fit.family === 'uniform') {
    const lo = fit.params.min ?? 0;
    const hi = fit.params.max ?? 1;
    return (p) => lo + p * (hi - lo);
  }
  // normal / unknown → standard normal reference (post-standardisation)
  const mu = fit.params.mu ?? 0;
  const sigma = fit.params.sigma ?? 1;
  return (p) => mu + sigma * normalQuantile(p);
}

export function QQPlot(props: QQPlotProps) {
  const { values, inverseCdf, width = 160, height = 120, label = 'Q-Q plot', testId } = props;

  const points = useMemo(() => {
    const invCdf = inverseCdf ?? defaultInverseCdf(values);
    return qqPoints(values, invCdf);
  }, [values, inverseCdf]);

  if (points.length === 0) {
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
          fontSize: 11,
          border: '1px dashed color-mix(in srgb, var(--border-default) 30%, transparent)',
          borderRadius: 6,
        }}
      >
        No data
      </div>
    );
  }

  const pad = { top: 6, right: 8, bottom: 18, left: 28 };
  const plotW = Math.max(10, width - pad.left - pad.right);
  const plotH = Math.max(10, height - pad.top - pad.bottom);

  // Axis ranges — include both observed and expected so the reference
  // line y = x lands diagonally.
  let minVal = Number.POSITIVE_INFINITY;
  let maxVal = Number.NEGATIVE_INFINITY;
  for (const p of points) {
    if (Number.isFinite(p.observed)) {
      if (p.observed < minVal) minVal = p.observed;
      if (p.observed > maxVal) maxVal = p.observed;
    }
    if (Number.isFinite(p.expected)) {
      if (p.expected < minVal) minVal = p.expected;
      if (p.expected > maxVal) maxVal = p.expected;
    }
  }
  if (!Number.isFinite(minVal) || !Number.isFinite(maxVal) || minVal === maxVal) {
    minVal = (Number.isFinite(minVal) ? minVal : 0) - 1;
    maxVal = (Number.isFinite(maxVal) ? maxVal : 0) + 1;
  }
  const span = maxVal - minVal;
  const scaleX = (x: number) => ((x - minVal) / span) * plotW;
  const scaleY = (y: number) => plotH - ((y - minVal) / span) * plotH;

  return (
    <div style={ROOT_STYLE}>
      <svg
        role="img"
        aria-label={label}
        width={width}
        height={height}
        viewBox={`0 0 ${width} ${height}`}
        data-testid={testId}
      >
        <g transform={`translate(${pad.left},${pad.top})`}>
          {/* Axes */}
          <line x1={0} y1={plotH} x2={plotW} y2={plotH} stroke={AXIS_COLOR} />
          <line x1={0} y1={0} x2={0} y2={plotH} stroke={AXIS_COLOR} />
          {/* Reference y = x line */}
          <line
            x1={scaleX(minVal)}
            y1={scaleY(minVal)}
            x2={scaleX(maxVal)}
            y2={scaleY(maxVal)}
            stroke={REFERENCE_COLOR}
            strokeDasharray="3,3"
            strokeWidth={1}
            data-testid={testId ? `${testId}-ref` : undefined}
          />
          {/* Scatter */}
          {points.map((p, i) => {
            if (!Number.isFinite(p.observed) || !Number.isFinite(p.expected)) return null;
            return (
              <circle
                key={i}
                cx={scaleX(p.expected)}
                cy={scaleY(p.observed)}
                r={2}
                fill={POINT_COLOR}
                fillOpacity={0.75}
                data-testid={testId ? `${testId}-pt-${i}` : undefined}
              />
            );
          })}
          {/* Axis labels */}
          <text x={0} y={plotH + 12} fontSize={9} fill="currentColor" opacity={0.6}>
            {minVal.toPrecision(3)}
          </text>
          <text x={plotW} y={plotH + 12} fontSize={9} fill="currentColor" opacity={0.6} textAnchor="end">
            {maxVal.toPrecision(3)}
          </text>
          <text x={-4} y={plotH} fontSize={9} fill="currentColor" opacity={0.6} textAnchor="end">
            obs
          </text>
          <text x={-4} y={10} fontSize={9} fill="currentColor" opacity={0.6} textAnchor="end">
            obs
          </text>
          <text x={plotW / 2} y={plotH + 12} fontSize={9} fill="currentColor" opacity={0.6} textAnchor="middle">
            expected
          </text>
        </g>
      </svg>
    </div>
  );
}
