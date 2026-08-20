/**
 * Pure statistical helpers — R7.2.
 *
 * Multi-run aggregation primitives that power the Stats Overlay on top
 * of the Monte Carlo / Sweep viewers. Kept deliberately DOM-free,
 * side-effect-free, and dependency-free so:
 *
 *   - Unit tests run without JSDOM.
 *   - Callers can compose them in report generators, CSV exporters, or
 *     future backend bridges without pulling in React.
 *   - Every function handles empty / single-element / NaN inputs
 *     gracefully (NaN out when the sample has no signal; never throws).
 *
 * Numerical notes:
 *   - Confidence intervals use Student's t for small samples (n < 30)
 *     and a fast polynomial t-quantile approximation. For n >= 30 the
 *     normal z-quantile is used — matches common stats textbooks.
 *   - Bootstrap CI is the percentile method (resample with replacement
 *     B times, take the [α/2, 1−α/2] quantiles of the bootstrap means).
 *     An explicit `rng` parameter keeps it deterministic in tests.
 *   - `fitDistribution` runs lightweight fits for normal / lognormal /
 *     uniform, scores each via Kolmogorov-Smirnov against the empirical
 *     CDF, and returns the best. When no fit beats the KS-reject
 *     threshold it falls back to `'unknown'`.
 */

// ── Basic moments ───────────────────────────────────────────────────

/** Filter to finite numbers only. */
function finite(values: number[]): number[] {
  const out: number[] = [];
  for (const v of values) if (Number.isFinite(v)) out.push(v);
  return out;
}

/** Arithmetic mean — NaN on empty input. */
export function mean(values: number[]): number {
  const v = finite(values);
  if (v.length === 0) return Number.NaN;
  let s = 0;
  for (const x of v) s += x;
  return s / v.length;
}

/**
 * Sample standard deviation (Bessel's correction, n-1 denominator).
 *
 * Returns 0 for n=1 (convention for single-point CI degeneration) and
 * NaN for empty input. Uses the textbook two-pass formulation — slower
 * than Welford's algorithm but numerically stable for the small-to-
 * medium samples the viewer deals with.
 */
export function stddev(values: number[]): number {
  const v = finite(values);
  if (v.length === 0) return Number.NaN;
  if (v.length === 1) return 0;
  const m = mean(v);
  let sq = 0;
  for (const x of v) {
    const d = x - m;
    sq += d * d;
  }
  return Math.sqrt(sq / (v.length - 1));
}

/** Population variance (divide by n). Used by skew/kurtosis. */
function populationVariance(values: number[], m: number): number {
  if (values.length === 0) return Number.NaN;
  let sq = 0;
  for (const x of values) {
    const d = x - m;
    sq += d * d;
  }
  return sq / values.length;
}

/**
 * Sample skewness (Fisher-Pearson with sample correction).
 * Returns 0 when the sample is too small or variance is 0.
 */
export function skewness(values: number[]): number {
  const v = finite(values);
  const n = v.length;
  if (n < 3) return 0;
  const m = mean(v);
  const varp = populationVariance(v, m);
  if (varp === 0) return 0;
  const sd = Math.sqrt(varp);
  let s = 0;
  for (const x of v) {
    const d = (x - m) / sd;
    s += d * d * d;
  }
  // Sample-adjusted (G1) — SciPy bias=False default.
  return (n / ((n - 1) * (n - 2))) * s;
}

/**
 * Excess kurtosis (sample adjusted, G2). Zero for a perfect Normal.
 * Returns 0 when the sample is too small or variance is 0.
 */
export function kurtosis(values: number[]): number {
  const v = finite(values);
  const n = v.length;
  if (n < 4) return 0;
  const m = mean(v);
  const varp = populationVariance(v, m);
  if (varp === 0) return 0;
  let s = 0;
  for (const x of v) {
    const d = (x - m) / Math.sqrt(varp);
    s += d * d * d * d;
  }
  // G2 — Fisher's definition with sample bias correction.
  const num = (n * (n + 1)) / ((n - 1) * (n - 2) * (n - 3));
  const corr = (3 * (n - 1) * (n - 1)) / ((n - 2) * (n - 3));
  return num * s - corr;
}

// ── Quantile ────────────────────────────────────────────────────────

/**
 * Linear-interpolated quantile (NumPy "linear" method).
 *
 * The MC histogram helpers already ship a similar helper; we duplicate
 * here to keep `statsHelpers` self-contained (no cross-feature imports
 * from `workflows/analyze`).
 */
