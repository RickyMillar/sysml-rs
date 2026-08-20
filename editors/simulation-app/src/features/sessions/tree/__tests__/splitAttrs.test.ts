/**
 * splitAttributesByActivity — partitioning tests.
 *
 * Pure unit tests against structural trees (no store / react-query).
 * Verifies the outputs/parameters buckets, ordering preservation,
 * and section id synthesis.
 */
import { describe, expect, it } from 'vitest';
import type { TreeNode } from '@/types/element';
import { buildModelTree } from '../buildModelTree';
import {
  outputsSectionId,
  parametersSectionId,
  splitAttributesByActivity,
} from '../splitAttrs';
import type {
  AttributeTreeNode,
  ModelTreeNode,
  SectionTreeNode,
} from '../types';

import { archetypeForKind } from './testHelpers';

function n(
  id: string,
  name: string | null,
  kind: string,
  children: TreeNode[] = [],
): TreeNode {
  return { id, name, kind, archetype: archetypeForKind(kind), children };
}

const URI = 'file:///w.sysml';

function stampTick(
  tree: readonly ModelTreeNode[],
  stamps: Record<string, number>,
): ModelTreeNode[] {
  const walk = (nodes: readonly ModelTreeNode[]): ModelTreeNode[] =>
    nodes.map((n) => {
      if (n.kind === 'attribute' && n.id in stamps) {
        return {
          ...(n as AttributeTreeNode),
          lastChangedTick: stamps[n.id],
          children: walk(n.children),
        } as ModelTreeNode;
      }
      return { ...n, children: walk(n.children) } as ModelTreeNode;
    });
  return walk(tree);
}

function samplePartTree(): ModelTreeNode[] {
  return buildModelTree(
    [
      n('part', 'GroupHead', 'PartUsage', [
        n('a1', 'bimetalTemp', 'AttributeUsage'),
        n('a2', 'ratedCurrent', 'AttributeUsage'),
        n('a3', 'curveType', 'AttributeUsage'),
        n('sm', 'StationStates', 'StateDefinition'),
        n('c', 'tempBand', 'ConstraintUsage'),
      ]),
    ],
    URI,
  );
}

function findPartChildren(
  tree: readonly ModelTreeNode[],
): readonly ModelTreeNode[] {
  return tree[0].children;
}

