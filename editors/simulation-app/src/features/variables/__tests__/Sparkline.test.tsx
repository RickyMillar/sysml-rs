/**
 * Sparkline — path generation + render guard rails.
 *
 * We intentionally test the pure path builder rather than mounting the
 * component via jsdom so the suite stays DOM-free. Rendering is exercised
 * via React's server-side renderer (renderToStaticMarkup), which ships
 * with the existing react-dom dep and doesn't need a jsdom environment.
 */

import { describe, it, expect } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { createElement } from 'react';
import {
  Sparkline,
  buildSparklinePath,
  MIN_SPARKLINE_SAMPLES,
} from '../Sparkline';

describe('buildSparklinePath', () => {
  it('returns an empty string for <2 samples', () => {
    expect(buildSparklinePath([], 60, 16)).toBe('');
    expect(buildSparklinePath([1], 60, 16)).toBe('');
  });

  it('renders first point at (padding,*) and last at (width-padding,*)', () => {
    const path = buildSparklinePath([0, 1, 2, 3], 60, 16, 1);
    const points = path.split(' ').map((p) => p.split(',').map(Number));
    expect(points[0][0]).toBeCloseTo(1, 2); // padding
    expect(points[points.length - 1][0]).toBeCloseTo(59, 2); // width - padding
  });

  it('maps min → bottom, max → top within the padded box', () => {
    const path = buildSparklinePath([0, 10], 60, 16, 1);
    const [first, last] = path.split(' ').map((p) => p.split(',').map(Number));
    // Min sample should sit at y = height - padding = 15
    expect(first[1]).toBeCloseTo(15, 2);
    // Max sample should sit at y = padding = 1
    expect(last[1]).toBeCloseTo(1, 2);
  });

  it('treats NaN / Infinity as missing (no crash, drops them to series min)', () => {
    const path = buildSparklinePath([1, Number.NaN, 5, Infinity], 60, 16);
    expect(path.split(' ')).toHaveLength(4);
    for (const part of path.split(' ')) {
      for (const n of part.split(',').map(Number)) {
        expect(Number.isFinite(n)).toBe(true);
      }
    }
  });

  it('handles constant series without division-by-zero', () => {
    const path = buildSparklinePath([5, 5, 5, 5], 60, 16);
    // All y values should be finite + equal (range collapses to 1 so
    // all y sit at the top of the padded box).
    const ys = path.split(' ').map((p) => Number(p.split(',')[1]));
    expect(new Set(ys).size).toBe(1);
    expect(Number.isFinite(ys[0])).toBe(true);
  });
});

describe('Sparkline component', () => {
  it(`renders null below ${MIN_SPARKLINE_SAMPLES} samples`, () => {
    const html = renderToStaticMarkup(createElement(Sparkline, { samples: [1, 2] }));
    expect(html).toBe('');
  });

  it('renders an <svg> with a polyline at full sample count', () => {
    const html = renderToStaticMarkup(
      createElement(Sparkline, { samples: [1, 2, 3, 4, 5] }),
    );
    expect(html).toContain('<svg');
    expect(html).toContain('<polyline');
    expect(html).toContain('role="img"');
  });

  it('honours custom width/height', () => {
    const html = renderToStaticMarkup(
      createElement(Sparkline, { samples: [1, 2, 3], width: 120, height: 32 }),
    );
    expect(html).toMatch(/width="120"/);
    expect(html).toMatch(/height="32"/);
    expect(html).toMatch(/viewBox="0 0 120 32"/);
  });

  it('honours color override', () => {
    // fixture value, not a design color
    const html = renderToStaticMarkup(
      createElement(Sparkline, { samples: [1, 2, 3], color: '#ff00ff' }),
    );
    expect(html).toContain('#ff00ff');
  });

  it('exposes an accessible label', () => {
    const html = renderToStaticMarkup(
      createElement(Sparkline, { samples: [1, 2, 3], ariaLabel: 'busbar temp' }),
    );
    expect(html).toContain('aria-label="busbar temp"');
  });
});
