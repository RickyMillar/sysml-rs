/**
 * createViewFlow — v2 projection-first pure logic (create-view-v2.md).
 */
import { describe, it, expect } from 'vitest';
import type { ModelTreeNode } from '@/features/sessions/tree/types';
import {
  VIEW_TYPES,
  buildScopeRows,
  defaultViewName,
  isValidViewName,
  kindAvailability,
  rewriteScratchSnippet,
} from '../createViewFlow';

const node = (
  kind: ModelTreeNode['kind'],
  name: string,
  children: ModelTreeNode[] = [],
  rawKind?: string,
): ModelTreeNode =>
  ({
    id: name,
    elementId: `id-${name}`,
    uri: 'file:///m.sysml',
    name,
    rawKind: rawKind ?? kind,
    kind,
    depth: 0,
    ownerPath: '',
    children,
  }) as ModelTreeNode;

const TREE = [
  node('part', 'motor', [
    node('port', 'shaft'),
    node('sm', 'MotorStates', [
      node('other', 'armed', [], 'StateUsage'),
      node('other', 'tripped', [], 'StateUsage'),
    ]),
    node('attribute', 'torque'),
    node('action', 'spinUp'),
  ]),
  node('part', 'housing'),
];

describe('VIEW_TYPES', () => {
  it("offers the renderer's real 8 kinds — no UseCase/Requirement (they resolve to General)", () => {
    const tokens = VIEW_TYPES.map((v) => v.token);
    expect(tokens).toEqual([
      'General', 'Interconnection', 'StateTransition', 'ActionFlow',
      'Sequence', 'Browser', 'Grid', 'Geometry',
    ]);
  });
});

describe('kindAvailability', () => {
  it('counts eligible targets per kind across the whole tree', () => {
    const counts = kindAvailability(TREE);
    expect(counts.StateTransition).toBe(1); // MotorStates
    expect(counts.Interconnection).toBe(2); // both parts
    expect(counts.ActionFlow).toBe(1); // spinUp
    expect(counts.Grid).toBe(1); // motor (has an attribute child)
    expect(counts.General).toBeGreaterThanOrEqual(3); // parts + sm + action
  });
});

describe('buildScopeRows', () => {
  it('StateTransition: only machines are selectable, shown under their ancestors, states inline', () => {
    const rows = buildScopeRows(TREE, 'StateTransition');
    // housing has no eligible descendants — pruned entirely.
    expect(rows.map((r) => r.node.name)).toEqual(['motor', 'MotorStates']);
    const motor = rows[0];
    const sm = rows[1];
    expect(motor.eligible).toBe(false); // group header only
    expect(sm.eligible).toBe(true);
    expect(sm.depth).toBe(1);
    expect(sm.hint).toBe('armed · tripped');
  });

  it('Interconnection: parts selectable with port-count hints', () => {
    const rows = buildScopeRows(TREE, 'Interconnection');
    const motor = rows.find((r) => r.node.name === 'motor')!;
    expect(motor.eligible).toBe(true);
    expect(motor.hint).toBe('1 port');
    const housing = rows.find((r) => r.node.name === 'housing')!;
    expect(housing.eligible).toBe(true);
    expect(housing.hint).toBeUndefined();
  });

  it('Grid: only attribute-bearing nodes are eligible', () => {
    const rows = buildScopeRows(TREE, 'Grid');
    expect(rows.filter((r) => r.eligible).map((r) => r.node.name)).toEqual(['motor']);
  });
});

describe('defaultViewName / isValidViewName', () => {
  it('builds a legal PascalCase identifier from dotted/snake paths', () => {
    expect(defaultViewName('motor.trip_unit')).toBe('MotorTripUnitView');
    expect(defaultViewName('breaker')).toBe('BreakerView');
    expect(defaultViewName('42fish')).toBe('FishView');
    expect(isValidViewName('9bad')).toBe(false);
    expect(isValidViewName('Good_1')).toBe(true);
  });
});

describe('rewriteScratchSnippet', () => {
  it('rewrites name + supertype, preserving expose refs; null on unknown shape', () => {
    expect(
      rewriteScratchSnippet('view scratch : InterconnectionView {\n    expose P::motor;\n}\n', 'MotorView', 'StateTransitionView'),
    ).toBe('view MotorView : StateTransitionView {\n    expose P::motor;\n}\n');
    expect(
      rewriteScratchSnippet('view def scratch :> GeneralView {\n    expose A;\n}\n', 'AView', 'SequenceView'),
    ).toBe('view def AView :> SequenceView {\n    expose A;\n}\n');
    expect(rewriteScratchSnippet('something else', 'X', 'General')).toBeNull();
  });
});
