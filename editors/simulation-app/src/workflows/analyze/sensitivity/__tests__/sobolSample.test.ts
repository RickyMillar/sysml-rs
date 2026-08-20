import { describe, it, expect } from 'vitest';
import type { ParamRange } from '@/engine/types';
import {
  sobolSample,
  sobolConcat,
  toChildrenParams,
  computeSobolIndices,
} from '../sobolSample';

const threeParams = (): ParamRange[] => [
  { name: 'a', min: 0, max: 1 },
  { name: 'b', min: 0, max: 1 },
  { name: 'c', min: 0, max: 1 },
];

describe('sobolSample', () => {
  it('produces matrices of the expected shapes', () => {
    const params = threeParams();
    const n = 16;
    const d = params.length;
    const { a, b, c } = sobolSample(params, { n, seed: 42 });
    expect(a).toHaveLength(n);
    expect(b).toHaveLength(n);
    expect(c).toHaveLength(d);
    for (const ci of c) expect(ci).toHaveLength(n);
    for (const row of a) expect(row).toHaveLength(d);
  });

  it('C_i mixes column i from B; other columns come from A', () => {
    const params = threeParams();
    const n = 8;
    const d = params.length;
    const { a, b, c } = sobolSample(params, { n, seed: 42 });
    for (let i = 0; i < d; i++) {
      for (let j = 0; j < n; j++) {
        for (let k = 0; k < d; k++) {
          const got = c[i][j][k];
          const expected = k === i ? b[j][k] : a[j][k];
          expect(Math.abs(got - expected)).toBeLessThan(1e-12);
        }
      }
    }
  });

  it('is deterministic under a fixed seed', () => {
    const params = threeParams();
    const a = sobolSample(params, { n: 8, seed: 13 });
    const b = sobolSample(params, { n: 8, seed: 13 });
    expect(a.a).toEqual(b.a);
    expect(a.b).toEqual(b.b);
    expect(a.c).toEqual(b.c);
  });

  it('sobolConcat uses [A; B; C_0..C_{d-1}] order', () => {
    const params = threeParams();
    const n = 4;
    const d = params.length;
    const mats = sobolSample(params, { n, seed: 1 });
    const cat = sobolConcat(mats);
    expect(cat).toHaveLength(n + n + d * n);
    expect(cat.slice(0, n)).toEqual(mats.a);
    expect(cat.slice(n, 2 * n)).toEqual(mats.b);
  });

  it('toChildrenParams maps rows into {name: value} objects', () => {
    const params = [
      { name: 'x', min: 0, max: 1 },
      { name: 'y', min: 0, max: 1 },
    ];
    const rows = [
      [0.1, 0.2],
      [0.3, 0.4],
    ];
    expect(toChildrenParams(params, rows)).toEqual([
      { x: 0.1, y: 0.2 },
      { x: 0.3, y: 0.4 },
    ]);
  });
});

describe('computeSobolIndices — Ishigami', () => {
  // Ishigami function — canonical Sobol test problem.
  // f(x1, x2, x3) = sin(x1) + 7·sin(x2)^2 + 0.1·x3^4·sin(x1)
  // with x_i ~ U(-π, π).
  // Analytical: S_1≈0.3139, S_2≈0.4424, S_3=0;
  //             S_T1≈0.5576, S_T2≈0.4424, S_T3≈0.2437.
  const ishigami = (x: readonly number[]): number =>
    Math.sin(x[0]) + 7 * Math.sin(x[1]) ** 2 + 0.1 * x[2] ** 4 * Math.sin(x[0]);

  it('converges toward the analytical indices at N=2048', () => {
    const pi = Math.PI;
    const params: ParamRange[] = [
      { name: 'x1', min: -pi, max: pi },
      { name: 'x2', min: -pi, max: pi },
      { name: 'x3', min: -pi, max: pi },
    ];
    const n = 2048;
    const { a, b, c } = sobolSample(params, { n, seed: 42 });
    const yA = a.map(ishigami);
    const yB = b.map(ishigami);
    const yC = c.map((ci) => ci.map(ishigami));
    const out = computeSobolIndices(params, yA, yB, yC);
    expect(out).toHaveLength(3);

    const s1Ref = [0.3139, 0.4424, 0.0];
    const stRef = [0.5576, 0.4424, 0.2437];

    for (let i = 0; i < 3; i++) {
      expect(Math.abs(out[i].s1 - s1Ref[i])).toBeLessThan(0.15);
      expect(Math.abs(out[i].st - stRef[i])).toBeLessThan(0.2);
      // S_T ≥ S within noise tolerance.
      expect(out[i].st).toBeGreaterThan(out[i].s1 - 0.05);
    }

    // x3 has zero first-order, nonzero total-order.
    expect(Math.abs(out[2].s1)).toBeLessThan(0.15);
    expect(out[2].st).toBeGreaterThan(0.05);
  });

  it('returns zero indices for a constant output', () => {
    const params = threeParams();
    const n = 16;
    const yA = Array.from({ length: n }, () => 1);
    const yB = Array.from({ length: n }, () => 1);
    const yC = params.map(() => Array.from({ length: n }, () => 1));
    const out = computeSobolIndices(params, yA, yB, yC);
    for (const r of out) {
      expect(r.s1).toBe(0);
      expect(r.st).toBe(0);
    }
  });
});
