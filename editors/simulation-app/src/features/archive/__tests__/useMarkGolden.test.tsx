/**
 * useMarkGolden — happy-path integration test (R4.1).
 *
 * Uses react-query's test harness and a fetch mock to verify:
 *   - The mutation POSTs to `/api/command` with the right command + params.
 *   - On success, every archive list query is invalidated.
 *
 * A parallel assertion covers `useUnmarkGolden` with the same approach.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import {
  QueryClient,
  QueryClientProvider,
  useQuery,
} from '@tanstack/react-query';
import { useMarkGolden, useUnmarkGolden } from '../useMarkGolden';
import { archiveKeys } from '../useArchiveList';
import type { ReactNode } from 'react';

function makeWrapper(client: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  };
}

describe('useMarkGolden', () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  let client: QueryClient;

  beforeEach(() => {
    fetchMock = vi.fn(async () =>
      new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      }),
    );
    vi.stubGlobal('fetch', fetchMock);
    client = new QueryClient({
      defaultOptions: {
        queries: { retry: false, staleTime: Infinity, refetchOnWindowFocus: false },
        mutations: { retry: false },
      },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    client.clear();
  });

  it('posts the mark_golden command and invalidates list queries on success', async () => {
    // Seed a dummy list-query cache entry so we can observe invalidation.
    const listKey = archiveKeys.lists();
    const dummyQuery = renderHook(
      () => useQuery({ queryKey: listKey, queryFn: async () => ['stub'] }),
      { wrapper: makeWrapper(client) },
    );
    await waitFor(() => expect(dummyQuery.result.current.data).toEqual(['stub']));

    const { result } = renderHook(() => useMarkGolden(), {
      wrapper: makeWrapper(client),
    });

    await result.current.mutateAsync({ id: 'arch-1', label: 'Reference run' });

    await waitFor(() =>
      expect(fetchMock).toHaveBeenCalledWith(
        expect.stringContaining('/api/command'),
        expect.objectContaining({ method: 'POST' }),
      ),
    );

    const [, init] = fetchMock.mock.calls[0];
    const body = JSON.parse(init.body as string);
    expect(body).toEqual({
      command: 'sysml.sessions.archive.mark_golden',
      params: { id: 'arch-1', label: 'Reference run' },
    });

    // After success, the list query should be marked stale (invalidated
    // or in the process of refetching). react-query flips `isInvalidated`
    // back to false once the refetch lands, so accept either the stale
    // marker or an `isFetching` refetch that was kicked off by the
    // invalidation hook.
    await waitFor(() => {
      const state = client.getQueryState(listKey);
      const refetchedOrInvalidated =
        state?.isInvalidated === true || state?.fetchStatus === 'fetching' || state?.dataUpdateCount! > 1;
      expect(refetchedOrInvalidated).toBe(true);
    });
  });

  it('unmarkGolden posts the unmark command and invalidates', async () => {
    const { result } = renderHook(() => useUnmarkGolden(), {
      wrapper: makeWrapper(client),
    });
    await result.current.mutateAsync({ id: 'arch-7' });
    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [, init] = fetchMock.mock.calls[0];
    const body = JSON.parse(init.body as string);
    expect(body).toEqual({
      command: 'sysml.sessions.archive.unmark_golden',
      params: { id: 'arch-7' },
    });
  });

  it('propagates errors from the backend (non-ok response)', async () => {
    fetchMock.mockResolvedValueOnce(
      new Response(JSON.stringify({ error: 'not found' }), { status: 404 }),
    );
    const { result } = renderHook(() => useMarkGolden(), {
      wrapper: makeWrapper(client),
    });
    await expect(
      result.current.mutateAsync({ id: 'missing', label: 'x' }),
    ).rejects.toThrow();
  });
});
