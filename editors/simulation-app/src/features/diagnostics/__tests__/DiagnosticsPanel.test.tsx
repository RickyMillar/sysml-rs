/**
 * DiagnosticsPanel — render + interaction tests (R6.1).
 *
 * Covers the R6.1 acceptance criteria:
 *   - Panel render: empty / populated / filtered / loading / error
 *   - Grouping by URI with count badge + collapse
 *   - Severity checkbox + search + scope filter
 *   - Row Enter/Space activation fires selection + navigate
 *   - Registry: descriptor appended at id 'diagnostics', position 'utility'
 *
 * Backend traffic is mocked at the `fetch` boundary — every call hits
 * `/api/command` with a JSON body, so we route by `body.command` and
 * `body.params.uri` to shape the response per URI.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  cleanup,
  render,
  screen,
  fireEvent,
  waitFor,
  within,
} from '@testing-library/react';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { MemoryRouter } from 'react-router-dom';
import type { ReactNode } from 'react';
import { DiagnosticsPanel } from '../DiagnosticsPanel';
import { diagnosticsPanel } from '@/shared/panels/diagnostics';
import { panelRegistry, findPanel } from '@/shared/panels/registry';
import type { Diagnostic } from '@/engine/types';

type DiagsByUri = Record<string, Diagnostic[]>;

interface HarnessOpts {
  diagsByUri?: DiagsByUri;
  uris?: string[];
  activeUri?: string | null;
  failFetch?: boolean;
  navigate?: (path: string) => void;
  select?: (uri: string | null, elementId: string | null) => void;
  extractElementId?: (entry: {
    uri: string;
    diagnostic: Diagnostic;
  }) => string | null;
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
      return new Response(JSON.stringify({ error: 'backend down' }), {
        status: 500,
      });
    }
    const body = init?.body ? JSON.parse(String(init.body)) : {};
    if (body.command === 'sysml.diagnostics') {
      const uri: string = body.params?.uri ?? '';
      const diags = opts.diagsByUri?.[uri] ?? [];
      return new Response(JSON.stringify(diags), {
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
        <MemoryRouter initialEntries={['/run']}>{children}</MemoryRouter>
      </QueryClientProvider>
    );
  }

  const utils = render(
    <Wrapper>
      <DiagnosticsPanel
        uris={opts.uris ?? Object.keys(opts.diagsByUri ?? {})}
        activeUri={opts.activeUri ?? null}
        testHooks={{
          navigate: opts.navigate,
          select: opts.select,
          extractElementId: opts.extractElementId,
        }}
      />
    </Wrapper>,
  );
  return { client, fetchMock, ...utils };
}

async function waitForSettled() {
  await waitFor(() => {
    expect(screen.queryByTestId('diagnostics-panel-loading')).toBeNull();
  });
}

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
});

describe('DiagnosticsPanel — registry', () => {
  it('is appended to the panel registry as a utility panel', () => {
    const found = findPanel('diagnostics');
    expect(found).toBeDefined();
    expect(found).toBe(diagnosticsPanel);
    expect(diagnosticsPanel.defaultPosition).toBe('utility');
    expect(diagnosticsPanel.id).toBe('diagnostics');
    expect(diagnosticsPanel.icon).toBe('bug_report');
    // Registry order: diagnosticsPanel should be appended after archivePanel.
    const ids = panelRegistry.map((p) => p.id);
    expect(ids).toContain('diagnostics');
    expect(ids.indexOf('diagnostics')).toBeGreaterThan(ids.indexOf('archive'));
  });

  it('shares the collapsed neutral accent with all utility panels (ninebar: no per-panel accents)', () => {
    const utilityAccents = panelRegistry
      .filter((p) => p.defaultPosition === 'utility')
      .map((p) => p.accentColor);
    const unique = new Set(utilityAccents);
    expect(unique.size).toBe(1);
    expect(unique.has('var(--text-secondary)')).toBe(true);
  });
});

describe('DiagnosticsPanel — empty state', () => {
  it('renders the "no diagnostics" empty state when every URI is clean', async () => {
    mountPanel({ diagsByUri: {} });
    await waitForSettled();
    // No URIs means no queries run and loading never flips true — the
    // zero-entries empty state is shown immediately.
    expect(
      screen.getByTestId('diagnostics-panel-empty-total'),
    ).toBeInTheDocument();
  });

  it('renders the "no diagnostics" empty state when every URI returns []', async () => {
    mountPanel({
      uris: ['file:///a.sysml', 'file:///b.sysml'],
      diagsByUri: { 'file:///a.sysml': [], 'file:///b.sysml': [] },
    });
    await waitForSettled();
    expect(
      screen.getByTestId('diagnostics-panel-empty-total'),
    ).toBeInTheDocument();
  });
});

describe('DiagnosticsPanel — populated', () => {
  const diags: DiagsByUri = {
    'file:///a.sysml': [
      {
        severity: 'error',
        message: 'Unexpected token',
        code: 'E001',
        span: { file: 'file:///a.sysml', start: 10, end: 20, line: 3, col: 5 },
      },
      {
        severity: 'warning',
        message: 'Unused import foo',
        span: { file: 'file:///a.sysml', start: 50, end: 60, line: 5, col: 1 },
      },
    ],
    'file:///b.sysml': [
      {
        severity: 'info',
        message: 'Deprecated call to bar',
        span: { file: 'file:///b.sysml', start: 0, end: 4, line: 1, col: 1 },
      },
    ],
  };

  it('renders one group per URI with the correct count badge', async () => {
    mountPanel({ diagsByUri: diags });
    await waitForSettled();

    const headerA = screen.getByTestId(
      'diagnostics-group-header-file:///a.sysml',
    );
    const headerB = screen.getByTestId(
      'diagnostics-group-header-file:///b.sysml',
    );
    expect(headerA).toBeInTheDocument();
    expect(headerB).toBeInTheDocument();
    expect(
      screen.getByTestId('diagnostics-group-count-file:///a.sysml').textContent,
    ).toBe('2');
    expect(
      screen.getByTestId('diagnostics-group-count-file:///b.sysml').textContent,
    ).toBe('1');
  });

  it('renders severity chip, code, and message on each row', async () => {
    mountPanel({ diagsByUri: diags });
    await waitForSettled();
    expect(screen.getByText('Unexpected token')).toBeInTheDocument();
    expect(screen.getByText('E001')).toBeInTheDocument();
    expect(screen.getByTestId('diagnostic-severity-chip-error')).toBeInTheDocument();
    expect(screen.getByTestId('diagnostic-severity-chip-warning')).toBeInTheDocument();
  });

  it('collapses and expands groups when the header is clicked', async () => {
    mountPanel({ diagsByUri: diags });
    await waitForSettled();

    const header = screen.getByTestId(
      'diagnostics-group-header-file:///a.sysml',
    );
    const beforeItems = screen.getByTestId(
      'diagnostics-group-items-file:///a.sysml',
    );
    expect(beforeItems).toBeInTheDocument();

    fireEvent.click(header);
    expect(
      screen.queryByTestId('diagnostics-group-items-file:///a.sysml'),
    ).toBeNull();

    fireEvent.click(header);
    expect(
      screen.getByTestId('diagnostics-group-items-file:///a.sysml'),
    ).toBeInTheDocument();
  });
});

describe('DiagnosticsPanel — filters', () => {
  const diags: DiagsByUri = {
    'file:///a.sysml': [
      { severity: 'error', message: 'bad a', code: 'E010' },
      { severity: 'warning', message: 'warn a' },
    ],
    'file:///b.sysml': [
      { severity: 'info', message: 'info b' },
      { severity: 'warning', message: 'warn b' },
    ],
  };

  it('hides diagnostics whose severity checkbox is unchecked', async () => {
    mountPanel({ diagsByUri: diags });
    await waitForSettled();

    // Toggle off warnings.
    const warningBox = screen.getByTestId('diagnostics-panel-severity-warning');
    fireEvent.click(warningBox);

    // Warnings now gone; errors + info remain.
    expect(screen.queryByText('warn a')).toBeNull();
    expect(screen.queryByText('warn b')).toBeNull();
    expect(screen.getByText('bad a')).toBeInTheDocument();
    expect(screen.getByText('info b')).toBeInTheDocument();
  });

  it('narrows via substring search on message', async () => {
    mountPanel({ diagsByUri: diags });
    await waitForSettled();

    const searchBox = screen.getByTestId('diagnostics-panel-search');
    fireEvent.change(searchBox, { target: { value: 'warn' } });

    expect(screen.getByText('warn a')).toBeInTheDocument();
    expect(screen.getByText('warn b')).toBeInTheDocument();
    expect(screen.queryByText('bad a')).toBeNull();
    expect(screen.queryByText('info b')).toBeNull();
  });

  it('scopes to the current file when the scope toggle is flipped', async () => {
    mountPanel({ diagsByUri: diags, activeUri: 'file:///a.sysml' });
    await waitForSettled();

    const currentScope = screen.getByTestId('diagnostics-panel-scope-current');
    fireEvent.click(currentScope);

    expect(screen.getByText('bad a')).toBeInTheDocument();
    expect(screen.getByText('warn a')).toBeInTheDocument();
    expect(screen.queryByText('info b')).toBeNull();
    expect(screen.queryByText('warn b')).toBeNull();
  });

  it('shows the "no matches" empty state when filters narrow to zero', async () => {
    mountPanel({ diagsByUri: diags });
    await waitForSettled();

    const searchBox = screen.getByTestId('diagnostics-panel-search');
    fireEvent.change(searchBox, { target: { value: 'nothing-matches-zzz' } });

    expect(
      screen.getByTestId('diagnostics-panel-empty-filtered'),
    ).toBeInTheDocument();

    // Clear-filters button restores everything.
    const clear = screen.getByTestId('diagnostics-panel-clear-filters');
    fireEvent.click(clear);
    expect(screen.getByText('bad a')).toBeInTheDocument();
  });
});

describe('DiagnosticsPanel — loading / error', () => {
  it('renders the error state when the fetch fails', async () => {
    mountPanel({ uris: ['file:///a.sysml'], failFetch: true });
    await waitFor(() => {
      expect(screen.getByTestId('diagnostics-panel-error')).toBeInTheDocument();
    });
  });
});

describe('DiagnosticsPanel — row activation', () => {
  it('fires select + navigate on row click', async () => {
    const navigate = vi.fn();
    const select = vi.fn();
    mountPanel({
      diagsByUri: {
        'file:///a.sysml': [
          {
            severity: 'error',
            message: 'boom',
            span: {
              file: 'file:///a.sysml',
              start: 0,
              end: 3,
              line: 2,
              col: 4,
            },
          },
        ],
      },
      navigate,
      select,
    });
    await waitForSettled();

    const list = screen.getByTestId('diagnostics-panel-list');
    const rowButton = within(list).getByTestId('diagnostic-row-error');
    fireEvent.click(rowButton);

    expect(select).toHaveBeenCalledWith('file:///a.sysml', null);
    expect(navigate).toHaveBeenCalledWith('/run');
  });

  it('activates on Enter keypress', async () => {
    const navigate = vi.fn();
    const select = vi.fn();
    mountPanel({
      diagsByUri: {
        'file:///a.sysml': [
          { severity: 'warning', message: 'hey', span: { file: 'file:///a.sysml', start: 0, end: 1 } },
        ],
      },
      navigate,
      select,
    });
    await waitForSettled();

    const rowButton = screen.getByTestId('diagnostic-row-warning');
    fireEvent.keyDown(rowButton, { key: 'Enter' });

    expect(select).toHaveBeenCalledWith('file:///a.sysml', null);
    expect(navigate).toHaveBeenCalledWith('/run');
  });

  it('activates on Space keypress', async () => {
    const navigate = vi.fn();
    const select = vi.fn();
    mountPanel({
      diagsByUri: {
        'file:///a.sysml': [
          { severity: 'info', message: 'heads up', span: { file: 'file:///a.sysml', start: 0, end: 1 } },
        ],
      },
      navigate,
      select,
    });
    await waitForSettled();

    const rowButton = screen.getByTestId('diagnostic-row-info');
    fireEvent.keyDown(rowButton, { key: ' ' });

    expect(select).toHaveBeenCalledWith('file:///a.sysml', null);
    expect(navigate).toHaveBeenCalledWith('/run');
  });

  it('forwards the extracted element id when the hook returns one', async () => {
    const navigate = vi.fn();
    const select = vi.fn();
    mountPanel({
      diagsByUri: {
        'file:///a.sysml': [
          { severity: 'error', message: 'target', span: { file: 'file:///a.sysml', start: 0, end: 1 } },
        ],
      },
      navigate,
      select,
      extractElementId: () => 'element-123',
    });
    await waitForSettled();

    fireEvent.click(screen.getByTestId('diagnostic-row-error'));
    expect(select).toHaveBeenCalledWith('file:///a.sysml', 'element-123');
  });

  it('falls back to the parent URI when the diagnostic has no span', async () => {
    const navigate = vi.fn();
    const select = vi.fn();
    mountPanel({
      diagsByUri: {
        'file:///orphan.sysml': [{ severity: 'error', message: 'spanless' }],
      },
      navigate,
      select,
    });
    await waitForSettled();

    fireEvent.click(screen.getByTestId('diagnostic-row-error'));
    expect(select).toHaveBeenCalledWith('file:///orphan.sysml', null);
  });

  it('exposes an accessible label containing severity + message + location', async () => {
    mountPanel({
      diagsByUri: {
        'file:///a.sysml': [
          {
            severity: 'error',
            message: 'Unexpected token',
            code: 'E001',
            span: { file: 'file:///a.sysml', start: 10, end: 20, line: 3, col: 5 },
          },
        ],
      },
    });
    await waitForSettled();

    const row = screen.getByTestId('diagnostic-row-error');
    const ariaLabel = row.getAttribute('aria-label') ?? '';
    expect(ariaLabel).toContain('Error');
    expect(ariaLabel).toContain('Unexpected token');
    expect(ariaLabel).toContain('E001');
    expect(ariaLabel).toContain('a.sysml');
    expect(ariaLabel).toContain('3:5');
  });
});
