/**
 * seriesAccess — tiny shared helpers used by all three compare modes.
 *
 * The three modes (ensemble / golden / two-design) all receive
 * time-series data as an array of `TimePoint` (monotonic `t`, scalar
 * `v`). These helpers wrap the boring per-tick sampling + numeric
 * hygiene so each mode's file stays focused on its own logic.
 *
 * Pure functions only. No React, no IO. Exported for direct unit test
 * from each mode's `__tests__/`.
 */
import type { TimePoint } from '../../../features/sessions/types';

/** True if `n` is a finite number (rejects NaN / Infinity / non-numeric). */
export function isFiniteNumber(n: unknown): n is number {
  return typeof n === 'number' && Number.isFinite(n);
}

/**
 * Sample a series at `tick` using last-known-value semantics.
 *
 * `TimePoint.t` is the tick index (or time in arbitrary units — modes
 * treat it as monotonic, not as "ms"). Returns `null` when no point
 * at-or-before `tick` exists or when the sampled value is non-finite.
 *
 * Binary-search keeps the ensemble's per-tick stats row O(N log M).
 */
export function sampleAtTick(points: TimePoint[], tick: number): number | null {
  if (points.length === 0) return null;
  // Binary search for the last index with t <= tick.
  let lo = 0;
  let hi = points.length - 1;
  if (points[0].t > tick) return null;
  while (lo < hi) {
    const mid = (lo + hi + 1) >>> 1;
    if (points[mid].t <= tick) lo = mid;
    else hi = mid - 1;
  }
  const v = points[lo].v;
  return isFiniteNumber(v) ? v : null;
}

/**
 * Zip two series to a common tick grid (union of both), sampling each
 * with last-known-value semantics at every grid tick. Empty inputs
 * yield an empty array.
 *
 * Used by `twoDesign`'s delta computation and `golden`'s pass/fail
 * per-tick loop.
 */
export function zipSeries(
  a: TimePoint[],
  b: TimePoint[],
): Array<{ t: number; a: number | null; b: number | null }> {
  const ticks = new Set<number>();
  for (const p of a) ticks.add(p.t);
  for (const p of b) ticks.add(p.t);
  const sorted = [...ticks].sort((x, y) => x - y);
  return sorted.map((t) => ({ t, a: sampleAtTick(a, t), b: sampleAtTick(b, t) }));
}
