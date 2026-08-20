/**
 * useSessionModelTree — composition tests.
 *
 * Verifies the hook returns the merged tree for the loaded workspace,
 * overlays live session snapshots, and tracks session-switch cleanly.
 *
 * Keeps the mocking narrow: we exercise the real buildModelTree +
 * mergeLiveState paths (covered by their own suites) and only mock
 * the outer react-query / zustand surfaces.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, renderHook } from '@testing-library/react';
import {
  QueryClient,
  QueryClientProvider,
} from '@tanstack/react-query';
import type { ReactNode } from 'react';
import { useSessionModelTree } from '../useSessionModelTree';
import { useSessionLiveStore } from '../../sessionLiveStore';
import { useWorkspaceUIStore } from '@/features/workspace/store';
import { packageKeys } from '@/features/packages/queries';
import type { TreeNode } from '@/types/element';
import type { AttributeTreeNode, SmTreeNode } from '../types';

import { archetypeForKind } from './testHelpers';

function n(
  id: string,
  name: string | null,
  kind: string,
  children: TreeNode[] = [],
): TreeNode {
  return { id, name, kind, archetype: archetypeForKind(kind), children };
}

function makeWrapper(client: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

const ROOT = '/tmp/ws';
const URI_A = 'file:///a.sysml';
const URI_B = 'file:///b.sysml';

const TREE_A: TreeNode[] = [
  n('root-a', 'ProductionCell', 'PartUsage', [
    n('v', 'temperature', 'AttributeUsage'),
    n('sm', 'StationStates', 'StateDefinition'),
  ]),
];

const TREE_B: TreeNode[] = [
  n('root-b', 'Enclosure', 'PartUsage', [
    n('t', 'temp', 'AttributeUsage'),
  ]),
];

describe('useSessionModelTree', () => {
  let client: QueryClient;

  beforeEach(() => {
    // Reset global stores to a clean slate.
    useSessionLiveStore.getState().reset();
    useWorkspaceUIStore.setState({ workspaceRoot: ROOT });

    client = new QueryClient({
      defaultOptions: {
        queries: { retry: false, staleTime: Infinity, refetchOnWindowFocus: false },
      },
    });

    // Pre-populate the react-query caches the hook reads. The hook
    // consumes the ONE workspace-scoped tree (`__workspace__`) —
    // seeded here as the backend's merged-graph response would look:
    // both files' roots in one list.
    client.setQueryData(packageKeys.workspace(ROOT), { uris: [URI_A, URI_B] });
    client.setQueryData(
      ['workspace-scoped-tree', URI_A, URI_B],
      [...TREE_A, ...TREE_B],
    );
  });

  afterEach(() => {
    cleanup();
    client.clear();
    vi.restoreAllMocks();
  });

  it('merges both files at the root when no snapshot has arrived yet', () => {
    const { result } = renderHook(() => useSessionModelTree(), {
      wrapper: makeWrapper(client),
    });
    expect(result.current.tree.map((n) => n.name).sort()).toEqual([
      'Enclosure',
      'ProductionCell',
    ]);
    expect(result.current.liveSessionId).toBeNull();
    // No snapshot yet: attribute values are undefined.
    const sb = result.current.tree.find((n) => n.name === 'ProductionCell')!;
    const v = sb.children.find((n) => n.name === 'temperature') as AttributeTreeNode;
    expect(v.value).toBeUndefined();
  });

  it('overlays live scalar values once a snapshot lands', () => {
    const { result } = renderHook(() => useSessionModelTree(), {
      wrapper: makeWrapper(client),
    });

    act(() => {
      useSessionLiveStore.getState().applyHello('session-1', {
        tick: 0,
        time_ms: 0,
        completed: false,
        subsystems: { StationStates: { current_state: 'armed', completed: false, kind_label: 'sm' } },
        scalar_vars: { 'ProductionCell.temperature': 12.5 },
        string_vars: {},
        constraint_results: [],
      });
    });

    const sb = result.current.tree.find((n) => n.name === 'ProductionCell')!;
    const v = sb.children.find((n) => n.name === 'temperature') as AttributeTreeNode;
    const sm = sb.children.find((n) => n.name === 'StationStates') as SmTreeNode;
    expect(v.value).toBe(12.5);
    expect(sm.currentState).toBe('armed');
    expect(result.current.liveSessionId).toBe('session-1');
  });

  it('re-renders with updated values after applyTick (multi-tick independence)', () => {
    const { result } = renderHook(() => useSessionModelTree(), {
      wrapper: makeWrapper(client),
    });

    act(() => {
      useSessionLiveStore.getState().applyHello('session-1', {
        tick: 0,
        time_ms: 0,
        completed: false,
        subsystems: {},
        scalar_vars: { 'ProductionCell.temperature': 1 },
        string_vars: {},
        constraint_results: [],
      });
    });
    act(() => {
      useSessionLiveStore.getState().applyTick({
        tick: 1,
        time_ms: 100,
        completed: false,
        scalar_changed: { 'ProductionCell.temperature': 2 },
      });
    });
    const v = result.current.tree
      .find((n) => n.name === 'ProductionCell')!
      .children.find((n) => n.name === 'temperature') as AttributeTreeNode;
    expect(v.value).toBe(2);
  });

  it('returns an empty tree when no workspace is loaded', () => {
    useWorkspaceUIStore.setState({ workspaceRoot: null });
    const emptyClient = new QueryClient();
    const { result } = renderHook(() => useSessionModelTree(), {
      wrapper: makeWrapper(emptyClient),
    });
    expect(result.current.tree).toEqual([]);
  });

  it('groupByPackage=true (default) surfaces real Package nodes as containers', () => {
    // Workspace response whose top-level elements ARE packages (the
    // common SysML shape): each renders as a container, contents nested.
    client.setQueryData(
      ['workspace-scoped-tree', URI_A, URI_B],
      [
        n('pa', 'PkgOne', 'Package', TREE_A),
        n('pb', 'PkgTwo', 'Package', TREE_B),
      ],
    );
    const { result } = renderHook(
      () => useSessionModelTree({ groupByPackage: true }),
      { wrapper: makeWrapper(client) },
    );
    const roots = result.current.tree;
    // One container per package, sorted by name, folder-tagged.
    expect(roots.map((r) => r.name)).toEqual(['PkgOne', 'PkgTwo']);
    expect(roots.every((r) => r.rawKind === 'Package')).toBe(true);
    // The model content lives one level deeper (ownership preserved).
    const pkgOne = roots.find((r) => r.name === 'PkgOne')!;
    expect(pkgOne.children.map((c) => c.name)).toContain('ProductionCell');
  });

  it('groupByPackage=true merges same-named packages (defensive no-op vs backend unification)', () => {
    // The backend's merged workspace graph unifies a package reopened
    // across files into ONE element, so two same-named roots should
    // never arrive — but if they do, the display normaliser merges
    // rather than rendering a confusing duplicate container.
    client.setQueryData(
      ['workspace-scoped-tree', URI_A, URI_B],
      [
        n('pa', 'Shared', 'Package', [n('x', 'PartX', 'PartUsage')]),
        n('pb', 'Shared', 'Package', [n('y', 'PartY', 'PartUsage')]),
      ],
    );
    const { result } = renderHook(
      () => useSessionModelTree({ groupByPackage: true }),
      { wrapper: makeWrapper(client) },
    );
    const roots = result.current.tree;
    expect(roots).toHaveLength(1);
    expect(roots[0].name).toBe('Shared');
    expect(roots[0].children.map((c) => c.name).sort()).toEqual([
      'PartX',
      'PartY',
    ]);
  });

  it('drops LibraryPackage roots (Run shows the model, not the stdlib)', () => {
    client.setQueryData(
      ['workspace-scoped-tree', URI_A, URI_B],
      [
        n('pa', 'PkgOne', 'Package', TREE_A),
        n('lib1', 'ISQ', 'LibraryPackage', [n('li', 'metre', 'AttributeUsage')]),
        n('lib2', 'ScalarValues', 'LibraryPackage'),
      ],
    );
    const { result } = renderHook(() => useSessionModelTree(), {
      wrapper: makeWrapper(client),
    });
    expect(result.current.tree.map((r) => r.name)).toEqual(['PkgOne']);
  });

  it('groupByPackage=false flattens packages and sorts by type', () => {
    const { result } = renderHook(
      () => useSessionModelTree({ groupByPackage: false }),
      { wrapper: makeWrapper(client) },
    );
    // No Package wrappers — the response's top-level nodes come through
    // directly. Only one archetype ('part') here, so sort is a no-op
    // but existence of both names still proves the projection.
    expect(result.current.tree.map((n) => n.rawKind)).not.toContain('Package');
    expect(result.current.tree.map((n) => n.name).sort()).toEqual([
      'Enclosure',
      'ProductionCell',
    ]);
  });

  it('per-node source_uri wins over the response uri for file attribution', () => {
    client.setQueryData(
      ['workspace-scoped-tree', URI_A, URI_B],
      [
        {
          ...n('pa', 'PkgOne', 'Package', [
            { ...n('sw', 'ProductionCell', 'PartUsage'), source_uri: URI_A },
          ]),
          source_uri: URI_A,
        },
      ],
    );
    const { result } = renderHook(() => useSessionModelTree(), {
      wrapper: makeWrapper(client),
    });
    const pkg = result.current.tree[0];
    expect(pkg.uri).toBe(URI_A);
    expect(pkg.children[0].uri).toBe(URI_A);
  });

  it('clears liveSessionId on live-store reset', () => {
    const { result } = renderHook(() => useSessionModelTree(), {
      wrapper: makeWrapper(client),
    });
    act(() => {
      useSessionLiveStore.getState().applyHello('session-x', {
        tick: 0,
        time_ms: 0,
        completed: false,
        subsystems: {},
        scalar_vars: {},
        string_vars: {},
        constraint_results: [],
      });
    });
    expect(result.current.liveSessionId).toBe('session-x');
    act(() => {
      useSessionLiveStore.getState().reset();
    });
    expect(result.current.liveSessionId).toBeNull();
  });

  describe('expectedSessionId reset-path guard (UX closeout #4 / #17)', () => {
    it('omitting expectedSessionId preserves prior behaviour exactly (values merge regardless of session identity)', () => {
      const { result } = renderHook(() => useSessionModelTree(), {
        wrapper: makeWrapper(client),
      });
      act(() => {
        useSessionLiveStore.getState().applyHello('session-1', {
          tick: 0,
          time_ms: 0,
          completed: false,
          subsystems: {},
          scalar_vars: { 'ProductionCell.temperature': 12.5 },
          string_vars: {},
          constraint_results: [],
        });
      });
      const sb = result.current.tree.find((n) => n.name === 'ProductionCell')!;
      const v = sb.children.find((n) => n.name === 'temperature') as AttributeTreeNode;
      expect(v.value).toBe(12.5);
    });

    it('treats the snapshot as absent when expectedSessionId does not match the live store session', () => {
      const { result } = renderHook(
        () => useSessionModelTree({ expectedSessionId: 'session-other' }),
        { wrapper: makeWrapper(client) },
      );
      act(() => {
        useSessionLiveStore.getState().applyHello('session-1', {
          tick: 5,
          time_ms: 500,
          completed: false,
          subsystems: { StationStates: { current_state: 'armed', completed: false, kind_label: 'sm' } },
          scalar_vars: { 'ProductionCell.temperature': 12.5 },
          string_vars: {},
          constraint_results: [],
        });
      });
      // liveSessionId still reports the real store value — only the
      // MERGE is short-circuited, not the hook's own session tracking.
      expect(result.current.liveSessionId).toBe('session-1');
      expect(result.current.currentTick).toBe(0);
      const sb = result.current.tree.find((n) => n.name === 'ProductionCell')!;
      const v = sb.children.find((n) => n.name === 'temperature') as AttributeTreeNode;
      const sm = sb.children.find((n) => n.name === 'StationStates') as SmTreeNode;
      expect(v.value).toBeUndefined();
      expect(sm.currentState).toBeUndefined();
    });

    it('resumes merging once expectedSessionId catches up to the live store session (no stale reprocessing, no permanent lockout)', () => {
      const { result, rerender } = renderHook(
        ({ expectedSessionId }: { expectedSessionId: string | null }) =>
          useSessionModelTree({ expectedSessionId }),
        {
          wrapper: makeWrapper(client),
          initialProps: { expectedSessionId: null as string | null },
        },
      );
      // Target-switch in flight: RunWorkflow already nulled
      // activeSessionId, but the old session's stream hasn't reset yet.
      act(() => {
        useSessionLiveStore.getState().applyHello('session-old', {
          tick: 10,
          time_ms: 1000,
          completed: false,
          subsystems: {},
          scalar_vars: { 'ProductionCell.temperature': 99 },
          string_vars: {},
          constraint_results: [],
        });
      });
      let sb = result.current.tree.find((n) => n.name === 'ProductionCell')!;
      let v = sb.children.find((n) => n.name === 'temperature') as AttributeTreeNode;
      expect(v.value).toBeUndefined();

      // New session starts for the newly-selected target; the caller
      // now expects it.
      rerender({ expectedSessionId: 'session-new' });
      act(() => {
        useSessionLiveStore.getState().applyHello('session-new', {
          tick: 0,
          time_ms: 0,
          completed: false,
          subsystems: {},
          scalar_vars: { 'ProductionCell.temperature': 1 },
          string_vars: {},
          constraint_results: [],
        });
      });
      sb = result.current.tree.find((n) => n.name === 'ProductionCell')!;
      v = sb.children.find((n) => n.name === 'temperature') as AttributeTreeNode;
      expect(v.value).toBe(1);
      expect(result.current.liveSessionId).toBe('session-new');
    });
  });
});
