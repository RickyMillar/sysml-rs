/**
 * Semantic-tokens wiring contract for `attachLspClient`.
 *
 * The server (`sysml-lsp-server`) ships a full semantic-tokens
 * implementation and advertises a 15-type + 6-modifier legend on
 * `initialize`. VS Code / Zed already consume those tokens through
 * their native LSP clients. This spec pins the simulation-app side:
 *
 *  1. The `initialize` request advertises `semanticTokens` capability
 *     so the server actually emits the legend + full frames.
 *  2. Once initialize resolves with a legend, the client registers a
 *     Monaco semantic-tokens provider whose `getLegend()` returns the
 *     server's legend unchanged.
 *  3. The provider's `provideDocumentSemanticTokens` round-trips
 *     `textDocument/semanticTokens/full` and hands the integer-array
 *     payload to Monaco as a `Uint32Array` (LSP and Monaco share the
 *     same encoding, so no transformation needed).
 *
 * The harness stubs `monaco` and `WebSocket` so the test runs in
 * jsdom without pulling the monaco-editor module graph.
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { attachLspClient } from '../lspClient';

// ── WebSocket stub ───────────────────────────────────────────────────

interface PendingRequest {
  id: number;
  method: string;
  params: unknown;
}

class FakeSocket {
  static readonly OPEN = 1;
  static readonly CONNECTING = 0;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;
  static instances: FakeSocket[] = [];

  readyState = FakeSocket.OPEN;
  onopen: (() => void) | null = null;
  onmessage: ((ev: { data: string }) => void) | null = null;
  onerror: (() => void) | null = null;
  onclose: (() => void) | null = null;
  sent: string[] = [];
  pending: PendingRequest[] = [];

  constructor(public url: string) {
    FakeSocket.instances.push(this);
    // Fire onopen on the next microtask to match real WS lifecycle.
    queueMicrotask(() => this.onopen?.());
  }

  send(payload: string): void {
    this.sent.push(payload);
    const parsed = JSON.parse(payload) as PendingRequest & {
      id?: number;
      method: string;
    };
    if (typeof parsed.id === 'number') {
      this.pending.push({
        id: parsed.id,
        method: parsed.method,
        params: parsed.params,
      });
    }
  }

  /** Test helper: deliver a JSON-RPC response for a pending request. */
  replyTo(method: string, result: unknown): void {
    const req = this.pending.find((p) => p.method === method);
    if (!req) throw new Error(`no pending request for method ${method}`);
    this.pending = this.pending.filter((p) => p !== req);
    this.onmessage?.({
      data: JSON.stringify({ jsonrpc: '2.0', id: req.id, result }),
    });
  }

  close(): void {
    this.readyState = FakeSocket.CLOSED;
    this.onclose?.();
  }
}

// ── Monaco stub ──────────────────────────────────────────────────────

type ProviderEntry = {
  languageId: string;
  provider: {
    getLegend: () => { tokenTypes: string[]; tokenModifiers: string[] };
    provideDocumentSemanticTokens: (model: unknown) => Promise<unknown>;
    releaseDocumentSemanticTokens: () => void;
  };
};

const semanticTokenProviders: ProviderEntry[] = [];

const monacoStub = {
  Uri: { parse: (s: string) => ({ toString: () => s }) },
  editor: {
    setModelMarkers: vi.fn(),
  },
  languages: {
    CompletionItemKind: { Text: 1 },
    registerHoverProvider: vi.fn(() => ({ dispose: vi.fn() })),
    registerCompletionItemProvider: vi.fn(() => ({ dispose: vi.fn() })),
    registerDefinitionProvider: vi.fn(() => ({ dispose: vi.fn() })),
    registerDocumentSemanticTokensProvider: vi.fn(
      (languageId: string, provider: ProviderEntry['provider']) => {
        semanticTokenProviders.push({ languageId, provider });
        return { dispose: vi.fn() };
      },
    ),
  },
} as unknown as Parameters<typeof attachLspClient>[1];

// ── Editor stub ──────────────────────────────────────────────────────

const URI = 'file:///abs/path/Layout.sysml';

function fakeEditor() {
  const model = {
    getValue: () => 'package P;',
    uri: { toString: () => URI },
  };
  return {
    getModel: () => model,
    onDidChangeModelContent: vi.fn(() => ({ dispose: vi.fn() })),
  } as unknown as Parameters<typeof attachLspClient>[0];
}

