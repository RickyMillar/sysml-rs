/**
 * Ensemble mode (R4.3) — N runs of the SAME model.
 *
 * UX goal: detect reproducibility issues, outlier runs, spread /
 * variance across a population of repeated runs with identical inputs
 * (or inputs drawn from the same distribution, for Monte-Carlo-style
 * playback).
 *
 * Main view (from the shell's default waveform overlay) gets a
 * per-variable statistic row at the shared playhead tick: mean, σ,
 * p5, p95, plus a flag marking which session ids are outliers (values
 * outside `sigmaThreshold · σ`).
 *
 * The output is a small "ensemble report" keyed by variable name.
 * This is not a Verdict — it's a descriptive statistic set for the
 * analyst. Verdicts are the golden mode's job.
 */
import { createElement } from 'react';
import type { ReactNode } from 'react';
import type { CompareContext, CompareMode } from '../compareMode';
import type { TimePoint } from '../../../features/sessions/types';
import { isFiniteNumber, sampleAtTick } from './seriesAccess';

// ── Pure helpers ─────────────────────────────────────────────────────

/** A single session's time series for one variable. */
export interface EnsembleRun {
  sessionId: string;
  points: TimePoint[];
}

/** Descriptive stats for one variable at one tick. */
export interface EnsembleStats {
  /** Number of sessions that contributed a finite sample. */
  n: number;
  mean: number;
  /** Population σ (divide by n, not n-1; the ensemble IS the population for this UI). */
  sigma: number;
  /** 5th / 95th percentile (linear interpolation). Null when n < 2. */
  p5: number | null;
  p95: number | null;
  min: number;
  max: number;
}

/** Zeros + null percentiles for empty/degenerate inputs. */
function emptyStats(): EnsembleStats {
  return { n: 0, mean: 0, sigma: 0, p5: null, p95: null, min: 0, max: 0 };
}

/**
 * Linear-interpolation percentile on a sorted ascending array.
 * `q` is in [0, 1]. Returns null for empty arrays. Single-element
 * arrays return that element.
 */
export function percentile(sorted: number[], q: number): number | null {
  if (sorted.length === 0) return null;
  if (sorted.length === 1) return sorted[0];
  const clamped = Math.min(1, Math.max(0, q));
  const pos = clamped * (sorted.length - 1);
  const lo = Math.floor(pos);
  const hi = Math.ceil(pos);
  if (lo === hi) return sorted[lo];
  const frac = pos - lo;
  return sorted[lo] * (1 - frac) + sorted[hi] * frac;
}

/**
 * Descriptive stats over a set of finite sample values. Drops NaN /
 * non-numeric inputs silently. Returns `emptyStats()` when zero finite
 * values remain.
 */
export function summarizeSamples(values: Array<number | null>): EnsembleStats {
  const finite: number[] = [];
  for (const v of values) if (isFiniteNumber(v)) finite.push(v);
  if (finite.length === 0) return emptyStats();

  let sum = 0;
  for (const v of finite) sum += v;
  const mean = sum / finite.length;

  let sqErr = 0;
  for (const v of finite) sqErr += (v - mean) ** 2;
  const sigma = Math.sqrt(sqErr / finite.length);

  const sorted = [...finite].sort((a, b) => a - b);
  return {
    n: finite.length,
    mean,
    sigma,
    p5: percentile(sorted, 0.05),
    p95: percentile(sorted, 0.95),
    min: sorted[0],
    max: sorted[sorted.length - 1],
  };
}

/**
 * Per-variable per-tick stats over a population of runs.
 *
 * `runs` holds one series per session; `tick` is the shared playhead.
 * Each run is sampled with last-known-value semantics; missing samples
 * are dropped (not treated as 0). `n` on the returned stats reflects
 * the number of contributing runs.
 */
export function computeEnsembleStats(
  runs: EnsembleRun[],
  tick: number,
): EnsembleStats {
  const values = runs.map((r) => sampleAtTick(r.points, tick));
  return summarizeSamples(values);
}

/**
 * Flag indices whose sample is outside `sigmaThreshold · σ` of the
 * ensemble mean. Pure numeric helper — operates on a raw `number[]`
 * (contract signature from the R4.3 plan).
 *
 * Rules:
 *   - 0- or 1-element inputs never report outliers (no spread).
 *   - Zero-σ populations never report outliers (every value is the mean).
 *   - Non-finite entries are treated as "not an outlier" and excluded
 *     from the mean + σ computation (they neither trigger nor hide).
 *   - `sigmaThreshold` must be > 0; non-positive or non-finite values
 *     yield an empty set.
 *   - Boundary rule: strictly `>` (samples sitting exactly on the
 *     threshold count as inside).
 *
 * Returns indices into the original `values` array (not session ids).
 */
