import { describe, it, expect } from 'vitest';
import type { ParamRange } from '@/engine/types';
import {
  morrisSample,
  toChildrenParams,
  computeMorrisIndices,
} from '../morrisSample';

const threeParams = (): ParamRange[] => [
  { name: 'a', min: 0, max: 1 },
  { name: 'b', min: 0, max: 1 },
  { name: 'c', min: 0, max: 1 },
];

describe('morrisSample', () => {
  it('produces r*(d+1) rows of length d', () => {
    const params = threeParams();
    const rows = morrisSample(params, { r: 5, p: 4, seed: 42 });
    expect(rows).toHaveLength(5 * (3 + 1));
    for (const row of rows) {
      expect(row).toHaveLength(3);
    }
  });

  it('consecutive rows within a trajectory differ in exactly one parameter', () => {
    const params = threeParams();
    const r = 4;
    const p = 4;
    const d = params.length;
    const rows = morrisSample(params, { r, p, seed: 1234 });
    for (let t = 0; t < r; t++) {
      const base = t * (d + 1);
      for (let step = 0; step < d; step++) {
        const a = rows[base + step];
        const b = rows[base + step + 1];
        let diff = 0;
        for (let k = 0; k < d; k++) if (Math.abs(a[k] - b[k]) > 1e-12) diff++;
        expect(diff).toBe(1);
      }
    }
  });

  it('is deterministic under a fixed seed', () => {
    const params = threeParams();
    const a = morrisSample(params, { r: 4, p: 4, seed: 7 });
    const b = morrisSample(params, { r: 4, p: 4, seed: 7 });
    expect(a).toEqual(b);
    const c = morrisSample(params, { r: 4, p: 4, seed: 8 });
    expect(a).not.toEqual(c);
  });

  it('respects the param range bounds', () => {
    const params: ParamRange[] = [
      { name: 'x', min: -2, max: 10 },
      { name: 'y', min: 1, max: 3 },
    ];
    const rows = morrisSample(params, { r: 3, p: 4, seed: 99 });
    for (const row of rows) {
      expect(row[0]).toBeGreaterThanOrEqual(-2 - 1e-9);
      expect(row[0]).toBeLessThanOrEqual(10 + 1e-9);
      expect(row[1]).toBeGreaterThanOrEqual(1 - 1e-9);
      expect(row[1]).toBeLessThanOrEqual(3 + 1e-9);
    }
  });

  it('toChildrenParams maps rows into {name: value} objects', () => {
    const params = threeParams();
    const rows = [
      [0.1, 0.2, 0.3],
      [0.4, 0.5, 0.6],
    ];
    const children = toChildrenParams(params, rows);
    expect(children).toEqual([
      { a: 0.1, b: 0.2, c: 0.3 },
      { a: 0.4, b: 0.5, c: 0.6 },
    ]);
  });
});

describe('computeMorrisIndices', () => {
  it('recovers slopes of a linear function to high precision', () => {
    const params = threeParams();
    const r = 10;
    const p = 4;
    const rows = morrisSample(params, { r, p, seed: 42 });
    // y = 3*a + 1*b + 0*c
    const y = rows.map((row) => 3 * row[0] + 1 * row[1] + 0 * row[2]);

    const out = computeMorrisIndices(params, rows, y, p);
    expect(out).toHaveLength(3);
    expect(out[0].name).toBe('a');
    expect(out[1].name).toBe('b');
    expect(out[2].name).toBe('c');

    expect(Math.abs(out[0].mu - 3)).toBeLessThan(1e-9);
    expect(Math.abs(out[1].mu - 1)).toBeLessThan(1e-9);
    expect(Math.abs(out[2].mu - 0)).toBeLessThan(1e-9);

    // Linear function → EEs are constant → σ ≈ 0.
    for (const r of out) expect(r.sigma).toBeLessThan(1e-9);
  });

  it('produces nonzero σ for a nonlinear / interaction-rich function', () => {
    const params = threeParams();
    const r = 20;
    const p = 4;
    const rows = morrisSample(params, { r, p, seed: 42 });
    // y = a*b + c  → b has interaction with a, so EE_b varies with a.
    const y = rows.map((row) => row[0] * row[1] + row[2]);
    const out = computeMorrisIndices(params, rows, y, p);
    const sigmaB = out[1].sigma;
    expect(sigmaB).toBeGreaterThan(0.1);
  });

  it('rejects mismatched trajectory row count', () => {
    const params = threeParams();
    const rows = [[0, 0, 0]]; // not divisible by (d+1) = 4
    expect(() => computeMorrisIndices(params, rows, [0], 4)).toThrow();
  });
});
