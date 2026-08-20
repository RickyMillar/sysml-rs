/**
 * Pure histogram helpers for Monte Carlo outcome analysis (R5.7).
 *
 * These helpers are intentionally DOM-free and side-effect-free so they
 * can be unit-tested cheaply and reused by the HTML/Markdown report
 * generators (R3.7 style) and downstream CSV export (R5.9).
 *
 * Contract:
 *   - `quantile` uses linear interpolation between neighbouring samples
 *     (SciPy `numpy.quantile` default "linear" method). The input is
 *     sorted internally; callers do not need to pre-sort.
 *   - `buildHistogram` produces equal-width bins spanning `[min, max]`.
 *     When all values are identical, a single synthetic bin is emitted
 *     so renderers have something to draw. Empty inputs produce zero
 *     bins and NaN stats — the viewer's empty state handles that case.
 */

/** A single histogram bin. `lower` inclusive, `upper` exclusive except
 *  the final bin which includes `upper` so `max` lands inside. */
export interface HistogramBin {
  lower: number;
  upper: number;
  count: number;
}

/** Summary statistics produced alongside a histogram. */
export interface HistogramStats {
  mean: number;
  sigma: number;
  p5: number;
  p50: number;
  p95: number;
}

/** Output of `buildHistogram`. */
export interface HistogramResult {
  bins: HistogramBin[];
  stats: HistogramStats;
}

/**
 * Return the `q` quantile of `values` (0 ≤ q ≤ 1).
 *
 * Uses linear interpolation between adjacent samples — matching NumPy's
 * default "linear" method. Returns NaN for empty input; returns the sole
 * value unchanged for length-1 input. Handles out-of-range `q` by
 * clamping to `[0, 1]`.
 */
export function quantile(values: number[], q: number): number {
  if (values.length === 0) return Number.NaN;
  if (values.length === 1) return values[0];
  const clamped = Math.min(1, Math.max(0, q));
  // Sort a copy — we never mutate the caller's array.
  const sorted = [...values].sort((a, b) => a - b);
  const pos = clamped * (sorted.length - 1);
  const lo = Math.floor(pos);
  const hi = Math.ceil(pos);
  if (lo === hi) return sorted[lo];
  const frac = pos - lo;
  return sorted[lo] + (sorted[hi] - sorted[lo]) * frac;
}

/**
 * Compute mean, population σ, p5/p50/p95 for a sample array.
 *
 * Exposed separately so callers that only need stats (e.g. the CSV
 * export's metric aggregator) don't have to build a full histogram.
 */
export function computeStats(values: number[]): HistogramStats {
  if (values.length === 0) {
    return {
      mean: Number.NaN,
      sigma: Number.NaN,
      p5: Number.NaN,
      p50: Number.NaN,
      p95: Number.NaN,
    };
  }
  let sum = 0;
  for (const v of values) sum += v;
  const mean = sum / values.length;
  let sqSum = 0;
  for (const v of values) {
    const d = v - mean;
    sqSum += d * d;
  }
  const sigma = Math.sqrt(sqSum / values.length);
  return {
    mean,
    sigma,
    p5: quantile(values, 0.05),
    p50: quantile(values, 0.5),
    p95: quantile(values, 0.95),
  };
}

/**
 * Bin `values` into `binCount` equal-width bins between min and max.
 *
 * Degenerate cases:
 *   - empty values → zero bins, NaN stats
 *   - all values equal → single synthetic bin of width 1 centered on the value
 *   - binCount ≤ 0 → defaults to 1
 */
export function buildHistogram(values: number[], binCount: number): HistogramResult {
  const stats = computeStats(values);
  if (values.length === 0) {
    return { bins: [], stats };
  }
  const effectiveBins = Math.max(1, Math.floor(binCount) || 1);
  let min = values[0];
  let max = values[0];
  for (const v of values) {
    if (v < min) min = v;
    if (v > max) max = v;
  }
  if (min === max) {
    // All samples equal — render a single centered bin so the viewer
    // has something to draw. Width of 1 is arbitrary but visually stable.
    return {
      bins: [{ lower: min - 0.5, upper: max + 0.5, count: values.length }],
      stats,
    };
  }
  const width = (max - min) / effectiveBins;
  const bins: HistogramBin[] = [];
  for (let i = 0; i < effectiveBins; i++) {
    bins.push({
      lower: min + i * width,
      upper: min + (i + 1) * width,
      count: 0,
    });
  }
  for (const v of values) {
    // The final bin is inclusive of the upper bound so `max` lands in the
    // last bucket rather than spilling off the end (avoids the classic
    // "one sample missing" artifact with floats at the upper boundary).
    let idx = Math.floor((v - min) / width);
    if (idx >= effectiveBins) idx = effectiveBins - 1;
    if (idx < 0) idx = 0;
    bins[idx].count += 1;
  }
  return { bins, stats };
}

/**
 * Gaussian kernel density estimate evaluated on a grid.
 *
 * Used by the optional KDE overlay in the histogram viewer. Bandwidth is
 * picked via Silverman's rule of thumb unless the caller supplies one.
 * Returns `[]` for empty input so the viewer can short-circuit.
 */
export function kde(
  values: number[],
  grid: number[],
  bandwidth?: number,
): number[] {
  if (values.length === 0 || grid.length === 0) return [];
  const stats = computeStats(values);
  const sigma = stats.sigma || 1;
  const n = values.length;
  // Silverman's rule. Guard against σ === 0 (all equal) by clamping.
  const h = bandwidth ?? Math.max(1e-9, 1.06 * sigma * Math.pow(n, -1 / 5));
  const normConst = 1 / (Math.sqrt(2 * Math.PI) * h * n);
  const out = new Array<number>(grid.length);
  for (let gi = 0; gi < grid.length; gi++) {
    const x = grid[gi];
    let acc = 0;
    for (let i = 0; i < n; i++) {
      const u = (x - values[i]) / h;
      acc += Math.exp(-0.5 * u * u);
    }
    out[gi] = normConst * acc;
  }
  return out;
}
