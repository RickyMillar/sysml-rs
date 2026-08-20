/**
 * buildModelTree — classification + pruning tests.
 *
 * Pure unit tests. No renderer, no live session data — just the
 * taxonomic and structural contract Phase B2's hook (and ultimately
 * the B1 renderer) will rely on.
 */

import { describe, it, expect } from 'vitest';
import type { TreeNode } from '@/types/element';
import {
  buildModelTree,
  classifyTreeNode,
  collectByKind,
  extractSmTopology,
  findPathToNode,
  resolveFocusPath,
  walkModelTree,
} from '../buildModelTree';
import type { ModelTreeNode } from '../types';
import { archetypeForKind } from './testHelpers';

function n(
  id: string,
  name: string | null,
  kind: string,
  children: TreeNode[] = [],
): TreeNode {
  return { id, name, kind, archetype: archetypeForKind(kind), children };
}

/** Variant of `n` for transition nodes — adds backend-projected
 *  `source` / `target` short names (R2.2). */
function tn(
  id: string,
  name: string | null,
  source: string | undefined,
  target: string | undefined,
  children: TreeNode[] = [],
): TreeNode {
  return {
    id,
    name,
    kind: 'TransitionUsage',
    archetype: 'other',
    children,
    source,
    target,
  };
}

/** Variant of `n` for SM nodes — attaches backend-projected
 *  static `transitions` list (R2.1 fixup; the `user_facing` view
 *  filters TransitionUsage children, so the backend surfaces
 *  the static transition descriptors directly on the SM
 *  TreeNode). */
function sm(
  id: string,
  name: string | null,
  kind: string,
  children: TreeNode[] = [],
  transitions: ReadonlyArray<{
    id: string;
    name?: string;
    source?: string;
    target?: string;
  }> = [],
): TreeNode {
  return {
    id,
    name,
    kind,
    archetype: archetypeForKind(kind),
    children,
    transitions,
  };
}

const URI = 'file:///production cell.sysml';

describe('classifyTreeNode (R2.4 — backend-projected archetype)', () => {
  // R2.4 of the backend-first cleansing audit lifted the FE's static
  // `KIND_MAP` into the backend's `Archetype` projection. The backend's
  // `classify_archetype` helper (in `crates/tooling/sysml-service/src/
  // query.rs`) is the authoritative classifier; the FE just reads the
  // `archetype` field off each TreeNode and applies the calc → ode
  // upgrade based on `is_ode`.
  //
  // These tests pin the FE's contract: whatever archetype the backend
  // emits, that's what we render. The full hierarchy coverage
  // (RequirementUsage → constraint, EnumerationUsage → attribute,
  // ExhibitStateUsage → sm, etc.) lives in the backend test
  // `test_classify_archetype_covers_subtype_families`.

  it('reads the backend-projected archetype directly', () => {
    expect(classifyTreeNode(n('p', 'x', 'PartUsage'))).toBe('part');
    expect(classifyTreeNode(n('a', 'x', 'AttributeUsage'))).toBe('attribute');
    expect(classifyTreeNode(n('s', 'x', 'StateDefinition'))).toBe('sm');
    expect(classifyTreeNode(n('c', 'x', 'ConstraintUsage'))).toBe('constraint');
    expect(classifyTreeNode(n('k', 'x', 'CalculationUsage'))).toBe('calc');
  });

  it('upgrades calc → ode when is_ode is set (GAP-ODE-001)', () => {
    const ode: TreeNode = {
      id: 'k',
      name: 'dvdt',
      kind: 'CalculationUsage',
      archetype: 'calc',
      is_ode: true,
      children: [],
    };
    expect(classifyTreeNode(ode)).toBe('ode');
  });

  it('leaves calc as calc when is_ode is absent or false', () => {
    const plainNoFlag: TreeNode = {
      id: 'k',
      name: 'scale',
      kind: 'CalculationUsage',
      archetype: 'calc',
      children: [],
    };
    const explicitFalse: TreeNode = {
      id: 'k2',
      name: 'normalize',
      kind: 'CalculationDefinition',
      archetype: 'calc',
      is_ode: false,
      children: [],
    };
    expect(classifyTreeNode(plainNoFlag)).toBe('calc');
    expect(classifyTreeNode(explicitFalse)).toBe('calc');
  });

  it('does not upgrade non-calc kinds even when is_ode is set (defensive)', () => {
    // A misbehaving backend could stamp is_ode on a non-calc; the FE
    // only fires the upgrade for archetype === 'calc'.
    const misflagged: TreeNode = {
      id: 'a',
      name: 'misflagged',
      kind: 'AttributeUsage',
      archetype: 'attribute',
      is_ode: true,
      children: [],
    };
    expect(classifyTreeNode(misflagged)).toBe('attribute');
  });

  it('falls back to "other" when the backend emits archetype: other', () => {
    // The backend stamps `Archetype::Other` for kinds outside the
    // mapped families (Package, Comment, Subclassification, …); the
    // FE renders whatever it gets without re-classifying.
    expect(classifyTreeNode(n('p', 'pkg', 'Package'))).toBe('other');
    expect(classifyTreeNode(n('p', 'made-up', 'Totally_Made_Up'))).toBe('other');
  });
});

