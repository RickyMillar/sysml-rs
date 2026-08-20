/**
 * label-layout resolver tests (diagram-layout-quality brief §1).
 */
import { describe, expect, it } from 'vitest';
import {
  chipSize,
  elideForLod,
  pointAt,
  resolveEdgeLabels,
  wrapLabel,
  LABEL_CHAR_W,
  LABEL_LINE_H,
  WRAP_CH,
  type Rect,
} from '../label-layout';

const overlaps = (a: Rect, b: Rect) =>
  a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;

describe('wrapLabel / chipSize', () => {
  it('keeps short labels on one line', () => {
    expect(wrapLabel('typing')).toEqual(['typing']);
  });

  it('wraps long guard text at the wrap column on word boundaries', () => {
    const lines = wrapLabel('when i_drive <= threshold and t_dead has elapsed');
    expect(lines.length).toBeGreaterThan(1);
    for (const l of lines) expect(l.length).toBeLessThanOrEqual(WRAP_CH);
  });

  it('hard-splits an unbreakable token at the wrap column', () => {
    // Asserted against WRAP_CH, not a literal — the column is a tuning lever
    // (it was narrowed to shrink inline label chips, which drag their edges
    // sideways), and the BEHAVIOUR under test is the hard split, not the value.
    const lines = wrapLabel('a'.repeat(70));
    const full = Math.floor(70 / WRAP_CH);
    expect(lines.length).toBe(full + (70 % WRAP_CH ? 1 : 0));
    for (const l of lines.slice(0, full)) expect(l).toBe('a'.repeat(WRAP_CH));
    if (70 % WRAP_CH) expect(lines[lines.length - 1]).toBe('a'.repeat(70 % WRAP_CH));
  });

  it('sizes the chip from the longest line × line count', () => {
    const { width, height } = chipSize(['abcd', 'ab']);
    expect(width).toBe(4 * LABEL_CHAR_W + 8);
    expect(height).toBe(2 * LABEL_LINE_H);
  });
});

describe('pointAt (polyline arc-length parameterisation)', () => {
  const L = [
    { x: 0, y: 0 },
    { x: 100, y: 0 },
    { x: 100, y: 100 },
  ];

  it('finds the midpoint along the path, not the midpoint vertex', () => {
    const { point } = pointAt(L, 0.5);
    expect(point).toEqual({ x: 100, y: 0 }); // 100 of 200 total length
  });

  it('reports the local segment direction', () => {
    expect(pointAt(L, 0.25).dir).toEqual({ x: 1, y: 0 });
    expect(pointAt(L, 0.75).dir).toEqual({ x: 0, y: 1 });
  });

  it('clamps t to [0,1]', () => {
    expect(pointAt(L, -1).point).toEqual({ x: 0, y: 0 });
    expect(pointAt(L, 2).point).toEqual({ x: 100, y: 100 });
  });
});

