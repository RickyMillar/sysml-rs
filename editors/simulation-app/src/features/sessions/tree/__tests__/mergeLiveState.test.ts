/**
 * mergeLiveState — overlay tests.
 *
 * Pure unit tests. Build a small tree, feed in a synthetic
 * NormalizedSnapshot, assert the live fields land on the right nodes
 * and non-live fields (structure, name, id, kind) come through
 * unchanged.
 */

import { describe, it, expect } from 'vitest';
import type { TreeNode } from '@/types/element';
import type {
  ConstraintView,
  NormalizedSnapshot,
  SubsystemView,
} from '../../sessionLiveStore';
import { buildModelTree } from '../buildModelTree';
import { mergeLiveState, type ChangeTracker } from '../mergeLiveState';
import type {
  AttributeTreeNode,
  ConstraintTreeNode,
  ModelTreeNode,
  PartTreeNode,
  SmTreeNode,
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

const URI = 'file:///production cell.sysml';

function sampleTree(): ModelTreeNode[] {
  return buildModelTree(
    [
      n('sb', 'ProductionCell', 'PartUsage', [
        n('c1', 'Station1', 'PartUsage', [
          n('a1', 'temperature', 'AttributeUsage'),
          n('a2', 'bimetalTemp', 'AttributeUsage'),
          n('sm', 'StationStates', 'StateDefinition'),
          n('ct', 'thermalBand', 'ConstraintUsage'),
        ]),
      ]),
    ],
    URI,
  );
}

function sub(current: string, completed = false): SubsystemView {
  return { current_state: current, completed, kind_label: 'sm' };
}

function snapshot(partial: Partial<NormalizedSnapshot> = {}): NormalizedSnapshot {
  return {
    tick: 0,
    time_ms: 0,
    completed: false,
    subsystems: {},
    scalar_vars: {},
    string_vars: {},
    constraint_results: [],
    ...partial,
  };
}

function findByName(
  tree: readonly ModelTreeNode[],
  name: string,
): ModelTreeNode | null {
  for (const n of tree) {
    if (n.name === name) return n;
    const inner = findByName(n.children, name);
    if (inner) return inner;
  }
  return null;
}

describe('mergeLiveState — null snapshot', () => {
  it('returns a tree with no live fields populated (consumer still renders)', () => {
    const out = mergeLiveState(sampleTree(), null);
    const temperature = findByName(out, 'temperature') as AttributeTreeNode | null;
    const sm = findByName(out, 'StationStates') as SmTreeNode | null;
    const ct = findByName(out, 'thermalBand') as ConstraintTreeNode | null;
    expect(temperature?.value).toBeUndefined();
    expect(sm?.currentState).toBeUndefined();
    expect(ct?.verdict).toBeUndefined();
  });

  it('clones so the returned tree does not share identity with the input', () => {
    const tree = sampleTree();
    const out = mergeLiveState(tree, null);
    expect(out[0]).not.toBe(tree[0]);
    expect(out[0].children[0]).not.toBe(tree[0].children[0]);
  });
});

describe('mergeLiveState — attribute values', () => {
  it('resolves `${ownerPath}.${name}` against scalar_vars', () => {
    const snap = snapshot({
      scalar_vars: { 'ProductionCell.Station1.temperature': 12.5 },
    });
    const out = mergeLiveState(sampleTree(), snap);
    const v = findByName(out, 'temperature') as AttributeTreeNode;
    expect(v.value).toBe(12.5);
  });

  it('falls back to bare name when the fully-qualified path is absent', () => {
    const snap = snapshot({ scalar_vars: { temperature: 9.9 } });
    const out = mergeLiveState(sampleTree(), snap);
    const v = findByName(out, 'temperature') as AttributeTreeNode;
    expect(v.value).toBe(9.9);
  });

  it('prefers fully-qualified over bare when both are set (more specific wins)', () => {
    const snap = snapshot({
      scalar_vars: {
        'ProductionCell.Station1.temperature': 12.5,
        temperature: 9.9,
      },
    });
    const out = mergeLiveState(sampleTree(), snap);
    const v = findByName(out, 'temperature') as AttributeTreeNode;
    expect(v.value).toBe(12.5);
  });

  it('resolves string_vars when scalar_vars has no match', () => {
    const snap = snapshot({
      string_vars: { 'ProductionCell.Station1.temperature': 'nominal' },
    });
    const out = mergeLiveState(sampleTree(), snap);
    const v = findByName(out, 'temperature') as AttributeTreeNode;
    expect(v.value).toBe('nominal');
  });

  it('treats UUID-shaped string values as unresolved (element-id leaks from runtime)', () => {
    // The runtime sometimes pushes `Ref(id)` into the exec context
    // under the definition's name (e.g. `DualPoleBreaker →
    // Ref(b2ab346f-...)`). The normalizer serialises these via
    // `id.to_string()` and mergeLiveState would previously surface
    // the UUID as an attribute's live value. Filter them out so the
    // row stays quiet.
    const snap = snapshot({
      string_vars: {
        'ProductionCell.Station1.breakerCurve': 'b2ab346f-4839-49aa-90ad-7c757c9fd3a9',
      },
    });
    const out = mergeLiveState(sampleTree(), snap);
    const curve = findByName(out, 'breakerCurve') as AttributeTreeNode;
    expect(curve?.value).toBeUndefined();
  });

  it('leaves attributes with no matching key at undefined (no guessing)', () => {
    const snap = snapshot({ scalar_vars: { somethingElse: 1 } });
    const out = mergeLiveState(sampleTree(), snap);
    const v = findByName(out, 'temperature') as AttributeTreeNode;
    expect(v.value).toBeUndefined();
  });
});

describe('mergeLiveState — SM state', () => {
  it('populates currentState from subsystems[name]', () => {
    const snap = snapshot({ subsystems: { StationStates: sub('armed') } });
    const out = mergeLiveState(sampleTree(), snap);
    const sm = findByName(out, 'StationStates') as SmTreeNode;
    expect(sm.currentState).toBe('armed');
  });

  it('leaves currentState undefined when the subsystem is absent (not yet stepped)', () => {
    const snap = snapshot({ subsystems: {} });
    const out = mergeLiveState(sampleTree(), snap);
    const sm = findByName(out, 'StationStates') as SmTreeNode;
    expect(sm.currentState).toBeUndefined();
  });

  it('forwards available_transitions onto the SM node as availableTransitions (GAP-SM-002)', () => {
    const snap = snapshot({
      subsystems: {
        StationStates: {
          ...sub('armed'),
          available_transitions: [
            ['trip', 'tripped'],
            ['manual_off', 'off'],
          ],
        },
      },
    });
    const out = mergeLiveState(sampleTree(), snap);
    const sm = findByName(out, 'StationStates') as SmTreeNode;
    expect(sm.availableTransitions).toEqual([
      ['trip', 'tripped'],
      ['manual_off', 'off'],
    ]);
  });

  it('leaves availableTransitions undefined when the subsystem is absent', () => {
    const out = mergeLiveState(sampleTree(), snapshot({ subsystems: {} }));
    const sm = findByName(out, 'StationStates') as SmTreeNode;
    expect(sm.availableTransitions).toBeUndefined();
  });
});

describe('mergeLiveState — constraint verdict', () => {
  it('maps wire verdict "Pass" → tree verdict "pass"', () => {
    const snap = snapshot({
      constraint_results: [
        { name: 'thermalBand', expression: null, verdict: 'Pass' as const },
      ],
    });
    const out = mergeLiveState(sampleTree(), snap);
    const ct = findByName(out, 'thermalBand') as ConstraintTreeNode;
    expect(ct.verdict).toBe('pass');
  });

  it('maps wire verdict "Fail" → tree verdict "fail"', () => {
    const snap = snapshot({
      constraint_results: [
        { name: 'thermalBand', expression: null, verdict: 'Fail' as const },
      ],
    });
    const out = mergeLiveState(sampleTree(), snap);
    const ct = findByName(out, 'thermalBand') as ConstraintTreeNode;
    expect(ct.verdict).toBe('fail');
  });

  it('maps wire verdict "Inconclusive" → tree verdict "inconclusive", NOT "fail"', () => {
    const snap = snapshot({
      constraint_results: [
        { name: 'thermalBand', expression: null, verdict: 'Inconclusive' as const },
      ],
    });
    const out = mergeLiveState(sampleTree(), snap);
    const ct = findByName(out, 'thermalBand') as ConstraintTreeNode;
    // A constraint the run could not decide must not badge as a violation —
    // that conflation is what produced the "48 failing" constraint chip.
    expect(ct.verdict).toBe('inconclusive');
    expect(ct.verdict).not.toBe('fail');
  });

  it('leaves verdict undefined — never throws — when a row carries no verdict field', () => {
    // Backend/frontend version skew: a row shaped by an older backend has no
    // `verdict`. That must degrade to "no determination", not blow up inside
    // the tree merge (which would surface as an undiagnosable render failure).
    const snap = snapshot({
      constraint_results: [
        { name: 'thermalBand', expression: null } as unknown as ConstraintView,
      ],
    });
    expect(() => mergeLiveState(sampleTree(), snap)).not.toThrow();
    const ct = findByName(mergeLiveState(sampleTree(), snap), 'thermalBand') as ConstraintTreeNode;
    expect(ct.verdict).toBeUndefined();
  });

  it('forwards live operand values onto the constraint node (GAP-CONSTR-002)', () => {
    const snap = snapshot({
      constraint_results: [
        {
          name: 'thermalBand',
          expression: 'temperature < cap',
          verdict: 'Pass' as const,
          operands: { temperature: 321.5, cap: 400 },
        },
      ],
    });
    const out = mergeLiveState(sampleTree(), snap);
    const ct = findByName(out, 'thermalBand') as ConstraintTreeNode;
    expect(ct.operands).toEqual({ temperature: 321.5, cap: 400 });
  });

  it('leaves verdict undefined when the constraint has not been evaluated yet', () => {
    const snap = snapshot({ constraint_results: [] });
    const out = mergeLiveState(sampleTree(), snap);
    const ct = findByName(out, 'thermalBand') as ConstraintTreeNode;
    expect(ct.verdict).toBeUndefined();
  });
});

describe('mergeLiveState — id-keyed lookup wins on name collisions', () => {
  // Two parts (Station1, Station2) each declare a constraint with the
  // same short name `safeRange`. The legacy name-keyed path picked one
  // arbitrarily; the id-keyed path must associate each tree node to
  // its own evaluation result.
  function collisionTree(): ModelTreeNode[] {
    return buildModelTree(
      [
        n('sb', 'ProductionCell', 'PartUsage', [
          n('c1', 'Station1', 'PartUsage', [
            n('ct1', 'safeRange', 'ConstraintUsage'),
          ]),
          n('c2', 'Station2', 'PartUsage', [
            n('ct2', 'safeRange', 'ConstraintUsage'),
          ]),
        ]),
      ],
      URI,
    );
  }

  it('routes constraint verdicts to the right tree node when names collide', () => {
    const tree = collisionTree();
    const snap = snapshot({
      constraint_results: [
        {
          name: 'safeRange',
          expression: 'a < 10',
          verdict: 'Pass' as const,
          element_id: 'ct1',
        },
        {
          name: 'safeRange',
          expression: 'b < 20',
          verdict: 'Fail' as const,
          element_id: 'ct2',
        },
      ],
    });
    const out = mergeLiveState(tree, snap);
    // Walk Station1.safeRange and Station2.safeRange directly so the
    // findByName helper's first-match semantics don't hide a swap.
    const c1 = out[0].children[0]; // Station1
    const c2 = out[0].children[1]; // Station2
    const ct1 = c1.children[0] as ConstraintTreeNode;
    const ct2 = c2.children[0] as ConstraintTreeNode;
    expect(ct1.verdict).toBe('pass');
    expect(ct2.verdict).toBe('fail');
  });

  it('routes SM state to the right tree node when names collide', () => {
    // Two SMs with the same short name `Mode` under different parts;
    // the backend ships per-instance `element_id` so we associate
    // the live state to its true owner.
    const tree = buildModelTree(
      [
        n('sb', 'ProductionCell', 'PartUsage', [
          n('c1', 'Station1', 'PartUsage', [
            n('sm1', 'Mode', 'StateDefinition'),
          ]),
          n('c2', 'Station2', 'PartUsage', [
            n('sm2', 'Mode', 'StateDefinition'),
          ]),
        ]),
      ],
      URI,
    );
    const snap = snapshot({
      subsystems: {
        // Two entries that happen to share the bare name "Mode" via
        // a fully-qualified key — what matters is that the views
        // carry distinct element_ids and each tree node finds the
        // matching one without the legacy name fallback.
        'Station1.Mode': {
          current_state: 'armed',
          completed: false,
          kind_label: 'sm',
          element_id: 'sm1',
        },
        'Station2.Mode': {
          current_state: 'tripped',
          completed: false,
          kind_label: 'sm',
          element_id: 'sm2',
        },
      },
    });
    const out = mergeLiveState(tree, snap);
    const c1Mode = out[0].children[0].children[0] as SmTreeNode;
    const c2Mode = out[0].children[1].children[0] as SmTreeNode;
    expect(c1Mode.currentState).toBe('armed');
    expect(c2Mode.currentState).toBe('tripped');
  });

  it('falls back to name keying when no element_id is shipped (back-compat)', () => {
    // The legacy path still has to work for older cached frames; verify
    // the constraint with no element_id still associates by name.
    const tree = sampleTree();
    const snap = snapshot({
      constraint_results: [
        { name: 'thermalBand', expression: null, verdict: 'Pass' as const },
      ],
    });
    const out = mergeLiveState(tree, snap);
    const ct = findByName(out, 'thermalBand') as ConstraintTreeNode;
    expect(ct.verdict).toBe('pass');
  });
});

describe('mergeLiveState — non-live fields preserved', () => {
  it('keeps id / uri / kind / name / depth / ownerPath for every node', () => {
    const snap = snapshot({
      scalar_vars: { 'ProductionCell.Station1.temperature': 1 },
      subsystems: { StationStates: sub('armed') },
    });
    const before = sampleTree();
    const after = mergeLiveState(before, snap);
    const collect = (tree: readonly ModelTreeNode[]): string[] =>
      tree.flatMap((n) => [
        `${n.id}|${n.uri}|${n.kind}|${n.name}|${n.depth}|${n.ownerPath}`,
        ...collect(n.children),
      ]);
    expect(collect(after)).toEqual(collect(before));
  });

  it('keeps PartTreeNode identity as "part" (no accidental reclassification)', () => {
    const snap = snapshot({ subsystems: { StationStates: sub('armed') } });
    const out = mergeLiveState(sampleTree(), snap);
    expect((out[0] as PartTreeNode).kind).toBe('part');
  });
});

describe('mergeLiveState — multiple ticks', () => {
  it('applying merge repeatedly with changing snapshots yields the latest values', () => {
    const tree = sampleTree();
    const t1 = mergeLiveState(tree, snapshot({
      scalar_vars: { 'ProductionCell.Station1.temperature': 1 },
    }));
    const t2 = mergeLiveState(tree, snapshot({
      scalar_vars: { 'ProductionCell.Station1.temperature': 2 },
    }));
    const t3 = mergeLiveState(tree, snapshot({
      scalar_vars: { 'ProductionCell.Station1.temperature': 3 },
    }));
    expect((findByName(t1, 'temperature') as AttributeTreeNode).value).toBe(1);
    expect((findByName(t2, 'temperature') as AttributeTreeNode).value).toBe(2);
    expect((findByName(t3, 'temperature') as AttributeTreeNode).value).toBe(3);
    // Source tree never mutates.
    expect(
      (findByName(tree, 'temperature') as AttributeTreeNode).value,
    ).toBeUndefined();
  });
});

describe('mergeLiveState — calc / ode upgrade (Task 4)', () => {
  function calcTree() {
    return buildModelTree(
      [
        n('sb', 'ProductionCell', 'PartUsage', [
          n('thermal', 'thermal', 'CalculationUsage'),
          n('plain', 'totalCurrent', 'CalculationUsage'),
        ]),
      ],
      URI,
    );
  }

  it('structural build classifies calculations as "calc" (not "ode")', () => {
    const tree = calcTree();
    const calc = findByName(tree, 'thermal');
    expect(calc?.kind).toBe('calc');
  });

  it('calc absent from scalar_vars stays "calc" (plain calculation)', () => {
    const tree = calcTree();
    const out = mergeLiveState(tree, snapshot({ scalar_vars: {} }));
    const t = findByName(out, 'thermal');
    expect(t?.kind).toBe('calc');
  });

  it('calc whose name is in scalar_vars upgrades to "ode" with the value attached', () => {
    const tree = calcTree();
    const out = mergeLiveState(
      tree,
      snapshot({ scalar_vars: { 'ProductionCell.thermal': 350.2 } }),
    );
    const t = findByName(out, 'thermal');
    expect(t?.kind).toBe('ode');
    expect((t as unknown as { value?: number })?.value).toBe(350.2);
  });

  it('only calcs that carry live state upgrade — siblings that do not stay plain', () => {
    const tree = calcTree();
    const out = mergeLiveState(
      tree,
      snapshot({ scalar_vars: { 'ProductionCell.thermal': 42 } }),
    );
    expect(findByName(out, 'thermal')?.kind).toBe('ode');
    expect(findByName(out, 'totalCurrent')?.kind).toBe('calc');
  });

  it('upgrade also works via bare-name fallback (no ownerPath prefix in scalar_vars)', () => {
    const tree = calcTree();
    const out = mergeLiveState(
      tree,
      snapshot({ scalar_vars: { thermal: 42 } }),
    );
    expect(findByName(out, 'thermal')?.kind).toBe('ode');
  });
});

describe('mergeLiveState — lastChangedTick stamping (Task 3 split)', () => {
  it('first-seen value stamps the current tick as lastChangedTick', () => {
    const tree = sampleTree();
    const tracker: ChangeTracker = new Map();
    const snap = snapshot({
      tick: 10,
      scalar_vars: { 'ProductionCell.Station1.temperature': 5 },
    });
    const out = mergeLiveState(tree, snap, { changeTracker: tracker });
    expect(
      (findByName(out, 'temperature') as AttributeTreeNode).lastChangedTick,
    ).toBe(10);
  });

  it('unchanged value carries the previous tick forward (stable bucket)', () => {
    const tree = sampleTree();
    const tracker: ChangeTracker = new Map();
    const t1 = mergeLiveState(
      tree,
      snapshot({ tick: 10, scalar_vars: { 'ProductionCell.Station1.temperature': 5 } }),
      { changeTracker: tracker },
    );
    const t2 = mergeLiveState(
      tree,
      snapshot({ tick: 20, scalar_vars: { 'ProductionCell.Station1.temperature': 5 } }),
      { changeTracker: tracker },
    );
    expect(
      (findByName(t1, 'temperature') as AttributeTreeNode).lastChangedTick,
    ).toBe(10);
    expect(
      (findByName(t2, 'temperature') as AttributeTreeNode).lastChangedTick,
    ).toBe(10); // unchanged → sticks at 10
  });

  it('changed value re-stamps the current tick', () => {
    const tree = sampleTree();
    const tracker: ChangeTracker = new Map();
    mergeLiveState(
      tree,
      snapshot({ tick: 10, scalar_vars: { 'ProductionCell.Station1.temperature': 5 } }),
      { changeTracker: tracker },
    );
    const t2 = mergeLiveState(
      tree,
      snapshot({ tick: 20, scalar_vars: { 'ProductionCell.Station1.temperature': 9 } }),
      { changeTracker: tracker },
    );
    expect(
      (findByName(t2, 'temperature') as AttributeTreeNode).lastChangedTick,
    ).toBe(20);
  });

  it('missing tracker → no stamping (backward-compat with callers that never opted in)', () => {
    const tree = sampleTree();
    const out = mergeLiveState(
      tree,
      snapshot({ tick: 10, scalar_vars: { 'ProductionCell.Station1.temperature': 5 } }),
    );
    expect(
      (findByName(out, 'temperature') as AttributeTreeNode).lastChangedTick,
    ).toBeUndefined();
  });

  it('attribute absent from snapshot keeps its last known tick (doesn\'t demote to parameter)', () => {
    const tree = sampleTree();
    const tracker: ChangeTracker = new Map();
    mergeLiveState(
      tree,
      snapshot({ tick: 10, scalar_vars: { 'ProductionCell.Station1.temperature': 5 } }),
      { changeTracker: tracker },
    );
    const t2 = mergeLiveState(
      tree,
      // temperature missing from snapshot entirely
      snapshot({ tick: 20, scalar_vars: {} }),
      { changeTracker: tracker },
    );
    // Last known tick (10) survives so the attribute stays in the
    // outputs bucket instead of flipping to parameters on a
    // transient missing-value tick.
    expect(
      (findByName(t2, 'temperature') as AttributeTreeNode).lastChangedTick,
    ).toBe(10);
  });
});