describe('buildModelTree — Port + Connection archetypes', () => {
  // Commit 1 of the model-tree rework adds Port and Connection as their
  // own archetypes. The backend's `is_user_facing_noise` was inverted to
  // a show-by-default policy, so PortUsage / ConnectionUsage / FlowUsage
  // / etc. now survive `view=user_facing` and arrive at the FE. The FE's
  // build switch routes them to dedicated `'port'` / `'connection'`
  // discriminated-union variants.

  it('renders a PortUsage as a port node', () => {
    const input: TreeNode[] = [
      n('p', 'valve', 'PartUsage', [
        n('port1', 'phaseIn', 'PortUsage'),
      ]),
    ];
    const tree = buildModelTree(input, URI);
    const port = tree[0].children[0];
    expect(port.kind).toBe('port');
    expect(port.name).toBe('phaseIn');
    expect(port.rawKind).toBe('PortUsage');
  });

  it('renders a ConnectionUsage as a connection node', () => {
    const input: TreeNode[] = [
      n('p', 'production cell', 'PartUsage', [
        n('c1', 'wire', 'ConnectionUsage'),
      ]),
    ];
    const tree = buildModelTree(input, URI);
    const conn = tree[0].children[0];
    expect(conn.kind).toBe('connection');
    expect(conn.name).toBe('wire');
    expect(conn.rawKind).toBe('ConnectionUsage');
  });

  it('renders a FlowUsage as a connection node', () => {
    const input: TreeNode[] = [
      n('p', 'production cell', 'PartUsage', [
        n('f1', 'powerFlow', 'FlowUsage'),
      ]),
    ];
    const tree = buildModelTree(input, URI);
    const flow = tree[0].children[0];
    expect(flow.kind).toBe('connection');
  });

  it('renders a PortDefinition as a port node', () => {
    const input: TreeNode[] = [
      n('pd', 'PowerPort', 'PortDefinition'),
    ];
    const tree = buildModelTree(input, URI);
    expect(tree[0].kind).toBe('port');
  });

  it('renders a ConjugatedPortDefinition as a port node', () => {
    const input: TreeNode[] = [
      n('pd', '~PowerPort', 'ConjugatedPortDefinition'),
    ];
    const tree = buildModelTree(input, URI);
    expect(tree[0].kind).toBe('port');
  });

  it('maps backend default_collapsed → defaultCollapsed on Port nodes', () => {
    // Commit 2: backend sets default_collapsed=true on Port +
    // Connection nodes that have inlined children. The FE preserves
    // that signal under camelCase for the seeder to consume.
    const input: TreeNode[] = [
      {
        id: 'p1',
        name: 'phaseIn',
        kind: 'PortUsage',
        archetype: 'port',
        default_collapsed: true,
        children: [
          n('a1', 'temperature', 'AttributeUsage'),
        ],
      },
    ];
    const tree = buildModelTree(input, URI);
    expect(tree[0].kind).toBe('port');
    expect(tree[0].defaultCollapsed).toBe(true);
  });

  it('maps backend default_collapsed → defaultCollapsed on Connection nodes', () => {
    const input: TreeNode[] = [
      {
        id: 'c1',
        name: 'wire',
        kind: 'ConnectionUsage',
        archetype: 'connection',
        default_collapsed: true,
        children: [
          n('a1', 'current', 'AttributeUsage'),
        ],
      },
    ];
    const tree = buildModelTree(input, URI);
    expect(tree[0].kind).toBe('connection');
    expect(tree[0].defaultCollapsed).toBe(true);
  });

  it('omits defaultCollapsed when the backend did not set it', () => {
    const input: TreeNode[] = [n('p1', 'phaseIn', 'PortUsage')];
    const tree = buildModelTree(input, URI);
    expect(tree[0].defaultCollapsed).toBeUndefined();
  });
});