describe('resolveEdgeLabels', () => {
  const bounds: Rect = { x: 0, y: 0, width: 600, height: 400 };

  it('places an unobstructed center label on its path midpoint', () => {
    const [label] = resolveEdgeLabels({
      obstacles: [],
      edges: [
        {
          edgeId: 'e1',
          points: [
            { x: 0, y: 100 },
            { x: 200, y: 100 },
          ],
          centerText: 'flow',
          secondaryTexts: [],
        },
      ],
      bounds,
    });
    expect(label.degraded).toBe(false);
    expect(label.anchor).toEqual({ x: 100, y: 100 });
  });

  it('slides a label along its path away from an obstacle (never detaching)', () => {
    const [label] = resolveEdgeLabels({
      // Obstacle square sitting exactly on the path midpoint.
      obstacles: [{ x: 80, y: 80, width: 40, height: 40 }],
      edges: [
        {
          edgeId: 'e1',
          points: [
            { x: 0, y: 100 },
            { x: 200, y: 100 },
          ],
          centerText: 'flow',
          secondaryTexts: [],
        },
      ],
      bounds,
    });
    expect(label.degraded).toBe(false);
    // Moved off the blocked midpoint, still vertically near the path (a slide
    // along the line or the ±12px perpendicular offset — never free-floating).
    expect(label.anchor.x).not.toBe(100);
    expect(Math.abs(label.anchor.y - 100)).toBeLessThanOrEqual(12);
  });

  it('de-conflicts two labels crossing the same region (G2)', () => {
    const mk = (id: string, y: number) => ({
      edgeId: id,
      points: [
        { x: 0, y },
        { x: 200, y },
      ],
      centerText: 'transition-label',
      secondaryTexts: [],
    });
    // Two horizontal edges 4px apart — naive midpoints would overprint.
    const labels = resolveEdgeLabels({ obstacles: [], edges: [mk('a', 100), mk('b', 104)], bounds });
    expect(labels).toHaveLength(2);
    expect(overlaps(labels[0].rect, labels[1].rect)).toBe(false);
    expect(labels.every((l) => !l.degraded)).toBe(true);
  });

  it('tags the minimum-overlap fallback as degraded when no slot is clean', () => {
    const [label] = resolveEdgeLabels({
      // Everything within reach of the (short) path is occupied.
      obstacles: [{ x: -100, y: -100, width: 800, height: 600 }],
      edges: [
        {
          edgeId: 'e1',
          points: [
            { x: 90, y: 100 },
            { x: 110, y: 100 },
          ],
          centerText: 'x',
          secondaryTexts: [],
        },
      ],
      bounds,
    });
    expect(label.degraded).toBe(true);
  });

  it('stacks a secondary label below the center chip when clear', () => {
    const labels = resolveEdgeLabels({
      obstacles: [],
      edges: [
        {
          edgeId: 'e1',
          points: [
            { x: 0, y: 100 },
            { x: 300, y: 100 },
          ],
          centerText: 'go [armed]',
          secondaryTexts: ['[via port]'],
        },
      ],
      bounds,
    });
    expect(labels.map((l) => l.kind)).toEqual(['center', 'secondary']);
    const [center, sub] = labels;
    expect(sub.rect.y).toBeGreaterThanOrEqual(center.rect.y + center.rect.height);
    expect(overlaps(center.rect, sub.rect)).toBe(false);
  });

  it('orders center labels by ascending path length (short edges first)', () => {
    const labels = resolveEdgeLabels({
      obstacles: [],
      edges: [
        {
          edgeId: 'long',
          points: [
            { x: 0, y: 100 },
            { x: 400, y: 100 },
          ],
          centerText: 'label',
          secondaryTexts: [],
        },
        {
          edgeId: 'short',
          points: [
            { x: 180, y: 96 },
            { x: 220, y: 96 },
          ],
          centerText: 'label',
          secondaryTexts: [],
        },
      ],
      bounds,
    });
    // The short edge placed first (fewest candidates) and kept its midpoint;
    // the long edge slid off to a clean slot — no overlap between them.
    const short = labels.find((l) => l.edgeId === 'short')!;
    const long = labels.find((l) => l.edgeId === 'long')!;
    expect(short.anchor).toEqual({ x: 200, y: 96 });
    expect(overlaps(short.rect, long.rect)).toBe(false);
  });

  it('honors a preferred (elk-native) anchor when clear', () => {
    const [label] = resolveEdgeLabels({
      obstacles: [],
      edges: [
        {
          edgeId: 'e1',
          points: [
            { x: 0, y: 100 },
            { x: 200, y: 100 },
          ],
          centerText: 'flow',
          secondaryTexts: [],
          preferredAt: { x: 60, y: 100 },
        },
      ],
      bounds,
    });
    expect(label.anchor).toEqual({ x: 60, y: 100 });
  });
});

describe('elideForLod', () => {
  it('keeps full text at full LOD, elides at reduced/glyph', () => {
    const long = 'when i_drive <= threshold';
    expect(elideForLod(long, true)).toBe(long);
    expect(elideForLod(long, false)).toBe('when i_drive…');
    expect(elideForLod('short', false)).toBe('short');
  });
});
