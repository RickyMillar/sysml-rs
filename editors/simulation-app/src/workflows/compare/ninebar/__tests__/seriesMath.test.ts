/**
 * Unit tests for the Phase 6 compare tick-domain math
 * (`seriesMath.ts`) — pure functions, no React.
 */
import { describe, expect, it } from 'vitest';
import {
  buildSampleMatrix,
  classifyVariableDiff,
  diffEntryAtOrBefore,
  pairDivergenceMask,
  pointsToTickSeries,
  sessionDtMs,
} from '../seriesMath';

describe('sessionDtMs', () => {
  it('derives dt from tick + time_ms', () => {
    expect(sessionDtMs({ tick: 100, time_ms: 10 })).toBeCloseTo(0.1);
    expect(sessionDtMs({ tick: 5856, time_ms: 0.5856 })).toBeCloseTo(0.0001);
  });

  it('refuses a dt for un-stepped sessions (no guessing)', () => {
    expect(sessionDtMs({ tick: 0, time_ms: 0 })).toBeNull();
    expect(sessionDtMs({ tick: 0, time_ms: 5 })).toBeNull();
    expect(sessionDtMs({ tick: 3, time_ms: 0 })).toBeNull();
  });
});

describe('pointsToTickSeries', () => {
  it('maps time_ms to integer ticks via dt', () => {
    const pts = [
      { time_ms: 0, value: 1 },
      { time_ms: 0.2, value: 2 },
      { time_ms: 0.4, value: 3 },
    ];
    expect(pointsToTickSeries(pts, 0.1)).toEqual([
      { tick: 0, value: 1 },
      { tick: 2, value: 2 },
      { tick: 4, value: 3 },
    ]);
  });

  it('drops non-finite values and refuses a bad dt', () => {
    expect(
      pointsToTickSeries([{ time_ms: 1, value: NaN }], 0.1),
    ).toEqual([]);
    expect(pointsToTickSeries([{ time_ms: 1, value: 2 }], 0)).toEqual([]);
  });
});

describe('buildSampleMatrix', () => {
  it('places exact samples and linearly interpolates gaps', () => {
    const m = buildSampleMatrix(
      [
        [
          { tick: 0, value: 0 },
          { tick: 4, value: 8 },
        ],
      ],
      4,
    );
    expect(m[0]).toEqual([0, 2, 4, 6, 8]);
  });

  it('leaves NaN outside a session\'s recorded range (honest gaps)', () => {
    const m = buildSampleMatrix(
      [
        [
          { tick: 2, value: 5 },
          { tick: 3, value: 7 },
        ],
      ],
      5,
    );
    expect(Number.isNaN(m[0][0])).toBe(true);
    expect(Number.isNaN(m[0][1])).toBe(true);
    expect(m[0][2]).toBe(5);
    expect(m[0][3]).toBe(7);
    expect(Number.isNaN(m[0][4])).toBe(true);
    expect(Number.isNaN(m[0][5])).toBe(true);
  });

  it('returns an all-NaN row for an empty series', () => {
    const m = buildSampleMatrix([[]], 2);
    expect(m[0].every((v) => Number.isNaN(v))).toBe(true);
    expect(m[0]).toHaveLength(3);
  });
});

describe('classifyVariableDiff (manufacture added/removed from null-on-one-side)', () => {
  it('classifies the three layers', () => {
    expect(classifyVariableDiff({ name: 'x', a_value: null, b_value: 1 })).toBe('added');
    expect(classifyVariableDiff({ name: 'x', a_value: 1, b_value: null })).toBe('removed');
    expect(classifyVariableDiff({ name: 'x', a_value: 1, b_value: 2 })).toBe('modified');
  });
});

describe('diffEntryAtOrBefore', () => {
  const diffs = [{ tick: 5 }, { tick: 9 }, { tick: 20 }];
  it('returns the latest entry at or before the playhead', () => {
    expect(diffEntryAtOrBefore(diffs, 4)).toBeNull();
    expect(diffEntryAtOrBefore(diffs, 5)?.tick).toBe(5);
    expect(diffEntryAtOrBefore(diffs, 12)?.tick).toBe(9);
    expect(diffEntryAtOrBefore(diffs, 99)?.tick).toBe(20);
  });
});

describe('pairDivergenceMask', () => {
  it('marks exactly the backend-reported diff ticks', () => {
    const mask = pairDivergenceMask([{ tick: 1 }, { tick: 3 }], 4);
    expect(mask).toEqual([0, 1, 0, 1, 0]);
  });

  it('ignores out-of-domain ticks', () => {
    expect(pairDivergenceMask([{ tick: 10 }], 2)).toEqual([0, 0, 0]);
  });
});
