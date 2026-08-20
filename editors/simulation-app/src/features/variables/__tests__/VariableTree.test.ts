/**
 * VariableTree — grouping, hierarchy, flattening, and partitioning.
 *
 * Pure-function tests (no React, no DOM). These pin the behaviour the
 * render code relies on — any refactor that breaks grouping, sort order,
 * or the pinned/rest partition will fail here first.
 */

import { describe, it, expect } from 'vitest';
import {
  buildTree,
  flattenTree,
  partitionPinned,
  computeFilterCounts,
  formatVariableValue,
  type VariableEntry,
} from '../VariableTree';

function e(
  name: string,
  overrides: Partial<VariableEntry> = {},
): VariableEntry {
  return { name, value: 0, ...overrides };
}

describe('buildTree — grouping & hierarchy', () => {
  it('single-segment names become top-level leaves', () => {
    const tree = buildTree([e('alpha'), e('beta')]);
    expect(tree.map((n) => ({ label: n.label, isLeaf: n.isLeaf, depth: n.depth })))
      .toEqual([
        { label: 'alpha', isLeaf: true, depth: 0 },
        { label: 'beta',  isLeaf: true, depth: 0 },
      ]);
  });

  it('dotted names split into nested groups', () => {
    const tree = buildTree([
      e('circuit1.breaker.phaseIn.flow'),
      e('circuit1.breaker.phaseIn.voltage'),
      e('circuit1.busbar.temp'),
    ]);
    expect(tree).toHaveLength(1);
    const [circuit] = tree;
    expect(circuit.label).toBe('circuit1');
    expect(circuit.depth).toBe(0);
    expect(circuit.leafCount).toBe(3);

    const busbar = circuit.children.find((c) => c.label === 'busbar')!;
    expect(busbar.leafCount).toBe(1);
    expect(busbar.children[0].label).toBe('temp');
    expect(busbar.children[0].isLeaf).toBe(true);
    expect(busbar.children[0].depth).toBe(2);

    const breaker = circuit.children.find((c) => c.label === 'breaker')!;
    expect(breaker.leafCount).toBe(2);
    const phaseIn = breaker.children[0];
    expect(phaseIn.label).toBe('phaseIn');
    expect(phaseIn.children.map((c) => c.label)).toEqual(['flow', 'voltage']);
  });

  it('sorts groups before leaves, alphabetically within each', () => {
    const tree = buildTree([
      e('zeta'),             // top-level leaf (no dot)
      e('alpha'),            // top-level leaf
      e('module.a'),         // group
      e('anotherGroup.b'),   // group (earlier alphabetically)
    ]);
    // Groups first (alphabetical), then top-level leaves (alphabetical).
    expect(tree.map((n) => n.label)).toEqual(['anotherGroup', 'module', 'alpha', 'zeta']);
  });

  it('leafCount is the transitive count, not immediate children', () => {
    const tree = buildTree([
      e('a.b.c.d'),
      e('a.b.c.e'),
      e('a.b.f'),
    ]);
    expect(tree[0].leafCount).toBe(3);
    expect(tree[0].children[0].leafCount).toBe(3); // a.b
    const abc = tree[0].children[0].children.find((n) => n.label === 'c')!;
    expect(abc.leafCount).toBe(2);
  });

  it('hides backend book-keeping variables by default', () => {
    const tree = buildTree([
      e('__t_ms'),
      e('tick'),
      e('clock_time'),
      e('real'),
    ]);
    expect(tree.map((n) => n.label)).toEqual(['real']);
  });

  it('respects a custom hidden predicate when callers need the plumbing', () => {
    const tree = buildTree([e('__x'), e('y')], { hidden: (n) => n === 'y' });
    expect(tree.map((n) => n.label)).toEqual(['__x']);
  });

  it('leaves the entry attached to the leaf node for rendering', () => {
    const tree = buildTree([e('circuit.v', { value: 42, unit: 'V' })]);
    const leaf = tree[0].children[0];
    expect(leaf.entry?.name).toBe('circuit.v');
    expect(leaf.entry?.value).toBe(42);
    expect(leaf.entry?.unit).toBe('V');
  });
});