describe('buildModelTree — backend-first contract (R2.1)', () => {
  // R2.1 of the backend-cleansing audit moved noise filtering server-
  // side. The FE asks for `?view=user_facing`; the backend's
  // `is_user_facing_noise` mirrors the old PRUNE_KINDS set and drops
  // memberships, type bindings, expression AST, ports, flows,
  // connections, transitions, and chrome before the response is sent.
  //
  // These tests assert the FE no longer filters anything itself —
  // whatever the backend gives, we render. Backend-side tests
  // (`crates/tooling/sysml-service/src/query.rs::test_model_tree_user_
  // facing_drops_noise_kinds`) cover the actual noise filtering.

  it('renders whatever children the backend hands over without secondary filtering', () => {
    // Even kinds that USED to be pruned (PortUsage, FlowUsage,
    // OwningMembership) survive when present — the FE trusts the
    // backend. In production those never arrive under user_facing,
    // but a `view=full` consumer (or a backend regression) shouldn't
    // get its content silently dropped client-side.
    const input = [
      n('root', 'ProductionCell', 'PartUsage', [
        n('m1', null, 'OwningMembership'),
        n('a1', 'temperature', 'AttributeUsage'),
        n('p1', 'phaseIn', 'PortUsage'),
      ]),
    ];
    const out = buildModelTree(input, URI);
    // Three children make it through; the membership / port classify
    // as 'other' and the attribute as 'attribute'. Sibling sort puts
    // the named attribute ahead of unnamed others.
    const names = out[0].children.map((c) => c.name).sort();
    expect(names).toContain('temperature');
    expect(names).toContain('phaseIn');
  });

  it('keeps named "other" nodes alongside their children', () => {
    const input = [
      n('p1', 'ProductionCell', 'PartUsage', [
        n('wrap', 'ElectricalGroup', 'MadeUpKind', [
          n('a1', 'current', 'AttributeUsage'),
        ]),
      ]),
    ];
    const out = buildModelTree(input, URI);
    const wrap = out[0].children[0];
    expect(wrap.kind).toBe('other');
    expect(wrap.name).toBe('ElectricalGroup');
    expect(wrap.children[0].name).toBe('current');
  });

  it('flattens packages so namespacing chrome does not clutter the Run page (default)', () => {
    const input = [
      n('pkg', 'Electrical', 'Package', [
        n('p1', 'ProductionCell', 'PartUsage'),
        n('p2', 'Valve', 'PartUsage'),
      ]),
    ];
    const out = buildModelTree(input, URI);
    // Package vanishes; its parts rise to the root.
    expect(out.map((m) => m.name)).toEqual(['ProductionCell', 'Valve']);
    expect(out.every((m) => m.depth === 0)).toBe(true);
  });

  it('flattenPackages:false keeps the package as an "other" container', () => {
    const input = [
      n('pkg', 'Root', 'Package', [
        n('p1', 'ProductionCell', 'PartUsage'),
      ]),
    ];
    const out = buildModelTree(input, URI, { flattenPackages: false });
    expect(out).toHaveLength(1);
    expect(out[0].kind).toBe('other');
    expect(out[0].rawKind).toBe('Package');
    expect(out[0].children[0].kind).toBe('part');
    expect(out[0].children[0].depth).toBe(1);
  });
});

