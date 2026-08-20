/**
 * sobolSample — Sobol variance-based sample matrix generator (R7.4).
 *
 * Pure, deterministic mirror of the Rust `sobol_sample_matrix` /
 * `compute_sobol_indices` helpers. Uses Saltelli's 2002 crossing-matrix
 * method: two base matrices A, B (N × d) plus `d` mix matrices C_i
 * where column `i` of C_i is drawn from B and every other column from
 * A. Total runs if concatenated: `N · (d + 2)`.
 *
 * Estimators used by `computeSobolIndices` (matches the Rust backend):
 *
 *   Var   = mean((A∪B - mean(A∪B))²)         (2N-sample variance)
 *   S_i   = mean(y_B · (y_C_i − y_A)) / Var  (Saltelli 2002)
 *   S_Ti  = mean((y_A − y_C_i)²) / (2·Var)   (Jansen 1999)
 *
 * Note S_T ≥ S by construction (except for noise at finite N); the
 * tests pin this invariant on an Ishigami run.
 */

import type { ParamRange } from '@/engine/types';

// ── RNG (matches morrisSample.ts; copy rather than cross-import to
//     keep each pure helper self-contained and independently bundled). ──

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

  constructor(seed: bigint) {
    this.state = (seed + 0x9e3779b97f4a7c15n) & 0xffffffffffffffffn;
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
}

// ── Core API ────────────────────────────────────────────────────────

export interface SobolSampleOptions {
  /** Base sample size N. Total runs = `N · (d + 2)`. */
  n: number;
  /** RNG seed — determinism pin. */
  seed: number;
}

export interface SobolMatrices {
  /** Base matrix A: N rows × d parameter values. */
  a: number[][];
  /** Base matrix B (independent of A). */
  b: number[][];
  /** `d` mix matrices; `c[i]` is A with column `i` replaced by B's. */
  c: number[][][];
}

/** Generate the Saltelli (A, B, C_i) design in original parameter ranges. */
export function sobolSample(
  params: readonly ParamRange[],
  opts: SobolSampleOptions,
): SobolMatrices {
  const { n, seed } = opts;
  const d = params.length;
  if (d === 0) throw new Error('sobol requires at least one parameter');
  if (n < 1) throw new Error('sobol requires N >= 1');

  // Decorrelated RNGs for A and B. Uses XOR of a constant like the Rust
  // side so the two base matrices are statistically independent.
  const rngA = new Rng(BigInt(Math.floor(seed)));
  const rngB = new Rng(BigInt(Math.floor(seed)) ^ 0xdeadbeefcafef00dn);

  const aNorm: number[][] = [];
  const bNorm: number[][] = [];
  for (let j = 0; j < n; j++) {
    const rowA = new Array<number>(d);
    const rowB = new Array<number>(d);
    for (let i = 0; i < d; i++) {
      rowA[i] = rngA.nextUnit();
      rowB[i] = rngB.nextUnit();
    }
    aNorm.push(rowA);
    bNorm.push(rowB);
  }

  const a = aNorm.map((row) => unmapRow(params, row));
  const b = bNorm.map((row) => unmapRow(params, row));

  const c: number[][][] = [];
  for (let i = 0; i < d; i++) {
    const ci: number[][] = [];
    for (let j = 0; j < n; j++) {
      const row = aNorm[j].slice();
      row[i] = bNorm[j][i];
      ci.push(unmapRow(params, row));
    }
    c.push(ci);
  }

  return { a, b, c };
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
 * Concatenate the Sobol design into the batch-create row order:
 * `[A_0..A_{N-1}, B_0..B_{N-1}, C_0_0..C_{d-1}_{N-1}]`.
 */
export function sobolConcat(m: SobolMatrices): number[][] {
  const out: number[][] = [];
  out.push(...m.a);
  out.push(...m.b);
  for (const ci of m.c) out.push(...ci);
  return out;
}

/** Same shape as `toChildrenParams` on the Morris helper. */
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
 * Compute first-order (`s1`) and total-order (`st`) Sobol indices.
 *
 * Uses a 2N-sample variance estimate (A ∪ B) for lower noise at small
 * N, Saltelli's B·(C−A) for S_i, and Jansen's (A−C)²/2 for S_Ti.
 */
export function computeSobolIndices(
  params: readonly ParamRange[],
  yA: readonly number[],
  yB: readonly number[],
  yCByParam: readonly (readonly number[])[],
): Array<{ name: string; s1: number; st: number }> {
  const d = params.length;
  const n = yA.length;
  if (yB.length !== n)
    throw new Error(`yB length ${yB.length} != yA length ${n}`);
  if (yCByParam.length !== d)
    throw new Error(`yC blocks ${yCByParam.length} != d ${d}`);
  for (const ci of yCByParam) {
    if (ci.length !== n)
      throw new Error(`yC block length ${ci.length} != yA length ${n}`);
  }
  if (n === 0) throw new Error('sobol N must be > 0');

  const combined: number[] = [];
  for (const v of yA) combined.push(v);
  for (const v of yB) combined.push(v);
  const meanY = mean(combined);
  let varY = 0;
  for (const v of combined) varY += (v - meanY) ** 2;
  varY /= combined.length;

  if (varY === 0 || !Number.isFinite(varY)) {
    return params.map((pr) => ({ name: pr.name, s1: 0, st: 0 }));
  }

  return params.map((pr, i) => {
    const yCi = yCByParam[i];
    // S_i = (1/N) Σ y_B · (y_C_i − y_A) / Var   (Saltelli 2002)
    let s1Num = 0;
    for (let j = 0; j < n; j++) s1Num += yB[j] * (yCi[j] - yA[j]);
    s1Num /= n;
    const s1 = s1Num / varY;

    // S_Ti = (1/(2N)) Σ (y_A − y_C_i)² / Var     (Jansen 1999)
    let stNum = 0;
    for (let j = 0; j < n; j++) stNum += (yA[j] - yCi[j]) ** 2;
    stNum /= 2 * n;
    const st = stNum / varY;

    return { name: pr.name, s1, st };
  });
}

function mean(xs: readonly number[]): number {
  if (xs.length === 0) return 0;
  let s = 0;
  for (const v of xs) s += v;
  return s / xs.length;
}
