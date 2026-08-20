/**
 * ArchivePanel — render + interaction tests (R4.1).
 *
 * Covers the acceptance criteria from the task brief:
 *   - Panel render: empty / populated / filtered / filters-match-nothing
 *   - Row Restore action: navigate('/run') + setActiveSession invoked
 *   - Registry: descriptor appended at id 'archive', position 'utility'
 *
 * Backend traffic is mocked at the `fetch` boundary — every call hits
 * `/api/command` with a JSON body, so we route by `body.command` to
 * shape the response.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, fireEvent, within, waitFor } from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import { ArchivePanel } from '../ArchivePanel';
import { archivePanel } from '@/shared/panels/archive';
import { panelRegistry, findPanel } from '@/shared/panels/registry';
import { useSessionStore } from '@/features/sessions/store';
import type { ArchivedSessionSummary } from '../types';
import type { ReactNode } from 'react';

const NOW = 1_700_000_000_000;

function makeEntries(overrides: Partial<ArchivedSessionSummary>[] = []): ArchivedSessionSummary[] {
  return overrides.map((o, idx) => ({
    id: o.id ?? `entry-${idx}`,
    label: o.label ?? `Label ${idx}`,
    origin: o.origin ?? 'run',
    workspace_uri: o.workspace_uri ?? 'file:///ws',
    created_at: o.created_at ?? NOW - 60_000 * (idx + 1),
    ended_at: o.ended_at ?? null,
    ticks: o.ticks ?? 10,
    is_golden: o.is_golden ?? false,
    golden_label: o.golden_label,
    verdict_counts: o.verdict_counts,
  }));
}

interface HarnessOpts {
  entries?: ArchivedSessionSummary[];
  failFetch?: boolean;
  navigate?: (path: string) => void;
  setActiveSession?: (id: string | null) => void;
  copyToClipboard?: (text: string) => void;
}

function mountPanel(opts: HarnessOpts = {}) {
  const client = new QueryClient({
    defaultOptions: {
      queries: { retry: false, staleTime: Infinity, refetchOnWindowFocus: false },
      mutations: { retry: false },
    },
  });
  const fetchMock = vi.fn(async (_url: string, init?: RequestInit) => {
    if (opts.failFetch) {
      return new Response(JSON.stringify({ error: 'backend down' }), { status: 500 });
    }
    const body = init?.body ? JSON.parse(String(init.body)) : {};
    if (body.command === 'sysml.sessions.archive.list') {
      return new Response(
        JSON.stringify({ entries: opts.entries ?? [] }),
        { status: 200, headers: { 'Content-Type': 'application/json' } },
      );
    }
    if (
      body.command === 'sysml.sessions.archive.mark_golden' ||
      body.command === 'sysml.sessions.archive.unmark_golden'
    ) {
      return new Response(JSON.stringify({ ok: true }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    return new Response(JSON.stringify({}), { status: 200 });
  });
  vi.stubGlobal('fetch', fetchMock);

  function Wrapper({ children }: { children: ReactNode }) {
    return (
      <QueryClientProvider client={client}>
        <MemoryRouter initialEntries={['/archive']}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  }

  const utils = render(
    <Wrapper>
      <ArchivePanel
        workspaceUri={null}
        testHooks={{
          navigate: opts.navigate,
          setActiveSession: opts.setActiveSession,
          copyToClipboard: opts.copyToClipboard,
          now: NOW,
        }}
      />
    </Wrapper>,
  );
  return { client, fetchMock, ...utils };
}

/**
 * Wait for the panel to finish its initial list-query so the list is
 * rendered (or the empty/error state settled). Retries a handful of
 * microtasks because react-query transitions through isLoading → data
 * asynchronously.
 */
async function waitForArchiveSettled() {
  await waitFor(() => {
    expect(screen.queryByTestId('archive-panel-loading')).toBeNull();
  });
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  useSessionStore.setState({ activeSessionId: null });
});

describe('ArchivePanel — registry', () => {
  it('is registered in panelRegistry with utility position', () => {
    expect(panelRegistry).toContain(archivePanel);
    expect(findPanel('archive')).toBe(archivePanel);
    expect(archivePanel.defaultPosition).toBe('utility');
    expect(archivePanel.id).toBe('archive');
    expect(archivePanel.icon).toBe('archive');
  });

  it('shares the collapsed neutral accent with the Variables pane (ninebar: no per-panel accents)', () => {
    const variables = findPanel('variables');
    expect(variables).toBeTruthy();
    expect(archivePanel.accentColor).toBe('var(--text-secondary)');
    expect(variables!.accentColor).toBe('var(--text-secondary)');
  });
});