describe('flattenTree', () => {
  const entries = [
    e('a.b.c'),
    e('a.b.d'),
    e('a.e'),
  ];
  const tree = buildTree(entries);

  it('fully-expanded tree flattens in depth-first order', () => {
    const flat = flattenTree(tree, new Set());
    expect(flat.map((n) => n.path)).toEqual(['a', 'a.b', 'a.b.c', 'a.b.d', 'a.e']);
  });

  it('collapsed subtrees drop their descendants but keep the group', () => {
    const flat = flattenTree(tree, new Set(['a.b']));
    expect(flat.map((n) => n.path)).toEqual(['a', 'a.b', 'a.e']);
  });
});

describe('partitionPinned', () => {
  it('splits pinned into its own list, sorted alphabetically', () => {
    const list = [e('zulu'), e('alpha'), e('mike')];
    const pinned = new Set(['alpha', 'zulu']);
    const { pinned: pin, rest } = partitionPinned(list, pinned);
    expect(pin.map((n) => n.name)).toEqual(['alpha', 'zulu']);
    expect(rest.map((n) => n.name)).toEqual(['zulu', 'alpha', 'mike'].filter((n) => !pinned.has(n)));
  });

  it('short-circuits when no names are pinned', () => {
    const list = [e('a'), e('b')];
    const result = partitionPinned(list, new Set());
    expect(result.pinned).toEqual([]);
    expect(result.rest).toBe(list); // identity guarantee
  });
});

describe('computeFilterCounts', () => {
  const list: VariableEntry[] = [
    e('a', { constraint: 'pass' }),
    e('b', { constraint: 'fail' }),
    e('c', { constraint: 'inconclusive' }),
    e('d', { constraint: 'error' }),
    e('e'), // no constraint
    e('f', { lastChangedTick: 9 }),
    e('g', { lastChangedTick: 1 }),
    e('__hidden'),
  ];

  it('counts every chip correctly against the visible set', () => {
    const counts = computeFilterCounts(list, {
      pinned: new Set(['a', 'missing']),
      currentTick: 10,
      recentWindow: 3,
    });
    expect(counts.all).toBe(7); // __hidden removed
    expect(counts.passing).toBe(1);
    expect(counts.failing).toBe(1);
    expect(counts.inconclusive).toBe(1);
    expect(counts.error).toBe(1);
    expect(counts.pinned).toBe(1); // only 'a' actually visible & pinned
    expect(counts.changed).toBe(1); // only 'f' (tick 9 vs 10 w/ window 3)
  });
});

describe('formatVariableValue', () => {
  it('renders em-dash for null/undefined', () => {
    expect(formatVariableValue(null)).toBe('\u2014');
  });

  it('formats numbers with significant digits and trims trailing zeros', () => {
    expect(formatVariableValue(12.3456789)).toBe('12.346');
    expect(formatVariableValue(100)).toBe('100');
    expect(formatVariableValue(0)).toBe('0');
  });

  it('switches to exponential for very small or very large magnitudes', () => {
    expect(formatVariableValue(1e-5)).toMatch(/^1.000e-5$/);
    expect(formatVariableValue(1e8)).toMatch(/e\+?\d+$/);
  });

  it('appends units when provided', () => {
    expect(formatVariableValue(273.15, 'K')).toBe('273.15 K');
    expect(formatVariableValue('on', 'state')).toBe('on state');
  });

  it('renders booleans as lowercase strings', () => {
    expect(formatVariableValue(true)).toBe('true');
    expect(formatVariableValue(false)).toBe('false');
  });

  it('JSON-compacts structured values', () => {
    expect(formatVariableValue({ magnitude: 1, unit: 'V' } as Record<string, unknown>))
      .toContain('"magnitude":1');
  });
});
