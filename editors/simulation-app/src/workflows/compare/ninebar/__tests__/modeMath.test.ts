/**
 * Unit tests for the mode-math bridges (`modeMath.ts`).
 */
import { describe, expect, it } from 'vitest';
import {
  archivedSnapshotsToSeries,
  ensembleAtTick,
  rowToTimePoints,
} from '../modeMath';

describe('rowToTimePoints', () => {
  it('drops NaN gaps and keeps tick indexing', () => {
    expect(rowToTimePoints([1, NaN, 3])).toEqual([
      { t: 0, v: 1 },
      { t: 2, v: 3 },
    ]);
  });
});

describe('archivedSnapshotsToSeries', () => {
  it('extracts per-variable series keyed by the snapshot tick', () => {
    const series = archivedSnapshotsToSeries([
      { tick: 0, variables: { current: 1.5, tripped: false } },
      { tick: 2, variables: { current: 2.5, tripped: true } },
    ]);
    expect(series.current).toEqual([
      { t: 0, v: 1.5 },
      { t: 2, v: 2.5 },
    ]);
    // booleans coerce 1/0 like the live ingest path
    expect(series.tripped).toEqual([
      { t: 0, v: 0 },
      { t: 2, v: 1 },
    ]);
  });

  it('skips bookkeeping vars, non-numerics, and malformed snapshots', () => {
    const series = archivedSnapshotsToSeries([
      null,
      'garbage',
      { tick: 1, variables: { __internal: 9, t_ms: 5, label: 'x', ok: { value: 7 } } },
      { variables: { ok: { re: 3 } } }, // no tick → falls back to index
    ]);
    expect(Object.keys(series)).toEqual(['ok']);
    expect(series.ok).toEqual([
      { t: 1, v: 7 },
      { t: 3, v: 3 },
    ]);
  });
});

describe('ensembleAtTick', () => {
  it('summarizes the cross-session column and flags outliers', () => {
    const samples = [
      [1, 1, 1],
      [1.1, 1.1, 1.1],
      [0.9, 0.9, 0.9],
      [1, 1, 50], // wild outlier at tick 2
    ];
    const at = ensembleAtTick(samples, 2, 1.5);
    expect(at.stats.n).toBe(4);
    expect(at.outlierIndices).toEqual([3]);
  });

  it('reads the last sample for shorter sessions (frozen semantics)', () => {
    const samples = [[5], [1, 2, 3]];
    const at = ensembleAtTick(samples, 2);
    expect(at.stats.n).toBe(2);
    expect(at.stats.min).toBe(3);
    expect(at.stats.max).toBe(5);
  });

  it('never flags outliers on a flat ensemble (σ = 0)', () => {
    const at = ensembleAtTick(
      [
        [2, 2],
        [2, 2],
      ],
      1,
    );
    expect(at.outlierIndices).toEqual([]);
  });
});
