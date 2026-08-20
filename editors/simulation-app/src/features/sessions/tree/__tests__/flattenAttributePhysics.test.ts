/**
 * flattenAttributePhysics — covers the dotted-path projection behaviour
 * the Variables pane (R3.1) relies on.
 */
import { describe, it, expect } from 'vitest';
import { flattenAttributePhysics } from '../flattenAttributePhysics';
import type { TreeNode } from '@/types/element';

function attr(name: string, unit?: string, isq?: string, children: TreeNode[] = []): TreeNode {
  return {
    id: `id-${name}`,
    name,
    kind: 'AttributeUsage',
    archetype: 'attribute',
    children,
    unit,
    isq_dimension: isq,
  };
}

function part(name: string, children: TreeNode[]): TreeNode {
  return {
    id: `id-${name}`,
    name,
    kind: 'PartUsage',
    archetype: 'part',
    children,
  };
}

describe('flattenAttributePhysics', () => {
  it('returns an empty map for an empty input', () => {
    expect(flattenAttributePhysics([])).toEqual(new Map());
  });

  it('records root-level AttributeUsage nodes by short name', () => {
    const trees: TreeNode[] = [attr('temperature', 'K', 'Θ')];
    const out = flattenAttributePhysics(trees);
    expect(out.get('temperature')).toEqual({ unit: 'K', isq_dimension: 'Θ' });
    expect(out.size).toBe(1);
  });

  it('joins ancestor names with "." for nested attributes', () => {
    const trees: TreeNode[] = [
      part('circuit1', [
        part('busbar', [
          attr('T_busbar', 'K', 'Θ'),
        ]),
      ]),
    ];
    const out = flattenAttributePhysics(trees);
    expect(out.get('circuit1.busbar.T_busbar')).toEqual({
      unit: 'K',
      isq_dimension: 'Θ',
    });
    expect(out.has('T_busbar')).toBe(false);
  });

  it('skips non-AttributeUsage nodes (parts, packages, etc.)', () => {
    const trees: TreeNode[] = [
      part('circuit1', [
        attr('voltage', 'V'),
      ]),
    ];
    const out = flattenAttributePhysics(trees);
    // The PartUsage parent must NOT be entered.
    expect(out.has('circuit1')).toBe(false);
    // The attribute child must be entered with its dotted path.
    expect(out.get('circuit1.voltage')).toEqual({
      unit: 'V',
      isq_dimension: undefined,
    });
  });

  it('skips AttributeUsage nodes that carry neither unit nor isq_dimension', () => {
    const trees: TreeNode[] = [
      part('circuit1', [
        attr('bare_attr'), // no unit, no isq_dimension
        attr('with_unit', 'V'),
      ]),
    ];
    const out = flattenAttributePhysics(trees);
    expect(out.has('circuit1.bare_attr')).toBe(false);
    expect(out.has('circuit1.with_unit')).toBe(true);
  });

  it('skips unnamed AttributeUsage nodes (no legitimate dotted path)', () => {
    const trees: TreeNode[] = [
      part('circuit1', [
        { id: 'x', name: null, kind: 'AttributeUsage', archetype: 'attribute', children: [], unit: 'V' },
      ]),
    ];
    const out = flattenAttributePhysics(trees);
    expect(out.size).toBe(0);
  });

  it('records partial info when only one of unit / isq_dimension is set', () => {
    const trees: TreeNode[] = [attr('only_unit', 'V')];
    const out = flattenAttributePhysics(trees);
    expect(out.get('only_unit')).toEqual({ unit: 'V', isq_dimension: undefined });
  });

  it('walks deeply nested trees without losing intermediate parents', () => {
    const trees: TreeNode[] = [
      part('a', [
        part('b', [
          part('c', [
            attr('leaf', 'm/s'),
          ]),
        ]),
      ]),
    ];
    const out = flattenAttributePhysics(trees);
    expect(out.get('a.b.c.leaf')).toEqual({ unit: 'm/s', isq_dimension: undefined });
  });
});
