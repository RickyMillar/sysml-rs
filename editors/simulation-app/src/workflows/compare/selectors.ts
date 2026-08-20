/**
 * Pure selectors / math for the R4.2 compare workflow.
 *
 * Keep this file framework-free — every helper is a plain function over
 * arrays so it's unit-testable without React, Zustand, or react-query.
 */

/**
 * `samples[s][t]` = value of session `s` at tick `t`. `NaN` represents
 * "this session had no value at this tick" (e.g. a shorter session that
 * already finished). Lengths may differ; callers should zero-pad to the
 * max length via `normaliseSamples` below.
 */
export type SamplesBySession = number[][];

/**
 * Normalise ragged samples to a rectangular matrix of shape
 * `[S][maxTicks]`. Missing values are filled with `NaN` so divergence
 * computations can skip them rather than treat them as zeros.
 */
export function normaliseSamples(samples: SamplesBySession): SamplesBySession {
  const maxTicks = samples.reduce((m, row) => Math.max(m, row.length), 0);
  return samples.map((row) => {
    if (row.length === maxTicks) return row.slice();
    const padded = new Array<number>(maxTicks);
    for (let t = 0; t < maxTicks; t++) {
      padded[t] = t < row.length ? row[t] : NaN;
    }
    return padded;
  });
}

/**
 * Per-tick divergence score for one variable across N sessions.
 *
 * For each tick t:
 *   score[t] = (max_s v[s][t] - min_s v[s][t]) / range(variable)
 * where `range(variable)` is the global max-min across all samples.
 *
 * Sessions contributing `NaN` at tick `t` are ignored for that tick.
 * If fewer than 2 sessions have values, the score is 0 (no pair to
 * diverge). If the global range is 0 (constant), all scores are 0.
 *
 * Return shape: `number[]` of length `maxTicks` in `[0, 1]`.
 */
export function computeDivergence(samples: SamplesBySession): number[] {
  const rect = normaliseSamples(samples);
  const S = rect.length;
  const T = rect[0]?.length ?? 0;

  if (S === 0 || T === 0) return [];
  if (S === 1) return new Array<number>(T).fill(0);

  // Global range across all finite values.
  let gMin = Infinity;
  let gMax = -Infinity;
  for (let s = 0; s < S; s++) {
    for (let t = 0; t < T; t++) {
      const v = rect[s][t];
      if (Number.isFinite(v)) {
        if (v < gMin) gMin = v;
        if (v > gMax) gMax = v;
      }
    }
  }
  const range = gMax - gMin;
  if (!Number.isFinite(range) || range <= 0) return new Array<number>(T).fill(0);

  const out = new Array<number>(T).fill(0);
  for (let t = 0; t < T; t++) {
    let mn = Infinity;
    let mx = -Infinity;
    let present = 0;
    for (let s = 0; s < S; s++) {
      const v = rect[s][t];
      if (Number.isFinite(v)) {
        if (v < mn) mn = v;
        if (v > mx) mx = v;
        present++;
      }
    }
    if (present < 2) continue;
    const spread = mx - mn;
    out[t] = Math.max(0, Math.min(1, spread / range));
  }
  return out;
}

/**
 * The tick at which divergence is maximal. `-1` if the divergence
 * array is empty or all zeros (nothing to scrub to).
 */
export function peakDivergenceTick(divergence: number[]): number {
  let best = -1;
  let peak = 0;
  for (let t = 0; t < divergence.length; t++) {
    if (divergence[t] > peak) {
      peak = divergence[t];
      best = t;
    }
  }
  return best;
}

/**
 * Cross-session variance of a variable across all ticks, used for the
 * "auto-pick top-N interesting variables" heuristic.
 *
 * For each tick, compute the variance across sessions; then average
 * those per-tick variances. Missing values (NaN) are skipped. Returns
 * 0 for constants or 1-session inputs.
 */
export function crossSessionVariance(samples: SamplesBySession): number {
  const rect = normaliseSamples(samples);
  const S = rect.length;
  const T = rect[0]?.length ?? 0;
  if (S < 2 || T === 0) return 0;

  let total = 0;
  let countedTicks = 0;

  for (let t = 0; t < T; t++) {
    let sum = 0;
    let sumSq = 0;
    let n = 0;
    for (let s = 0; s < S; s++) {
      const v = rect[s][t];
      if (Number.isFinite(v)) {
        sum += v;
        sumSq += v * v;
        n++;
      }
    }
    if (n < 2) continue;
    const mean = sum / n;
    const variance = sumSq / n - mean * mean;
    total += Math.max(0, variance);
    countedTicks++;
  }
  if (countedTicks === 0) return 0;
  return total / countedTicks;
}

/**
 * Auto-pick the top-N most interesting variables by cross-session
 * variance. Input: `variables` is a per-variable `SamplesBySession`.
 * Output: variable names sorted by descending variance, capped at `n`.
 * Ties break alphabetically for deterministic output.
 */
export function autoPickVariables(
  variables: Record<string, SamplesBySession>,
  n = 6,
): string[] {
  const scored = Object.entries(variables).map(
    ([name, samples]) => [name, crossSessionVariance(samples)] as const,
  );
  scored.sort((a, b) => {
    if (b[1] !== a[1]) return b[1] - a[1];
    return a[0].localeCompare(b[0]);
  });
  return scored.slice(0, n).map(([name]) => name);
}

/**
 * Map a divergence score in [0, 1] to a heat-band CSS color string.
 * Uses OKLCH for smooth perceptual ramp (cold → warm).
 *
 * 0.0 → transparent (no divergence; background shows through)
 * 0.3 → cool blue-grey
 * 1.0 → hot warning red
 */
export function divergenceColor(score: number): string {
  const clamped = Math.max(0, Math.min(1, score));
  if (clamped <= 0) return 'transparent';
  // oklch(L C H) with alpha = score so quiet regions fade out.
  const alpha = Math.min(1, 0.25 + 0.75 * clamped);
  const hue = 240 - clamped * 220; // 240 (blue) → 20 (red)
  return `oklch(62% ${0.18 * clamped + 0.04} ${hue} / ${alpha.toFixed(3)})`;
}

/**
 * The playhead extent is the length of the longest picked session.
 * Shorter sessions freeze on their last tick — callers handle that by
 * clamping the index into each session's own samples.
 */
export function playheadMaxTick(sessionTickCounts: number[]): number {
  if (sessionTickCounts.length === 0) return 0;
  const longest = Math.max(0, ...sessionTickCounts);
  return Math.max(0, longest - 1);
}