describe('buildModelTree — metadata', () => {
  it('stamps depth as 0 at the root and increments with nesting', () => {
    const input = [
      n('p1', 'ProductionCell', 'PartUsage', [
        n('p2', 'Station1', 'PartUsage', [
          n('p3', 'GroupHead', 'PartUsage'),
        ]),
      ]),
    ];
    const out = buildModelTree(input, URI);
    expect(out[0].depth).toBe(0);
    expect(out[0].children[0].depth).toBe(1);
    expect(out[0].children[0].children[0].depth).toBe(2);
  });

  it('builds dotted ownerPath using element names', () => {
    const input = [
      n('p1', 'ProductionCell', 'PartUsage', [
        n('p2', 'Station1', 'PartUsage', [
          n('a1', 'temperature', 'AttributeUsage'),
        ]),
      ]),
    ];
    const out = buildModelTree(input, URI);
    expect(out[0].ownerPath).toBe('');
    expect(out[0].children[0].ownerPath).toBe('ProductionCell');
    expect(out[0].children[0].children[0].ownerPath).toBe('ProductionCell.Station1');
  });

  it('preserves id and uri on every produced node', () => {
    const input = [n('p1', 'ProductionCell', 'PartUsage')];
    const out = buildModelTree(input, URI);
    expect(out[0].id).toBe('p1');
    expect(out[0].uri).toBe(URI);
  });

  it('uses "(unnamed)" for elements without a name', () => {
    const input = [n('p1', null, 'PartUsage')];
    const out = buildModelTree(input, URI);
    expect(out[0].name).toBe('(unnamed)');
  });

  it('does NOT attach live-state fields (that is the merge step\'s job)', () => {
    const input = [n('a1', 'temperature', 'AttributeUsage')];
    const out = buildModelTree(input, URI);
    const attr = out[0];
    if (attr.kind !== 'attribute')
      throw new Error('expected attribute node');
    expect(attr.value).toBeUndefined();
    expect(attr.unit).toBeUndefined();
    expect(attr.verdict).toBeUndefined();
  });
});

describe('buildModelTree — real-ish production cell shape', () => {
  it('produces the expected nested archetypes for a espresso-production-cell-like input', () => {
    // R3.2 + R3.4: backend is authoritative for sibling order, so this
    // fixture is built in the order the backend would emit (Part →
    // Sm → Constraint → Calc → Attribute). The FE renders backend
    // order verbatim.
    const input: TreeNode[] = [
      n('pkg', 'ProductionCell', 'Package', [
        n('sb', 'ProductionCell', 'PartUsage', [
          n('main', 'Main Switch', 'PartUsage', [
            n('sm1', 'MainSwitchStates', 'StateDefinition'),
          ]),
          n('c1', 'Station 1', 'PartUsage', [
            n('groupHead', 'GroupHead', 'PartUsage', [
              // Pre-sorted as the backend would emit:
              // sm → constraint → attribute.
              n('sm2', 'StationStates', 'StateDefinition'),
              n('ct', 'tempBand', 'ConstraintUsage'),
              n('temp', 'bimetalTemp', 'AttributeUsage'),
            ]),
          ]),
          n('ode', 'thermal', 'CalculationUsage'),
        ]),
      ]),
    ];
    const tree = buildModelTree(input, URI);
    // Package collapses; ProductionCell rises to root.
    expect(tree.map((m) => m.name)).toEqual(['ProductionCell']);
    const sb = tree[0];
    expect(sb.kind).toBe('part');
    // Main Switch + Station 1 + thermal calc all sit under
    // ProductionCell. Two parts before the calc — input was pre-sorted.
    const childKinds = sb.children.map((c) => c.kind);
    expect(childKinds).toEqual(['part', 'part', 'calc']);
    // Find GroupHead by name so the lookup doesn't depend on which sibling
    // part comes first.
    const groupHead = sb.children
      .flatMap((c) => c.children)
      .find((c) => c.name === 'GroupHead');
    expect(groupHead).toBeDefined();
    // GroupHead's children render in the order they arrive from the
    // backend — the FE no longer re-sorts.
    expect(groupHead!.children.map((c) => c.kind)).toEqual([
      'sm',
      'constraint',
      'attribute',
    ]);
  });
});

