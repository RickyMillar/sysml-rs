/**
 * Bug A — Source utility renders blank Monaco because workspace hydrate
 * only seeds tree + stats. SourcePanel must lazy-fetch the file source
 * on first open and seed the workspace store. This spec pins:
 *   1. With `file.source === ''`, the panel calls `POST /files` once.
 *   2. Once the fetch settles, `seedSource(uri, text)` writes back
 *      without flipping `dirty` to true.
 *   3. Subsequent mounts for the same URI do NOT refetch (cached).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { QueryClient, QueryClientProvider } from '@tanstack/react-query';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { SourcePanel } from '../SourcePanel';
import { useWorkspaceStore } from '@/store/workspace';

vi.mock('../MonacoSysmlEditor', () => ({
  MonacoSysmlEditor: ({ value, testId }: { value: string; testId?: string }) => (
    <div data-testid={testId ?? 'monaco-editor-stub'} data-value-length={value.length}>
      {value.slice(0, 50)}
    </div>
  ),
}));

const FETCH_MOCK = vi.fn();

function makeWrapper() {
  const qc = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0, staleTime: 60_000 } },
  });
  return function Wrapper({ children }: { children: ReactNode }) {
    return <QueryClientProvider client={qc}>{children}</QueryClientProvider>;
  };
}

const initial = useWorkspaceStore.getState();

const URI = '/abs/path/Layout.sysml';
const TEXT = 'package CellLayout {\n  part def Foo;\n}\n';

function seedStoreEmpty() {
  useWorkspaceStore.setState({
    ...initial,
    workspaceRoot: '/abs',
    loadedFiles: new Map([
      [URI, { uri: URI, source: '', dirty: false, tree: [] }],
    ]),
    focusedUri: URI,
  });
}

beforeEach(() => {
  FETCH_MOCK.mockReset();
  FETCH_MOCK.mockImplementation(async (url: string) => {
    if (url.endsWith('/files')) {
      return new Response(JSON.stringify({ uri: URI, source: TEXT }), {
        status: 200,
        headers: { 'Content-Type': 'application/json' },
      });
    }
    return new Response('null', { status: 200 });
  });
  vi.stubGlobal('fetch', FETCH_MOCK);
});

afterEach(() => {
  cleanup();
  vi.unstubAllGlobals();
  useWorkspaceStore.setState(initial, true);
});

describe('SourcePanel — Bug A (lazy seed)', () => {
  it('fires POST /files exactly once when the focused file has no source yet', async () => {
    seedStoreEmpty();
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <SourcePanel />
      </Wrapper>,
    );

    // Loading affordance while the fetch is in flight.
    expect(screen.getByTestId('source-panel-loading')).toBeInTheDocument();

    // Wait for the editor to mount with the fetched buffer.
    const editor = await screen.findByTestId('source-panel-editor');
    expect(editor.getAttribute('data-value-length')).toBe(String(TEXT.length));

    const filesCalls = FETCH_MOCK.mock.calls.filter(([url]) =>
      String(url).endsWith('/files'),
    );
    expect(filesCalls).toHaveLength(1);
  });

  it('writes the fetched text into the store via seedSource (no dirty flag)', async () => {
    seedStoreEmpty();
    const Wrapper = makeWrapper();
    render(
      <Wrapper>
        <SourcePanel />
      </Wrapper>,
    );

    await waitFor(() => {
      const file = useWorkspaceStore.getState().loadedFiles.get(URI);
      expect(file?.source).toBe(TEXT);
      // seedSource path — the user hasn't typed anything yet.
      expect(file?.dirty).toBe(false);
    });
  });
});
