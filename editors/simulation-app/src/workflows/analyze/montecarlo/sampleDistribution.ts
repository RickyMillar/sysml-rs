/**
 * sampleDistribution — pure samplers for the Monte Carlo config panel.
 *
 * Every sampler is a total function of `(params, rng)` where `rng` is a
 * callable `() => number` yielding a uniform sample in `[0, 1)`. Tests
 * inject a deterministic RNG via `seedableRng(seed)`; production code
 * can pass the same helper to reproduce a run exactly from a single
 * integer seed.
 *
 * Public surface:
 *   - `seedableRng(seed?)` — Mulberry32 uniform `[0, 1)` generator.
 *   - `sampleNormal`  — Box–Muller (two-uniforms → one normal).
 *   - `sampleUniform` — inverse-CDF on `[min, max)`.
 *   - `sampleTriangular` — standard `min/mode/max` inverse-CDF.
 *   - `sampleCustomCdf` — user-supplied monotone CDF, linear interp.
 *   - `parseCustomCdfPoints` — parse the textarea format
 *       `"x, cdf\nx, cdf"` used by `DistributionEditor` into a typed
 *       array; throws on non-monotone / missing endpoints.
 *   - `generateChildrenParams(distributions, count, seed)` — compose
 *     the samplers for every selected parameter into a flat `Array<
 *     Record<string, number>>` that the backend consumes as the
 *     `children_params` list for `sysml.batch.create`.
 *
 * Design notes:
 *   - Everything here is side-effect-free and deterministic given a
 *     seed. Do NOT call `Math.random` from this module.
 *   - Errors throw `Error` with a short reason; the config panel
 *     surfaces these inline under the distribution editor.
 *   - Distribution types are closed unions; adding a new kind means
 *     updating `Distribution` + `sampleDistribution` + the editor.
 */

// ── Seedable RNG ────────────────────────────────────────────────────

/**
 * Mulberry32 — 32-bit seedable PRNG yielding a uniform `[0, 1)`. Chosen
 * for brevity + decent statistical properties at this scale (< 100k
 * draws). Good enough for Monte Carlo configuration; NOT cryptographic.
 *
 * Same seed → identical sequence across platforms / JS engines.
 */