describe('buildModelTree — backend-authoritative order (R3.2 + R3.4)', () => {
  // R3.2 + R3.4 of the backend-first cleansing audit moved sibling
  // (name, kind) dedupe and archetype-rank sort server-side. The
  // backend's `build_tree_node` (`crates/tooling/sysml-service/src/
  // query.rs`) applies both before serialising. The FE preserves
  // whatever order the backend hands it — no FE-side sort, no
  // FE-side dedupe. Backend tests `test_model_tree_dedupes_typed_
  // def_inlined_children` and `test_model_tree_sorts_children_by_
  // archetype` cover the actual dedupe / sort behaviour.

  it('preserves the order the backend emitted (no FE-side re-sort)', () => {
    // Fixture pretends the backend handed down a deliberately
    // unusual order — the FE must render it verbatim. If the FE
    // ever sneaks a sort back in, this test fails.
    const input: TreeNode[] = [
      n('p', 'Root', 'PartUsage', [
        n('a', 'value', 'AttributeUsage'),
        n('sm', 'states', 'StateDefinition'),
        n('part', 'sub', 'PartUsage'),
        n('calc', 'scale', 'CalculationUsage'),
        n('c', 'bound', 'ConstraintUsage'),
      ]),
    ];
    const tree = buildModelTree(input, URI);
    expect(tree[0].children.map((c) => c.name)).toEqual([
      'value',
      'states',
      'sub',
      'scale',
      'bound',
    ]);
  });

  it('renders duplicate (name, rawKind) siblings verbatim — backend dedupes', () => {
    // The FE no longer dedupes. If the backend hands down four
    // `power` ItemUsages (it shouldn't post-R3.2, but we still
    // shouldn't silently drop them client-side), the FE renders
    // every one. Backend test `test_model_tree_dedupes_typed_def_
    // inlined_children` covers the actual collapse.
    const input: TreeNode[] = [
      n('p', 'valve', 'PartUsage', [
        n('i1', 'power', 'ItemUsage'),
        n('i2', 'power', 'ItemUsage'),
        n('i3', 'power', 'ItemUsage'),
        n('i4', 'cmd', 'ItemUsage'),
      ]),
    ];
    const tree = buildModelTree(input, URI);
    const names = tree[0].children.map((c) => c.name);
    expect(names).toEqual(['power', 'power', 'power', 'cmd']);
  });

  it('preserves root-level order from the backend (no FE-side re-sort)', () => {
    // Deliberately reverse-archetype-rank input — the FE doesn't
    // sort it, so the output mirrors authoring order.
    const input: TreeNode[] = [
      n('a1', 'rootAttr', 'AttributeUsage'),
      n('p1', 'rootPart', 'PartUsage'),
    ];
    const tree = buildModelTree(input, URI);
    expect(tree.map((n) => n.name)).toEqual(['rootAttr', 'rootPart']);
  });
});