const LEGEND = {
  tokenTypes: [
    'namespace',
    'type',
    'class',
    'struct',
    'property',
    'variable',
    'parameter',
    'function',
    'keyword',
    'comment',
    'string',
    'number',
    'operator',
    'interface',
    'enum',
  ],
  tokenModifiers: [
    'definition',
    'declaration',
    'readonly',
    'static',
    'abstract',
    'deprecated',
  ],
};

// ── Tests ────────────────────────────────────────────────────────────

afterEach(() => {
  FakeSocket.instances = [];
  semanticTokenProviders.length = 0;
  vi.unstubAllGlobals();
});

describe('attachLspClient — semantic tokens', () => {
  it('advertises semanticTokens in the initialize capabilities', async () => {
    vi.stubGlobal('WebSocket', FakeSocket as unknown as typeof WebSocket);

    attachLspClient(fakeEditor(), monacoStub, {
      uri: URI,
      lspUrl: 'ws://stub/lsp',
    });

    // Wait for onopen → initialize to be sent.
    await new Promise((r) => queueMicrotask(() => r(null)));
    const socket = FakeSocket.instances[0]!;
    const initSent = socket.sent.find((s) =>
      s.includes('"method":"initialize"'),
    );
    expect(initSent).toBeDefined();
    const parsed = JSON.parse(initSent!) as {
      params: {
        capabilities?: { textDocument?: { semanticTokens?: unknown } };
      };
    };
    const cap = parsed.params.capabilities?.textDocument?.semanticTokens as
      | { requests: { full: boolean } }
      | undefined;
    expect(cap).toBeDefined();
    expect(cap!.requests.full).toBe(true);
  });

  it('registers a Monaco semantic-tokens provider once initialize resolves with a legend', async () => {
    vi.stubGlobal('WebSocket', FakeSocket as unknown as typeof WebSocket);

    const onReady = vi.fn();
    attachLspClient(fakeEditor(), monacoStub, {
      uri: URI,
      lspUrl: 'ws://stub/lsp',
      onReady,
    });

    // Wait for the initialize request to land.
    await new Promise((r) => queueMicrotask(() => r(null)));
    const socket = FakeSocket.instances[0]!;

    // Reply to initialize with a legend, then wait for the post-init
    // microtasks (notify('initialized'), provider registration, didOpen).
    socket.replyTo('initialize', {
      capabilities: {
        semanticTokensProvider: { legend: LEGEND, full: true },
      },
    });
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    expect(onReady).toHaveBeenCalledTimes(1);
    expect(semanticTokenProviders).toHaveLength(1);
    const entry = semanticTokenProviders[0]!;
    expect(entry.languageId).toBe('sysml');
    expect(entry.provider.getLegend()).toEqual(LEGEND);
  });

  it('returns a Uint32Array of the server-emitted token data to Monaco', async () => {
    vi.stubGlobal('WebSocket', FakeSocket as unknown as typeof WebSocket);

    attachLspClient(fakeEditor(), monacoStub, {
      uri: URI,
      lspUrl: 'ws://stub/lsp',
    });
    await new Promise((r) => queueMicrotask(() => r(null)));
    const socket = FakeSocket.instances[0]!;
    socket.replyTo('initialize', {
      capabilities: {
        semanticTokensProvider: { legend: LEGEND, full: true },
      },
    });
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    const provider = semanticTokenProviders[0]!.provider;
    const model = { uri: { toString: () => URI } };
    const tokenPromise = provider.provideDocumentSemanticTokens(model);

    // Mirror the request the provider should have sent.
    await new Promise((r) => setTimeout(r, 0));
    const tokensData = [0, 0, 7, 8, 0]; // (deltaLine, deltaStart, length, type, mods)
    socket.replyTo('textDocument/semanticTokens/full', {
      resultId: 'r1',
      data: tokensData,
    });

    const tokens = (await tokenPromise) as {
      resultId?: string;
      data: Uint32Array;
    } | null;
    expect(tokens).not.toBeNull();
    expect(tokens!.resultId).toBe('r1');
    expect(tokens!.data).toBeInstanceOf(Uint32Array);
    expect(Array.from(tokens!.data)).toEqual(tokensData);
  });

  it('skips provider registration when the server omits a legend', async () => {
    vi.stubGlobal('WebSocket', FakeSocket as unknown as typeof WebSocket);

    attachLspClient(fakeEditor(), monacoStub, {
      uri: URI,
      lspUrl: 'ws://stub/lsp',
    });
    await new Promise((r) => queueMicrotask(() => r(null)));
    const socket = FakeSocket.instances[0]!;
    socket.replyTo('initialize', { capabilities: {} });
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    expect(semanticTokenProviders).toHaveLength(0);
  });
});
