/**
 * TimeSeriesBuffer — Float64Array ring buffer for simulation time-series
 * data per ADR-008.
 *
 * Uses a fixed-capacity ring per variable backed by Float64Array. When the
 * buffer is full, the oldest data is overwritten. Designed for the 100MB
 * hard memory cap specified in ADR-008.
 */

/** Bytes per Float64 element. */
const BYTES_PER_F64 = 8;

/** Default memory budget: 100 MB. */
const DEFAULT_MEMORY_BUDGET_BYTES = 100 * 1024 * 1024;

/**
 * A single ring buffer backed by Float64Array.
 * Overwrites oldest entries when full.
 */
class Ring {
  readonly buf: Float64Array;
  readonly capacity: number;
  /** Write head — always advances, modulo capacity gives position. */
  head = 0;
  /** Number of written entries (capped at capacity). */
  count = 0;

  constructor(capacity: number) {
    this.capacity = capacity;
    this.buf = new Float64Array(capacity);
  }

  /** Append a single value. Overwrites oldest if full. */
  push(value: number): void {
    this.buf[this.head % this.capacity] = value;
    this.head++;
    if (this.count < this.capacity) this.count++;
  }

  /**
   * Return the logical array of values in insertion order.
   * Returns a new Float64Array (avoids exposing the ring internals).
   */
  toArray(): Float64Array {
    if (this.count < this.capacity) {
      // Buffer hasn't wrapped yet — simple slice
      return this.buf.slice(0, this.count);
    }
    // Wrapped: oldest data starts at (head % capacity)
    const start = this.head % this.capacity;
    const result = new Float64Array(this.capacity);
    result.set(this.buf.subarray(start), 0);
    result.set(this.buf.subarray(0, start), this.capacity - start);
    return result;
  }

  /** Read the value at logical index `i` (0 = oldest). */
  at(i: number): number {
    if (i < 0 || i >= this.count) return NaN;
    if (this.count < this.capacity) {
      return this.buf[i];
    }
    const start = this.head % this.capacity;
    return this.buf[(start + i) % this.capacity];
  }

  /**
   * Read values at specific logical indices via direct indexed `at()`
   * lookups — O(indices.length), independent of `count`. Used by the F2b
   * decimated read path so a chart-budget-sized sample never touches the
   * whole ring.
   */
  atIndices(indices: readonly number[]): Float64Array {
    const result = new Float64Array(indices.length);
    for (let k = 0; k < indices.length; k++) {
      result[k] = this.at(indices[k]!);
    }
    return result;
  }
}

/**
 * Compute a uniform-stride index sample of `[0, n)` bounded to roughly
 * `maxPoints` entries — O(min(n, maxPoints)) to build, and always ends
 * with `n - 1` so the most recent point is never stride-skipped (e.g. a
 * trip tick at the tail of a run must always show up on the chart).
 */
function decimationIndices(n: number, maxPoints: number): number[] {
  if (n <= 0) return [];
  if (n <= maxPoints) {
    const indices = new Array<number>(n);
    for (let i = 0; i < n; i++) indices[i] = i;
    return indices;
  }
  const stride = Math.ceil(n / maxPoints);
  const indices: number[] = [];
  for (let i = 0; i < n; i += stride) indices.push(i);
  if (indices[indices.length - 1] !== n - 1) indices.push(n - 1);
  return indices;
}

export class TimeSeriesBuffer {
  private readonly maxPoints: number;
  private readonly timestamps: Ring;
  private readonly series: Map<string, Ring> = new Map();

  /**
   * @param maxPoints Maximum data points to retain. If not provided,
   *   derived from the memory budget / estimated series count.
   * @param estimatedSeriesCount Expected number of variables (used to
   *   derive maxPoints from memory budget when maxPoints is omitted).
   * @param memoryBudgetBytes Total memory budget (default: 100 MB).
   */
  constructor(
    maxPoints?: number,
    estimatedSeriesCount = 10,
    memoryBudgetBytes = DEFAULT_MEMORY_BUDGET_BYTES,
  ) {
    if (maxPoints !== undefined) {
      this.maxPoints = maxPoints;
    } else {
      // Each point costs: 1 timestamp + N series values, all Float64
      const bytesPerPoint = BYTES_PER_F64 * (1 + estimatedSeriesCount);
      this.maxPoints = Math.floor(memoryBudgetBytes / bytesPerPoint);
    }
    this.timestamps = new Ring(this.maxPoints);
  }

