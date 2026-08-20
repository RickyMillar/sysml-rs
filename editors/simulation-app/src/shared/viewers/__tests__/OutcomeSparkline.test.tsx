/**
 * The shape behind a sweep outcome's number.
 *
 * A column of near-identical readings is ambiguous: it can mean the model is
 * genuinely insensitive to the factor, or that every run stopped before it
 * did anything. `examples/radiation-cooling` reported ~990 K across a whole
 * five-way study for the second reason, and nothing on the surface could tell
 * the two apart. The curve can — provided it renders honestly in the cases
 * that matter, which is what this pins.
 */

import { describe, it, expect, afterEach } from 'vitest';
import { cleanup, render, screen } from '@testing-library/react';
import { OutcomeSparkline, describeSeries } from '../OutcomeSparkline';

afterEach(cleanup);

/** A cooling curve, roughly the fixture's shape. */
const COOLING: [number, number][] = Array.from({ length: 20 }, (_, i) => [
  i * 100_000,
  300 + 700 * Math.exp(-i / 4),
]);

/** Five ticks that never moved — the "study saw nothing" case. */
const FLAT: [number, number][] = Array.from({ length: 5 }, (_, i) => [i * 250, 990.0367]);

function points(testId = 'spark'): string[] {
  const poly = screen.getByTestId(testId).querySelector('polyline');
  return (poly?.getAttribute('points') ?? '').trim().split(/\s+/).filter(Boolean);
}

describe('OutcomeSparkline', () => {
  it('draws one vertex per retained sample', () => {
    render(<OutcomeSparkline series={COOLING} testId="spark" />);
    expect(points()).toHaveLength(COOLING.length);
  });

  it('renders a series that never moved as a flat line, not as nothing', () => {
    // Dividing by a zero range would produce NaN coordinates and an invisible
    // polyline — which would read as "no data" when the finding is "no
    // change". Those are different answers.
    render(<OutcomeSparkline series={FLAT} testId="spark" />);
    const ys = points().map((p) => Number(p.split(',')[1]));
    expect(ys.every(Number.isFinite)).toBe(true);
    expect(new Set(ys).size).toBe(1);
  });

  it('spans the full box for a series that does move', () => {
    render(<OutcomeSparkline series={COOLING} height={16} testId="spark" />);
    const ys = points().map((p) => Number(p.split(',')[1]));
    // Hottest sample at the top, coldest at the bottom, both inside the box.
    expect(Math.min(...ys)).toBeLessThan(2);
    expect(Math.max(...ys)).toBeGreaterThan(14);
  });

  it('drops non-finite samples rather than breaking the path', () => {
    const withGaps: [number, number][] = [
      [0, 10],
      [1, Number.NaN],
      [2, 30],
      [3, Number.POSITIVE_INFINITY],
      [4, 20],
    ];
    render(<OutcomeSparkline series={withGaps} testId="spark" />);
    const coords = points();
    expect(coords).toHaveLength(3);
    expect(coords.every((c) => !c.includes('NaN'))).toBe(true);
  });

  it('renders nothing for a series too short to be a shape', () => {
    const { container } = render(<OutcomeSparkline series={[[0, 5]]} testId="spark" />);
    expect(container.querySelector('svg')).toBeNull();
  });

  it('renders nothing for an empty series', () => {
    const { container } = render(<OutcomeSparkline series={[]} testId="spark" />);
    expect(container.querySelector('svg')).toBeNull();
  });

  it('carries its description for screen readers and hover alike', () => {
    render(<OutcomeSparkline series={COOLING} title="1000 K → 314 K" testId="spark" />);
    const svg = screen.getByTestId('spark');
    expect(svg.getAttribute('aria-label')).toBe('1000 K → 314 K');
    expect(svg.querySelector('title')?.textContent).toBe('1000 K → 314 K');
  });
});

describe('describeSeries', () => {
  it('states where the run ran from and to, in model time', () => {
    const text = describeSeries(
      [
        [0, 1000],
        [2_000_000, 314.229],
      ],
      'K',
    );
    expect(text).toContain('1000 K');
    expect(text).toContain('314.229 K');
    expect(text).toContain('33.33 min');
  });

  it('says a value was flat rather than describing a change of zero', () => {
    // This sentence is the one that would have caught the original defect.
    expect(describeSeries(FLAT)).toMatch(/^flat at 990\.037/);
  });

  it('omits a unit the model never declared', () => {
    const text = describeSeries([
      [0, 1],
      [1000, 2],
    ]);
    expect(text).toBe('1 → 2 across 1 s of model time');
  });

  it('is explicit when no trace was retained', () => {
    expect(describeSeries([])).toBe('no trace retained for this run');
    expect(describeSeries([[0, 1]])).toBe('no trace retained for this run');
  });

  it('scales the span to readable units', () => {
    expect(describeSeries([[0, 1], [45_000, 2]])).toContain('45 s');
    expect(describeSeries([[0, 1], [600_000, 2]])).toContain('10 min');
    expect(describeSeries([[0, 1], [7_200_000, 2]])).toContain('2 h');
  });
});
