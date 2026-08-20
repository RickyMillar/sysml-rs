/**
 * ModelTreeView — recursion + state plumbing tests.
 *
 * The pure container: given a tree + an expandedSet + focusedId,
 * render the right rows in the right order and forward the right
 * callbacks. Row internals are covered by ModelTreeNodeRow's own
 * suite; this suite pins the container.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { ModelTreeView } from '../ModelTreeView';
import { buildModelTree } from '../buildModelTree';
import type { TreeNode } from '@/types/element';
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

afterEach(() => {
  cleanup();
});

function sampleTree(): ModelTreeNode[] {
  // R3.4: backend is authoritative for sibling order. Fixture is in
  // the order the backend would emit (Sm → Attribute under c1).
  return buildModelTree(
    [
      n('sb', 'ProductionCell', 'PartUsage', [
        n('c1', 'Station1', 'PartUsage', [
          n('sm', 'StationStates', 'StateDefinition'),
          n('t', 'bimetalTemp', 'AttributeUsage'),
        ]),
        n('c2', 'Station2', 'PartUsage'),
      ]),
    ],
    'file:///w.sysml',
  );
}

describe('ModelTreeView — empty state', () => {
  it('renders an empty placeholder when tree is empty', () => {
    render(
      <ModelTreeView
        tree={[]}
        expandedSet={new Set()}
        onToggleExpand={vi.fn()}
      />,
    );
    expect(screen.getByTestId('model-tree')).toHaveAttribute(
      'data-empty',
      'true',
    );
    expect(screen.getByTestId('model-tree-empty')).toBeInTheDocument();
  });

  it('does not render the empty placeholder when tree has content', () => {
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set()}
        onToggleExpand={vi.fn()}
      />,
    );
    expect(screen.getByTestId('model-tree')).not.toHaveAttribute('data-empty');
    expect(screen.queryByTestId('model-tree-empty')).toBeNull();
  });
});

describe('ModelTreeView — expand / collapse', () => {
  it('only root rows render when nothing is expanded', () => {
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set()}
        onToggleExpand={vi.fn()}
      />,
    );
    expect(screen.getByTestId('model-tree-node-sb')).toBeInTheDocument();
    expect(screen.queryByTestId('model-tree-node-c1')).toBeNull();
    expect(screen.queryByTestId('model-tree-node-c2')).toBeNull();
  });

  it('expanding ProductionCell reveals its two direct children', () => {
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb'])}
        onToggleExpand={vi.fn()}
      />,
    );
    expect(screen.getByTestId('model-tree-node-c1')).toBeInTheDocument();
    expect(screen.getByTestId('model-tree-node-c2')).toBeInTheDocument();
    // But the Station1 grandchildren are still collapsed.
    expect(screen.queryByTestId('model-tree-node-t')).toBeNull();
    expect(screen.queryByTestId('model-tree-node-sm')).toBeNull();
  });

  it('expanding ProductionCell + Station1 reveals the leaf attribute and sm', () => {
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb', 'c1'])}
        onToggleExpand={vi.fn()}
      />,
    );
    expect(screen.getByTestId('model-tree-node-t')).toBeInTheDocument();
    expect(screen.getByTestId('model-tree-node-sm')).toBeInTheDocument();
  });

  it('onToggleExpand fires with the node id when a chevron is clicked', () => {
    const onToggle = vi.fn();
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set()}
        onToggleExpand={onToggle}
      />,
    );
    fireEvent.click(screen.getByTestId('model-tree-node-sb-chevron'));
    expect(onToggle).toHaveBeenCalledWith('sb');
  });

  it('renders rows in DFS order with children immediately after parent', () => {
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb', 'c1'])}
        onToggleExpand={vi.fn()}
      />,
    );
    const tree = screen.getByTestId('model-tree');
    // Attribute rows use role="row" (delegated to AttributeRow) while
    // part / sm / constraint / ode / other rows use role="treeitem".
    // Gather by testid shape so all archetypes are counted in DFS order.
    const rowPattern = /^model-tree-node-[^-]+$/;
    const ids = Array.from(
      within(tree).queryAllByTestId(rowPattern),
    ).map((el) => el.getAttribute('data-testid') ?? '');
    // Children within each parent are now archetype-sorted
    // (part → sm → constraint → ode → calc → attribute). c1's
    // state machine `sm` outranks the attribute `t`, so `sm`
    // renders first under c1.
    expect(ids).toEqual([
      'model-tree-node-sb',
      'model-tree-node-c1',
      'model-tree-node-sm',
      'model-tree-node-t',
      'model-tree-node-c2',
    ]);
  });
});

describe('ModelTreeView — selection', () => {
  it('focusedId marks that row as selected', () => {
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb'])}
        onToggleExpand={vi.fn()}
        focusedId="c1"
      />,
    );
    expect(screen.getByTestId('model-tree-node-c1')).toHaveAttribute(
      'data-selected',
      'true',
    );
    expect(screen.getByTestId('model-tree-node-c2')).not.toHaveAttribute(
      'data-selected',
    );
  });

  it('onSelectNode fires with the clicked node', () => {
    const onSelect = vi.fn();
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb'])}
        onToggleExpand={vi.fn()}
        onSelectNode={onSelect}
      />,
    );
    fireEvent.click(screen.getByTestId('model-tree-node-c1'));
    expect(onSelect).toHaveBeenCalledOnce();
    expect(onSelect.mock.calls[0][0].id).toBe('c1');
    expect(onSelect.mock.calls[0][0].name).toBe('Station1');
  });
});

describe('ModelTreeView — pin + edit + context menu', () => {
  it('onTogglePin fires with the node when an attribute row pin is clicked', () => {
    const onTogglePin = vi.fn();
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb', 'c1'])}
        onToggleExpand={vi.fn()}
        onTogglePin={onTogglePin}
      />,
    );
    fireEvent.click(screen.getByTestId('model-tree-node-t-pin'));
    expect(onTogglePin).toHaveBeenCalledOnce();
    expect(onTogglePin.mock.calls[0][0].id).toBe('t');
  });

  it('pinnedIds marks the attribute row as pinned', () => {
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb', 'c1'])}
        onToggleExpand={vi.fn()}
        pinnedIds={new Set(['t'])}
        onTogglePin={vi.fn()}
      />,
    );
    expect(screen.getByTestId('model-tree-node-t')).toHaveAttribute(
      'data-pinned',
      'true',
    );
  });

  it('editable + onEditAttribute together surface the edit pencil', () => {
    const onEdit = vi.fn();
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb', 'c1'])}
        onToggleExpand={vi.fn()}
        editable
        onEditAttribute={onEdit}
      />,
    );
    fireEvent.click(screen.getByTestId('model-tree-node-t-edit'));
    expect(onEdit).toHaveBeenCalledOnce();
    expect(onEdit.mock.calls[0][0].id).toBe('t');
  });

  it('onContextMenu fires with the node + position', () => {
    const onContextMenu = vi.fn();
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb'])}
        onToggleExpand={vi.fn()}
        onContextMenu={onContextMenu}
      />,
    );
    fireEvent.contextMenu(screen.getByTestId('model-tree-node-c1'), {
      clientX: 50,
      clientY: 60,
    });
    expect(onContextMenu).toHaveBeenCalledOnce();
    expect(onContextMenu.mock.calls[0][0].id).toBe('c1');
    expect(onContextMenu.mock.calls[0][1]).toEqual({ x: 50, y: 60 });
  });
});

describe('ModelTreeView — sparkline lookup', () => {
  it('calls getSparklineSamples for every rendered attribute', () => {
    const getSparkline = vi.fn((node: ModelTreeNode) =>
      node.kind === 'attribute' ? [1, 2, 3] : [],
    );
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb', 'c1'])}
        onToggleExpand={vi.fn()}
        getSparklineSamples={getSparkline}
      />,
    );
    // Called for every rendered row (parts + attribute + sm) — the
    // container doesn't pre-filter by kind because the lookup is
    // already cheap at the ring-buffer source.
    expect(getSparkline).toHaveBeenCalled();
    const calledIds = getSparkline.mock.calls.map((c) => c[0].id);
    expect(calledIds).toContain('t'); // attribute
  });
});

describe('ModelTreeView — archetype section headers', () => {
  it('renders a header per non-empty archetype group when ≥2 kinds mix', () => {
    // Station1 mixes attribute + sm → both headers should render.
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb', 'c1'])}
        onToggleExpand={vi.fn()}
      />,
    );
    // Headers are testid'd `<prefix>-section-<kind>-<first-child-id>`.
    expect(
      screen.getByTestId('model-tree-section-sm-sm'),
    ).toHaveTextContent('State machines');
    expect(
      screen.getByTestId('model-tree-section-attribute-t'),
    ).toHaveTextContent('Attributes');
  });

  it('omits headers when every child shares a single archetype', () => {
    // ProductionCell's children are Station1 + Station2, both parts —
    // so no "Parts" label is rendered (the homogeneity is obvious).
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb'])}
        onToggleExpand={vi.fn()}
      />,
    );
    expect(screen.queryByTestId('model-tree-section-part-c1')).toBeNull();
  });

  // Two-root fixture with a mixed root level AND a mixed nested group:
  // a Model part (owning an inner SM + attribute) alongside a top-level
  // state machine. Exercises the sectionHeaderScope contrast.
  function mixedDepthsTree(): ModelTreeNode[] {
    return buildModelTree(
      [
        n('p1', 'Model', 'PartUsage', [
          n('nsm', 'InnerSM', 'StateDefinition'),
          n('na', 'innerAttr', 'AttributeUsage'),
        ]),
        n('rsm', 'TopSM', 'StateDefinition'),
      ],
      'file:///m.sysml',
    );
  }

  it("scope 'all' (default) labels nested groups at every depth", () => {
    render(
      <ModelTreeView
        tree={mixedDepthsTree()}
        expandedSet={new Set(['p1'])}
        onToggleExpand={vi.fn()}
      />,
    );
    // Root headers present …
    expect(screen.getByTestId('model-tree-section-part-p1')).toHaveTextContent(
      'Parts',
    );
    expect(screen.getByTestId('model-tree-section-sm-rsm')).toHaveTextContent(
      'State machines',
    );
    // … and so are the nested ones under the Model part.
    expect(screen.getByTestId('model-tree-section-sm-nsm')).toHaveTextContent(
      'State machines',
    );
    expect(
      screen.getByTestId('model-tree-section-attribute-na'),
    ).toHaveTextContent('Attributes');
  });

  it("scope 'root' emits headers only for the top-level group", () => {
    render(
      <ModelTreeView
        tree={mixedDepthsTree()}
        expandedSet={new Set(['p1'])}
        onToggleExpand={vi.fn()}
        sectionHeaderScope="root"
      />,
    );
    // Root-level headers still render exactly once per archetype …
    expect(screen.getByTestId('model-tree-section-part-p1')).toHaveTextContent(
      'Parts',
    );
    expect(screen.getByTestId('model-tree-section-sm-rsm')).toHaveTextContent(
      'State machines',
    );
    // … but the nested mixed group under the Model part has NONE:
    // containment + the per-row type icon carry that instead. This is
    // what kills the duplicate "State machines" / "Calculations" labels
    // in the Run tree.
    expect(screen.queryByTestId('model-tree-section-sm-nsm')).toBeNull();
    expect(screen.queryByTestId('model-tree-section-attribute-na')).toBeNull();
    // The nested rows themselves are still there (unlabelled).
    expect(screen.getByTestId('model-tree-node-nsm')).toBeInTheDocument();
    expect(screen.getByTestId('model-tree-node-na')).toBeInTheDocument();
  });
});

describe('ModelTreeView — big-model virtualization (UX closeout #4 / #17)', () => {
  function bigTree(count: number): ModelTreeNode[] {
    const leaves = Array.from({ length: count }, (_, i) =>
      n(`leaf${i}`, `v${i}`, 'AttributeUsage'),
    );
    return buildModelTree(
      [n('root', 'Big', 'PartUsage', leaves)],
      'file:///big.sysml',
    );
  }

  it('does NOT virtualize a small tree — every existing fixture / hybrid-scale tree renders exactly as before', () => {
    render(
      <ModelTreeView
        tree={sampleTree()}
        expandedSet={new Set(['sb', 'c1'])}
        onToggleExpand={vi.fn()}
      />,
    );
    expect(screen.getByTestId('model-tree')).not.toHaveAttribute('data-virtualized');
    expect(screen.queryByTestId('model-tree-virtual-track')).toBeNull();
    // Every row present — no windowing below the threshold.
    expect(screen.getByTestId('model-tree-node-t')).toBeInTheDocument();
    expect(screen.getByTestId('model-tree-node-sm')).toBeInTheDocument();
  });

  it('virtualizes a tree above ROW_VIRTUALIZATION_THRESHOLD: mounts only a window of rows, not all of them', () => {
    const count = 350;
    render(
      <ModelTreeView
        tree={bigTree(count)}
        expandedSet={new Set(['root'])}
        onToggleExpand={vi.fn()}
      />,
    );
    expect(screen.getByTestId('model-tree')).toHaveAttribute('data-virtualized', 'true');
    expect(screen.getByTestId('model-tree-virtual-track')).toBeInTheDocument();

    // The root row (index 0) is always in the initial window.
    expect(screen.getByTestId('model-tree-node-root')).toBeInTheDocument();

    // Far fewer than `count` leaf rows are actually mounted...
    const mountedLeaves = screen.queryAllByTestId(/^model-tree-node-leaf\d+$/);
    expect(mountedLeaves.length).toBeGreaterThan(0);
    expect(mountedLeaves.length).toBeLessThan(count);

    // ...and a leaf far past the initial viewport isn't mounted at all.
    expect(screen.queryByTestId('model-tree-node-leaf349')).toBeNull();
  });

  it('a row inside the mounted window still wires its callbacks correctly while virtualized', () => {
    const onToggle = vi.fn();
    const onSelect = vi.fn();
    render(
      <ModelTreeView
        tree={bigTree(350)}
        expandedSet={new Set(['root'])}
        onToggleExpand={onToggle}
        onSelectNode={onSelect}
      />,
    );
    fireEvent.click(screen.getByTestId('model-tree-node-leaf0'));
    expect(onSelect).toHaveBeenCalledOnce();
    expect(onSelect.mock.calls[0][0].id).toBe('leaf0');

    fireEvent.click(screen.getByTestId('model-tree-node-root-chevron'));
    expect(onToggle).toHaveBeenCalledWith('root');
  });

  it('the track height reflects the full (unmounted-included) row count, so the scrollbar stays honest', () => {
    const count = 350;
    render(
      <ModelTreeView
        tree={bigTree(count)}
        expandedSet={new Set(['root'])}
        onToggleExpand={vi.fn()}
      />,
    );
    const track = screen.getByTestId('model-tree-virtual-track');
    // count + 1 rows (root + N leaves) at the `--row-dense` (16px)
    // estimate — ninebar Phase 1 density tier; ModelTreeView's
    // ESTIMATED_ROW_HEIGHT_PX now mirrors the token instead of the old
    // ad hoc 24px guess.
    const expectedMinHeight = count * 16;
    expect(parseFloat(track.style.height)).toBeGreaterThanOrEqual(expectedMinHeight);
  });
});