  /** Current number of stored data points. */
  get length(): number {
    return this.timestamps.count;
  }

  /** Maximum data points before wrapping. */
  get capacity(): number {
    return this.maxPoints;
  }

  /** Names of all tracked variables. */
  get variableNames(): string[] {
    return [...this.series.keys()];
  }

  /**
   * Append one data point. New variable names are automatically allocated.
   * Variables not present in `values` receive NaN for this tick.
   */
  append(timestamp: number, values: Record<string, number>): void {
    this.timestamps.push(timestamp);

    // Ensure all incoming keys have a ring
    for (const key of Object.keys(values)) {
      if (!this.series.has(key)) {
        const ring = new Ring(this.maxPoints);
        // Back-fill with NaN for prior points
        for (let i = 0; i < this.timestamps.count - 1; i++) {
          ring.push(NaN);
        }
        this.series.set(key, ring);
      }
    }

    // Push values (or NaN for missing keys)
    for (const [key, ring] of this.series) {
      const v = values[key];
      ring.push(v !== undefined ? v : NaN);
    }
  }

  /**
   * Get a range of data. If start/end are omitted, returns all data.
   * start/end are timestamp values (inclusive bounds, best-effort).
   */
  getRange(
    start?: number,
    end?: number,
  ): { timestamps: Float64Array; series: Record<string, Float64Array> } {
    const allTs = this.timestamps.toArray();

    if (start === undefined && end === undefined) {
      const result: Record<string, Float64Array> = {};
      for (const [key, ring] of this.series) {
        result[key] = ring.toArray();
      }
      return { timestamps: allTs, series: result };
    }

    // Find index bounds
    let lo = 0;
    let hi = allTs.length;
    if (start !== undefined) {
      while (lo < allTs.length && allTs[lo] < start) lo++;
    }
    if (end !== undefined) {
      hi = lo;
      while (hi < allTs.length && allTs[hi] <= end) hi++;
    }

    const timestamps = allTs.slice(lo, hi);
    const result: Record<string, Float64Array> = {};
    for (const [key, ring] of this.series) {
      const full = ring.toArray();
      result[key] = full.slice(lo, hi);
    }
    return { timestamps, series: result };
  }

  /**
   * F2b: get a uniformly-strided sample of every series bounded to
   * `maxPoints`, computed in O(maxPoints * seriesCount) via direct
   * indexed ring reads — *not* O(totalStoredPoints). Unlike `getRange()`
   * (which copies each ring's full backing array before slicing),
   * this never materializes more than `maxPoints` values per series.
   *
   * All series (and the timestamps ring) share the same `count` because
   * `append()` pushes exactly one value into every ring per tick, so the
   * same index set applies to all of them and results stay time-aligned
   * across variables (important for XY/phase plots that join series by
   * exact timestamp).
   *
   * Intended for the live chart hot path; callers needing exact
   * aggregates (KPI min/max/mean/rms) or lossless CSV export must keep
   * using `getRange()` / `getTimeSeries()`.
   */
  getDecimatedRange(
    maxPoints: number,
  ): { timestamps: Float64Array; series: Record<string, Float64Array> } {
    const indices = decimationIndices(this.timestamps.count, maxPoints);
    const timestamps = this.timestamps.atIndices(indices);
    const result: Record<string, Float64Array> = {};
    for (const [key, ring] of this.series) {
      result[key] = ring.atIndices(indices);
    }
    return { timestamps, series: result };
  }

  /** Estimated memory usage in bytes. */
  memoryUsageBytes(): number {
    // timestamps ring + one ring per series variable
    const ringCount = 1 + this.series.size;
    return ringCount * this.maxPoints * BYTES_PER_F64;
  }

  /** Clear all data (keeps capacity and variable names). */
  clear(): void {
    this.timestamps.head = 0;
    this.timestamps.count = 0;
    for (const ring of this.series.values()) {
      ring.head = 0;
      ring.count = 0;
    }
  }
}
