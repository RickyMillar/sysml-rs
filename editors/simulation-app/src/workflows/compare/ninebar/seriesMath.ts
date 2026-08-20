/**
 * seriesMath — pure tick-domain helpers for the Phase 6 Compare data
 * layer. Framework-free (plain functions over arrays) so the math is
 * unit-testable without React or react-query.
 *
 * The compare surface is TICK-centric (the contract's compare mandates
 * — `fork_point_tick`, `forkable_ticks`, `diff_timeline` — all speak
 * ticks), while the series wire (`sessions.timeseries_decimated`) is
 * time-keyed (`time_ms` per point). Each session's fixed step size
 * bridges the two: `tick ≈ round(time_ms / dt_ms)`. For the sessions
 * Compare is built for (fork siblings, ensemble re-runs, two designs
 * on one scenario) dt is identical across picks and the mapping is
 * exact — every recorded `time_ms` IS a tick's timestamp.
 *
 * Honesty note on decimation: the N-way divergence signal computed
 * here runs over LTTB-decimated series with linear reconstruction
 * between kept samples, so it is a navigation aid, not an exact diff.
 * PAIR mode gets the exact answer from the backend
 * (`sysml.sessions.diff_timeline`) and overlays it — see
 * `compareData.ts`.
 */

import type { VariableDiff } from '@/features/sessions/types';
import type { SamplesBySession } from '../selectors';

/** One decimated series point as it arrives on the wire. */
export interface SeriesPoint {
  time_ms: number;
  value: number;
}

/**
 * A session's effective step size in ms, derived from its summary
 * (`time_ms / tick`). `null` when the session hasn't ticked yet —
 * there is no honest dt to report, and callers must skip the session
 * rather than assume one.
 */
export function sessionDtMs(summary: {
  tick: number;
  time_ms: number;
}): number | null {
  if (!Number.isFinite(summary.tick) || summary.tick <= 0) return null;
  if (!Number.isFinite(summary.time_ms) || summary.time_ms <= 0) return null;
  return summary.time_ms / summary.tick;
}

/**
 * Convert decimated wire points to `(tick, value)` pairs using the
 * session's dt. Non-finite values are dropped; ticks are rounded to
 * the nearest integer (exact when dt divides the recorded times, which
 * it does for per-tick recordings).
 */
export function pointsToTickSeries(
  points: SeriesPoint[],
  dtMs: number,
): Array<{ tick: number; value: number }> {
  if (!Number.isFinite(dtMs) || dtMs <= 0) return [];
  const out: Array<{ tick: number; value: number }> = [];
  for (const p of points) {
    if (!Number.isFinite(p.value) || !Number.isFinite(p.time_ms)) continue;
    out.push({ tick: Math.round(p.time_ms / dtMs), value: p.value });
  }
  return out;
}

/**
 * Build the rectangular `samples[s][t]` matrix the divergence
 * selectors consume, from per-session tick series.
 *
 * Between a session's kept samples the value is LINEARLY interpolated
 * (LTTB keeps shape-defining points; linear reconstruction between
 * them is the standard reading). OUTSIDE a session's recorded range
 * the value is `NaN` — a session that ended (or started, for a fork
 * whose buffer begins at its fork point) simply has no data there,
 * and NaN is what the selectors and the missing-data dimming expect.
 *
 * `maxTick` bounds the matrix width (playhead domain); pass the max
 * across picked sessions' summaries so short series don't truncate
 * the domain.
 */
export function buildSampleMatrix(
  perSession: Array<Array<{ tick: number; value: number }>>,
  maxTick: number,
): SamplesBySession {
  const T = Math.max(0, Math.floor(maxTick)) + 1;
  return perSession.map((series) => {
    const row = new Array<number>(T).fill(NaN);
    if (series.length === 0) return row;
    const sorted = [...series].sort((a, b) => a.tick - b.tick);
    let i = 0;
    for (let t = sorted[0].tick; t <= sorted[sorted.length - 1].tick && t < T; t++) {
      if (t < 0) continue;
      while (i + 1 < sorted.length && sorted[i + 1].tick <= t) i++;
      const a = sorted[i];
      if (a.tick === t || i + 1 >= sorted.length) {
        row[t] = a.value;
        continue;
      }
      const b = sorted[i + 1];
      const span = b.tick - a.tick;
      row[t] =
        span <= 0 ? a.value : a.value + ((b.value - a.value) * (t - a.tick)) / span;
    }
    return row;
  });
}

/**
 * Classification of one `VariableDiff` side pair — manufactures the
 * added/removed layer from null-on-one-side (plan task: "diff-added/
 * removed/modified semantic tokens + manufacture from
 * null-on-one-side"). `a` is the left/base session, `b` the right.
 */
export type PairDiffKind = 'added' | 'removed' | 'modified';

export function classifyVariableDiff(d: VariableDiff): PairDiffKind {
  if (d.a_value === null && d.b_value !== null) return 'added';
  if (d.a_value !== null && d.b_value === null) return 'removed';
  return 'modified';
}

/**
 * The sparse `tick_diffs` entry at-or-before a tick (the state the
 * pair was last known to differ in when scrubbing). `null` when the
 * playhead sits before the first recorded difference.
 */
export function diffEntryAtOrBefore<T extends { tick: number }>(
  tickDiffs: T[],
  tick: number,
): T | null {
  let best: T | null = null;
  for (const d of tickDiffs) {
    if (d.tick > tick) break; // tick_diffs are oldest → newest
    best = d;
  }
  return best;
}

/**
 * Exact per-tick divergence mask for PAIR mode, from the backend's
 * sparse `tick_diffs` (1 where the sessions differ, 0 where they are
 * known equal). Length `maxTick + 1`. This is the gutter's authority
 * when exactly two sessions are picked — bit-exact backend comparison,
 * no decimation approximation.
 */
export function pairDivergenceMask(
  tickDiffs: Array<{ tick: number }>,
  maxTick: number,
): number[] {
  const T = Math.max(0, Math.floor(maxTick)) + 1;
  const out = new Array<number>(T).fill(0);
  for (const d of tickDiffs) {
    if (d.tick >= 0 && d.tick < T) out[d.tick] = 1;
  }
  return out;
}

/**
 * Downsample a per-tick score array into `buckets` max-pooled bins for
 * gutter rendering (a 5,856-tick run cannot paint one DOM band per
 * tick). Max-pooling, never averaging: a single divergent tick inside
 * a bucket must stay visible, not be diluted away.
 */
export function bucketScores(scores: number[], buckets: number): number[] {
  if (buckets <= 0 || scores.length === 0) return [];
  const n = Math.min(buckets, scores.length);
  const out = new Array<number>(n).fill(0);
  for (let t = 0; t < scores.length; t++) {
    const b = Math.min(n - 1, Math.floor((t / scores.length) * n));
    const v = scores[t];
    if (Number.isFinite(v) && v > out[b]) out[b] = v;
  }
  return out;
}

/** The tick a gutter bucket starts at (for click-to-scrub). */
export function bucketStartTick(
  bucket: number,
  buckets: number,
  tickCount: number,
): number {
  if (buckets <= 0 || tickCount <= 0) return 0;
  const n = Math.min(buckets, tickCount);
  return Math.min(tickCount - 1, Math.round((bucket / n) * tickCount));
}
