/**
 * stateGraphLayout — pure geometry tests.
 */
import { describe, expect, it } from 'vitest';
import { layoutStateGraph } from '../detail/stateGraphLayout';
import type {
  SmStateDescriptor,
  SmTransitionDescriptor,
} from '../types';

const S = (id: string, name: string): SmStateDescriptor => ({ id, name });

function T(
  id: string,
  name: string,
  source?: string,
  target?: string,
): SmTransitionDescriptor {
  return { id, name, source, target };
}

describe('layoutStateGraph — nodes', () => {
  it('empty state list → empty nodes + edges', () => {
    const layout = layoutStateGraph([], []);
    expect(layout.nodes).toHaveLength(0);
    expect(layout.edges).toHaveLength(0);
  });

  it('single state sits at the centre', () => {
    const layout = layoutStateGraph([S('s1', 'only')], [], {
      width: 200,
      height: 200,
    });
    expect(layout.nodes[0].cx).toBe(100);
    expect(layout.nodes[0].cy).toBe(100);
  });

  it('multiple states distribute around a circle starting at 12 o\'clock', () => {
    const layout = layoutStateGraph(
      [S('a', 'a'), S('b', 'b'), S('c', 'c'), S('d', 'd')],
      [],
      { width: 200, height: 200 },
    );
    const cy0 = layout.nodes[0].cy;
    // First node is up from centre (12 o'clock).
    expect(cy0).toBeLessThan(100);
    // All four sit on the same distance from centre.
    const dists = layout.nodes.map((n) =>
      Math.hypot(n.cx - 100, n.cy - 100),
    );
    for (const d of dists) expect(d).toBeCloseTo(dists[0], 3);
  });
});

describe('layoutStateGraph — edges', () => {
  const threeStates = [S('a', 'armed'), S('t', 'tripped'), S('r', 'reset')];

  it('named transition between two states gets a curved path', () => {
    const layout = layoutStateGraph(threeStates, [
      T('t1', 'armed_to_tripped', 'armed', 'tripped'),
    ]);
    expect(layout.edges[0].sourceId).toBe('a');
    expect(layout.edges[0].targetId).toBe('t');
    expect(layout.edges[0].path).toMatch(/^M .+ Q /);
    expect(layout.edges[0].selfLoop).toBe(false);
  });

  it('self-loop transition renders an arc above the node', () => {
    const layout = layoutStateGraph([S('a', 'armed')], [
      T('t1', 'armed_to_armed', 'armed', 'armed'),
    ]);
    expect(layout.edges[0].selfLoop).toBe(true);
    expect(layout.edges[0].path).toMatch(/^M .+ A /);
  });

  it('unknown source / target → path null (renderer skips)', () => {
    const layout = layoutStateGraph(threeStates, [
      T('t1', 'mystery', undefined, undefined),
    ]);
    expect(layout.edges[0].path).toBeNull();
    expect(layout.edges[0].sourceId).toBeUndefined();
  });

  it('bidirectional pair bows in opposite directions', () => {
    const layout = layoutStateGraph(threeStates, [
      T('t1', 'armed_to_tripped', 'armed', 'tripped'),
      T('t2', 'tripped_to_armed', 'tripped', 'armed'),
    ]);
    const q1 = /Q (-?[\d.]+) (-?[\d.]+) /.exec(layout.edges[0].path ?? '');
    const q2 = /Q (-?[\d.]+) (-?[\d.]+) /.exec(layout.edges[1].path ?? '');
    expect(q1).not.toBeNull();
    expect(q2).not.toBeNull();
    const mid1X = Number(q1![1]);
    const mid1Y = Number(q1![2]);
    const mid2X = Number(q2![1]);
    const mid2Y = Number(q2![2]);
    const midX = (layout.nodes[0].cx + layout.nodes[1].cx) / 2;
    const midY = (layout.nodes[0].cy + layout.nodes[1].cy) / 2;
    // Each edge's control point offsets from the straight-line
    // midpoint in some direction; for the opposite-direction pair,
    // the offsets should be mirror-images — sum of the offset
    // vectors ≈ 0.
    const offset1X = mid1X - midX;
    const offset1Y = mid1Y - midY;
    const offset2X = mid2X - midX;
    const offset2Y = mid2Y - midY;
    // Layout rounds path coords to one decimal, so tolerance ~0.1.
    expect(offset1X + offset2X).toBeCloseTo(0, 0);
    expect(offset1Y + offset2Y).toBeCloseTo(0, 0);
    // And at least one axis' offset should be non-zero so the bow
    // actually exists.
    expect(Math.hypot(offset1X, offset1Y)).toBeGreaterThan(1);
  });

  it('edge label falls back to name when both source + target resolve', () => {
    const layout = layoutStateGraph(threeStates, [
      T('t1', 'armed_to_tripped', 'armed', 'tripped'),
    ]);
    expect(layout.edges[0].label).toBe('armed_to_tripped');
  });
});

describe('layoutStateGraph — case-insensitive lookup', () => {
  it('matches source / target against state names case-insensitively', () => {
    const layout = layoutStateGraph(
      [S('a', 'Armed'), S('t', 'Tripped')],
      [T('t1', 'armed_to_tripped', 'armed', 'tripped')],
    );
    expect(layout.edges[0].sourceId).toBe('a');
    expect(layout.edges[0].targetId).toBe('t');
  });
});
