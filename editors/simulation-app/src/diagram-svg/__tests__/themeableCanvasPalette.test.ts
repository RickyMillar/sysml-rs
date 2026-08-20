/**
 * The canvas palette is emitted from Rust as a single dark table (the theme is
 * deliberately kept out of the salsa cache key). `themeableCanvasPalette`
 * rewrites it to CSS custom properties so the frontend can flip it — see the
 * function's doc comment for why the indirection lives here.
 *
 * These pin the two properties the light canvas depends on: every colour
 * becomes overridable, and the emitted value survives as the fallback so a
 * missing `--canvas-*` cannot regress the dark ground.
 */
import { describe, expect, it } from 'vitest';
import { themeableCanvasPalette } from '../palette';

describe('themeableCanvasPalette', () => {
  it('wraps a colour in var() keyed on its slot, keeping the emitted value as fallback', () => {
    const out = themeableCanvasPalette({ bg: 'oklch(24% 0.014 55)' });
    expect(out.bg).toBe('var(--canvas-bg, oklch(24% 0.014 55))');
  });

  it('derives nested slot names from the path', () => {
    const out = themeableCanvasPalette({
      block: { fill: '#111', stroke: '#222', header: '#333' },
      sim: { active: '#444' },
    });
    expect(out.block.fill).toBe('var(--canvas-block-fill, #111)');
    expect(out.block.stroke).toBe('var(--canvas-block-stroke, #222)');
    expect(out.sim.active).toBe('var(--canvas-sim-active, #444)');
  });

  it('slugifies underscores so slots read as CSS custom properties', () => {
    const out = themeableCanvasPalette({
      grid_minor: '#aaa',
      node_fallback: { fill: '#bbb' },
      port: { in_: '#ccc' },
    });
    expect(out.grid_minor).toBe('var(--canvas-grid-minor, #aaa)');
    expect(out.node_fallback.fill).toBe('var(--canvas-node-fallback-fill, #bbb)');
    // trailing underscore is an identifier escape, not part of the slot name
    expect(out.port.in_).toBe('var(--canvas-port-in, #ccc)');
  });

  it('passes non-string values through untouched', () => {
    const out = themeableCanvasPalette({
      sim: { inactive_opacity: 0.45, active: '#fff' },
      nothing: null,
    });
    expect(out.sim.inactive_opacity).toBe(0.45);
    expect(out.sim.active).toBe('var(--canvas-sim-active, #fff)');
    expect(out.nothing).toBeNull();
  });

  it('does not mutate the emitted palette', () => {
    const emitted = { bg: '#000', block: { fill: '#111' } };
    themeableCanvasPalette(emitted);
    expect(emitted.bg).toBe('#000');
    expect(emitted.block.fill).toBe('#111');
  });
});
