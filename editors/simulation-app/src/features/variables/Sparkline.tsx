/**
 * Sparkline — tiny inline SVG trend glyph for the Variables pane (R2.2).
 *
 * Renders the last N numeric samples of a time-series as a 60x16 polyline.
 * Pure, stateless — no observers, no react-query, no store access. The
 * parent decides when to mount (e.g. via IntersectionObserver) and which
 * points to hand in.
 *
 * We intentionally ship a purpose-built component (rather than reusing
 * @/components/charts/Sparkline) so the path-generation logic can be
 * exported and unit-tested as a pure function, and so the pane can style
 * a min/max baseline specific to the Cameo-parity layout without leaking
 * that into other callers.
 */

import type { CSSProperties } from 'react';

export interface SparklineProps {
  /** Last N sample values. Order: oldest first, newest last. */
  samples: number[];
  /** Pixel width. Default 60 (matches R2.2 brief). */
  width?: number;
  /** Pixel height. Default 16 (matches R2.2 brief). */
  height?: number;
  /** Stroke color; defaults to currentColor so callers theme via CSS. */
  color?: string;
  /** Stroke width. Default 1.25. */
  strokeWidth?: number;
  /** Optional extra className (e.g. for reduced-motion tweaks). */
  className?: string;
  /** Optional inline style overrides. */
  style?: CSSProperties;
  /** Accessible label; falls back to "sparkline". */
  ariaLabel?: string;
}

/**
 * Pure path generator — exported for unit tests and for consumers that
 * want to embed the same shape in a larger SVG.
 */
export function buildSparklinePath(
  samples: readonly number[],
  width: number,
  height: number,
  padding = 1,
): string {
  if (samples.length < 2) return '';
  // Treat NaN / Infinity as missing by clamping to the series min.
  const cleaned = samples.map((v) => (Number.isFinite(v) ? v : 0));
  const min = Math.min(...cleaned);
  const max = Math.max(...cleaned);
  const range = max - min || 1;
  const last = cleaned.length - 1;
  const innerW = Math.max(0, width - padding * 2);
  const innerH = Math.max(0, height - padding * 2);
  const parts: string[] = [];
  for (let i = 0; i < cleaned.length; i++) {
    const x = padding + (i / last) * innerW;
    const y = padding + innerH - ((cleaned[i] - min) / range) * innerH;
    parts.push(`${x.toFixed(2)},${y.toFixed(2)}`);
  }
  return parts.join(' ');
}

/**
 * Minimum samples required before we show the glyph at all. Two points
 * collapse to a flat line which adds visual noise for no information.
 */
export const MIN_SPARKLINE_SAMPLES = 3;

export function Sparkline({
  samples,
  width = 60,
  height = 16,
  color = 'currentColor',
  strokeWidth = 1.25,
  className,
  style,
  ariaLabel = 'sparkline',
}: SparklineProps) {
  if (samples.length < MIN_SPARKLINE_SAMPLES) return null;
  const points = buildSparklinePath(samples, width, height);
  if (!points) return null;
  return (
    <svg
      width={width}
      height={height}
      viewBox={`0 0 ${width} ${height}`}
      role="img"
      aria-label={ariaLabel}
      className={className}
      style={{ verticalAlign: 'middle', flexShrink: 0, ...style }}
    >
      <polyline
        points={points}
        fill="none"
        stroke={color}
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
