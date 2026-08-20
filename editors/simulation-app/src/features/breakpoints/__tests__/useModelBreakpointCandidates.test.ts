/**
 * useModelBreakpointCandidates (BP-UX1) — model-sourced candidates must
 * exist with NO session, populate from the tree's kinds correctly, and
 * carry rich detail (what · where) so the combobox can show — and
 * fuzzy-match — the owning state machine of every state.
 */
import { describe, it, expect, vi, beforeEach } from 'vitest';
import { renderHook } from '@testing-library/react';
import { useModelBreakpointCandidates } from '../useModelBreakpointCandidates';
import type { ModelTreeNode } from '@/features/sessions/tree/types';

const node = (
  kind: ModelTreeNode['kind'],
  name: string,
  ownerPath: string,
  children: ModelTreeNode[] = [],
  rawKind?: string,
): ModelTreeNode =>
  ({
    id: `${ownerPath}.${name}`,
    elementId: `${ownerPath}.${name}`,
    uri: 'file:///m.sysml',
    name,
    rawKind: rawKind ?? kind,
    kind,
    depth: ownerPath ? ownerPath.split('.').length : 0,
    ownerPath,
    children,
  }) as ModelTreeNode;

let mockTree: ModelTreeNode[] = [];

vi.mock('@/features/sessions/tree/useSessionModelTree', () => ({
  useSessionModelTree: () => ({ tree: mockTree, isLoading: false }),
}));

vi.mock('@/shared/metrics/registry', () => ({
  metricRegistry: {
    list: () => [
      { name: 'streamed.only_var', source: 'variable' },
      { name: 'not_a_var', source: 'kpi' },
    ],
  },
}));

describe('useModelBreakpointCandidates', () => {
  beforeEach(() => {
    mockTree = [
      node('part', 'motor', '', [
        node('attribute', 'torque', 'motor'),
        node('calc', 'power', 'motor'),
        node('sm', 'MotorStates', 'motor', [
          node('other', 'armed', 'motor.MotorStates', [], 'StateUsage'),
          node('other', 'tripped', 'motor.MotorStates', [], 'ExhibitStateUsage'),
        ]),
        node('constraint', 'torque_limit', 'motor'),
        node('ode', 'OmegaDerivative', 'motor'),
        node('port', 'shaftOut', 'motor'), // neither bucket
      ]),
    ];
  });

  it('buckets variables (dotted) and elements (rich items with what · where detail)', () => {
    const { result } = renderHook(() => useModelBreakpointCandidates());
    const { variableCandidates: vars, elementCandidates: els } = result.current;

    expect(vars).toContain('motor.torque');
    expect(vars).toContain('motor.power');
    expect(vars).toContain('streamed.only_var'); // registry supplement
    expect(vars).not.toContain('not_a_var');

    const values = els.map((c) => (typeof c === 'string' ? c : c.value));
    const detail = (v: string) => {
      const hit = els.find((c) => typeof c !== 'string' && c.value === v);
      return hit && typeof hit !== 'string' ? hit.detail : undefined;
    };

    // States carry their owning machine in the detail — searchable +
    // rendered in the suggestion row.
    expect(values).toContain('armed');
    expect(detail('armed')).toBe('state · MotorStates');
    expect(detail('tripped')).toBe('state · MotorStates');
    expect(detail('motor.MotorStates.armed')).toBe('state · MotorStates');

    // Machines/constraints/odes carry kind + owner path.
    expect(detail('MotorStates')).toBe('state machine · motor');
    expect(detail('torque_limit')).toBe('constraint · motor');
    expect(detail('OmegaDerivative')).toBe('ode · motor');

    // Ports are neither bucket.
    expect(values).not.toContain('shaftOut');
    expect(vars).not.toContain('motor.shaftOut');
  });

  it('works with an empty tree (no session, no model)', () => {
    mockTree = [];
    const { result } = renderHook(() => useModelBreakpointCandidates());
    expect(result.current.elementCandidates).toEqual([]);
    expect(result.current.variableCandidates).toEqual(['streamed.only_var']);
  });
});
