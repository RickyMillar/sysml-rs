/**
 * partHealth — pure aggregation + key-signal picker.
 */
import { describe, expect, it } from 'vitest';
import {
  aggregateHealth,
  countArchetypes,
  pickKeySignals,
} from '../detail/partHealth';
import type {
  AttributeTreeNode,
  ConstraintTreeNode,
  ModelTreeNode,
  PartTreeNode,
} from '../types';
import type { ConstraintVerdict } from '@/features/variables/VariableTree';

function part(
  id: string,
  children: ModelTreeNode[] = [],
  depth = 0,
): PartTreeNode {
  return {
    id,
    elementId: id,
    uri: 'test',
    name: id,
    rawKind: 'PartUsage',
    kind: 'part',
    depth,
    ownerPath: '',
    children,
  };
}

function attribute(
  id: string,
  lastChangedTick?: number,
  depth = 1,
): AttributeTreeNode {
  return {
    id,
    elementId: id,
    uri: 'test',
    name: id,
    rawKind: 'AttributeUsage',
    kind: 'attribute',
    depth,
    ownerPath: 'root',
    children: [],
    lastChangedTick,
  };
}

function constraint(
  id: string,
  verdict?: ConstraintVerdict,
  depth = 1,
): ConstraintTreeNode {
  return {
    id,
    elementId: id,
    uri: 'test',
    name: id,
    rawKind: 'ConstraintUsage',
    kind: 'constraint',
    depth,
    ownerPath: 'root',
    children: [],
    verdict,
  };
}

describe('aggregateHealth', () => {
  it('returns unobserved when there are no constraints', () => {
    const tree = part('p', [attribute('a')]);
    expect(aggregateHealth(tree)).toBe('unobserved');
  });

  it('returns unobserved when all verdicts are undefined', () => {
    const tree = part('p', [constraint('c1'), constraint('c2')]);
    expect(aggregateHealth(tree)).toBe('unobserved');
  });

  it('returns pass when every verdict passes', () => {
    const tree = part('p', [
      constraint('c1', 'pass'),
      constraint('c2', 'pass'),
    ]);
    expect(aggregateHealth(tree)).toBe('pass');
  });

  it('returns fail if any constraint fails', () => {
    const tree = part('p', [
      constraint('c1', 'pass'),
      constraint('c2', 'fail'),
    ]);
    expect(aggregateHealth(tree)).toBe('fail');
  });

  it('treats error as fail', () => {
    const tree = part('p', [
      constraint('c1', 'pass'),
      constraint('c2', 'error'),
    ]);
    expect(aggregateHealth(tree)).toBe('fail');
  });

  it('returns inconclusive when a mix of pass + inconclusive', () => {
    const tree = part('p', [
      constraint('c1', 'pass'),
      constraint('c2', 'inconclusive'),
    ]);
    expect(aggregateHealth(tree)).toBe('inconclusive');
  });

  it('aggregates across nested sub-parts', () => {
    const child = part('child', [constraint('c_inner', 'fail')], 1);
    const root = part('root', [constraint('c_top', 'pass'), child]);
    expect(aggregateHealth(root)).toBe('fail');
  });
});

describe('pickKeySignals', () => {
  it('returns empty when no attributes are observed', () => {
    const tree = part('p', [attribute('a'), attribute('b')]);
    expect(pickKeySignals(tree)).toEqual([]);
  });

  it('drops attributes with no lastChangedTick', () => {
    const tree = part('p', [
      attribute('a'),
      attribute('b', 3),
      attribute('c'),
    ]);
    const picks = pickKeySignals(tree);
    expect(picks.map((p) => p.node.id)).toEqual(['b']);
  });

  it('sorts by lastChangedTick desc', () => {
    const tree = part('p', [
      attribute('a', 1),
      attribute('b', 5),
      attribute('c', 3),
    ]);
    const picks = pickKeySignals(tree);
    expect(picks.map((p) => p.node.id)).toEqual(['b', 'c', 'a']);
  });

  it('respects the limit', () => {
    const tree = part('p', [
      attribute('a', 1),
      attribute('b', 2),
      attribute('c', 3),
      attribute('d', 4),
      attribute('e', 5),
    ]);
    const picks = pickKeySignals(tree, 2);
    expect(picks.map((p) => p.node.id)).toEqual(['e', 'd']);
  });

  it('breaks ties by depth (shallower first) then name', () => {
    const deepAttr = attribute('deep', 5, 3);
    const shallowAttr = attribute('shallow', 5, 1);
    const tree = part('p', [deepAttr, shallowAttr]);
    const picks = pickKeySignals(tree);
    expect(picks.map((p) => p.node.id)).toEqual(['shallow', 'deep']);
  });
});

describe('countArchetypes', () => {
  it('counts sub-parts excluding self', () => {
    const sub = part('sub', [], 1);
    const root = part('root', [sub]);
    expect(countArchetypes(root).subParts).toBe(1);
  });

  it('counts each archetype independently', () => {
    const root = part('root', [
      attribute('a'),
      attribute('b'),
      constraint('c'),
    ]);
    const counts = countArchetypes(root);
    expect(counts.attributes).toBe(2);
    expect(counts.constraints).toBe(1);
    expect(counts.stateMachines).toBe(0);
    expect(counts.odes).toBe(0);
    expect(counts.subParts).toBe(0);
  });
});
