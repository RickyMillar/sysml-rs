/**
 * modeMath — thin PURE bridges between the Phase 6 data layer's
 * tick-domain matrices and the R4.3 mode math (ensemble / golden /
 * two-design). The heavy math stays in `../modes/*` (one home);
 * these helpers only convert shapes and compose.
 */

import type { TimePoint } from '@/features/sessions/types';
import {
  DEFAULT_SIGMA_THRESHOLD,
  summarizeSamples,
  type EnsembleStats,
} from '../modes/ensemble';

/** Matrix row (`samples[s]`) → sparse TimePoint series (NaN dropped). */
export function rowToTimePoints(row: number[]): TimePoint[] {
  const out: TimePoint[] = [];
  for (let t = 0; t < row.length; t++) {
    const v = row[t];
    if (Number.isFinite(v)) out.push({ t, v });
  }
  return out;
}

/**
 * Extract per-variable TimePoint series from a backend archived
 * record's `snapshots` (serialized ExecutionSnapshots, oldest →
 * newest, each carrying `tick` + `variables`). Same value coercions
 * as the live ingest path: finite numbers pass, booleans become 1/0,
 * `{value}` / `{re}` wrappers unwrap. Anything else is skipped —
 * missing stays missing.
 */
export function archivedSnapshotsToSeries(
  snapshots: unknown[],
): Record<string, TimePoint[]> {
  const series: Record<string, TimePoint[]> = {};
  for (let i = 0; i < snapshots.length; i++) {
    const snap = snapshots[i];
    if (!snap || typeof snap !== 'object') continue;
    const obj = snap as Record<string, unknown>;
    const tick = typeof obj.tick === 'number' && Number.isFinite(obj.tick) ? obj.tick : i;
    const vars = obj.variables;
    if (!vars || typeof vars !== 'object') continue;
    for (const [name, raw] of Object.entries(vars as Record<string, unknown>)) {
      if (name.startsWith('__') || name === 't_ms' || name === 'tick') continue;
      const v = coerceNumeric(raw);
      if (v === null) continue;
      (series[name] ??= []).push({ t: tick, v });
    }
  }
  return series;
}

function coerceNumeric(raw: unknown): number | null {
  if (typeof raw === 'number' && Number.isFinite(raw)) return raw;
  if (typeof raw === 'boolean') return raw ? 1 : 0;
  if (raw && typeof raw === 'object' && !Array.isArray(raw)) {
    const obj = raw as Record<string, unknown>;
    if (typeof obj.value === 'number' && Number.isFinite(obj.value)) return obj.value;
    if (typeof obj.re === 'number' && Number.isFinite(obj.re)) return obj.re;
  }
  return null;
}

/** Ensemble stats + outlier session indices for one variable at one
 *  tick, straight off the matrix column. */
export interface EnsembleAtTick {
  stats: EnsembleStats;
  /** Indices (into the session order) outside σ·threshold. */
  outlierIndices: number[];
}

export function ensembleAtTick(
  samples: number[][],
  tick: number,
  sigmaThreshold: number = DEFAULT_SIGMA_THRESHOLD,
): EnsembleAtTick {
  const values = samples.map((row) => {
    const v = row[Math.min(tick, row.length - 1)];
    return Number.isFinite(v) ? v : null;
  });
  const stats = summarizeSamples(values);
  const outlierIndices: number[] = [];
  if (stats.n >= 2 && stats.sigma > 0) {
    values.forEach((v, i) => {
      if (v !== null && Math.abs(v - stats.mean) > sigmaThreshold * stats.sigma) {
        outlierIndices.push(i);
      }
    });
  }
  return { stats, outlierIndices };
}
