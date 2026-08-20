/**
 * Performance monitoring utilities per ADR-008 performance budget.
 *
 * Constants and helpers for validating that the simulation app stays
 * within its rendering and memory budgets.
 */

// ── Performance Budget Constants ─────────────────────────────────────

export const PerfBudget = {
  /** Maximum decimated points per chart series (LTTB cap). */
  MAX_RENDER_POINTS: 1_500,

  /** Maximum Float64Array ring buffer memory in MB. */
  MAX_MEMORY_MB: 100,

  /** Target chart render time in ms (one 60fps frame). */
  CHART_RENDER_MS: 16,

  /** Overlay throttle interval in ms. */
  OVERLAY_THROTTLE_MS: 100,
} as const;

// ── Render timing wrapper ────────────────────────────────────────────

/**
 * Wraps a synchronous render call with `performance.now()` timing.
 * Warns to console if execution exceeds `budgetMs` (defaults to CHART_RENDER_MS).
 *
 * Returns the result of `fn()`.
 */
export function measureRender<T>(
  label: string,
  fn: () => T,
  budgetMs: number = PerfBudget.CHART_RENDER_MS,
): T {
  const start = performance.now();
  const result = fn();
  const elapsed = performance.now() - start;

  if (elapsed > budgetMs) {
    console.warn(
      `[perf] "${label}" took ${elapsed.toFixed(2)}ms (budget: ${budgetMs}ms)`,
    );
  }

  return result;
}

// ── Memory estimation ────────────────────────────────────────────────

/** Bytes per Float64 element. */
const BYTES_PER_F64 = 8;

/**
 * Estimate memory usage for Float64Array storage of time-series data.
 *
 * Each series stores `pointsPerSeries` Float64 values for both time and
 * value channels, so total = seriesCount * pointsPerSeries * 2 * 8 bytes.
 *
 * @returns Estimated byte count.
 */
export function estimateMemoryUsage(
  seriesCount: number,
  pointsPerSeries: number,
): number {
  // Two channels per series: time (t) and value (v)
  return seriesCount * pointsPerSeries * 2 * BYTES_PER_F64;
}