describe('splitAttributesByActivity', () => {
  it('recent change → outputs section, stale change → parameters section', () => {
    const tree = stampTick(samplePartTree(), {
      a1: 100, // recent (tick - 100 = 5 ≤ 20)
      a2: 50, // stale (tick - 50 = 55 > 20)
    });
    // a3 has no stamp → parameters.
    const out = splitAttributesByActivity(tree, /* currentTick */ 105);
    const children = findPartChildren(out);
    // Sections first, then non-attribute siblings (sm + constraint).
    expect(children[0].kind).toBe('section');
    expect(children[1].kind).toBe('section');
    const outputs = children[0] as SectionTreeNode;
    const params = children[1] as SectionTreeNode;
    expect(outputs.sectionKind).toBe('outputs');
    expect(outputs.children.map((c) => c.name)).toEqual(['bimetalTemp']);
    expect(params.sectionKind).toBe('parameters');
    expect(params.children.map((c) => c.name).sort()).toEqual([
      'curveType',
      'ratedCurrent',
    ]);
  });

  it('attribute with no value (no lastChangedTick) → rendered flat (no section header when no outputs)', () => {
    const tree = samplePartTree(); // no stamps at all
    const out = splitAttributesByActivity(tree, 42);
    const children = findPartChildren(out);
    // No Outputs section → skip the Parameters header too. Attributes
    // render flat under the part so the user doesn't see a stack of
    // "Parameters (N)" rows that say nothing meaningful.
    expect(children.filter((c) => c.kind === 'section')).toHaveLength(0);
    const flatNames = children.map((c) => c.name);
    // Attributes + non-attributes both present, sorted for stability.
    expect(flatNames.sort()).toEqual([
      'StationStates',
      'bimetalTemp',
      'curveType',
      'ratedCurrent',
      'tempBand',
    ]);
  });

  it('renders Parameters section ONLY when Outputs is also present', () => {
    const stamped = stampTick(samplePartTree(), { a1: 100 }); // 1 output
    const out = splitAttributesByActivity(stamped, 105);
    const children = findPartChildren(out);
    const sections = children.filter((c) => c.kind === 'section');
    // With an Outputs partner the Parameters header earns its keep.
    expect(sections.map((s) => (s as SectionTreeNode).sectionKind)).toEqual([
      'outputs',
      'parameters',
    ]);
  });

  it('non-attribute children keep their relative order AFTER the sections', () => {
    const tree = stampTick(samplePartTree(), { a1: 100 });
    const out = splitAttributesByActivity(tree, 100);
    const children = findPartChildren(out);
    // Expect: Outputs(1) · Parameters(2) · StationStates · tempBand.
    const nonAttrNames = children
      .filter((c) => c.kind !== 'section')
      .map((c) => c.name);
    expect(nonAttrNames).toEqual(['StationStates', 'tempBand']);
  });

  it('staleTicks tunable — 0 forces every attribute into parameters', () => {
    const tree = stampTick(samplePartTree(), {
      a1: 100,
      a2: 100,
      a3: 100,
    });
    const out = splitAttributesByActivity(tree, 100, { staleTicks: 0 });
    // Every lastChangedTick exactly equals currentTick, so
    // tick - lastChangedTick == 0 ≤ 0 — all three land in outputs.
    const stillOutputs = findPartChildren(out);
    const outSection = stillOutputs[0] as SectionTreeNode;
    expect(outSection.sectionKind).toBe('outputs');
    expect(outSection.count).toBe(3);
    // After one extra tick every attribute is stale → all parameters.
    // With no outputs to partner, the Parameters header is skipped
    // and attributes render flat under the part.
    const veryStale = splitAttributesByActivity(tree, 101, {
      staleTicks: 0,
    });
    const children = findPartChildren(veryStale);
    expect(children.filter((c) => c.kind === 'section')).toHaveLength(0);
  });

  it('parts with zero attributes get no section headers (just carry children through)', () => {
    const tree = buildModelTree(
      [
        n('p', 'Empty', 'PartUsage', [
          n('sm', 'InnerSM', 'StateDefinition'),
        ]),
      ],
      URI,
    );
    const out = splitAttributesByActivity(tree, 42);
    const children = findPartChildren(out);
    expect(children.map((c) => c.kind)).toEqual(['sm']);
  });

  it('section ids are synthesised from the parent part id (stable across ticks)', () => {
    const tree = stampTick(samplePartTree(), { a1: 100 });
    const out = splitAttributesByActivity(tree, 100);
    const children = findPartChildren(out);
    const outputs = children.find(
      (c) => c.kind === 'section' && c.name.startsWith('Outputs'),
    )!;
    const params = children.find(
      (c) => c.kind === 'section' && c.name.startsWith('Parameters'),
    )!;
    expect(outputs.id).toBe(outputsSectionId('part'));
    expect(params.id).toBe(parametersSectionId('part'));
  });

  it('count suffix in the section name reflects the bucket size', () => {
    const tree = stampTick(samplePartTree(), {
      a1: 100,
      a2: 100,
      a3: 100,
    });
    const out = splitAttributesByActivity(tree, 100);
    const children = findPartChildren(out);
    const outputs = children[0] as SectionTreeNode;
    expect(outputs.name).toBe('Outputs (3)');
    expect(outputs.count).toBe(3);
  });

  it('recurses through nested parts — sub-part attributes also get split', () => {
    const tree = buildModelTree(
      [
        n('sb', 'ProductionCell', 'PartUsage', [
          n('a', 'temperature', 'AttributeUsage'),
          n('c1', 'Station1', 'PartUsage', [
            n('t', 'bimetalTemp', 'AttributeUsage'),
          ]),
        ]),
      ],
      URI,
    );
    const stamped = stampTick(tree, { a: 100, t: 100 });
    const out = splitAttributesByActivity(stamped, 100);
    // ProductionCell has its attribute split.
    const sbChildren = out[0].children;
    expect(sbChildren.find((c) => c.kind === 'section')).toBeDefined();
    // Inner Station1 part is preserved and also has its attribute split.
    const innerPart = sbChildren.find((c) => c.name === 'Station1')!;
    expect(
      innerPart.children.some(
        (c) => c.kind === 'section' && c.name.startsWith('Outputs'),
      ),
    ).toBe(true);
  });
});
