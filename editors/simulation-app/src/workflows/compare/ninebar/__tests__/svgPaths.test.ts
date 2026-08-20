/**
 * Unit tests for the diff-canvas SVG path builders (`svgPaths.ts`).
 */
import { describe, expect, it } from 'vitest';
import { envelopePath, linePath, valueDomain, xFor, yFor } from '../svgPaths';
import { bucketScores, bucketStartTick } from '../seriesMath';

const SCALE = { width: 100, height: 10, maxTick: 4, yMin: 0, yMax: 10 };

describe('valueDomain', () => {
  it('finds the finite min/max across sessions', () => {
    expect(valueDomain([[1, 5, NaN], [0, 10, 2]])).toEqual({ yMin: 0, yMax: 10 });
  });

  it('pads a flat signal so it never sits on an edge', () => {
    const d = valueDomain([[3, 3, 3]]);
    expect(d).not.toBeNull();
    expect(d!.yMin).toBeLessThan(3);
    expect(d!.yMax).toBeGreaterThan(3);
  });

  it('returns null when nothing is finite', () => {
    expect(valueDomain([[NaN, NaN]])).toBeNull();
  });
});

describe('linePath', () => {
  it('draws a connected polyline over finite ticks', () => {
    expect(linePath([0, 5, 10, NaN, NaN], SCALE)).toBe(
      'M0.00,10.00L25.00,5.00L50.00,0.00',
    );
  });

  it('breaks the pen across NaN gaps (missing is never drawn through)', () => {
    const d = linePath([0, NaN, 10], SCALE);
    expect(d).toBe('M0.00,10.00M50.00,0.00');
    expect(d.match(/M/g)).toHaveLength(2);
  });

  it('is empty for an all-NaN row', () => {
    expect(linePath([NaN, NaN], SCALE)).toBe('');
  });
});

describe('envelopePath', () => {
  it('fills between cross-session min and max where ≥2 sessions have data', () => {
    const d = envelopePath(
      [
        [0, 2, 4],
        [10, 8, 6],
      ],
      { ...SCALE, maxTick: 2 },
    );
    // upper (max) run left→right, lower (min) run right→left, closed.
    expect(d.startsWith('M')).toBe(true);
    expect(d.endsWith('Z')).toBe(true);
  });

  it('contributes nothing where only one session has data', () => {
    expect(
      envelopePath(
        [
          [1, 1, 1],
          [NaN, NaN, NaN],
        ],
        { ...SCALE, maxTick: 2 },
      ),
    ).toBe('');
  });
});

describe('bucketScores (gutter max-pooling)', () => {
  it('max-pools so a single divergent tick survives downsampling', () => {
    const scores = new Array<number>(1000).fill(0);
    scores[777] = 0.9;
    const buckets = bucketScores(scores, 100);
    expect(buckets).toHaveLength(100);
    expect(Math.max(...buckets)).toBe(0.9);
  });

  it('passes through when buckets ≥ ticks', () => {
    expect(bucketScores([0, 1, 0], 10)).toEqual([0, 1, 0]);
  });
});

describe('bucketStartTick', () => {
  it('maps a bucket back to its starting tick for click-to-scrub', () => {
    expect(bucketStartTick(0, 100, 1000)).toBe(0);
    expect(bucketStartTick(50, 100, 1000)).toBe(500);
    expect(bucketStartTick(99, 100, 1000)).toBe(990);
  });
});

describe('scales', () => {
  it('x and y map linearly and invert y', () => {
    expect(xFor(2, SCALE)).toBe(50);
    expect(yFor(0, SCALE)).toBe(10);
    expect(yFor(10, SCALE)).toBe(0);
  });
});