export function detectOutliers(values: number[], sigmaThreshold: number): number[] {
  if (
    values.length < 2 ||
    !isFiniteNumber(sigmaThreshold) ||
    sigmaThreshold <= 0
  ) {
    return [];
  }
  const finite: number[] = [];
  for (const v of values) if (isFiniteNumber(v)) finite.push(v);
  if (finite.length < 2) return [];
  let sum = 0;
  for (const v of finite) sum += v;
  const mean = sum / finite.length;
  let sqErr = 0;
  for (const v of finite) sqErr += (v - mean) ** 2;
  const sigma = Math.sqrt(sqErr / finite.length);
  if (sigma === 0) return [];
  const cutoff = sigmaThreshold * sigma;
  const out: number[] = [];
  for (let i = 0; i < values.length; i += 1) {
    const v = values[i];
    if (!isFiniteNumber(v)) continue;
    if (Math.abs(v - mean) > cutoff) out.push(i);
  }
  return out;
}

/**
 * Session-aware outlier detection — a thin wrapper over `detectOutliers`
 * that samples each run at a tick and returns session ids. Used by the
 * shell's ensemble stats row; the pure helper is the primary export.
 */
export function detectOutlierSessions(
  runs: EnsembleRun[],
  tick: number,
  sigmaThreshold: number,
): string[] {
  if (runs.length < 2) return [];
  const samples = runs.map((r) => sampleAtTick(r.points, tick));
  // detectOutliers wants `number[]`; map non-finite → NaN so indices line up.
  const numeric: number[] = samples.map((v) => (isFiniteNumber(v) ? v : Number.NaN));
  const indices = detectOutliers(numeric, sigmaThreshold);
  return indices.map((i) => runs[i].sessionId);
}

/** One row of the ensemble report. */
export interface EnsembleReportRow {
  variable: string;
  stats: EnsembleStats;
  outlierSessionIds: string[];
}

/**
 * Build the ensemble report: one row per variable, stats computed at
 * the shared tick, outlier ids flagged by `sigmaThreshold`.
 */
export function buildEnsembleReport(
  runsByVariable: Record<string, EnsembleRun[]>,
  tick: number,
  sigmaThreshold: number,
): EnsembleReportRow[] {
  const rows: EnsembleReportRow[] = [];
  for (const [variable, runs] of Object.entries(runsByVariable)) {
    rows.push({
      variable,
      stats: computeEnsembleStats(runs, tick),
      outlierSessionIds: detectOutlierSessions(runs, tick, sigmaThreshold),
    });
  }
  // Stable alpha order — deterministic rendering.
  rows.sort((a, b) => a.variable.localeCompare(b.variable));
  return rows;
}

// ── React surface ────────────────────────────────────────────────────

/**
 * Default sigma threshold for outlier flagging. 2σ ≈ 95% of a Gaussian
 * population sits inside; anything outside is the "interesting tail".
 */
export const DEFAULT_SIGMA_THRESHOLD = 2;

function EnsembleConfigPanel(_ctx: CompareContext): ReactNode {
  // Minimal UX stub — the shell provides the variable picker (per W's
  // contract). This panel adds the sigma-threshold control only. The
  // shell wires state; this render is pure.
  return createElement(
    'div',
    {
      'data-testid': 'ensemble-config',
      style: {
        padding: 12,
        display: 'flex',
        flexDirection: 'column',
        gap: 8,
        fontSize: 12,
        color: 'var(--on-surface)',
      },
    },
    createElement(
      'div',
      { style: { fontWeight: 600 } },
      'Outlier threshold',
    ),
    createElement(
      'label',
      { style: { display: 'flex', gap: 6, alignItems: 'center' } },
      createElement('span', null, 'σ multiplier'),
      createElement('input', {
        type: 'number',
        defaultValue: DEFAULT_SIGMA_THRESHOLD,
        min: 0.5,
        step: 0.5,
        'data-testid': 'ensemble-sigma-input',
        style: {
          width: 64,
          background: 'var(--surface-container)',
          color: 'var(--on-surface)',
          border: '1px solid var(--outline-variant)',
          borderRadius: 4,
          padding: '2px 6px',
        },
      }),
    ),
    createElement(
      'div',
      { style: { opacity: 0.7 } },
      'Runs outside this band at the playhead tick are flagged.',
    ),
  );
}

/**
 * The ensemble mode. The shell's default waveform overlay is reused —
 * `mainRender` is omitted so W's default paints. The shell is expected
 * to consume `buildEnsembleReport` for the stats row it renders below
 * the waveform.
 */
export const ensembleMode: CompareMode = {
  id: 'ensemble',
  label: 'Ensemble',
  description: 'Spread / variance / reproducibility across N runs of the same model.',
  configRender: EnsembleConfigPanel,
};
