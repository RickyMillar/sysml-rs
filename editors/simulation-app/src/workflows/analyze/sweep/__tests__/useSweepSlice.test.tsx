/**
 * Tests for useSweepSlice (R5.4).
 *
 * Covers:
 *   - Calls the backend with `{ command: 'sysml.batch.slice', params }`
 *     using the exact `{ batch_id, filter }` shape.
 *   - Handles empty results without throwing.
 *   - Re-keys + refetches when the filter changes.
 *   - Is disabled (no fetch) when batchId is null or filter is null.
 *   - `fetchSweepSlice` (imperative helper) works standalone.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import type { ReactNode } from 'react';
import type { SliceFilter } from '@/engine/types';
import { fetchSweepSlice, useSweepSlice } from '../useSweepSlice';

function makeWrapper(client: QueryClient) {
  return function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    );
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

describe('useSweepSlice', () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  let client: QueryClient;

  beforeEach(() => {
    fetchMock = vi.fn(async () =>
      jsonResponse({
        children: [
          {
            id: 'c0',
            session_id: 's0',
            status: 'complete',
            params: { voltage: 12 },
          },
        ],
      }),
    );
    vi.stubGlobal('fetch', fetchMock);
    client = new QueryClient({
      defaultOptions: {
        queries: {
          retry: false,
          staleTime: Infinity,
          refetchOnWindowFocus: false,
        },
        mutations: { retry: false },
      },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    client.clear();
  });

  it('posts to /api/command with the right command + batch_id + filter', async () => {
    const filter: SliceFilter = {
      param_predicate: { param: 'voltage', op: 'gt', value: 10 },
    };
    const { result } = renderHook(() => useSweepSlice('batch-7', filter), {
      wrapper: makeWrapper(client),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));

    expect(fetchMock).toHaveBeenCalledTimes(1);
    const [, init] = fetchMock.mock.calls[0];
    const body = JSON.parse(init.body as string);
    expect(body).toEqual({
      command: 'sysml.batch.slice',
      params: { batch_id: 'batch-7', filter },
    });
    expect(result.current.data).toEqual([
      {
        id: 'c0',
        session_id: 's0',
        status: 'complete',
        params: { voltage: 12 },
      },
    ]);
  });

  it('handles an empty children array without throwing', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({ children: [] }));
    const filter: SliceFilter = { only_status: 'complete' };
    const { result } = renderHook(() => useSweepSlice('b', filter), {
      wrapper: makeWrapper(client),
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([]);
  });

  it('is parked (no fetch) when filter is null', async () => {
    const { result } = renderHook(() => useSweepSlice('b', null), {
      wrapper: makeWrapper(client),
    });
    // Give react-query a tick to consider running. It should not.
    await new Promise((r) => setTimeout(r, 20));
    expect(result.current.isFetching).toBe(false);
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('is parked when batchId is null or empty', async () => {
    const filter: SliceFilter = { only_verdict: 'fail' };
    const { rerender } = renderHook(
      ({ id }: { id: string | null }) => useSweepSlice(id, filter),
      {
        wrapper: makeWrapper(client),
        initialProps: { id: null as string | null },
      },
    );
    await new Promise((r) => setTimeout(r, 10));
    expect(fetchMock).not.toHaveBeenCalled();

    rerender({ id: '' });
    await new Promise((r) => setTimeout(r, 10));
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it('refetches when the filter object changes (cache miss)', async () => {
    const { rerender, result } = renderHook(
      ({ filter }: { filter: SliceFilter }) =>
        useSweepSlice('b', filter),
      {
        wrapper: makeWrapper(client),
        initialProps: {
          filter: {
            param_predicate: { param: 'voltage', op: 'gt', value: 10 },
          } as SliceFilter,
        },
      },
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(fetchMock).toHaveBeenCalledTimes(1);

    rerender({
      filter: {
        param_predicate: { param: 'voltage', op: 'gt', value: 20 },
      } as SliceFilter,
    });
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));
  });
});

describe('fetchSweepSlice (imperative helper)', () => {
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    fetchMock = vi.fn(async () =>
      jsonResponse({
        children: [
          {
            id: 'x',
            session_id: null,
            status: 'pending',
            params: { t: 1 },
          },
        ],
      }),
    );
    vi.stubGlobal('fetch', fetchMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('invokes the command and returns the children array', async () => {
    const out = await fetchSweepSlice('batch-1', { only_status: 'pending' });
    expect(out).toHaveLength(1);
    expect(out[0].id).toBe('x');
    const [, init] = fetchMock.mock.calls[0];
    expect(JSON.parse(init.body as string)).toEqual({
      command: 'sysml.batch.slice',
      params: { batch_id: 'batch-1', filter: { only_status: 'pending' } },
    });
  });

  it('returns [] when the backend omits children', async () => {
    fetchMock.mockResolvedValueOnce(jsonResponse({}));
    const out = await fetchSweepSlice('batch-1', {});
    expect(out).toEqual([]);
  });
});