export function quantile(values: number[], q: number): number {
  const v = finite(values);
  if (v.length === 0) return Number.NaN;
  if (v.length === 1) return v[0];
  const c = Math.min(1, Math.max(0, q));
  const sorted = [...v].sort((a, b) => a - b);
  const pos = c * (sorted.length - 1);
  const lo = Math.floor(pos);
  const hi = Math.ceil(pos);
  if (lo === hi) return sorted[lo];
  return sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo);
}

// ── Normal / t distribution primitives ──────────────────────────────

/** Standard normal PDF. */
function normalPdf(x: number): number {
  return Math.exp(-0.5 * x * x) / Math.sqrt(2 * Math.PI);
}

/**
 * Standard normal CDF via the Abramowitz-Stegun erf approximation
 * (accurate to ~1e-7). Sufficient for overlay-grade stats.
 */
export function normalCdf(x: number): number {
  // erf via A&S 7.1.26
  const sign = x < 0 ? -1 : 1;
  const ax = Math.abs(x) / Math.SQRT2;
  const a1 = 0.254829592;
  const a2 = -0.284496736;
  const a3 = 1.421413741;
  const a4 = -1.453152027;
  const a5 = 1.061405429;
  const p = 0.3275911;
  const t = 1.0 / (1.0 + p * ax);
  const y =
    1.0 -
    (((((a5 * t + a4) * t) + a3) * t + a2) * t + a1) *
      t *
      Math.exp(-ax * ax);
  return 0.5 * (1 + sign * y);
}

/**
 * Inverse standard normal CDF (probit). Beasley-Springer-Moro approximation.
 * Accurate to ~4e-4 over the usable confidence range (0.001..0.999) —
 * plenty for 95% / 99% intervals.
 */
