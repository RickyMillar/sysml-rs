/**
 * ModelTreeNodeRow — one-row render contract.
 *
 * Unit tests for each archetype's presentation. Container concerns
 * (expand state, selection, recursion) are parametrised in; this
 * suite asserts that once the inputs are set, the row paints the
 * right chrome for each kind.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { ModelTreeNodeRow } from '../ModelTreeNodeRow';
import type {
  AttributeTreeNode,
  ConstraintTreeNode,
  ModelTreeNode,
  OdeTreeNode,
  PartTreeNode,
  SmTreeNode,
  OtherTreeNode,
  ActionTreeNode,
} from '../types';

afterEach(() => {
  cleanup();
});

function baseNode(
  overrides: Partial<ModelTreeNode> & Pick<ModelTreeNode, 'kind' | 'id'>,
) {
  return {
    uri: 'file:///w.sysml',
    name: 'TestNode',
    rawKind: 'PartUsage',
    depth: 0,
    ownerPath: '',
    children: [],
    ...overrides,
  } as ModelTreeNode;
}

describe('ModelTreeNodeRow — part', () => {
  it('renders name, kind icon, and data-kind="part"', () => {
    const node = baseNode({ kind: 'part', id: 'p1', name: 'ProductionCell' }) as PartTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    const row = screen.getByTestId('model-tree-node-p1');
    expect(row).toHaveAttribute('data-kind', 'part');
    expect(row).toHaveTextContent('ProductionCell');
  });

  it('renders a one-liner when set', () => {
    const node = baseNode({
      kind: 'part',
      id: 'p1',
      name: 'GroupHead',
    }) as PartTreeNode;
    node.oneLiner = 'state: armed';
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(screen.getByTestId('model-tree-node-p1-oneliner')).toHaveTextContent(
      'state: armed',
    );
  });

  it('omits the one-liner testid when oneLiner is absent', () => {
    const node = baseNode({ kind: 'part', id: 'p1' }) as PartTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(screen.queryByTestId('model-tree-node-p1-oneliner')).toBeNull();
  });
});

describe('ModelTreeNodeRow — expand chevron', () => {
  it('leaves show a spacer (no chevron button)', () => {
    const node = baseNode({ kind: 'part', id: 'leaf' }) as PartTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(screen.queryByTestId('model-tree-node-leaf-chevron')).toBeNull();
  });

  it('nodes with children render a chevron that fires onToggleExpand', () => {
    const child = baseNode({ kind: 'attribute', id: 'a' }) as AttributeTreeNode;
    const node = baseNode({ kind: 'part', id: 'p', children: [child] }) as PartTreeNode;
    const onToggle = vi.fn();
    render(
      <ModelTreeNodeRow
        node={node}
        expanded={false}
        onToggleExpand={onToggle}
      />,
    );
    fireEvent.click(screen.getByTestId('model-tree-node-p-chevron'));
    expect(onToggle).toHaveBeenCalledOnce();
  });

  it('chevron click does not bubble to row onSelect', () => {
    const child = baseNode({ kind: 'attribute', id: 'a' }) as AttributeTreeNode;
    const node = baseNode({ kind: 'part', id: 'p', children: [child] }) as PartTreeNode;
    const onToggle = vi.fn();
    const onSelect = vi.fn();
    render(
      <ModelTreeNodeRow
        node={node}
        expanded={false}
        onToggleExpand={onToggle}
        onSelect={onSelect}
      />,
    );
    fireEvent.click(screen.getByTestId('model-tree-node-p-chevron'));
    expect(onToggle).toHaveBeenCalledOnce();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('expanded=true sets aria-expanded="true" on the row', () => {
    const child = baseNode({ kind: 'attribute', id: 'a' }) as AttributeTreeNode;
    const node = baseNode({ kind: 'part', id: 'p', children: [child] }) as PartTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={true} />);
    expect(screen.getByTestId('model-tree-node-p')).toHaveAttribute(
      'aria-expanded',
      'true',
    );
  });
});

describe('ModelTreeNodeRow — sm', () => {
  it('shows current state in a badge pill', () => {
    const node = baseNode({
      kind: 'sm',
      id: 'sm1',
      name: 'StationStates',
      rawKind: 'StateDefinition',
    }) as SmTreeNode;
    node.currentState = 'armed';
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(screen.getByTestId('model-tree-node-sm1-state')).toHaveTextContent(
      'armed',
    );
  });

  it('shows em-dash when no state has been observed yet', () => {
    const node = baseNode({ kind: 'sm', id: 'sm1' }) as SmTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(screen.getByTestId('model-tree-node-sm1-state')).toHaveTextContent(
      '—',
    );
  });
});

describe('ModelTreeNodeRow — constraint', () => {
  it('renders a verdict dot with data-verdict="pass" on pass', () => {
    const node = baseNode({
      kind: 'constraint',
      id: 'c1',
    }) as ConstraintTreeNode;
    node.verdict = 'pass';
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(
      screen.getByTestId('model-tree-node-c1-verdict'),
    ).toHaveAttribute('data-verdict', 'pass');
  });

  it('renders "none" when no verdict has arrived yet', () => {
    const node = baseNode({ kind: 'constraint', id: 'c1' }) as ConstraintTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(
      screen.getByTestId('model-tree-node-c1-verdict'),
    ).toHaveAttribute('data-verdict', 'none');
  });

  it('distinguishes fail from pass via data-verdict (not just colour)', () => {
    const node = baseNode({ kind: 'constraint', id: 'c1' }) as ConstraintTreeNode;
    node.verdict = 'fail';
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(
      screen.getByTestId('model-tree-node-c1-verdict'),
    ).toHaveAttribute('data-verdict', 'fail');
  });
});

describe('ModelTreeNodeRow — ode', () => {
  it('shows stability status label', () => {
    const node = baseNode({ kind: 'ode', id: 'o1' }) as OdeTreeNode;
    node.status = 'stable';
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(screen.getByTestId('model-tree-node-o1-status')).toHaveTextContent(
      'stable',
    );
  });

  it('falls back to "unknown" when status is absent', () => {
    const node = baseNode({ kind: 'ode', id: 'o1' }) as OdeTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(screen.getByTestId('model-tree-node-o1-status')).toHaveTextContent(
      'unknown',
    );
  });
});

describe('ModelTreeNodeRow — attribute special case', () => {
  it('delegates to AttributeRow (value + unit render via its contract)', () => {
    const node = baseNode({
      kind: 'attribute',
      id: 'a1',
      name: 'bimetalTemp',
      rawKind: 'AttributeUsage',
    }) as AttributeTreeNode;
    node.value = 298.15;
    node.unit = 'K';
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    // AttributeRow namespaces with our prefix, so the row appears under it.
    const row = screen.getByTestId('model-tree-node-a1');
    expect(row).toHaveTextContent('bimetalTemp');
    expect(row).toHaveTextContent('K');
    expect(row).toHaveTextContent('298.15');
  });

  it('fires onTogglePin / onEdit through to the AttributeRow affordances', () => {
    const node = baseNode({
      kind: 'attribute',
      id: 'a1',
    }) as AttributeTreeNode;
    const onTogglePin = vi.fn();
    const onEdit = vi.fn();
    render(
      <ModelTreeNodeRow
        node={node}
        expanded={false}
        onTogglePin={onTogglePin}
        editable
        onEdit={onEdit}
      />,
    );
    fireEvent.click(screen.getByTestId('model-tree-node-a1-pin'));
    fireEvent.click(screen.getByTestId('model-tree-node-a1-edit'));
    expect(onTogglePin).toHaveBeenCalledOnce();
    expect(onEdit).toHaveBeenCalledOnce();
  });
});

describe('ModelTreeNodeRow — runnable actions', () => {
  it('renders an Analyze button for AnalysisCase rows without selecting the row', () => {
    const node = baseNode({
      kind: 'action',
      id: 'a-case',
      name: 'TradeStudy',
      rawKind: 'AnalysisCaseUsage',
    }) as ActionTreeNode;
    const onLaunch = vi.fn();
    const onSelect = vi.fn();
    render(
      <ModelTreeNodeRow
        node={node}
        expanded={false}
        onLaunchRunnable={onLaunch}
        onSelect={onSelect}
      />,
    );
    const launch = screen.getByTestId('model-tree-node-a-case-launch');
    expect(launch).toHaveTextContent('Analyze');
    fireEvent.click(launch);
    expect(onLaunch).toHaveBeenCalledWith(node);
    expect(onSelect).not.toHaveBeenCalled();
  });

  it('renders a Verify button for VerificationCase rows', () => {
    const node = baseNode({
      kind: 'action',
      id: 'v-case',
      name: 'RequirementCheck',
      rawKind: 'VerificationCaseDefinition',
    }) as ActionTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={false} onLaunchRunnable={vi.fn()} />);
    expect(screen.getByTestId('model-tree-node-v-case-launch')).toHaveTextContent('Verify');
  });
});

describe('ModelTreeNodeRow — other', () => {
  it('renders the raw kind as a muted right-side suffix', () => {
    const node = baseNode({
      kind: 'other',
      id: 'x',
      name: 'phaseIn',
      rawKind: 'PortUsage',
    }) as OtherTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(
      screen.getByTestId('model-tree-node-x-rawkind'),
    ).toHaveTextContent('PortUsage');
  });
});

describe('ModelTreeNodeRow — selection + context menu', () => {
  it('onSelect fires on row click (non-attribute)', () => {
    const node = baseNode({ kind: 'part', id: 'p' }) as PartTreeNode;
    const onSelect = vi.fn();
    render(
      <ModelTreeNodeRow node={node} expanded={false} onSelect={onSelect} />,
    );
    fireEvent.click(screen.getByTestId('model-tree-node-p'));
    expect(onSelect).toHaveBeenCalledOnce();
  });

  it('selected=true reflects as data-selected="true"', () => {
    const node = baseNode({ kind: 'part', id: 'p' }) as PartTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={false} selected />);
    expect(screen.getByTestId('model-tree-node-p')).toHaveAttribute(
      'data-selected',
      'true',
    );
  });

  it('onContextMenu fires with mouse coordinates', () => {
    const node = baseNode({ kind: 'part', id: 'p' }) as PartTreeNode;
    const onContextMenu = vi.fn();
    render(
      <ModelTreeNodeRow
        node={node}
        expanded={false}
        onContextMenu={onContextMenu}
      />,
    );
    fireEvent.contextMenu(screen.getByTestId('model-tree-node-p'), {
      clientX: 100,
      clientY: 200,
    });
    expect(onContextMenu).toHaveBeenCalledWith({ x: 100, y: 200 });
  });
});

describe('ModelTreeNodeRow — depth indent', () => {
  it('stamps data-depth with the node depth', () => {
    const node = baseNode({ kind: 'part', id: 'p', depth: 3 }) as PartTreeNode;
    render(<ModelTreeNodeRow node={node} expanded={false} />);
    expect(screen.getByTestId('model-tree-node-p')).toHaveAttribute(
      'data-depth',
      '3',
    );
  });
});
