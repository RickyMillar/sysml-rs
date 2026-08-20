/**
 * morrisSample — Morris Elementary Effects trajectory generator (R7.4).
 *
 * Pure, deterministic mirror of the Rust `morris_trajectories` backend
 * helper. Exposing it on the frontend lets the UI:
 *
 *   - preview how many runs the proposed `r` / `p` combination will
 *     generate before the user clicks Run (important for Morris because
 *     the run count scales as `r · (d + 1)`),
 *   - unit-test the full round-trip with the `computeMorrisIndices`
 *     helper without a backend, and
 *   - carry out the Ishigami-style synthetic-function test (see
 *     `__tests__/morrisSample.test.ts`) against a known answer.
 *
 * The algorithm matches Morris (1991) + Campolongo (2007): each
 * trajectory starts at a random base level in the lower half of the
 * discrete p-level grid, then perturbs one parameter at a time by
 * `Δ = p / (2(p-1))` in normalised [0, 1] space.
 *
 * Row order is trajectory-major: r trajectories × (d+1) points each.
 * Within a trajectory, consecutive rows differ in exactly one parameter
 * — that invariant is what `computeMorrisIndices` relies on to attribute
 * each elementary effect to the right parameter.
 */

import type { ParamRange } from '@/engine/types';

// ── Deterministic RNG ───────────────────────────────────────────────
//
// splitmix64-style — portable, seed-stable. Not cryptographic; fine for
// design-matrix generation where reproducibility is the requirement.

function splitmix64(state: bigint): { state: bigint; value: bigint } {
  const nextState = (state + 0x9e3779b97f4a7c15n) & 0xffffffffffffffffn;
  let z = nextState;
  z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & 0xffffffffffffffffn;
  z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & 0xffffffffffffffffn;
  z = z ^ (z >> 31n);
  return { state: nextState, value: z };
}

class Rng {
  private state: bigint;

  constructor(seed: number) {
    // BigInt cast keeps everything well-defined even for negative /
    // large seeds; the wrap keeps us in 64-bit range.
    this.state =
      (BigInt(Math.floor(seed)) + 0x9e3779b97f4a7c15n) &
      0xffffffffffffffffn;
  }

  nextU64(): bigint {
    const r = splitmix64(this.state);
    this.state = r.state;
    return r.value;
  }

  /** Uniform [0, 1) double — top 53 bits / 2^53. */
  nextUnit(): number {
    const v = Number(this.nextU64() >> 11n);
    return v / 2 ** 53;
  }

  /** Uniform integer in [0, n). */
  genRange(n: number): number {
    if (n <= 1) return 0;
    return Number(this.nextU64() % BigInt(n));
  }
}

// ── Core API ────────────────────────────────────────────────────────

export interface MorrisSampleOptions {
  /** Number of trajectories. Total runs = `r * (d + 1)`. */
  r: number;
  /** Level count (discretisation of normalised [0, 1]). */
  p: number;
  /** RNG seed — determinism pin. */
  seed: number;
}

/**
 * Generate `r` Morris trajectories. Returns `r · (d + 1)` rows, each a
 * length-`d` vector of parameter values in their original (unmapped)
 * ranges.
 *
 * Trajectory-major order: trajectory 0's (d+1) points come first, then
 * trajectory 1, etc. Within a trajectory consecutive rows differ in
 * exactly one parameter.
 */
export function morrisSample(
  params: readonly ParamRange[],
  opts: MorrisSampleOptions,
): number[][] {
  const { r, p, seed } = opts;
  const d = params.length;
  if (d === 0) throw new Error('morris requires at least one parameter');
  if (p < 2) throw new Error('morris requires p >= 2 levels');
  if (r < 1) throw new Error('morris requires r >= 1 trajectories');

  const deltaNorm = p / (2 * (p - 1));
  const half = Math.floor(p / 2);
  const baseLevels: number[] = [];
  for (let k = 0; k < half; k++) {
    baseLevels.push(k / (p - 1));
  }

  const rng = new Rng(seed);
  const out: number[][] = [];

  for (let t = 0; t < r; t++) {
    // Base point in normalised space.
    const xNorm = new Array<number>(d);
    for (let i = 0; i < d; i++) {
      xNorm[i] = baseLevels[rng.genRange(baseLevels.length)];
    }

    // Fisher-Yates permutation of 0..d-1.
    const perm = Array.from({ length: d }, (_, i) => i);
    for (let i = d - 1; i >= 1; i--) {
      const j = rng.genRange(i + 1);
      [perm[i], perm[j]] = [perm[j], perm[i]];
    }

    // Emit base point.
    out.push(unmapRow(params, xNorm));

    // Walk the permutation; perturb one parameter at each step.
    for (const pi of perm) {
      xNorm[pi] = Math.min(1, xNorm[pi] + deltaNorm);
      out.push(unmapRow(params, xNorm));
    }
  }

  return out;
}