export function normalQuantile(p: number): number {
  if (!(p > 0 && p < 1)) {
    if (p === 0) return Number.NEGATIVE_INFINITY;
    if (p === 1) return Number.POSITIVE_INFINITY;
    return Number.NaN;
  }
  // Beasley-Springer-Moro
  const a = [
    -3.969683028665376e1, 2.209460984245205e2, -2.759285104469687e2,
    1.38357751867269e2, -3.066479806614716e1, 2.506628277459239,
  ];
  const b = [
    -5.447609879822406e1, 1.615858368580409e2, -1.556989798598866e2,
    6.680131188771972e1, -1.328068155288572e1,
  ];
  const c = [
    -7.784894002430293e-3, -3.223964580411365e-1, -2.400758277161838,
    -2.549732539343734, 4.374664141464968, 2.938163982698783,
  ];
  const d = [
    7.784695709041462e-3, 3.224671290700398e-1, 2.445134137142996,
    3.754408661907416,
  ];
  const pLow = 0.02425;
  const pHigh = 1 - pLow;
  let q: number;
  let r: number;
  if (p < pLow) {
    q = Math.sqrt(-2 * Math.log(p));
    return (
      (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5]) /
      ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1)
    );
  }
  if (p <= pHigh) {
    q = p - 0.5;
    r = q * q;
    return (
      ((((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) *
        q) /
      (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1)
    );
  }
  q = Math.sqrt(-2 * Math.log(1 - p));
  return (
    -(((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5]) /
    ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1)
  );
}

/**
 * Student-t quantile approximation (Hill 1970).
 *
 * For df >= 30 the normal quantile is a good approximation; for small
 * df we apply the Hill correction so confidence intervals stay honest
 * on n < 30 samples.
 */
export function tQuantile(p: number, df: number): number {
  if (!(p > 0 && p < 1) || !Number.isFinite(df) || df <= 0) return Number.NaN;
  if (df >= 30) return normalQuantile(p);
  // Hill 1970 Algorithm 396 — approximation good to 4 decimal places.
  const z = normalQuantile(p);
  const g1 = (z * z * z + z) / 4;
  const g2 =
    (5 * z * z * z * z * z + 16 * z * z * z + 3 * z) / 96;
  const g3 =
    (3 * z * z * z * z * z * z * z + 19 * z * z * z * z * z + 17 * z * z * z -
      15 * z) /
    384;
  const g4 =
    (79 * z * z * z * z * z * z * z * z * z +
      776 * z * z * z * z * z * z * z +
      1482 * z * z * z * z * z -
      1920 * z * z * z -
      945 * z) /
    92160;
  return z + g1 / df + g2 / (df * df) + g3 / (df * df * df) + g4 / (df * df * df * df);
}

/**
 * Student-t CDF. Uses regularised incomplete beta for df >= 1; good
 * enough for p-value output on the overlay.
 */
export function tCdf(t: number, df: number): number {
  if (!Number.isFinite(t) || !Number.isFinite(df) || df <= 0) return Number.NaN;
  // Relationship: F(t; ν) = 1 − 0.5 * I_{ν/(ν+t^2)}(ν/2, 1/2), t > 0
  const x = df / (df + t * t);
  const p = 0.5 * regIncBeta(x, df / 2, 0.5);
  return t >= 0 ? 1 - p : p;
}

/** Regularised incomplete beta via continued fraction (Numerical Recipes). */
function regIncBeta(x: number, a: number, b: number): number {
  if (x <= 0) return 0;
  if (x >= 1) return 1;
  const lnBeta = logGamma(a) + logGamma(b) - logGamma(a + b);
  const front = Math.exp(Math.log(x) * a + Math.log(1 - x) * b - lnBeta) / a;
  if (x < (a + 1) / (a + b + 2)) {
    return front * betaCf(x, a, b);
  }
  return 1 - (Math.exp(Math.log(x) * a + Math.log(1 - x) * b - lnBeta) / b) * betaCf(1 - x, b, a);
}

function betaCf(x: number, a: number, b: number): number {
  const maxIter = 200;
  const eps = 3e-7;
  const qab = a + b;
  const qap = a + 1;
  const qam = a - 1;
  let c = 1;
  let d = 1 - (qab * x) / qap;
  if (Math.abs(d) < 1e-30) d = 1e-30;
  d = 1 / d;
  let h = d;
  for (let m = 1; m <= maxIter; m++) {
    const m2 = 2 * m;
    let aa = (m * (b - m) * x) / ((qam + m2) * (a + m2));
    d = 1 + aa * d;
    if (Math.abs(d) < 1e-30) d = 1e-30;
    c = 1 + aa / c;
    if (Math.abs(c) < 1e-30) c = 1e-30;
    d = 1 / d;
    h *= d * c;
    aa = (-(a + m) * (qab + m) * x) / ((a + m2) * (qap + m2));
    d = 1 + aa * d;
    if (Math.abs(d) < 1e-30) d = 1e-30;
    c = 1 + aa / c;
    if (Math.abs(c) < 1e-30) c = 1e-30;
    d = 1 / d;
    const del = d * c;
    h *= del;
    if (Math.abs(del - 1) < eps) break;
  }
  return h;
}

/** log-gamma (Lanczos). Used by the beta-function normaliser above. */
function logGamma(x: number): number {
  const g = 7;
  const c = [
    0.99999999999980993, 676.5203681218851, -1259.1392167224028,
    771.32342877765313, -176.61502916214059, 12.507343278686905,
    -0.13857109526572012, 9.9843695780195716e-6, 1.5056327351493116e-7,
  ];
  if (x < 0.5) {
    return Math.log(Math.PI / Math.sin(Math.PI * x)) - logGamma(1 - x);
  }
  x -= 1;
  let a = c[0];
  const t = x + g + 0.5;
  for (let i = 1; i < g + 2; i++) a += c[i] / (x + i);
  return 0.5 * Math.log(2 * Math.PI) + (x + 0.5) * Math.log(t) - t + Math.log(a);
}

// ── Confidence intervals ────────────────────────────────────────────

/** t/z-based confidence interval result. */
export interface ConfidenceInterval {
  lower: number;
  upper: number;
  mean: number;
  /** Standard error of the mean (σ/√n with sample σ). */
  sem: number;
}

/**
 * Confidence interval for the population mean.
 *
 * For n < 30 uses the Student-t critical value with df = n-1; for larger
 * samples the normal z-quantile is used. `confidence` is the two-sided
 * probability (0.95 ≡ 95% CI). Empty / single-element inputs return NaN
 * bounds so the UI can show a dash.
 */
export function confidenceInterval(
  values: number[],
  confidence: number,
): ConfidenceInterval {
  const v = finite(values);
  const n = v.length;
  const mu = mean(v);
  if (n === 0) {
    return { lower: Number.NaN, upper: Number.NaN, mean: Number.NaN, sem: Number.NaN };
  }
  if (n === 1) {
    return { lower: v[0], upper: v[0], mean: v[0], sem: 0 };
  }
  const sd = stddev(v);
  const sem = sd / Math.sqrt(n);
  const alpha = 1 - confidence;
  const p = 1 - alpha / 2;
  const critical = n < 30 ? tQuantile(p, n - 1) : normalQuantile(p);
  return {
    lower: mu - critical * sem,
    upper: mu + critical * sem,
    mean: mu,
    sem,
  };
}

/** Bootstrap CI result. Lower / upper match the chosen confidence level. */
export interface BootstrapCI {
  lower: number;
  upper: number;
}

/**
 * Bootstrap confidence interval of the mean via the percentile method.
 *
 * `iterations` ≥ 200 recommended for stable bounds; we clamp to ≥ 1.
 * `rng` is an injected uniform RNG — supply `Math.random` at runtime
 * and a seeded PRNG in tests for deterministic assertions.
 *
 * For n=0 returns NaN bounds. For n=1 returns the point as both bounds
 * (trivial CI — no resampling variance possible).
 */
export function bootstrapCI(
  values: number[],
  confidence: number,
  iterations: number,
  rng: () => number,
): BootstrapCI {
  const v = finite(values);
  const n = v.length;
  if (n === 0) return { lower: Number.NaN, upper: Number.NaN };
  if (n === 1) return { lower: v[0], upper: v[0] };
  const iters = Math.max(1, Math.floor(iterations) || 1);
  const means = new Array<number>(iters);
  for (let b = 0; b < iters; b++) {
    let s = 0;
    for (let i = 0; i < n; i++) {
      // Floor of rng()*n — a correct uniform sample with replacement
      // assuming rng() returns [0, 1).
      const idx = Math.min(n - 1, Math.floor(rng() * n));
      s += v[idx];
    }
    means[b] = s / n;
  }
  means.sort((a, b) => a - b);
  const alpha = 1 - confidence;
  const lo = alpha / 2;
  const hi = 1 - alpha / 2;
  return {
    lower: means[Math.min(iters - 1, Math.max(0, Math.floor(lo * (iters - 1))))],
    upper: means[Math.min(iters - 1, Math.max(0, Math.ceil(hi * (iters - 1))))],
  };
}

// ── Kolmogorov-Smirnov + distribution fit ───────────────────────────

/**
 * One-sample Kolmogorov-Smirnov statistic against an arbitrary CDF.
 *
 * Returns `sup |F_n(x) − F(x)|` where F_n is the empirical CDF. Empty
 * input returns NaN. The CDF callable must accept any real number and
 * return a probability in [0, 1].
 */
export function kolmogorovSmirnov(values: number[], cdf: (x: number) => number): number {
  const v = finite(values);
  const n = v.length;
  if (n === 0) return Number.NaN;
  const sorted = [...v].sort((a, b) => a - b);
  let d = 0;
  for (let i = 0; i < n; i++) {
    const f = cdf(sorted[i]);
    // Both one-sided D+ and D- contribute to the supremum.
    const dPlus = (i + 1) / n - f;
    const dMinus = f - i / n;
    if (dPlus > d) d = dPlus;
    if (dMinus > d) d = dMinus;
  }
  return d;
}

/** Family tag used by `fitDistribution`. */
export type DistributionFamily = 'normal' | 'lognormal' | 'uniform' | 'unknown';

/** Fit result — best-scoring family + MLE-style params + KS distance. */
export interface DistributionFit {
  family: DistributionFamily;
  params: Record<string, number>;
  ksStatistic: number;
}

/**
 * Fit a lightweight distribution (normal / lognormal / uniform) and
 * return the best by KS statistic.
 *
 * `'unknown'` is returned when:
 *   - n < 3 (too little data to fit anything meaningfully),
 *   - the best fit has `ksStatistic > threshold(n)` where threshold is
 *     the asymptotic 5% KS critical value 1.36/√n.
 *
 * Params per family:
 *   normal    → { mu, sigma }
 *   lognormal → { mu, sigma }  (parameters of the *log-transformed* data)
 *   uniform   → { min, max }
 */
export function fitDistribution(values: number[]): DistributionFit {
  const v = finite(values);
  const n = v.length;
  if (n < 3) {
    return { family: 'unknown', params: {}, ksStatistic: Number.NaN };
  }

  // Normal fit
  const muN = mean(v);
  const sigmaN = stddev(v);
  const ksNormal =
    sigmaN > 0
      ? kolmogorovSmirnov(v, (x) => normalCdf((x - muN) / sigmaN))
      : Number.POSITIVE_INFINITY;

  // Lognormal fit — only meaningful for strictly positive samples.
  let ksLog = Number.POSITIVE_INFINITY;
  let muL = Number.NaN;
  let sigmaL = Number.NaN;
  const allPositive = v.every((x) => x > 0);
  if (allPositive) {
    const logs = v.map((x) => Math.log(x));
    muL = mean(logs);
    sigmaL = stddev(logs);
    if (sigmaL > 0) {
      ksLog = kolmogorovSmirnov(v, (x) =>
        x <= 0 ? 0 : normalCdf((Math.log(x) - muL) / sigmaL),
      );
    }
  }

  // Uniform fit — [min, max]
  let minU = v[0];
  let maxU = v[0];
  for (const x of v) {
    if (x < minU) minU = x;
    if (x > maxU) maxU = x;
  }
  const ksUniform =
    maxU > minU
      ? kolmogorovSmirnov(v, (x) => {
          if (x <= minU) return 0;
          if (x >= maxU) return 1;
          return (x - minU) / (maxU - minU);
        })
      : Number.POSITIVE_INFINITY;

  // Pick the best — lowest KS distance wins.
  const candidates: DistributionFit[] = [
    { family: 'normal', params: { mu: muN, sigma: sigmaN }, ksStatistic: ksNormal },
    ...(allPositive
      ? [
          {
            family: 'lognormal' as const,
            params: { mu: muL, sigma: sigmaL },
            ksStatistic: ksLog,
          },
        ]
      : []),
    { family: 'uniform', params: { min: minU, max: maxU }, ksStatistic: ksUniform },
  ];
  candidates.sort((a, b) => a.ksStatistic - b.ksStatistic);
  const best = candidates[0];

  // Reject when best KS exceeds the asymptotic α=0.05 critical value —
  // `'unknown'` signals the viewer to surface a neutral chip rather than
  // misleading the user with a bad fit.
  const threshold = 1.36 / Math.sqrt(n);
  if (!Number.isFinite(best.ksStatistic) || best.ksStatistic > threshold) {
    return { family: 'unknown', params: best.params, ksStatistic: best.ksStatistic };
  }
  return best;
}

// ── One-sided t-test ────────────────────────────────────────────────

/** One-sided t-test result. */
export interface TTestResult {
  tStatistic: number;
  pValue: number;
  df: number;
}

/**
 * One-sided (upper-tail) one-sample t-test.
 *
 * H0: population mean == h0Mean.
 * H1: population mean >  h0Mean.
 *
 * `pValue` is the probability of observing a t as extreme as (or more
 * than) the sample's under H0. For empty / single-element samples all
 * three fields are NaN — the viewer renders the metric as a dash.
 */
export function tTestOneSided(sample: number[], h0Mean: number): TTestResult {
  const v = finite(sample);
  const n = v.length;
  if (n < 2) {
    return { tStatistic: Number.NaN, pValue: Number.NaN, df: Number.NaN };
  }
  const mu = mean(v);
  const sd = stddev(v);
  const sem = sd / Math.sqrt(n);
  if (sem === 0 || !Number.isFinite(sem)) {
    return { tStatistic: Number.NaN, pValue: Number.NaN, df: n - 1 };
  }
  const t = (mu - h0Mean) / sem;
  // Upper-tail p.
  const p = 1 - tCdf(t, n - 1);
  return { tStatistic: t, pValue: p, df: n - 1 };
}

// ── Q-Q helpers ─────────────────────────────────────────────────────

/**
 * Compute observed-vs-expected quantile pairs for a Q-Q plot against a
 * reference CDF's quantile function.
 *
 * Used by `QQPlot` — a rendering-agnostic numeric helper that lets the
 * plot itself stay a thin SVG component.
 */
export function qqPoints(
  values: number[],
  inverseCdf: (p: number) => number,
): { observed: number; expected: number }[] {
  const v = finite(values);
  const n = v.length;
  if (n === 0) return [];
  const sorted = [...v].sort((a, b) => a - b);
  const out: { observed: number; expected: number }[] = [];
  for (let i = 0; i < n; i++) {
    // Plotting positions (i + 0.5)/n — the "mid-point" convention.
    const p = (i + 0.5) / n;
    const expected = inverseCdf(p);
    out.push({ observed: sorted[i], expected });
  }
  return out;
}

/**
 * Deterministic Mulberry32 PRNG. Exposed for tests (callers pass their
 * own RNG to `bootstrapCI`). Returns a closure yielding [0, 1).
 */
export function createSeededRng(seed: number): () => number {
  let a = seed | 0;
  return () => {
    a = (a + 0x6d2b79f5) | 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
