/**
 * ViewsPanel — workspace-scope pin (Bucket 5-followup, 2026-05-05;
 * workspace-always collapse, scope-collapse W4a 2026-07-16).
 *
 * The panel always scopes to `__workspace__`, never the focused per-file
 * URI — otherwise authored views living in any file other than the
 * auto-promoted `focusedUri` would silently disappear from the drawer.
 * An explicit `uri` prop is the only per-file escape hatch.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { useWorkspaceStore } from '@/store/workspace';
import { ViewsPanel } from '@/features/views/ViewsPanel';

const FETCH_MOCK = vi.fn();

beforeEach(() => {
  FETCH_MOCK.mockReset();
  vi.stubGlobal('fetch', FETCH_MOCK);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  useWorkspaceStore.getState().reset();
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

function firstFetchBody(): { command?: string; params?: { uri?: string } } {
  return JSON.parse(String(FETCH_MOCK.mock.calls[0]?.[1]?.body ?? '{}'));
}

function fetchBodies(): Array<{
  command?: string;
  params?: {
    uri?: string;
    spec?: {
      filter?: { type?: string; viewpoint_id?: string | null; kinds?: string[] };
    };
  };
}> {
  return FETCH_MOCK.mock.calls.map((c) =>
    JSON.parse(String(c[1]?.body ?? '{}')),
  );
}

describe('ViewsPanel scope', () => {
  it('queries __workspace__ when a workspaceRoot is loaded, ignoring focusedUri', async () => {
    FETCH_MOCK.mockResolvedValueOnce(ok({ rows: [], total_estimate: 0 }));

    useWorkspaceStore.setState({
      workspaceRoot: '/some/root',
      focusedUri: 'file:///just-one.sysml',
    });

    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <ViewsPanel />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(FETCH_MOCK).toHaveBeenCalled();
    });
    const url = String(FETCH_MOCK.mock.calls[0]?.[0] ?? '');
    const body = firstFetchBody();
    expect(url).toContain('/api/command');
    expect(body.command).toBe('sysml.query');
    expect(body.params?.uri).toBe('__workspace__');
  });

  it('still queries __workspace__ when no workspaceRoot is set, ignoring focusedUri', async () => {
    FETCH_MOCK.mockResolvedValueOnce(ok({ rows: [], total_estimate: 0 }));

    useWorkspaceStore.setState({
      workspaceRoot: null,
      focusedUri: 'file:///solo.sysml',
    });

    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <ViewsPanel />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(FETCH_MOCK).toHaveBeenCalled();
    });
    const body = firstFetchBody();
    expect(body.command).toBe('sysml.query');
    expect(body.params?.uri).toBe('__workspace__');
  });

  it('honours the explicit uri prop as the per-file escape hatch', async () => {
    FETCH_MOCK.mockResolvedValueOnce(ok({ rows: [], total_estimate: 0 }));

    useWorkspaceStore.setState({
      workspaceRoot: '/some/root',
      focusedUri: null,
    });

    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <ViewsPanel uri="file:///solo.sysml" />
      </Wrapper>,
    );

    await waitFor(() => {
      expect(FETCH_MOCK).toHaveBeenCalled();
    });
    const body = firstFetchBody();
    expect(body.command).toBe('sysml.query');
    expect(body.params?.uri).toBe('file:///solo.sysml');
  });

  it('switches to viewpoint-scoped query when a viewpoint is picked, restores the cached unfiltered fetch on Clear', async () => {
    FETCH_MOCK.mockImplementation(async (_url: string, init: RequestInit) => {
      const body = JSON.parse(String(init.body ?? '{}'));
      const filter = body.params?.spec?.filter as
        | { type?: string; viewpoint_id?: string | null; kinds?: string[]; filters?: unknown[] }
        | undefined;
      const isViewpointSearch =
        filter?.type === 'kind' ||
        (filter?.type === 'all' &&
          Array.isArray(filter.filters) &&
          filter.filters.some(
            (f) => (f as { type?: string }).type === 'kind',
          ));
      if (isViewpointSearch) {
        return ok({
          rows: [
            {
              id: 'vp1',
              name: 'Performance',
              qualified_name: 'Pkg::Performance',
              kind: 'ViewpointDefinition',
              owner_id: null,
              source_span: null,
            },
          ],
          total_estimate: 1,
        });
      }
      return ok({ rows: [], total_estimate: 0 });
    });

    useWorkspaceStore.setState({
      workspaceRoot: '/some/root',
      focusedUri: null,
    });
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <ViewsPanel />
      </Wrapper>,
    );

    // Initial unfiltered fetch.
    await waitFor(() => {
      expect(
        fetchBodies().some(
          (b) =>
            b.command === 'sysml.query' &&
            b.params?.spec?.filter?.type === 'view' &&
            b.params?.spec?.filter?.viewpoint_id === null,
        ),
      ).toBe(true);
    });

    // Open the picker by focusing, then type — debounce is real timers
    // here, so wait the picker out.
    const input = screen.getByTestId('viewpoint-picker-input') as HTMLInputElement;
    act(() => {
      input.focus();
    });
    fireEvent.change(input, { target: { value: 'Perf' } });

    const row = await waitFor(
      () => screen.getByTestId('viewpoint-picker-row-vp1'),
      { timeout: 2000 },
    );

    const callsBeforePick = FETCH_MOCK.mock.calls.length;
    fireEvent.click(row);

    // Selection swapped to the viewpoint-scoped hook — wait for the
    // filtered fetch to land.
    await waitFor(() => {
      expect(
        fetchBodies().some(
          (b) =>
            b.command === 'sysml.query' &&
            b.params?.spec?.filter?.viewpoint_id === 'vp1',
        ),
      ).toBe(true);
    });
    expect(FETCH_MOCK.mock.calls.length).toBe(callsBeforePick + 1);
    expect(
      screen.getByTestId('viewpoint-picker-selected').textContent,
    ).toContain('Performance');

    // Clearing brings back the unfiltered list — react-query has the
    // earlier result cached, so no new fetch should fire.
    const callsBeforeClear = FETCH_MOCK.mock.calls.length;
    fireEvent.click(screen.getByTestId('viewpoint-picker-clear'));
    await waitFor(() => {
      expect(screen.queryByTestId('viewpoint-picker-selected')).toBeNull();
    });
    expect(FETCH_MOCK.mock.calls.length).toBe(callsBeforeClear);
  });

  it('renders the count from sysml.query view rows', async () => {
    FETCH_MOCK.mockResolvedValueOnce(
      ok({
        rows: [
          {
            id: 'a',
            name: 'AView',
            qualified_name: 'AView',
            kind: 'ViewDefinition',
            owner_id: null,
            source_span: null,
            expansion: {
              kind: 'view',
              data: { id: 'a', name: 'AView', kind: 'ViewDefinition', exposed: [], renderings: [], filters: [], source_span: null },
            },
          },
          {
            id: 'b',
            name: 'BView',
            qualified_name: 'BView',
            kind: 'ViewUsage',
            owner_id: null,
            source_span: null,
            expansion: {
              kind: 'view',
              data: { id: 'b', name: 'BView', kind: 'ViewUsage', exposed: [], renderings: [], filters: [], source_span: null },
            },
          },
        ],
        total_estimate: 2,
      }),
    );
    useWorkspaceStore.setState({
      workspaceRoot: '/some/root',
      focusedUri: null,
    });
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <ViewsPanel />
      </Wrapper>,
    );
    await waitFor(() => {
      expect(screen.getByText(/Views \(2\)/)).toBeTruthy();
    });
  });
});