function unmapRow(params: readonly ParamRange[], norm: readonly number[]): number[] {
  const row = new Array<number>(params.length);
  for (let i = 0; i < params.length; i++) {
    const pr = params[i];
    row[i] = pr.min + (pr.max - pr.min) * norm[i];
  }
  return row;
}

/**
 * Convert a list of parameter-value rows into the `children_params`
 * JSON payload consumed by `sysml.batch.create { kind: "sensitivity", ... }`.
 */
export function toChildrenParams(
  params: readonly ParamRange[],
  rows: readonly number[][],
): Array<Record<string, number>> {
  return rows.map((row) => {
    const obj: Record<string, number> = {};
    for (let i = 0; i < params.length; i++) {
      obj[params[i].name] = row[i];
    }
    return obj;
  });
}

/**
 * Morris Elementary Effects index computation — pure mirror of the Rust
 * `compute_morris_indices` helper.
 *
 * Given the trajectory rows (in the exact order emitted by
 * `morrisSample`) and the corresponding scalar output `y[i]` per row,
 * compute per-parameter μ* (mean of absolute EEs) and σ (stddev of EEs).
 */
export function computeMorrisIndices(
  params: readonly ParamRange[],
  trajectoryRows: readonly number[][],
  y: readonly number[],
  p: number,
): Array<{ name: string; mu: number; sigma: number }> {
  const d = params.length;
  const perTraj = d + 1;
  if (trajectoryRows.length % perTraj !== 0) {
    throw new Error(
      `trajectory row count ${trajectoryRows.length} not divisible by (d+1) = ${perTraj}`,
    );
  }
  if (trajectoryRows.length !== y.length) {
    throw new Error(
      `y (${y.length}) must match trajectory rows (${trajectoryRows.length})`,
    );
  }
  if (p < 2) throw new Error('p must be >= 2');
  const r = trajectoryRows.length / perTraj;
  const deltaNorm = p / (2 * (p - 1));

  const ees: number[][] = params.map(() => []);

  for (let t = 0; t < r; t++) {
    const base = t * perTraj;
    for (let step = 0; step < d; step++) {
      const i0 = base + step;
      const i1 = base + step + 1;
      // Detect which parameter moved.
      let changed = 0;
      let maxDiff = 0;
      for (let k = 0; k < d; k++) {
        const diff = Math.abs(trajectoryRows[i1][k] - trajectoryRows[i0][k]);
        if (diff > maxDiff) {
          maxDiff = diff;
          changed = k;
        }
      }
      if (maxDiff === 0) continue;
      const range = params[changed].max - params[changed].min;
      if (range === 0) continue;
      const ee = (y[i1] - y[i0]) / (deltaNorm * range);
      ees[changed].push(ee);
    }
  }

  return params.map((pr, i) => ({
    name: pr.name,
    mu: meanAbs(ees[i]),
    sigma: stddev(ees[i]),
  }));
}

function meanAbs(xs: readonly number[]): number {
  if (xs.length === 0) return 0;
  let s = 0;
  for (const v of xs) s += Math.abs(v);
  return s / xs.length;
}

function mean(xs: readonly number[]): number {
  if (xs.length === 0) return 0;
  let s = 0;
  for (const v of xs) s += v;
  return s / xs.length;
}

function stddev(xs: readonly number[]): number {
  if (xs.length < 2) return 0;
  const m = mean(xs);
  let s = 0;
  for (const v of xs) s += (v - m) ** 2;
  return Math.sqrt(s / xs.length);
}
