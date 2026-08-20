/**
 * useCausationTrace — react-query hook tests (R7.1).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { renderHook, waitFor, cleanup } from '@testing-library/react';
import type { ReactNode } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  useCausationTrace,
  causationTraceKeys,
  type CausalTraceRoot,
} from '../useCausationTrace';

vi.mock('@/shared/api/http', () => ({
  httpGet: vi.fn(),
  httpPost: vi.fn(),
}));

import { httpPost } from '@/shared/api/http';
const mockPost = httpPost as unknown as ReturnType<typeof vi.fn>;

function makeClient(): QueryClient {
  return new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 0 } },
  });
}

function wrapper(client: QueryClient) {
  return ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
}

beforeEach(() => {
  mockPost.mockReset();
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

describe('causationTraceKeys', () => {
  it('derives distinct keys for by-id vs by-tick roots', () => {
    const byId: CausalTraceRoot = {
      kind: 'by-id',
      sessionId: 's1',
      eventId: 'ev-1-0',
    };
    const byTick: CausalTraceRoot = {
      kind: 'by-tick',
      sessionId: 's1',
      tick: 1,
      target: 'speed',
    };
    expect(causationTraceKeys.byRoot(byId)).not.toEqual(
      causationTraceKeys.byRoot(byTick),
    );
  });

  it('uses a null-sentinel key when root is null', () => {
    expect(causationTraceKeys.byRoot(null)).toEqual([
      'causationTrace',
      'none',
    ]);
  });

  it('varies by maxDepth', () => {
    const a: CausalTraceRoot = {
      kind: 'by-id',
      sessionId: 's1',
      eventId: 'ev-1-0',
      maxDepth: 3,
    };
    const b: CausalTraceRoot = { ...a, maxDepth: 5 };
    expect(causationTraceKeys.byRoot(a)).not.toEqual(
      causationTraceKeys.byRoot(b),
    );
  });
});

describe('useCausationTrace', () => {
  it('is parked when root is null', async () => {
    mockPost.mockResolvedValue({ root: null, chain: [], max_depth_used: 5 });
    const client = makeClient();
    const { result } = renderHook(() => useCausationTrace(null), {
      wrapper: wrapper(client),
    });
    // Parked query stays in loading=false + no fetch.
    await waitFor(() => {
      expect(result.current.fetchStatus).toBe('idle');
    });
    expect(mockPost).not.toHaveBeenCalled();
  });

  it('dispatches sysml.causation.trace with by-id params', async () => {
    mockPost.mockResolvedValue({
      root: {
        id: 'ev-1-0',
        tick: 1,
        kind: 'variable_write',
        var: 'speed',
        old_value: 0,
        new_value: 100,
        actor: 'sm1',
        target: 'speed',
        detail: 'speed = 100',
        caused_by: [],
      },
      chain: [
        {
          id: 'ev-1-0',
          tick: 1,
          kind: 'variable_write',
          var: 'speed',
          old_value: 0,
          new_value: 100,
          actor: 'sm1',
          target: 'speed',
          detail: '',
          caused_by: [],
        },
      ],
      max_depth_used: 5,
    });

    const client = makeClient();
    const root: CausalTraceRoot = {
      kind: 'by-id',
      sessionId: 's1',
      eventId: 'ev-1-0',
    };
    const { result } = renderHook(() => useCausationTrace(root), {
      wrapper: wrapper(client),
    });

    await waitFor(() => {
      expect(result.current.isSuccess).toBe(true);
    });

    expect(mockPost).toHaveBeenCalledWith('/api/command', {
      command: 'sysml.causation.trace',
      params: {
        session_id: 's1',
        max_depth: 5,
        root_event_id: 'ev-1-0',
      },
    });
    expect(result.current.data?.chain.length).toBe(1);
  });

  it('dispatches sysml.causation.trace with by-tick params', async () => {
    mockPost.mockResolvedValue({ root: null, chain: [], max_depth_used: 5 });
    const client = makeClient();
    const root: CausalTraceRoot = {
      kind: 'by-tick',
      sessionId: 's1',
      tick: 7,
      target: 'speed',
      maxDepth: 3,
    };
    renderHook(() => useCausationTrace(root), { wrapper: wrapper(client) });

    await waitFor(() => {
      expect(mockPost).toHaveBeenCalled();
    });
    expect(mockPost).toHaveBeenCalledWith('/api/command', {
      command: 'sysml.causation.trace',
      params: {
        session_id: 's1',
        max_depth: 3,
        root_tick: 7,
        root_target: 'speed',
      },
    });
  });

  it('surfaces errors via isError', async () => {
    mockPost.mockRejectedValue(new Error('boom'));
    const client = makeClient();
    const root: CausalTraceRoot = {
      kind: 'by-id',
      sessionId: 's1',
      eventId: 'ev-1-0',
    };
    const { result } = renderHook(() => useCausationTrace(root), {
      wrapper: wrapper(client),
    });
    await waitFor(() => {
      expect(result.current.isError).toBe(true);
    });
    expect(result.current.error?.message).toBe('boom');
  });
});
