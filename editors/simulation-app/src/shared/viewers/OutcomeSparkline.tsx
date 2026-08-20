/**
 * OutcomeSparkline — the shape behind a sweep outcome's number.
 *
 * A sweep reports where each run ended. That is only meaningful if the run
 * got somewhere: `examples/radiation-cooling` reported 990 K across every
 * point of a five-way study, which looks like a converged answer and was
 * actually five runs that had barely started. A number cannot distinguish
 * those; a curve can, at a glance, across every row at once.
 *
 * Deliberately inline rather than hover-only. The diagnostic value is in
 * scanning a whole column and seeing that nothing bent — which a reader only
 * gets if the shapes are on screen together.
 */
import type { CSSProperties } from 'react';

export interface OutcomeSparklineProps {
  /** Decimated `[time_ms, value]` points, oldest first. */
  series: readonly (readonly [number, number])[];
  width?: number;
  height?: number;
  /** Accessible description; also the hover title. */
  title?: string;
  testId?: string;
}

/**
 * Render the trace as a polyline normalised into the box.
 *
 * A flat series (every value equal) draws a centre line rather than dividing
 * by a zero range — "it never moved" is a real result and must render as one,
 * not as an empty cell.
 */
export function OutcomeSparkline({
  series,
  width = 64,
  height = 16,
  title,
  testId,
}: OutcomeSparklineProps) {
  const points = series.filter(
    ([t, v]) => Number.isFinite(t) && Number.isFinite(v),
  );
  // One point is not a shape. Render nothing rather than a misleading dot.
  if (points.length < 2) return null;

  const times = points.map(([t]) => t);
  const values = points.map(([, v]) => v);
  const tMin = Math.min(...times);
  const tMax = Math.max(...times);
  const vMin = Math.min(...values);
  const vMax = Math.max(...values);
  const tSpan = tMax - tMin || 1;
  const vSpan = vMax - vMin;

  // Inset by half a stroke so the extremes are not clipped by the viewBox.
  const pad = 1;
  const x = (t: number) => pad + ((t - tMin) / tSpan) * (width - 2 * pad);
  const y = (v: number) =>
    vSpan === 0
      ? height / 2
      : height - pad - ((v - vMin) / vSpan) * (height - 2 * pad);

  const d = points.map(([t, v]) => `${x(t).toFixed(2)},${y(v).toFixed(2)}`).join(' ');
  const [lastT, lastV] = points[points.length - 1];

  return (
    <svg
      data-testid={testId}
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={title ?? 'outcome over time'}
      style={svgStyle}
      preserveAspectRatio="none"
    >
      {title ? <title>{title}</title> : null}
      <polyline
        points={d}
        fill="none"
        stroke="currentColor"
        strokeWidth={1}
        strokeLinejoin="round"
        strokeLinecap="round"
        opacity={0.75}
      />
      {/* Mark where the run ended — the value the table's number reports. */}
      <circle cx={x(lastT)} cy={y(lastV)} r={1.5} fill="currentColor" />
    </svg>
  );
}

const svgStyle: CSSProperties = {
  display: 'block',
  overflow: 'visible',
  color: 'var(--accent, currentColor)',
};

/**
 * Hover text for a trace: where it ran from and to, in model time.
 * Says explicitly when a value did not move, because that is the finding.
 */
export function describeSeries(
  series: readonly (readonly [number, number])[],
  unit?: string,
): string {
  const points = series.filter(([t, v]) => Number.isFinite(t) && Number.isFinite(v));
  if (points.length < 2) return 'no trace retained for this run';
  const [t0, v0] = points[0];
  const [t1, v1] = points[points.length - 1];
  const u = unit ? ` ${unit}` : '';
  const span = formatSeconds(t1 - t0);
  if (v0 === v1) return `flat at ${fmt(v0)}${u} across ${span} of model time`;
  return `${fmt(v0)}${u} → ${fmt(v1)}${u} across ${span} of model time`;
}

function fmt(v: number): string {
  return Number(v.toPrecision(6)).toString();
}

function formatSeconds(ms: number): string {
  const s = ms / 1000;
  if (s < 90) return `${Number(s.toPrecision(4))} s`;
  const m = s / 60;
  if (m < 90) return `${Number(m.toPrecision(4))} min`;
  return `${Number((m / 60).toPrecision(4))} h`;
}
