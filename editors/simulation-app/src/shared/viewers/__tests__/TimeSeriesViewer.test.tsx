/**
 * Tests for the Layer 1 EP6 TimeSeriesViewer.
 *
 * The tests focus on the contract the panel-side code depends on:
 *   1. `accepts` narrows the `ResultData` discriminated union correctly.
 *   2. `render` returns a ReactElement (not null / string) so panels can
 *      drop it into JSX without branching.
 *   3. The viewer's identity / kind tags match what other layers look up.
 *
 * We intentionally avoid mounting uPlot — the Vitest node environment
 * has no DOM, and React-render coverage lives in the Playwright suite.
 */
import { describe, it, expect } from 'vitest';
import { isValidElement } from 'react';
import { timeSeriesViewer } from '../TimeSeriesViewer';
import type { ResultData, TimeSeriesData } from '../types';

function makeData(): TimeSeriesData {
  return {
    kind: 'time-series',
    series: [
      { name: 'voltage', points: [{ t: 0, v: 0 }, { t: 1, v: 1 }] },
      { name: 'current', points: [{ t: 0, v: 5 }, { t: 1, v: 4.2 }], color: '#ff0000' }, // fixture value, not a design color
    ],
  };
}

describe('timeSeriesViewer', () => {
  it('tags itself as a time-series viewer with a stable id', () => {
    expect(timeSeriesViewer.kind).toBe('time-series');
    expect(timeSeriesViewer.id).toBe('time-series-uplot');
  });

  describe('accepts', () => {
    it('returns true for time-series payloads', () => {
      const data: ResultData = { kind: 'time-series' } as TimeSeriesData;
      expect(timeSeriesViewer.accepts(data)).toBe(true);
    });

    it('returns false for other kinds', () => {
      const cases: ResultData[] = [
        { kind: 'histogram' },
        { kind: 'timeline' },
        { kind: 'table' },
        { kind: 'heatmap' },
        { kind: 'parallel-coords' },
      ];
      for (const data of cases) {
        expect(timeSeriesViewer.accepts(data)).toBe(false);
      }
    });

    it('narrows the type so render can read series without casts', () => {
      const data: ResultData = makeData();
      if (timeSeriesViewer.accepts(data)) {
        // If the type guard works, TS knows `.series` is here.
        expect(data.series.length).toBe(2);
      } else {
        throw new Error('accepts returned false for time-series data');
      }
    });
  });

  describe('render', () => {
    it('returns a React element for a non-empty payload', () => {
      const node = timeSeriesViewer.render(makeData(), { height: 200 });
      expect(isValidElement(node)).toBe(true);
    });

    it('handles an empty series list without throwing', () => {
      const empty: TimeSeriesData = { kind: 'time-series', series: [] };
      const node = timeSeriesViewer.render(empty, {});
      expect(isValidElement(node)).toBe(true);
    });

    it('propagates the height hint to the chart wrapper', () => {
      const node = timeSeriesViewer.render(makeData(), { height: 321 });
      // The viewer delegates to a TimeSeriesChart element whose props
      // carry `axes`; assert the axes prop round-trips untouched.
      expect(isValidElement(node)).toBe(true);
      // React 19 stores props on `.props`.
      const props = (node as { props: { axes: { height: number } } }).props;
      expect(props.axes.height).toBe(321);
    });

    it('propagates optional width when supplied', () => {
      const node = timeSeriesViewer.render(makeData(), { width: 480 });
      const props = (node as { props: { axes: { width: number } } }).props;
      expect(props.axes.width).toBe(480);
    });
  });
});
