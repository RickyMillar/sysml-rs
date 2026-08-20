/**
 * Pure summary statistics for an attribute's sample history.
 *
 * Kept a standalone file so the stats helpers can be tested without
 * mounting React. The stats strip in `AttributeDetail` consumes these
 * plus the raw ring-buffer points.
 */

export interface AttributeStats {
  count: number;
  min: number;
  max: number;
  mean: number;
  stddev: number;
  /** First / last tick boundaries (milliseconds on the t axis). */
  tFirst: number;
  tLast: number;
}

/**
 * Compute numeric stats from a slice of `TimePoint[]` — assumes
 * every `v` is a finite number. Undefined / non-finite values are
 * skipped (so a handful of NaN samples from a stale tick don't
 * poison the whole summary).
 */
export function computeAttributeStats(
  points: readonly { t: number; v: number }[],
): AttributeStats | null {
  if (points.length === 0) return null;
  let count = 0;
  let min = Number.POSITIVE_INFINITY;
  let max = Number.NEGATIVE_INFINITY;
  let sum = 0;
  let tFirst = Number.POSITIVE_INFINITY;
  let tLast = Number.NEGATIVE_INFINITY;
  for (const p of points) {
    if (!Number.isFinite(p.v)) continue;
    count += 1;
    if (p.v < min) min = p.v;
    if (p.v > max) max = p.v;
    sum += p.v;
    if (p.t < tFirst) tFirst = p.t;
    if (p.t > tLast) tLast = p.t;
  }
  if (count === 0) return null;
  const mean = sum / count;
  // Two-pass stddev — numerically stable for the sample sizes we
  // expect (up to a few thousand ticks). No need for Welford here.
  let ss = 0;
  for (const p of points) {
    if (!Number.isFinite(p.v)) continue;
    const d = p.v - mean;
    ss += d * d;
  }
  const stddev = count > 1 ? Math.sqrt(ss / (count - 1)) : 0;
  return { count, min, max, mean, stddev, tFirst, tLast };
}
