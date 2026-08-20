/**
 * DebugDrawer — Phase 8 dev-only introspection surface.
 *
 * Pins three contracts:
 *   1. Each section fetches its corresponding backend command exactly
 *      once on mount.
 *   2. Toggling a section's auto-refresh checkbox drives repeat
 *      fetches at the 1s cadence; un-toggling stops them.
 *   3. With `VITE_DEBUG_DRAWER` unset (default), the UtilityDrawer
 *      does NOT register the toolbar affordance — the panel is
 *      effectively absent from the shell.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { DebugDrawer } from '@/features/utilities/DebugDrawer';
import { UtilityDrawer } from '@/features/utilities/UtilityDrawer';

const FETCH_MOCK = vi.fn();

function ok(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

function commandOf(call: unknown): string | undefined {
  const init = (call as [unknown, RequestInit | undefined])[1];
  const body = init?.body;
  if (typeof body !== 'string') return undefined;
  try {
    return (JSON.parse(body) as { command?: string }).command;
  } catch {
    return undefined;
  }
}

function countCalls(command: string): number {
  return FETCH_MOCK.mock.calls.filter((c) => commandOf(c) === command).length;
}

function makeWrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

beforeEach(() => {
  FETCH_MOCK.mockReset();
  FETCH_MOCK.mockImplementation(async (_url: string, init?: RequestInit) => {
    const body = typeof init?.body === 'string' ? init.body : '{}';
    let cmd = '';
    try {
      cmd = (JSON.parse(body) as { command?: string }).command ?? '';
    } catch {
      cmd = '';
    }
    if (cmd === 'sysml.workspace.info') {
      return ok([
        {
          uri: 'file:///model.sysml',
          tree: [],
          stats: { element_count: 17, relationship_count: 4 },
        },
      ]);
    }
    if (cmd === 'sysml.salsa.stats') {
      return ok({ executions: 42, validations: 13, hit_ratio: 0.236 });
    }
    if (cmd === 'sysml.cache.status') {
      return ok({ status: 'no_library' });
    }
    if (cmd === 'sysml.dependency.status') {
      return ok({ roots: [], summary: { total_dependencies: 0 } });
    }
    return ok({});
  });
  vi.stubGlobal('fetch', FETCH_MOCK);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.unstubAllEnvs();
  vi.useRealTimers();
});

describe('DebugDrawer', () => {
  it('fetches each backend command exactly once on mount', async () => {
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <DebugDrawer />
      </Wrapper>,
    );

    // react-query's queryFn runs in a microtask — flush.
    await act(async () => {
      await Promise.resolve();
      await Promise.resolve();
    });

    expect(countCalls('sysml.workspace.info')).toBe(1);
    expect(countCalls('sysml.salsa.stats')).toBe(1);
    expect(countCalls('sysml.cache.status')).toBe(1);
    expect(countCalls('sysml.dependency.status')).toBe(1);
  });

  it('renders fetched values once the queries settle', async () => {
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <DebugDrawer />
      </Wrapper>,
    );

    // React-query's async queryFn → useQuery state update chain
    // crosses several microtasks; use findByText so the assertion
    // retries until the section actually settles (configured for the
    // default 1s timeout in jest-dom).
    expect(await screen.findByText('23.6%')).toBeInTheDocument();
    // Workspace summary picked up the stats roll-up.
    expect(screen.getByText('Loaded URIs')).toBeInTheDocument();
  });

  it('auto-refresh toggle drives repeat fetches at the 1s cadence', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <DebugDrawer />
      </Wrapper>,
    );

    // Initial fetches settle.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const baseline = countCalls('sysml.salsa.stats');
    expect(baseline).toBe(1);

    // Without auto-refresh, no new fetch after 2.5 seconds.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2500);
    });
    expect(countCalls('sysml.salsa.stats')).toBe(baseline);

    // Enable auto-refresh on the salsa section.
    const toggle = screen.getByTestId(
      'debug-section-salsa-stats-autorefresh',
    ) as HTMLInputElement;
    await act(async () => {
      toggle.click();
    });

    // Two ticks of the 1s interval → two extra fetches.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1000);
    });
    expect(countCalls('sysml.salsa.stats')).toBeGreaterThanOrEqual(baseline + 2);

    // The other sections must not have started polling.
    expect(countCalls('sysml.workspace.info')).toBe(1);
    expect(countCalls('sysml.cache.status')).toBe(1);

    // Turn it off — counter must stop advancing.
    await act(async () => {
      toggle.click();
    });
    const settled = countCalls('sysml.salsa.stats');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2500);
    });
    expect(countCalls('sysml.salsa.stats')).toBe(settled);
  });
});

describe('UtilityDrawer Debug affordance gating', () => {
  it('does not surface the Debug toggle when VITE_DEBUG_DRAWER is unset', () => {
    // No `vi.stubEnv` — flag stays undefined, matching production /
    // default-dev behaviour.
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <UtilityDrawer />
      </Wrapper>,
    );

    expect(screen.queryByTestId('utility-toggle-debug')).toBeNull();
  });
});
