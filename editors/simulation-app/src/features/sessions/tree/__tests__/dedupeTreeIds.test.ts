import { describe, it, expect } from 'vitest';
import { dedupeTreeIds } from '../useSessionModelTree';
import type { PartTreeNode } from '../types';

const part = (id: string, name: string, children: PartTreeNode[] = []): PartTreeNode => ({
  id,
  elementId: id,
  uri: 'test://uri',
  name,
  rawKind: 'PartUsage',
  kind: 'part',
  depth: 0,
  ownerPath: '',
  children,
});

describe('dedupeTreeIds — deterministic rewrites', () => {
  it('same input produces same ids on every call (regression: SessionTreeV2 vs DetailPanel diverged trees)', () => {
    // Two parts share the same backend id (cross-file inlining).
    // dedupeTreeIds must rewrite the second occurrence to a stable
    // suffixed id so independent consumers (SessionTreeV2 +
    // DetailPanel) see identical ids and focusPath survives the
    // round trip.
    const buildTree = () => [
      part('shared', 'A'),
      part('root', 'Root', [part('shared', 'B'), part('shared', 'C')]),
    ];

    const t1 = buildTree();
    dedupeTreeIds(t1);
    const t2 = buildTree();
    dedupeTreeIds(t2);

    const collect = (nodes: readonly PartTreeNode[]): string[] =>
      nodes.flatMap((n) => [n.id, ...collect(n.children as PartTreeNode[])]);
    expect(collect(t1)).toEqual(collect(t2));
  });

  it('first occurrence keeps original id; second uses #1 suffix; third uses #2', () => {
    const tree = [part('x', 'first'), part('x', 'second'), part('x', 'third')];
    dedupeTreeIds(tree);
    expect(tree.map((n) => n.id)).toEqual(['x', 'x#1', 'x#2']);
    // elementId stash preserves the real backend id for live-value lookup.
    expect(tree.map((n) => n.elementId)).toEqual(['x', 'x', 'x']);
  });
});
