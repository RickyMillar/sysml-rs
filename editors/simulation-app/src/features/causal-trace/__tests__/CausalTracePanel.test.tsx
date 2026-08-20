/**
 * CausalTracePanel — render states (empty, loading, populated, error) (R7.1).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { render, screen, fireEvent, cleanup } from '@testing-library/react';
import type { ReactNode } from 'react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { CausalTracePanel } from '../CausalTracePanel';
import { useCausalTraceStore } from '../useCausalTraceStore';
import type { CausationEvent, CausationTraceResult } from '@/engine/types';

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

function Wrap({ children }: { children: ReactNode }) {
  const client = makeClient();
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

beforeEach(() => {
  mockPost.mockReset();
  useCausalTraceStore.setState({ root: null, refocusTick: 0 });
});

afterEach(() => {
  cleanup();
  vi.clearAllMocks();
});

function ev(overrides: Partial<CausationEvent> = {}): CausationEvent {
  return {
    id: 'ev-1-0',
    tick: 1,
    actor: 'sm1',
    target: 'speed',
    detail: '',
    caused_by: [],
    kind: 'variable_write',
    var: 'speed',
    old_value: 0,
    new_value: 100,
    ...overrides,
  } as CausationEvent;
}

describe('CausalTracePanel', () => {
  it('renders empty state when no root is selected', () => {
    render(
      <Wrap>
        <CausalTracePanel root={null} />
      </Wrap>,
    );
    expect(screen.getByTestId('causal-trace-no-root')).toBeDefined();
  });

  it('renders loading state while the query is in flight', () => {
    // Never-resolving promise keeps the hook in loading.
    mockPost.mockReturnValue(new Promise(() => {}));
    render(
      <Wrap>
        <CausalTracePanel
          root={{ kind: 'by-id', sessionId: 's1', eventId: 'ev-1-0' }}
        />
      </Wrap>,
    );
    expect(screen.getByTestId('causal-trace-loading')).toBeDefined();
  });

  it('renders a chain when the backend returns events', async () => {
    const result: CausationTraceResult = {
      root: ev({ id: 'ev-2-0', tick: 2 }),
      chain: [
        ev({ id: 'ev-2-0', tick: 2, detail: 'root' }),
        ev({
          id: 'ev-2-1',
          tick: 2,
          kind: 'transition_fire',
          from: 'A',
          to: 'B',
          event: 'go',
        } as Partial<CausationEvent>),
      ],
      max_depth_used: 5,
    };
    mockPost.mockResolvedValue(result);
    render(
      <Wrap>
        <CausalTracePanel
          root={{ kind: 'by-id', sessionId: 's1', eventId: 'ev-2-0' }}
        />
      </Wrap>,
    );
    await screen.findByTestId('causal-trace-chain');
    expect(screen.getByTestId('causation-row-0')).toBeDefined();
    expect(screen.getByTestId('causation-row-1')).toBeDefined();
  });

  it('renders the empty-chain hint when backend returns no chain', async () => {
    const result: CausationTraceResult = {
      root: null,
      chain: [],
      max_depth_used: 5,
    };
    mockPost.mockResolvedValue(result);
    render(
      <Wrap>
        <CausalTracePanel
          root={{ kind: 'by-tick', sessionId: 's1', tick: 2, target: 'speed' }}
        />
      </Wrap>,
    );
    await screen.findByTestId('causal-trace-empty');
  });

  it('renders the error state when the backend throws', async () => {
    mockPost.mockRejectedValue(new Error('backend down'));
    render(
      <Wrap>
        <CausalTracePanel
          root={{ kind: 'by-id', sessionId: 's1', eventId: 'ev-1-0' }}
        />
      </Wrap>,
    );
    const node = await screen.findByTestId('causal-trace-error');
    expect(node.textContent).toContain('backend down');
  });

  it('invokes onScrubTo when a chain row is clicked', async () => {
    const result: CausationTraceResult = {
      root: ev(),
      chain: [ev()],
      max_depth_used: 5,
    };
    mockPost.mockResolvedValue(result);
    const onScrubTo = vi.fn();
    render(
      <Wrap>
        <CausalTracePanel
          root={{ kind: 'by-id', sessionId: 's1', eventId: 'ev-1-0' }}
          onScrubTo={onScrubTo}
        />
      </Wrap>,
    );
    const row = await screen.findByTestId('causation-row-0');
    fireEvent.click(row);
    expect(onScrubTo).toHaveBeenCalledTimes(1);
    expect(onScrubTo.mock.calls[0]?.[0]?.id).toBe('ev-1-0');
  });

  it('falls back to the store when `root` prop is omitted', () => {
    useCausalTraceStore.setState({
      root: { kind: 'by-id', sessionId: 's1', eventId: 'ev-1-0' },
      refocusTick: 1,
    });
    mockPost.mockReturnValue(new Promise(() => {}));
    render(
      <Wrap>
        <CausalTracePanel />
      </Wrap>,
    );
    // Should show loading, not the no-root state.
    expect(screen.getByTestId('causal-trace-loading')).toBeDefined();
  });
});