describe('buildModelTree — Subclassification + ReferenceUsage backend filtering', () => {
  // Subclassification + ReferenceUsage are both in the backend's
  // `is_user_facing_noise` set (R2.1) — under `view=user_facing` they
  // never reach the FE. The matching backend test is
  // `crates/tooling/sysml-service/src/query.rs::test_model_tree_user_
  // facing_drops_noise_kinds`. The FE just renders whatever it
  // receives, so when those kinds DO arrive (e.g. a `view=full`
  // consumer) we keep them as `'other'` rows rather than silently
  // dropping them.
  it('renders Subclassification rows as "other" (no FE-side filtering)', () => {
    const input: TreeNode[] = [
      n('p', 'root', 'PartUsage', [
        n('sc1', null, 'Subclassification'),
        n('a1', 'temperature', 'AttributeUsage'),
      ]),
    ];
    const tree = buildModelTree(input, URI);
    const kinds = tree[0].children.map((c) => c.kind).sort();
    expect(kinds).toEqual(['attribute', 'other']);
  });

  it('renders ReferenceUsage rows as "other" (no FE-side filtering)', () => {
    const input: TreeNode[] = [
      n('p', 'root', 'PartUsage', [
        n('r', 'referencesRecord', 'ReferenceUsage'),
        n('a1', 'temperature', 'AttributeUsage'),
      ]),
    ];
    const tree = buildModelTree(input, URI);
    // Both rows survive client-side; backend would drop the
    // ReferenceUsage in user_facing mode before it ever arrives.
    const names = tree[0].children.map((c) => c.name).sort();
    expect(names).toEqual(['referencesRecord', 'temperature']);
  });
});

describe('buildModelTree — authoritative is_ode flag (GAP-ODE-001)', () => {
  it('upgrades a CalculationUsage to "ode" when the backend stamps is_ode: true', () => {
    const input: TreeNode[] = [
      n('pkg', 'Dynamics', 'PartUsage', [
        {
          id: 'dvdt',
          name: 'dvdt',
          kind: 'CalculationUsage',
          archetype: 'calc',
          is_ode: true,
          children: [],
        },
      ]),
    ];
    const tree = buildModelTree(input, URI);
    const calc = tree[0].children[0];
    expect(calc.kind).toBe('ode');
  });

  it('leaves a CalculationDefinition as "calc" when is_ode is absent or false', () => {
    const input: TreeNode[] = [
      n('pkg', 'Dynamics', 'PartUsage', [
        // is_ode omitted → plain calc
        n('plain', 'scale', 'CalculationUsage'),
        // is_ode explicitly false → plain calc
        {
          id: 'explicitFalse',
          name: 'normalize',
          kind: 'CalculationDefinition',
          archetype: 'calc',
          is_ode: false,
          children: [],
        },
      ]),
    ];
    const tree = buildModelTree(input, URI);
    const childKinds = tree[0].children.map((c) => c.kind);
    expect(childKinds).toEqual(['calc', 'calc']);
  });

  it('never promotes non-calc kinds even if is_ode is set (defensive)', () => {
    // Attributes / parts should NEVER accept an is_ode upgrade — the
    // backend should only stamp the flag on calcs, but the frontend
    // classifier guards anyway so a misbehaving backend can't change
    // an AttributeUsage into an ODE row.
    const input: TreeNode[] = [
      {
        id: 'a1',
        name: 'misflagged',
        kind: 'AttributeUsage',
        archetype: 'attribute',
        is_ode: true,
        children: [],
      },
    ];
    const tree = buildModelTree(input, URI);
    expect(tree[0].kind).toBe('attribute');
  });
});

