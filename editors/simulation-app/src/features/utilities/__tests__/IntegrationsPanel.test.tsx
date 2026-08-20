/**
 * IntegrationsPanel — Phase 7 contract test.
 *
 * Pins:
 *   1. Three sections render (MCP / REST / LSP) with the correct
 *      connection info derived from `window.location`.
 *   2. The tool/command count is pulled from `GET /commands`
 *      (`fetchCommandCatalog`); a mocked length renders as text.
 *   3. The MCP config snippets bake the binary-path field into both
 *      the Claude Desktop JSON and the `claude mcp add` line.
 *   4. Editing the binary path field rewrites both snippets live.
 *   5. "Test connection" hits `GET /health`; on success the ok
 *      indicator appears; on failure the error one does.
 *   6. "Copy" buttons call `navigator.clipboard.writeText` with the
 *      exact snippet text — the round-trip contract that makes the
 *      panel useful to a user wiring Claude Desktop.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from '@testing-library/react';
import type { ReactNode } from 'react';
import { IntegrationsPanel } from '@/features/utilities/IntegrationsPanel';
import { resetCommandCatalogCache } from '@/features/command-palette/commandCatalog';

// ── HTTP wire stub ───────────────────────────────────────────────────

const FETCH_MOCK = vi.fn();

function ok(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { 'Content-Type': 'application/json' },
  });
}

function fail(status: number, message: string): Response {
  return new Response(JSON.stringify({ error: message }), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function makeWrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

// ── Clipboard stub ───────────────────────────────────────────────────

const CLIPBOARD_WRITE = vi.fn();

beforeEach(() => {
  resetCommandCatalogCache();
  FETCH_MOCK.mockReset();
  FETCH_MOCK.mockImplementation(async (url: string) => {
    if (url.endsWith('/commands')) {
      return ok([
        { name: 'sysml.workspace.info', category: 'Query', description: '', params: [], returns: '', stateful: false },
        { name: 'sysml.query', category: 'Query', description: '', params: [], returns: '', stateful: false },
        { name: 'sysml.sessions.info', category: 'Execution', description: '', params: [], returns: '', stateful: false },
      ]);
    }
    if (url.endsWith('/health')) {
      return ok({ status: 'ok', version: '0.1.0' });
    }
    return ok({});
  });
  vi.stubGlobal('fetch', FETCH_MOCK);
  CLIPBOARD_WRITE.mockReset();
  CLIPBOARD_WRITE.mockResolvedValue(undefined);
  Object.assign(navigator, {
    clipboard: { writeText: CLIPBOARD_WRITE },
  });
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('IntegrationsPanel', () => {
  it('renders all three sections with origin-derived URLs', async () => {
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <IntegrationsPanel />
      </Wrapper>,
    );

    expect(screen.getByTestId('integrations-mcp')).toBeInTheDocument();
    expect(screen.getByTestId('integrations-rest')).toBeInTheDocument();
    expect(screen.getByTestId('integrations-lsp')).toBeInTheDocument();

    // jsdom default origin is http://localhost — the panel reads it.
    const restBase = within(
      screen.getByTestId('integrations-rest-base'),
    ).getByText(/http:\/\//);
    expect(restBase).toBeInTheDocument();

    const lspUrl = within(screen.getByTestId('integrations-lsp-url')).getByText(
      /^ws:\/\/.*\/lsp$/,
    );
    expect(lspUrl).toBeInTheDocument();
  });

  it('shows the live tool/command count fetched from /commands', async () => {
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <IntegrationsPanel />
      </Wrapper>,
    );

    // The MCP and REST sections both pull from `fetchCommandCatalog`
    // (the REST mirror of the MCP tool inventory) — both should
    // settle on `3` once the query resolves.
    await waitFor(() =>
      expect(
        within(screen.getByTestId('integrations-mcp-tool-count')).getByText(
          '3',
        ),
      ).toBeInTheDocument(),
    );
    await waitFor(() =>
      expect(
        within(
          screen.getByTestId('integrations-rest-command-count'),
        ).getByText('3'),
      ).toBeInTheDocument(),
    );
  });

  it('bakes the binary-path input into both MCP snippets and rewrites on edit', () => {
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <IntegrationsPanel />
      </Wrapper>,
    );

    const desktop = screen.getByTestId('integrations-mcp-desktop');
    const code = screen.getByTestId('integrations-mcp-code');

    // Default path is `sysml-api`.
    expect(desktop.textContent).toContain('"command": "sysml-api"');
    expect(desktop.textContent).toContain('"--mcp"');
    expect(code.textContent).toContain('claude mcp add sysml sysml-api --mcp');

    // Edit the binary path and watch both snippets rewrite.
    const input = screen.getByTestId(
      'integrations-binary-path',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: '/usr/local/bin/sysml-api' } });
    expect(desktop.textContent).toContain(
      '"command": "/usr/local/bin/sysml-api"',
    );
    expect(code.textContent).toContain(
      'claude mcp add sysml /usr/local/bin/sysml-api --mcp',
    );
  });

  it('Test connection button hits /health and renders the ok indicator', async () => {
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <IntegrationsPanel />
      </Wrapper>,
    );

    fireEvent.click(screen.getByTestId('integrations-rest-test'));

    const ok = await screen.findByTestId('integrations-rest-health-ok');
    expect(ok.textContent).toContain('ok');
    expect(ok.textContent).toContain('0.1.0');

    // /commands fires on mount; /health only after the click.
    const healthCalls = FETCH_MOCK.mock.calls.filter(([url]) =>
      String(url).endsWith('/health'),
    );
    expect(healthCalls).toHaveLength(1);
  });

  it('Test connection error path surfaces the failure message', async () => {
    FETCH_MOCK.mockImplementation(async (url: string) => {
      if (url.endsWith('/commands')) return ok([]);
      if (url.endsWith('/health')) return fail(503, 'service unavailable');
      return ok({});
    });
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <IntegrationsPanel />
      </Wrapper>,
    );

    fireEvent.click(screen.getByTestId('integrations-rest-test'));
    const err = await screen.findByTestId('integrations-rest-health-err');
    expect(err).toBeVisible();
  });

  it('Copy buttons write the exact snippet text to the clipboard', async () => {
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <IntegrationsPanel />
      </Wrapper>,
    );

    fireEvent.click(screen.getByTestId('integrations-mcp-desktop-copy'));
    await waitFor(() =>
      expect(CLIPBOARD_WRITE).toHaveBeenCalledWith(
        expect.stringContaining('"command": "sysml-api"'),
      ),
    );

    fireEvent.click(screen.getByTestId('integrations-mcp-code-copy'));
    await waitFor(() =>
      expect(CLIPBOARD_WRITE).toHaveBeenCalledWith(
        'claude mcp add sysml sysml-api --mcp',
      ),
    );

    fireEvent.click(screen.getByTestId('integrations-rest-curl-copy'));
    await waitFor(() =>
      expect(CLIPBOARD_WRITE).toHaveBeenLastCalledWith(
        expect.stringContaining('sysml.workspace.info'),
      ),
    );
  });
});
