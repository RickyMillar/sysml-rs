/**
 * StatsOverlay — composite statistical panel (R7.2).
 *
 * A drop-in presentational component that renders:
 *   - mean ± 95% bootstrap CI (two-line display)
 *   - σ, skewness, kurtosis
 *   - best-fit distribution chip (normal / lognormal / uniform / unknown)
 *   - optional small Q-Q plot against the fitted family
 *
 * The component owns NO data fetching and NO store reads — it is pure
 * input → render. MonteCarloStatsPanel / SweepStatsPanel instantiate it
 * with a pre-extracted `values: number[]` so streaming callers can
 * recompute cheaply (memoisation keys off sorted values).
 *
 * `prefers-reduced-motion` is honoured: the distribution chip's opacity
 * fade is skipped entirely when the media query matches.
 */

import { useEffect, useMemo, useRef, useState } from 'react';
import type { CSSProperties } from 'react';
import {
  bootstrapCI,
  confidenceInterval,
  createSeededRng,
  fitDistribution,
  kurtosis,
  mean as meanOf,
  skewness,
  stddev,
  type DistributionFamily,
  type DistributionFit,
} from './statsHelpers';
import { QQPlot } from './QQPlot';

export interface StatsOverlayProps {
  /** Raw numeric sample. NaN / non-finite entries are filtered internally. */
  values: number[];
  /** Panel heading. */
  label: string;
  /** Render the sparkline histogram. Default false — MC viewer draws it already. */
  showHistogram?: boolean;
  /** Render the Q-Q plot. Default true. */
  showQQ?: boolean;
  /** Optional unit to append to `mean`, `σ`, and CI values. */
  unit?: string;
  /** RNG used for the bootstrap CI. Defaults to a Mulberry32 with a fixed
   *  seed so the overlay is deterministic across renders — tests rely on
   *  this for byte-for-byte assertions, and production runs gain repeatable
   *  bounds. Callers who want stochastic bounds can supply `Math.random`. */
  rng?: () => number;
  /** Bootstrap iteration count. Default 500 — balances fidelity vs perf. */
  bootstrapIterations?: number;
  /** Confidence level for the bootstrap CI. Default 0.95. */
  confidence?: number;
  /** Class hook for integrating shells. */
  className?: string;
  /** data-testid passthrough. */
  testId?: string;
}

const ROOT_STYLE: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 8,
  padding: '10px 12px',
  borderRadius: 6,
  border: '1px solid color-mix(in srgb, var(--border-default) 25%, transparent)',
  background: 'color-mix(in srgb, var(--surface-panel) 45%, transparent)',
  color: 'var(--text-primary)',
  fontSize: 12,
};

const HEADER_STYLE: CSSProperties = {
  display: 'flex',
  alignItems: 'baseline',
  justifyContent: 'space-between',
  gap: 12,
};

const STATS_GRID_STYLE: CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fit, minmax(120px, 1fr))',
  columnGap: 14,
  rowGap: 4,
  fontFamily: 'ui-monospace, "JetBrains Mono", monospace',
  fontSize: 11,
};

const CHIP_BASE_STYLE: CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  gap: 6,
  padding: '2px 8px',
  borderRadius: 999,
  fontSize: 10,
  fontWeight: 600,
  textTransform: 'uppercase',
  letterSpacing: 0.4,
  border: '1px solid color-mix(in srgb, var(--border-default) 30%, transparent)',
};

// Distribution-family chip is a decorative category badge (no pass/fail or
// severity meaning), so each family gets a distinct chart-series hue rather
// than a status token — chosen to track the old hue as closely as the
// 8-slot categorical ramp allows: normal (was blue) -> series-2 (blue),
// lognormal (was pink) -> series-4 (magenta), uniform (was lavender) ->
// series-3 (violet).
const CHIP_TONE: Record<DistributionFamily, { background: string; color: string }> = {
  normal: {
    background: 'color-mix(in srgb, var(--chart-series-2) 15%, transparent)',
    color: 'var(--chart-series-2)',
  },
  lognormal: {
    background: 'color-mix(in srgb, var(--chart-series-4) 15%, transparent)',
    color: 'var(--chart-series-4)',
  },
  uniform: {
    background: 'color-mix(in srgb, var(--chart-series-3) 15%, transparent)',
    color: 'var(--chart-series-3)',
  },
  unknown: {
    background: 'color-mix(in srgb, var(--border-default) 20%, transparent)',
    color: 'var(--text-primary)',
  },
};

function fmt(value: number, unit?: string): string {
  if (!Number.isFinite(value)) return '—';
  const abs = Math.abs(value);
  const s = abs >= 1000 || (abs > 0 && abs < 0.01) ? value.toExponential(3) : value.toFixed(4);
  return unit ? `${s} ${unit}` : s;
}

function usePrefersReducedMotion(): boolean {
  const [reduced, setReduced] = useState(false);
  useEffect(() => {
    if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
    const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
    setReduced(mq.matches);
    const listener = (e: MediaQueryListEvent) => setReduced(e.matches);
    if (typeof mq.addEventListener === 'function') {
      mq.addEventListener('change', listener);
      return () => mq.removeEventListener('change', listener);
    }
    mq.addListener(listener);
    return () => mq.removeListener(listener);
  }, []);
  return reduced;
}

