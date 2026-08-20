/**
 * Splitter — clamp helper unit tests.
 *
 * The clamp function is where all the interesting logic lives; the
 * drag wiring (pointercapture + pointermove) is purposely skipped
 * because jsdom's PointerEvent doesn't forward `clientY` reliably
 * through React's synthetic-event layer. That path is exercised
 * via browser smoke tests.
 */
import { describe, expect, it } from 'vitest';
import { clampSplitPosition } from '../Splitter';

describe('clampSplitPosition', () => {
  it('respects the minimum (100 px default)', () => {
    expect(clampSplitPosition(40, 600)).toBe(100);
    expect(clampSplitPosition(-200, 600)).toBe(100);
  });

  it('respects the 60% maximum (default)', () => {
    expect(clampSplitPosition(800, 600)).toBe(360);
    expect(clampSplitPosition(360, 600)).toBe(360); // exactly at max
  });

  it('returns integer px (no sub-pixel shimmer)', () => {
    expect(clampSplitPosition(200.7, 600)).toBe(200);
  });

  it('minPx override — allows smaller minimums (e.g. 40)', () => {
    expect(clampSplitPosition(50, 600, { minPx: 40 })).toBe(50);
    expect(clampSplitPosition(30, 600, { minPx: 40 })).toBe(40);
  });

  it('maxFraction override — allow the detail pane to take 80%', () => {
    expect(clampSplitPosition(500, 600, { maxFraction: 0.8 })).toBe(480);
  });

  it('container 0 (pre-layout) returns the proposal unclamped', () => {
    // Avoids collapsing to minPx on the first paint where the
    // ResizeObserver hasn't fired yet.
    expect(clampSplitPosition(240, 0)).toBe(240);
  });

  it('non-finite proposals fall back to minPx', () => {
    expect(clampSplitPosition(NaN, 600)).toBe(100);
    expect(clampSplitPosition(Infinity, 600)).toBe(100);
  });

  it('maxPx can never fall below minPx (tiny containers)', () => {
    // 60% of 80 = 48, below the 100 px min. Clamp hands back minPx.
    expect(clampSplitPosition(200, 80)).toBe(100);
  });
});