describe('findPathToNode / resolveFocusPath', () => {
  const tree = buildModelTree(
    [
      n('sb', 'ProductionCell', 'PartUsage', [
        n('c1', 'Station1', 'PartUsage', [
          n('groupHead', 'GroupHead', 'PartUsage', [n('temp', 'bimetalTemp', 'AttributeUsage')]),
        ]),
      ]),
    ],
    URI,
  );

  it('findPathToNode returns the root→target chain for a deep id', () => {
    const path = findPathToNode(tree, 'temp');
    expect(path?.map((n) => n.name)).toEqual([
      'ProductionCell',
      'Station1',
      'GroupHead',
      'bimetalTemp',
    ]);
  });

  it('findPathToNode returns null when the id is not in the tree', () => {
    expect(findPathToNode(tree, 'nonexistent')).toBeNull();
  });

  it('resolveFocusPath walks id-by-id and returns the best prefix available', () => {
    const out = resolveFocusPath(tree, ['sb', 'c1', 'groupHead']);
    expect(out.map((n) => n.name)).toEqual([
      'ProductionCell',
      'Station1',
      'GroupHead',
    ]);
  });

  it('resolveFocusPath stops at the last valid id when path goes stale', () => {
    const out = resolveFocusPath(tree, ['sb', 'c1', 'gone']);
    expect(out.map((n) => n.name)).toEqual(['ProductionCell', 'Station1']);
  });

  it('resolveFocusPath([]) returns an empty chain (root focus)', () => {
    expect(resolveFocusPath(tree, [])).toEqual([]);
  });
});

describe('walkModelTree / collectByKind', () => {
  function sampleTree(): ModelTreeNode[] {
    // R3.4: backend hands down already-sorted children. Fixture
    // mirrors what the backend would emit (Part → Sm → Constraint
    // → Attribute) so DFS-order tests stay deterministic.
    const input: TreeNode[] = [
      n('p', 'Root', 'PartUsage', [
        n('p2', 'Inner', 'PartUsage', [
          n('c', 'ct', 'ConstraintUsage'),
          n('a2', 'x', 'AttributeUsage'),
        ]),
        n('s', 'SM', 'StateDefinition'),
        n('a1', 'v', 'AttributeUsage'),
      ]),
    ];
    return buildModelTree(input, URI);
  }

  it('walkModelTree visits every node in DFS order with correct parents', () => {
    const seen: Array<[string, string | null]> = [];
    walkModelTree(sampleTree(), (n, parent) => {
      seen.push([n.name, parent?.name ?? null]);
    });
    // Backend hands children down already-sorted (R3.4); FE
    // preserves order verbatim. DFS visit reflects that order.
    expect(seen).toEqual([
      ['Root', null],
      ['Inner', 'Root'],
      ['ct', 'Inner'],
      ['x', 'Inner'],
      ['SM', 'Root'],
      ['v', 'Root'],
    ]);
  });

  it('collectByKind returns every node of a given archetype', () => {
    const tree = sampleTree();
    expect(collectByKind(tree, 'attribute').map((n) => n.name)).toEqual([
      // Inner's `x` surfaces before Root-level `v` because Inner is
      // visited first under the backend's part-before-attribute order.
      'x',
      'v',
    ]);
    expect(collectByKind(tree, 'sm').map((n) => n.name)).toEqual(['SM']);
    expect(collectByKind(tree, 'constraint').map((n) => n.name)).toEqual(['ct']);
    expect(collectByKind(tree, 'ode')).toHaveLength(0);
  });
});

