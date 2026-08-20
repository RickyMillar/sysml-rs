import { describe, it, expect } from 'vitest';
import { countBand, zoomBand, effectiveLod, clampStroke, lodLabel } from '../lod';

describe('LOD bands (brief §4)', () => {
  it('count bands split at 50 / 150', () => {
    expect(countBand(1)).toBe('full');
    expect(countBand(50)).toBe('full');
    expect(countBand(51)).toBe('reduced');
    expect(countBand(150)).toBe('reduced');
    expect(countBand(151)).toBe('glyph');
    expect(countBand(400)).toBe('glyph'); // >250 truncated upstream; still glyph here
  });

  it('zoom bands split at 40% / 80%', () => {
    expect(zoomBand(1.5)).toBe('full');
    expect(zoomBand(0.81)).toBe('full');
    expect(zoomBand(0.8)).toBe('reduced'); // boundary: not > 0.8
    expect(zoomBand(0.4)).toBe('reduced');
    expect(zoomBand(0.39)).toBe('glyph');
    expect(zoomBand(0.1)).toBe('glyph');
  });

  it('effective LOD is the stricter of the two bands', () => {
    // sparse but zoomed out → zoom wins
    expect(effectiveLod(10, 0.3)).toBe('glyph');
    // dense but zoomed in → count wins
    expect(effectiveLod(200, 1.0)).toBe('glyph');
    expect(effectiveLod(100, 1.0)).toBe('reduced');
    // both full
    expect(effectiveLod(10, 1.0)).toBe('full');
    // count reduced, zoom full → reduced
    expect(effectiveLod(100, 1.0)).toBe('reduced');
    // count full, zoom reduced → reduced
    expect(effectiveLod(10, 0.5)).toBe('reduced');
  });

  it('clampStroke keeps a stroke at ≥1 device px', () => {
    // At 100% zoom, a 1-unit stroke is already 1 device px.
    expect(clampStroke(1, 1)).toBe(1);
    // At 25% zoom a 1-unit stroke would be 0.25px → widen to 4 units (=1px).
    expect(clampStroke(1, 0.25)).toBe(4);
    // A thick stroke stays as-authored when already ≥ min.
    expect(clampStroke(2.5, 1)).toBe(2.5);
  });

  it('lodLabel renders the crib pill suffix', () => {
    expect(lodLabel('full')).toBe('full LOD');
    expect(lodLabel('reduced')).toBe('reduced LOD');
    expect(lodLabel('glyph')).toBe('glyph LOD');
  });
});
