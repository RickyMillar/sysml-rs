/**
 * Tests for `useLoadWorkspace` (Bucket 5-followup, 2026-05-05).
 *
 * Pins:
 *   1. The mutationFn invokes `sysml.load_workspace` with the new root.
 *   2. Switching to a new root fires `clearWorkspace()` BEFORE the
 *      backend round-trip — stale loadedFiles are gone the moment the
 *      mutation starts.
 *   3. `error_count > 0` settles into `useWorkspaceUIStore.loadStatus`
 *      so the WorkspaceLoadErrorBanner surfaces them.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { renderHook, waitFor, act } from '@testing-library/react';
import type { ReactNode } from 'react';
import { useWorkspaceStore } from '@/store/workspace';
import { useWorkspaceUIStore } from '@/features/workspace/store';

const FETCH_MOCK = vi.fn();

beforeEach(() => {
  vi.stubGlobal('fetch', FETCH_MOCK);
  FETCH_MOCK.mockReset();
});

afterEach(() => {
  vi.unstubAllGlobals();
  useWorkspaceStore.getState().reset();
  useWorkspaceUIStore.setState({
    workspaceRoot: null,
    loadStatus: null,
  });
});

function ok(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

function makeWrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

describe('useLoadWorkspace', () => {
  it('POSTs sysml.load_workspace with the chosen root', async () => {
    FETCH_MOCK.mockResolvedValueOnce(
      ok({ loaded_uris: ['file:///x.sysml', '__workspace__'], error_count: 0, errors: [] }),
    );
    // Workspace-info hydrate fan-out:
    FETCH_MOCK.mockResolvedValueOnce(ok([]));

    const { useLoadWorkspace } = await import('../queries');
    const { result } = renderHook(() => useLoadWorkspace(), {
      wrapper: makeWrapper(),
    });

    await act(async () => {
      await result.current.mutateAsync('/some/root');
    });

    const firstCall = FETCH_MOCK.mock.calls[0];
    expect(firstCall[0]).toBe('/api/command');
    const body = JSON.parse(firstCall[1].body);
    expect(body).toEqual({
      command: 'sysml.load_workspace',
      params: { root: '/some/root' },
    });
  });

  it('fires clearWorkspace before the backend call when the root changes', async () => {
    // Seed: a previous workspace with one file loaded.
    useWorkspaceStore.setState({
      workspaceRoot: '/old/root',
      loadedFiles: new Map([
        [
          'file:///old.sysml',
          { uri: 'file:///old.sysml', source: '', dirty: false, tree: [] },
        ],
      ]),
    });

    let observedDuringFetch: number | null = null;
    FETCH_MOCK.mockImplementationOnce(async () => {
      observedDuringFetch = useWorkspaceStore.getState().loadedFiles.size;
      return ok({
        loaded_uris: ['file:///new.sysml', '__workspace__'],
        error_count: 0,
        errors: [],
      });
    });
    FETCH_MOCK.mockResolvedValueOnce(ok([]));

    const { useLoadWorkspace } = await import('../queries');
    const { result } = renderHook(() => useLoadWorkspace(), {
      wrapper: makeWrapper(),
    });

    await act(async () => {
      await result.current.mutateAsync('/new/root');
    });

    expect(observedDuringFetch).toBe(0);
  });

  it('does NOT clear when reloading the same root (refresh, not switch)', async () => {
    useWorkspaceStore.setState({
      workspaceRoot: '/same/root',
      loadedFiles: new Map([
        [
          'file:///kept.sysml',
          { uri: 'file:///kept.sysml', source: '', dirty: false, tree: [] },
        ],
      ]),
    });

    let observedDuringFetch: number | null = null;
    FETCH_MOCK.mockImplementationOnce(async () => {
      observedDuringFetch = useWorkspaceStore.getState().loadedFiles.size;
      return ok({
        loaded_uris: ['file:///kept.sysml', '__workspace__'],
        error_count: 0,
        errors: [],
      });
    });
    FETCH_MOCK.mockResolvedValueOnce(ok([]));

    const { useLoadWorkspace } = await import('../queries');
    const { result } = renderHook(() => useLoadWorkspace(), {
      wrapper: makeWrapper(),
    });

    await act(async () => {
      await result.current.mutateAsync('/same/root');
    });

    expect(observedDuringFetch).toBe(1);
  });

  it('same-root reload re-hydrates every URI and bumps reloadEpoch', async () => {
    // Seed: same root already loaded with a stale tree + stale source.
    useWorkspaceStore.setState({
      workspaceRoot: '/same/root',
      reloadEpoch: 0,
      loadedFiles: new Map([
        [
          'file:///kept.sysml',
          {
            uri: 'file:///kept.sysml',
            source: 'package Old;',
            dirty: false,
            tree: [{ id: 'dead-uuid', name: 'Old', kind: 'Package', archetype: 'other' as const, children: [] }],
          },
        ],
        [
          'file:///deleted.sysml',
          { uri: 'file:///deleted.sysml', source: '', dirty: false, tree: [] },
        ],
      ]),
      focusedUri: 'file:///deleted.sysml',
    });

    // load_workspace response: deleted.sysml is gone from disk.
    FETCH_MOCK.mockResolvedValueOnce(
      ok({ loaded_uris: ['file:///kept.sysml', '__workspace__'], error_count: 0, errors: [] }),
    );
    // workspace.info hydrate — MUST be asked for the already-loaded URI
    // again (reload semantics), returning the fresh tree.
    FETCH_MOCK.mockImplementationOnce(async (_url, init) => {
      const body = JSON.parse((init as RequestInit).body as string);
      expect(body.command).toBe('sysml.workspace.info');
      expect(body.params.uris).toEqual(['file:///kept.sysml']);
      return ok([
        {
          uri: 'file:///kept.sysml',
          tree: [{ id: 'fresh-uuid', name: 'New', kind: 'Package', archetype: 'other', children: [] }],
          stats: { elements_by_kind: { Package: 1 } },
        },
      ]);
    });
    // capabilities fetch (best-effort)
    FETCH_MOCK.mockResolvedValue(ok({}));

    const { useLoadWorkspace } = await import('../queries');
    const { result } = renderHook(() => useLoadWorkspace(), {
      wrapper: makeWrapper(),
    });

    await act(async () => {
      await result.current.mutateAsync('/same/root');
    });

    await waitFor(() => {
      const s = useWorkspaceStore.getState();
      expect(s.reloadEpoch).toBe(1);
      const kept = s.loadedFiles.get('file:///kept.sysml');
      expect(kept).toBeDefined();
      // Source cache dropped so SourcePanel re-fetches fresh text.
      expect(kept!.source).toBe('');
      // Tree replaced with the fresh element ids.
      expect(kept!.tree[0]?.id).toBe('fresh-uuid');
      // Files deleted on disk drop out; focus moves to a live file.
      expect(s.loadedFiles.has('file:///deleted.sysml')).toBe(false);
      expect(s.focusedUri).toBe('file:///kept.sysml');
    });
  });

  it('first load of a root does not bump reloadEpoch', async () => {
    FETCH_MOCK.mockResolvedValueOnce(
      ok({ loaded_uris: ['file:///x.sysml', '__workspace__'], error_count: 0, errors: [] }),
    );
    FETCH_MOCK.mockResolvedValue(ok([]));

    const { useLoadWorkspace } = await import('../queries');
    const { result } = renderHook(() => useLoadWorkspace(), {
      wrapper: makeWrapper(),
    });

    await act(async () => {
      await result.current.mutateAsync('/fresh/root');
    });

    await waitFor(() => {
      expect(useWorkspaceStore.getState().workspaceRoot).toBe('/fresh/root');
    });
    expect(useWorkspaceStore.getState().reloadEpoch).toBe(0);
  });

  it('writes errorCount + errors into useWorkspaceUIStore.loadStatus', async () => {
    FETCH_MOCK.mockResolvedValueOnce(
      ok({
        loaded_uris: ['file:///broken.sysml', '__workspace__'],
        error_count: 2,
        errors: [
          'file:///a.sysml:81: syntax error: expected ...',
          'file:///b.sysml:29: syntax error: expected ...',
        ],
      }),
    );
    FETCH_MOCK.mockResolvedValueOnce(ok([]));

    const { useLoadWorkspace } = await import('../queries');
    const { result } = renderHook(() => useLoadWorkspace(), {
      wrapper: makeWrapper(),
    });

    await act(async () => {
      await result.current.mutateAsync('/broken/root');
    });

    await waitFor(() => {
      const s = useWorkspaceUIStore.getState().loadStatus;
      expect(s).not.toBeNull();
      expect(s!.errorCount).toBe(2);
      expect(s!.errors).toHaveLength(2);
      expect(s!.dismissed).toBe(false);
    });
  });
});