export function seedableRng(seed?: number): () => number {
  // Accept undefined → time-based randomness; accept any int → folded
  // into a 32-bit state. We intentionally fold negatives and fractions
  // to avoid surprising NaN states.
  let state: number;
  if (seed === undefined || seed === null || Number.isNaN(seed)) {
    state = (Math.random() * 0x1_0000_0000) >>> 0;
  } else {
    state = Math.floor(Math.abs(seed)) >>> 0;
    if (state === 0) state = 1; // Mulberry32 doesn't love zero
  }

  return function next(): number {
    // The canonical Mulberry32 step.
    state = (state + 0x6d2b_79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

// ── Core samplers ───────────────────────────────────────────────────

/**
 * Box–Muller normal sampler. Consumes TWO draws from `rng` per output
 * (standard). We drop the second to keep the API simple — callers that
 * need tight entropy budgets should swap in Marsaglia polar later.
 */
export function sampleNormal(
  mean: number,
  sigma: number,
  rng: () => number,
): number {
  if (!Number.isFinite(mean)) throw new Error('sampleNormal: mean must be finite');
  if (!Number.isFinite(sigma)) throw new Error('sampleNormal: sigma must be finite');
  if (sigma < 0) throw new Error('sampleNormal: sigma must be >= 0');
  // Guard against u1 === 0 (log(0) = -Infinity) by clamping away from 0.
  const u1 = Math.max(rng(), Number.EPSILON);
  const u2 = rng();
  const r = Math.sqrt(-2 * Math.log(u1));
  const theta = 2 * Math.PI * u2;
  return mean + sigma * r * Math.cos(theta);
}

/** Uniform sample on `[min, max)`. `min <= max` required. */
export function sampleUniform(
  min: number,
  max: number,
  rng: () => number,
): number {
  if (!Number.isFinite(min) || !Number.isFinite(max)) {
    throw new Error('sampleUniform: bounds must be finite');
  }
  if (min > max) throw new Error('sampleUniform: min must be <= max');
  return min + (max - min) * rng();
}

/**
 * Triangular distribution inverse-CDF. Requires `min <= mode <= max`
 * with `min < max` (otherwise the distribution degenerates).
 */
export function sampleTriangular(
  min: number,
  mode: number,
  max: number,
  rng: () => number,
): number {
  if (!Number.isFinite(min) || !Number.isFinite(mode) || !Number.isFinite(max)) {
    throw new Error('sampleTriangular: bounds must be finite');
  }
  if (!(min <= mode && mode <= max)) {
    throw new Error('sampleTriangular: require min <= mode <= max');
  }
  if (min === max) return min;
  const u = rng();
  const f = (mode - min) / (max - min);
  if (u < f) {
    return min + Math.sqrt(u * (max - min) * (mode - min));
  }
  return max - Math.sqrt((1 - u) * (max - min) * (max - mode));
}

// ── Custom CDF ──────────────────────────────────────────────────────

/** A single (x, cumulative-probability) point on a user CDF. */
export interface CdfPoint {
  x: number;
  cdf: number;
}

/**
 * Inverse-CDF sampling over a user-supplied piecewise-linear CDF.
 * Requires the CDF to:
 *   - contain at least 2 points;
 *   - be monotone non-decreasing in `cdf`;
 *   - start at `cdf === 0` and end at `cdf === 1` (we rescale the
 *     endpoints when they are finite positive numbers but not exactly
 *     0/1, so users don't have to round).
 *
 * Throws on malformed input (non-monotone, NaN, <2 points).
 */
export function sampleCustomCdf(
  cdfPoints: readonly CdfPoint[],
  rng: () => number,
): number {
  if (cdfPoints.length < 2) {
    throw new Error('sampleCustomCdf: need at least 2 points');
  }
  const pts = normalizeCdfPoints(cdfPoints);
  const u = rng();
  // Find the segment `[cdf[i], cdf[i+1])` containing u. Linear scan is
  // fine for typical editor sizes (< 50 points).
  for (let i = 0; i < pts.length - 1; i++) {
    const a = pts[i];
    const b = pts[i + 1];
    if (u >= a.cdf && u <= b.cdf) {
      const span = b.cdf - a.cdf;
      if (span === 0) return a.x; // flat segment → snap to lower x
      const t = (u - a.cdf) / span;
      return a.x + t * (b.x - a.x);
    }
  }
  // Numeric slop: u === 1 and the last cdf slightly < 1.
  return pts[pts.length - 1].x;
}

/**
 * Parse the textarea format used by the custom-cdf editor:
 *
 *   `0, 0`
 *   `10, 0.5`
 *   `20, 1`
 *
 * Accepts whitespace / trailing commas, ignores blank lines, strips
 * `#` comments. Returns `CdfPoint[]` sorted by `x`. Throws on
 * malformed / non-monotone input.
 */
export function parseCustomCdfPoints(text: string): CdfPoint[] {
  if (!text || !text.trim()) {
    throw new Error('Custom CDF is empty');
  }
  const pts: CdfPoint[] = [];
  const lines = text.split('\n');
  for (let lineNo = 0; lineNo < lines.length; lineNo++) {
    const raw = lines[lineNo];
    const stripped = raw.replace(/#.*$/, '').trim();
    if (!stripped) continue;
    const parts = stripped.split(/[,\s]+/).filter(Boolean);
    if (parts.length !== 2) {
      throw new Error(
        `Line ${lineNo + 1}: expected "x, cdf" (got "${raw.trim()}")`,
      );
    }
    const x = Number(parts[0]);
    const cdf = Number(parts[1]);
    if (!Number.isFinite(x) || !Number.isFinite(cdf)) {
      throw new Error(`Line ${lineNo + 1}: non-numeric entry`);
    }
    pts.push({ x, cdf });
  }
  if (pts.length < 2) {
    throw new Error('Custom CDF needs at least 2 points');
  }
  return normalizeCdfPoints(pts);
}

/** Validate + normalize: sort by x, confirm monotone in cdf, rescale endpoints. */
function normalizeCdfPoints(points: readonly CdfPoint[]): CdfPoint[] {
  const sorted = [...points].sort((a, b) => a.x - b.x);
  for (let i = 1; i < sorted.length; i++) {
    if (sorted[i].cdf < sorted[i - 1].cdf) {
      throw new Error('Custom CDF must be monotone non-decreasing in cdf');
    }
  }
  const first = sorted[0];
  const last = sorted[sorted.length - 1];
  if (first.cdf < 0 || last.cdf <= first.cdf) {
    throw new Error('Custom CDF must span a positive probability range');
  }
  // Rescale so endpoints are exactly 0 and 1 (tolerate minor user slop).
  const span = last.cdf - first.cdf;
  const offset = first.cdf;
  return sorted.map((p) => ({
    x: p.x,
    cdf: (p.cdf - offset) / span,
  }));
}

// ── Distribution union ──────────────────────────────────────────────

/** Tag for the distribution kind — matches the dropdown in the editor. */
export type DistributionKind = 'normal' | 'uniform' | 'triangular' | 'custom-cdf';

export interface NormalDist {
  kind: 'normal';
  mean: number;
  sigma: number;
}

export interface UniformDist {
  kind: 'uniform';
  min: number;
  max: number;
}

export interface TriangularDist {
  kind: 'triangular';
  min: number;
  mode: number;
  max: number;
}

export interface CustomCdfDist {
  kind: 'custom-cdf';
  /** Raw textarea content — parsed just-in-time so the UI can show errors. */
  raw: string;
  /** Parsed points (set by the editor once `raw` validates). */
  points: readonly CdfPoint[];
}

export type Distribution =
  | NormalDist
  | UniformDist
  | TriangularDist
  | CustomCdfDist;

/**
 * Dispatch a single sample from any distribution. Throws if the
 * distribution is malformed (e.g. an unparsed CustomCdfDist without
 * points).
 */
export function sampleDistribution(dist: Distribution, rng: () => number): number {
  switch (dist.kind) {
    case 'normal':
      return sampleNormal(dist.mean, dist.sigma, rng);
    case 'uniform':
      return sampleUniform(dist.min, dist.max, rng);
    case 'triangular':
      return sampleTriangular(dist.min, dist.mode, dist.max, rng);
    case 'custom-cdf':
      return sampleCustomCdf(dist.points, rng);
  }
}

/**
 * True when a distribution has valid parameters and can be sampled.
 * Used by the config panel to gate the Run button.
 */
export function isDistributionValid(dist: Distribution): boolean {
  try {
    switch (dist.kind) {
      case 'normal':
        return Number.isFinite(dist.mean) && Number.isFinite(dist.sigma) && dist.sigma >= 0;
      case 'uniform':
        return Number.isFinite(dist.min) && Number.isFinite(dist.max) && dist.min <= dist.max;
      case 'triangular':
        return (
          Number.isFinite(dist.min) &&
          Number.isFinite(dist.mode) &&
          Number.isFinite(dist.max) &&
          dist.min <= dist.mode &&
          dist.mode <= dist.max
        );
      case 'custom-cdf':
        if (!dist.points || dist.points.length < 2) return false;
        // Re-run the normalisation check; throws on bad input.
        normalizeCdfPoints(dist.points);
        return true;
    }
  } catch {
    return false;
  }
}

/**
 * Build a default Distribution for a given kind. Used by the editor
 * when the user picks a new kind from the dropdown.
 */
export function defaultDistribution(kind: DistributionKind): Distribution {
  switch (kind) {
    case 'normal':
      return { kind: 'normal', mean: 0, sigma: 1 };
    case 'uniform':
      return { kind: 'uniform', min: 0, max: 1 };
    case 'triangular':
      return { kind: 'triangular', min: 0, mode: 0.5, max: 1 };
    case 'custom-cdf':
      return {
        kind: 'custom-cdf',
        raw: '0, 0\n1, 1',
        points: [
          { x: 0, cdf: 0 },
          { x: 1, cdf: 1 },
        ],
      };
  }
}

// ── Child-param generator ───────────────────────────────────────────

/**
 * Map of parameter name → `Distribution` the user wants to sample. The
 * same order is preserved in the output rows.
 */
export type DistributionMap = Readonly<Record<string, Distribution>>;

/**
 * Sample `count` child-param records. Shape:
 *
 *   `[ { paramA: 0.12, paramB: 4.3 }, { paramA: …, paramB: … }, … ]`
 *
 * The first argument's key insertion order dictates the key order of
 * every output record.
 *
 * Deterministic given `seed`: same `seed` + same `distributions` +
 * same `count` → identical output. Integers are the canonical seed
 * form; `undefined` means "use an ephemeral seed".
 */
export function generateChildrenParams(
  distributions: DistributionMap,
  count: number,
  seed?: number,
): Array<Record<string, number>> {
  if (!Number.isFinite(count) || count < 0) {
    throw new Error('generateChildrenParams: count must be a non-negative integer');
  }
  const n = Math.floor(count);
  if (n === 0) return [];

  const paramNames = Object.keys(distributions);
  if (paramNames.length === 0) return [];

  // Validate up-front so all rows are good or we throw cleanly.
  for (const name of paramNames) {
    const dist = distributions[name];
    if (!isDistributionValid(dist)) {
      throw new Error(
        `generateChildrenParams: parameter "${name}" has an invalid distribution`,
      );
    }
  }

  const rng = seedableRng(seed);
  const rows: Array<Record<string, number>> = [];
  for (let i = 0; i < n; i++) {
    const row: Record<string, number> = {};
    for (const name of paramNames) {
      row[name] = sampleDistribution(distributions[name], rng);
    }
    rows.push(row);
  }
  return rows;
}