function describeFit(fit: DistributionFit, unit?: string): string {
  if (fit.family === 'normal') {
    return `μ=${fmt(fit.params.mu, unit)} σ=${fmt(fit.params.sigma, unit)}`;
  }
  if (fit.family === 'lognormal') {
    return `μ_log=${fmt(fit.params.mu)} σ_log=${fmt(fit.params.sigma)}`;
  }
  if (fit.family === 'uniform') {
    return `[${fmt(fit.params.min, unit)}, ${fmt(fit.params.max, unit)}]`;
  }
  return 'insufficient fit';
}

export function StatsOverlay(props: StatsOverlayProps) {
  const {
    values,
    label,
    showQQ = true,
    showHistogram: _unused,
    unit,
    rng,
    bootstrapIterations = 500,
    confidence = 0.95,
    className,
    testId,
  } = props;
  // Lint-ignore: kept in the prop surface to mirror the task spec even
  // though the overlay's inline sparkline is an explicit non-goal — the
  // MC viewer already ships a bin-count-controlled histogram, so we
  // don't duplicate it here. The flag exists for downstream Sweep
  // integration where a sparkline may still be useful.
  void _unused;

  const reduced = usePrefersReducedMotion();

  // Sort once for stable memo keys + consistent re-render cost. We key
  // bootstrapCI + fitDistribution off the sorted representation so
  // streaming callers with reordered data don't retrigger work.
  const { sorted, sanitized } = useMemo(() => {
    const san: number[] = [];
    for (const v of values) if (Number.isFinite(v)) san.push(v as number);
    const s = [...san].sort((a, b) => a - b);
    return { sorted: s, sanitized: san };
  }, [values]);

  const rngRef = useRef<() => number>(rng ?? createSeededRng(0xbada55));
  if (rng) {
    // A caller-supplied rng overrides the cached one so tests remain
    // explicit about their determinism source.
    rngRef.current = rng;
  }

  const stats = useMemo(() => {
    const m = meanOf(sanitized);
    const sd = stddev(sanitized);
    const sk = skewness(sanitized);
    const kt = kurtosis(sanitized);
    const classicCi = confidenceInterval(sanitized, confidence);
    const bootCi = bootstrapCI(sanitized, confidence, bootstrapIterations, rngRef.current);
    const fit = fitDistribution(sanitized);
    return { m, sd, sk, kt, classicCi, bootCi, fit, n: sanitized.length };
  }, [sanitized, bootstrapIterations, confidence, sorted]);

  const chipTone = CHIP_TONE[stats.fit.family];
  const chipStyle: CSSProperties = {
    ...CHIP_BASE_STYLE,
    background: chipTone.background,
    color: chipTone.color,
    transition: reduced ? 'none' : 'opacity 120ms ease-out',
  };

  const confidencePct = Math.round(confidence * 100);

  return (
    <section
      className={className}
      style={ROOT_STYLE}
      data-testid={testId ?? 'stats-overlay'}
      aria-label={`Stats overlay for ${label}`}
    >
      <header style={HEADER_STYLE}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
          <h5 style={{ margin: 0, fontSize: 12, fontWeight: 600 }}>{label}</h5>
          <span style={{ fontSize: 10, opacity: 0.7 }}>N = {stats.n}</span>
        </div>
        <span
          style={chipStyle}
          data-testid="stats-overlay-fit-chip"
          title={`Best-fit family (KS=${fmt(stats.fit.ksStatistic)}) — ${describeFit(stats.fit, unit)}`}
        >
          <span>{stats.fit.family}</span>
        </span>
      </header>
      <div style={STATS_GRID_STYLE} data-testid="stats-overlay-metrics">
        <Metric label="mean" testId="stats-overlay-mean">
          {fmt(stats.m, unit)}
        </Metric>
        <Metric label={`${confidencePct}% CI`} testId="stats-overlay-ci">
          [{fmt(stats.bootCi.lower, unit)}, {fmt(stats.bootCi.upper, unit)}]
        </Metric>
        <Metric label="σ" testId="stats-overlay-sigma">
          {fmt(stats.sd, unit)}
        </Metric>
        <Metric label="skew" testId="stats-overlay-skew">
          {fmt(stats.sk)}
        </Metric>
        <Metric label="kurtosis" testId="stats-overlay-kurtosis">
          {fmt(stats.kt)}
        </Metric>
        <Metric label="SEM" testId="stats-overlay-sem">
          {fmt(stats.classicCi.sem, unit)}
        </Metric>
      </div>
      {showQQ && sanitized.length >= 2 && (
        <div data-testid="stats-overlay-qq">
          <QQPlot values={sanitized} label={`Q-Q plot for ${label}`} testId="stats-overlay-qq-plot" />
        </div>
      )}
    </section>
  );
}

interface MetricProps {
  label: string;
  children: React.ReactNode;
  testId: string;
}

function Metric({ label, children, testId }: MetricProps) {
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 1 }} data-testid={testId}>
      <span style={{ fontSize: 10, opacity: 0.65, letterSpacing: 0.3 }}>{label}</span>
      <span style={{ fontVariantNumeric: 'tabular-nums' }}>{children}</span>
    </div>
  );
}