describe('ArchivePanel — render states', () => {
  it('renders the empty-total empty state when backend returns no entries', async () => {
    mountPanel({ entries: [] });
    await waitForArchiveSettled();
    expect(screen.getByTestId('archive-panel')).toBeInTheDocument();
    expect(screen.getByTestId('archive-panel-empty-total')).toBeInTheDocument();
    expect(screen.queryByTestId('archive-panel-list')).toBeNull();
  });

  it('renders rows for every entry in the populated list', async () => {
    mountPanel({
      entries: makeEntries([
        { id: 'a', label: 'Run A' },
        { id: 'b', label: 'Verify B', origin: 'verify', is_golden: true, golden_label: 'golden' },
      ]),
    });
    await waitForArchiveSettled();
    const list = screen.getByTestId('archive-panel-list');
    expect(list).toBeInTheDocument();
    expect(within(list).getByText('Run A')).toBeInTheDocument();
    expect(within(list).getByText('Verify B')).toBeInTheDocument();
    // Golden star present on the golden row.
    expect(screen.getByTestId('archive-row-b')).toHaveAttribute('data-golden', 'true');
    expect(within(screen.getByTestId('archive-row-b')).getByTestId('archive-row-golden-star')).toBeInTheDocument();
  });

  it('applies the client search filter to narrow rows', async () => {
    mountPanel({
      entries: makeEntries([
        { id: 'a', label: 'Smoke run' },
        { id: 'b', label: 'Regression verify', origin: 'verify' },
      ]),
    });
    await waitForArchiveSettled();
    const search = screen.getByTestId('archive-panel-search') as HTMLInputElement;
    fireEvent.change(search, { target: { value: 'smoke' } });
    expect(screen.getByTestId('archive-row-a')).toBeInTheDocument();
    expect(screen.queryByTestId('archive-row-b')).toBeNull();
  });

  it('shows the filters-empty state when no entries match and offers a clear button', async () => {
    mountPanel({
      entries: makeEntries([
        { id: 'a', label: 'foo' },
        { id: 'b', label: 'bar' },
      ]),
    });
    await waitForArchiveSettled();
    const search = screen.getByTestId('archive-panel-search') as HTMLInputElement;
    fireEvent.change(search, { target: { value: 'no-such-match' } });
    expect(screen.getByTestId('archive-panel-empty-filtered')).toBeInTheDocument();
    const clearButton = screen.getByTestId('archive-panel-clear-filters');
    fireEvent.click(clearButton);
    // Back to all rows visible after clearing.
    expect(screen.getByTestId('archive-row-a')).toBeInTheDocument();
    expect(screen.getByTestId('archive-row-b')).toBeInTheDocument();
  });

  it('renders the error state when the backend rejects the list call', async () => {
    mountPanel({ failFetch: true });
    await waitForArchiveSettled();
    expect(screen.getByTestId('archive-panel-error')).toBeInTheDocument();
  });

  it('narrows to golden sessions when the toggle is active', async () => {
    // Backend echoes the client filter via mock — we return the same
    // two entries regardless of params, and client-side `filterArchive`
    // narrows to the golden row.
    mountPanel({
      entries: makeEntries([
        { id: 'a', label: 'Normal' },
        { id: 'b', label: 'Golden', is_golden: true },
      ]),
    });
    await waitForArchiveSettled();
    fireEvent.click(screen.getByTestId('archive-panel-only-golden'));
    // Toggling the filter kicks off a refetch for a new query key;
    // wait for the list to settle again before asserting.
    await waitFor(() => {
      expect(screen.queryByTestId('archive-panel-loading')).toBeNull();
      expect(screen.getByTestId('archive-row-b')).toBeInTheDocument();
    });
    expect(screen.queryByTestId('archive-row-a')).toBeNull();
  });
});

describe('ArchivePanel — row interactions', () => {
  it('Restore via row click invokes setActiveSession + navigate("/run")', async () => {
    const navigate = vi.fn();
    const setActiveSession = vi.fn();
    mountPanel({
      entries: makeEntries([{ id: 'arch-1', label: 'Pick me' }]),
      navigate,
      setActiveSession,
    });
    await waitForArchiveSettled();
    const row = screen.getByTestId('archive-row-arch-1');
    const button = within(row).getByTestId('archive-row-button');
    fireEvent.click(button);
    expect(setActiveSession).toHaveBeenCalledWith('arch-1');
    expect(navigate).toHaveBeenCalledWith('/run');
  });

  it('Enter key on the row button triggers Restore', async () => {
    const navigate = vi.fn();
    const setActiveSession = vi.fn();
    mountPanel({
      entries: makeEntries([{ id: 'arch-2', label: 'Kb row' }]),
      navigate,
      setActiveSession,
    });
    await waitForArchiveSettled();
    const button = within(screen.getByTestId('archive-row-arch-2')).getByTestId('archive-row-button');
    fireEvent.keyDown(button, { key: 'Enter' });
    expect(setActiveSession).toHaveBeenCalledWith('arch-2');
    expect(navigate).toHaveBeenCalledWith('/run');
  });

  it('opens the menu and copies the ID via "Copy ID"', async () => {
    const copy = vi.fn();
    mountPanel({
      entries: makeEntries([{ id: 'copy-me', label: 'Clip' }]),
      copyToClipboard: copy,
    });
    await waitForArchiveSettled();
    const row = screen.getByTestId('archive-row-copy-me');
    fireEvent.click(within(row).getByTestId('archive-row-menu-button'));
    const copyItem = within(row).getByTestId('archive-row-menu-copy-id');
    fireEvent.click(copyItem);
    expect(copy).toHaveBeenCalledWith('copy-me');
  });

  it('menu surfaces "Mark Golden" for non-golden rows and "Unmark Golden" for golden rows', async () => {
    mountPanel({
      entries: makeEntries([
        { id: 'non-golden', label: 'Plain' },
        { id: 'golden', label: 'Gold', is_golden: true },
      ]),
    });
    await waitForArchiveSettled();
    fireEvent.click(
      within(screen.getByTestId('archive-row-non-golden')).getByTestId('archive-row-menu-button'),
    );
    expect(
      within(screen.getByTestId('archive-row-non-golden')).getByTestId('archive-row-menu-mark-golden'),
    ).toBeInTheDocument();

    fireEvent.click(
      within(screen.getByTestId('archive-row-golden')).getByTestId('archive-row-menu-button'),
    );
    expect(
      within(screen.getByTestId('archive-row-golden')).getByTestId('archive-row-menu-unmark-golden'),
    ).toBeInTheDocument();
  });
});