describe('extractSmTopology', () => {
  it('pulls StateUsage children + reads transitions from the backend-projected field', () => {
    // Backend now surfaces transitions on the SM TreeNode itself
    // (R2.1 fixup) — TransitionUsage children are filtered in
    // `user_facing` view, so the FE no longer walks for them.
    const smNode = sm(
      'sm',
      'StationStates',
      'StateDefinition',
      [
        n('s1', 'armed', 'StateUsage'),
        n('s2', 'tripped', 'StateUsage'),
        // Noise — ignored.
        n('a1', 'ratedCurrent', 'AttributeUsage'),
        n('doc', null, 'Documentation'),
      ],
      [
        {
          id: 't1',
          name: 'armed_to_tripped',
          source: 'armed',
          target: 'tripped',
        },
      ],
    );
    const { states, transitions } = extractSmTopology(smNode);
    expect(states).toEqual([
      { id: 's1', name: 'armed' },
      { id: 's2', name: 'tripped' },
    ]);
    expect(transitions).toEqual([
      {
        id: 't1',
        name: 'armed_to_tripped',
        source: 'armed',
        target: 'tripped',
      },
    ]);
  });

  it('skips unnamed StateUsage children', () => {
    const smNode = n('sm', 'SM', 'StateDefinition', [
      n('s1', null, 'StateUsage'),
      n('s2', 'running', 'StateUsage'),
    ]);
    const { states } = extractSmTopology(smNode);
    expect(states).toEqual([{ id: 's2', name: 'running' }]);
  });

  it('carries ExhibitStateUsage too', () => {
    const smNode = n('sm', 'SM', 'StateDefinition', [
      n('s1', 'primary', 'ExhibitStateUsage'),
    ]);
    expect(extractSmTopology(smNode).states).toEqual([
      { id: 's1', name: 'primary' },
    ]);
  });

  it('transitions without backend-provided source/target stay undefined', () => {
    // Backend leaves source/target undefined when the TransitionUsage's
    // `source` / `target` props don't resolve (e.g. unresolved name, or
    // an oddly-shaped transition). FE no longer falls back to a regex.
    const smNode = sm(
      'sm',
      'SM',
      'StateDefinition',
      [],
      [{ id: 't1', name: 'weird transition' }],
    );
    const { transitions } = extractSmTopology(smNode);
    expect(transitions[0].source).toBeUndefined();
    expect(transitions[0].target).toBeUndefined();
    expect(transitions[0].name).toBe('weird transition');
  });

  it('reads transitions from the SM TreeNode field, not from children', () => {
    // Sanity-check the new contract: even if a stray TransitionUsage
    // child is present (e.g. `view=full`), `extractSmTopology` reads
    // exclusively from `smNode.transitions`. This is the property
    // that keeps the static state-graph correct in `user_facing`
    // mode where children are filtered.
    const smNode = sm(
      'sm',
      'SM',
      'StateDefinition',
      [
        // Stray TransitionUsage child — must be ignored.
        tn('child-t', 'should_not_be_used', 'a', 'b'),
      ],
      [{ id: 'projected-t', name: 'real_transition', source: 'x', target: 'y' }],
    );
    const { transitions } = extractSmTopology(smNode);
    expect(transitions).toEqual([
      {
        id: 'projected-t',
        name: 'real_transition',
        source: 'x',
        target: 'y',
      },
    ]);
  });
});

describe('buildModelTree — SM topology attached to SmTreeNode', () => {
  it('SmTreeNode carries states + transitions from the SM TreeNode projection', () => {
    const tree = buildModelTree(
      [
        sm(
          'sm',
          'StationStates',
          'StateDefinition',
          [
            n('s1', 'armed', 'StateUsage'),
            n('s2', 'tripped', 'StateUsage'),
          ],
          [
            {
              id: 't1',
              name: 'armed_to_tripped',
              source: 'armed',
              target: 'tripped',
            },
          ],
        ),
      ],
      URI,
    );
    const smOut = tree[0];
    if (smOut.kind !== 'sm') throw new Error('expected sm node');
    expect(smOut.states).toEqual([
      { id: 's1', name: 'armed' },
      { id: 's2', name: 'tripped' },
    ]);
    expect(smOut.transitions).toEqual([
      {
        id: 't1',
        name: 'armed_to_tripped',
        source: 'armed',
        target: 'tripped',
      },
    ]);
  });
});
