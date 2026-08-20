/**
 * Minimal LSP-over-WebSocket client for Monaco.
 *
 * Plan-1 wired the `/lsp` WebSocket to share the host's `SysmlService`
 * (commit 9aef3948); this is the FE consumer. It dials `/lsp`, drives
 * the LSP initialize handshake, mirrors a single Monaco model into the
 * server via `textDocument/didOpen` + `didChange`, and exposes
 * diagnostics / hover / completion through Monaco's provider APIs.
 *
 * Why hand-rolled instead of `monaco-languageclient`: modern (v8+)
 * versions require `monaco-vscode-api` services bootstrap and bring a
 * large dep graph. Our needs are narrow, so a thin direct client keeps
 * the dep surface and the failure modes small. Swap to the canonical
 * library later if/when we need full LSP coverage (rename, code
 * actions, etc.).
 *
 * Semantic tokens: the server (sysml-lsp-server) advertises a 15-type
 * + 6-modifier legend and emits LSP-standard semantic-token frames via
 * `textDocument/semanticTokens/full`. VS Code and Zed already consume
 * this automatically through their LSP clients. This module wires
 * Monaco's `registerDocumentSemanticTokensProvider` so the simulation
 * app gets the same spec-aware colouring (kind-classified
 * PartUsage / StateUsage / RequirementDef etc. rather than the
 * regex-only Monarch fallback).
 *
 * Lifecycle: `attachLspClient(editor, monaco, { uri, lspUrl })` opens
 * the socket, waits for `initialized`, then `didOpen`s the current
 * buffer. The returned disposable closes the socket and unregisters
 * the providers. Caller owns lifecycle (typically a React `useEffect`).
 */

import type { editor as MonacoEditor, IDisposable, languages, Position } from 'monaco-editor';
import { tauriStreamWsBase } from '../../shared/api/transport';

type Monaco = typeof import('monaco-editor');

interface AttachOptions {
  /** Document URI string. LSP uses this as the canonical identifier. */
  uri: string;
  /**
   * WebSocket URL for the `/lsp` endpoint. Caller usually computes this
   * from `VITE_API_BASE_URL` (HTTP) replaced to `ws`. Pass-through so
   * tests can stub a fake server.
   */
  lspUrl: string;
  /** Monaco language id to associate with the model (defaults to `sysml`). */
  languageId?: string;
  /**
   * Called once the server responds to `initialize` and the buffer has
   * been opened. Useful for tests / waveforms-style readiness signals.
   */
  onReady?: () => void;
  /** Diagnostic-marker owner key. Defaults to `sysml-lsp`. */
  markersOwner?: string;
}

interface LspClient extends IDisposable {
  /** Number of pending in-flight requests. Tests use this for readiness. */
  pendingRequests: () => number;
  /** True once `initialize` has resolved. */
  ready: () => boolean;
}

/** Severity mapping from LSP → Monaco. */
const SEVERITY_MAP: Record<number, number> = {
  1: 8, // Error
  2: 4, // Warning
  3: 2, // Info
  4: 1, // Hint
};

interface LspRange {
  start: { line: number; character: number };
  end: { line: number; character: number };
}

interface LspDiagnostic {
  range: LspRange;
  severity?: number;
  code?: string | number;
  source?: string;
  message: string;
}

/**
 * Semantic-token legend as the server publishes it in the
 * `initialize` response under
 * `capabilities.semanticTokensProvider.legend`. The integer-array
 * payloads emitted by `textDocument/semanticTokens/full` index into
 * these two lists, so the legend is required *before* we can
 * register a Monaco provider — Monaco demands the legend synchronously
 * from `getLegend()`.
 */
interface LspSemanticTokensLegend {
  tokenTypes: string[];
  tokenModifiers: string[];
}

interface LspSemanticTokensResult {
  resultId?: string;
  data: number[];
}

interface JsonRpcRequest {
  jsonrpc: '2.0';
  id?: number | string;
  method: string;
  params?: unknown;
}

interface JsonRpcResponse {
  jsonrpc: '2.0';
  id?: number | string;
  result?: unknown;
  error?: { code: number; message: string };
  method?: string;
  params?: unknown;
}

/**
 * Open a WebSocket to the given URL, drive the LSP handshake, and wire
 * Monaco providers. Returns a disposable that closes the socket and
 * detaches every provider/listener.
 */
export function attachLspClient(
  editor: MonacoEditor.IStandaloneCodeEditor,
  monaco: Monaco,
  opts: AttachOptions,
): LspClient {
  const { uri: rawUri, lspUrl, languageId = 'sysml', onReady, markersOwner = 'sysml-lsp' } = opts;

  // tower-lsp deserialises `textDocument.uri` into `url::Url`, which
  // rejects raw paths (`/home/...`) with `"relative URL without a
  // base"`. Every request that flows through `textDocument` — open,
  // didChange, hover, completion, definition, semanticTokens — has to
  // carry a `file://` URI or the server errors before reaching the
  // handler. We normalise once at attach time so the rest of the file
  // can pass `uri` around verbatim.
  const uri = rawUri.startsWith('file://')
    ? rawUri
    : `file://${rawUri.startsWith('/') ? '' : '/'}${rawUri}`;

  const ws = new WebSocket(lspUrl);
  let nextId = 1;
  let documentVersion = 1;
  let initialized = false;
  const pending = new Map<number, (resp: JsonRpcResponse) => void>();
  const disposables: IDisposable[] = [];

  // Capture the editor's actual model URI at attach time. @monaco-editor/react
  // creates an in-memory model with a URI like `inmemory://model/1` — the
  // file path we pass to the LSP (`uri`) never matches `model.uri.toString()`
  // verbatim. We still need to filter out provider calls for *other* models
  // on the same language (e.g. SneakPeek), so we compare against the URI of
  // the model the editor was holding at attach time.
  const attachedModelUri = editor.getModel()?.uri.toString() ?? null;
  const isOurModel = (modelUri: string): boolean =>
    attachedModelUri !== null && modelUri === attachedModelUri;

  const send = (msg: JsonRpcRequest) => {
    if (ws.readyState === WebSocket.OPEN) {
      ws.send(JSON.stringify(msg));
    }
  };

  const request = <T,>(method: string, params?: unknown): Promise<T> =>
    new Promise<T>((resolve, reject) => {
      const id = nextId++;
      pending.set(id, (resp) => {
        if (resp.error) reject(new Error(resp.error.message));
        else resolve(resp.result as T);
      });
      send({ jsonrpc: '2.0', id, method, params });
    });

  const notify = (method: string, params?: unknown) =>
    send({ jsonrpc: '2.0', method, params });

  // ── LSP → Monaco diagnostics ───────────────────────────────────────
  const applyDiagnostics = (diags: LspDiagnostic[]) => {
    const model = editor.getModel();
    if (!model) return;
    const markers: MonacoEditor.IMarkerData[] = diags.map((d) => ({
      severity: SEVERITY_MAP[d.severity ?? 1] ?? 8,
      message: d.message,
      source: d.source,
      code: d.code !== undefined ? String(d.code) : undefined,
      startLineNumber: d.range.start.line + 1,
      startColumn: d.range.start.character + 1,
      endLineNumber: d.range.end.line + 1,
      endColumn: d.range.end.character + 1,
    }));
    monaco.editor.setModelMarkers(model, markersOwner, markers);
  };

  // ── Hover / completion providers ──────────────────────────────────
  const positionToLsp = (position: Position) => ({
    line: position.lineNumber - 1,
    character: position.column - 1,
  });

  disposables.push(
    monaco.languages.registerHoverProvider(languageId, {
      provideHover: async (model, position) => {
        if (!initialized || !isOurModel(model.uri.toString())) return null;
        try {
          const result = await request<{
            contents?: string | { value?: string } | Array<{ value?: string } | string>;
            range?: LspRange;
          } | null>('textDocument/hover', {
            textDocument: { uri },
            position: positionToLsp(position),
          });
          if (!result || !result.contents) return null;
          const contents = Array.isArray(result.contents) ? result.contents : [result.contents];
          const value = contents
            .map((c) => (typeof c === 'string' ? c : c?.value ?? ''))
            .filter(Boolean)
            .join('\n\n');
          if (!value) return null;
          const r = result.range;
          return {
            contents: [{ value }],
            range: r
              ? {
                  startLineNumber: r.start.line + 1,
                  startColumn: r.start.character + 1,
                  endLineNumber: r.end.line + 1,
                  endColumn: r.end.character + 1,
                }
              : undefined,
          };
        } catch {
          return null;
        }
      },
    }),
  );

  disposables.push(
    monaco.languages.registerCompletionItemProvider(languageId, {
      triggerCharacters: ['.', ':', ' '],
      provideCompletionItems: async (model, position) => {
        if (!initialized || !isOurModel(model.uri.toString())) {
          return { suggestions: [] };
        }
        const word = model.getWordUntilPosition(position);
        const range = {
          startLineNumber: position.lineNumber,
          startColumn: word.startColumn,
          endLineNumber: position.lineNumber,
          endColumn: word.endColumn,
        };
        try {
          const result = await request<
            | {
                items?: Array<{ label: string; kind?: number; detail?: string; insertText?: string }>;
              }
            | Array<{ label: string; kind?: number; detail?: string; insertText?: string }>
            | null
          >('textDocument/completion', {
            textDocument: { uri },
            position: positionToLsp(position),
          });
          if (!result) return { suggestions: [] };
          const items = Array.isArray(result) ? result : result.items ?? [];
          const suggestions: languages.CompletionItem[] = items.map((it) => ({
            label: it.label,
            kind: (it.kind as languages.CompletionItemKind) ?? monaco.languages.CompletionItemKind.Text,
            detail: it.detail,
            insertText: it.insertText ?? it.label,
            range,
          }));
          return { suggestions };
        } catch {
          return { suggestions: [] };
        }
      },
    }),
  );

  disposables.push(
    monaco.languages.registerDefinitionProvider(languageId, {
      provideDefinition: async (model, position) => {
        if (!initialized || !isOurModel(model.uri.toString())) return null;
        try {
          const result = await request<
            | { uri: string; range: LspRange }
            | Array<{ uri: string; range: LspRange; targetUri?: string; targetRange?: LspRange }>
            | null
          >('textDocument/definition', {
            textDocument: { uri },
            position: positionToLsp(position),
          });
          if (!result) return null;
          const arr = Array.isArray(result) ? result : [result];
          return arr.map((loc) => {
            const targetUri = (loc as { targetUri?: string }).targetUri ?? (loc as { uri: string }).uri;
            const targetRange = (loc as { targetRange?: LspRange }).targetRange ?? (loc as { range: LspRange }).range;
            return {
              uri: monaco.Uri.parse(targetUri),
              range: {
                startLineNumber: targetRange.start.line + 1,
                startColumn: targetRange.start.character + 1,
                endLineNumber: targetRange.end.line + 1,
                endColumn: targetRange.end.character + 1,
              },
            };
          });
        } catch {
          return null;
        }
      },
    }),
  );

  // ── Monaco → LSP didChange ────────────────────────────────────────
  const changeListener = editor.onDidChangeModelContent(() => {
    if (!initialized) return;
    const model = editor.getModel();
    if (!model || !isOurModel(model.uri.toString())) return;
    documentVersion += 1;
    notify('textDocument/didChange', {
      textDocument: { uri, version: documentVersion },
      contentChanges: [{ text: model.getValue() }],
    });
  });
  disposables.push(changeListener);

  // ── Socket lifecycle ──────────────────────────────────────────────
  ws.onmessage = (event) => {
    let msg: JsonRpcResponse;
    try {
      msg = JSON.parse(event.data as string);
    } catch {
      return;
    }
    if (typeof msg.id === 'number' && pending.has(msg.id)) {
      const cb = pending.get(msg.id)!;
      pending.delete(msg.id);
      cb(msg);
      return;
    }
    if (msg.method === 'textDocument/publishDiagnostics') {
      const params = msg.params as { uri: string; diagnostics: LspDiagnostic[] };
      // Bug E fix: the server normalises whatever URI we send in
      // `didOpen` to a `file://` scheme before publishing. Our `uri`
      // is the raw file path; strict equality always misses. Compare
      // tolerant: accept either form by stripping `file://` from both
      // sides before checking. Equivalent to `uri === uri`-mod-scheme.
      const normalise = (u: string) =>
        u.startsWith('file://') ? u.slice('file://'.length) : u;
      if (normalise(params.uri) === normalise(uri)) {
        applyDiagnostics(params.diagnostics ?? []);
      }
    }
  };

  ws.onopen = () => {
    void (async () => {
      try {
        const initResult = await request<{
          capabilities?: {
            semanticTokensProvider?:
              | { legend?: LspSemanticTokensLegend }
              | { legend?: LspSemanticTokensLegend; full?: unknown }
              | null;
          };
        }>('initialize', {
          processId: null,
          rootUri: null,
          capabilities: {
            textDocument: {
              hover: { contentFormat: ['markdown', 'plaintext'] },
              completion: { completionItem: { snippetSupport: false } },
              definition: { linkSupport: true },
              publishDiagnostics: {},
              // Advertise that we can render semantic tokens so the
              // server actually emits the legend + full frames.
              semanticTokens: {
                requests: { range: false, full: true },
                tokenTypes: [],
                tokenModifiers: [],
                formats: ['relative'],
              },
            },
          },
        });
        notify('initialized', {});
        initialized = true;

        // Register the semantic-tokens provider once we know the legend.
        // Monaco fetches the legend synchronously from `getLegend()`, so
        // we can't register before initialize resolves. The provider
        // forwards every request straight to the server — the wire
        // format is identical between LSP and Monaco.
        const legend = initResult?.capabilities?.semanticTokensProvider?.legend;
        if (legend && legend.tokenTypes && legend.tokenModifiers) {
          disposables.push(
            monaco.languages.registerDocumentSemanticTokensProvider(languageId, {
              getLegend: () => legend,
              provideDocumentSemanticTokens: async (model) => {
                if (!isOurModel(model.uri.toString())) return null;
                try {
                  const result = await request<LspSemanticTokensResult | null>(
                    'textDocument/semanticTokens/full',
                    { textDocument: { uri } },
                  );
                  if (!result || !result.data) return null;
                  return {
                    resultId: result.resultId,
                    data: new Uint32Array(result.data),
                  };
                } catch {
                  return null;
                }
              },
              releaseDocumentSemanticTokens: () => {
                // No client-side cache to release; the server owns
                // resultId-based deltas (not wired yet — full only).
              },
            }),
          );
        }

        const model = editor.getModel();
        notify('textDocument/didOpen', {
          textDocument: {
            uri,
            languageId,
            version: documentVersion,
            text: model?.getValue() ?? '',
          },
        });
        onReady?.();
      } catch {
        // Initialize failed — leave initialized=false; providers no-op.
      }
    })();
  };

  ws.onerror = () => {
    // Surface as a no-op; downstream features fall back to read-only.
  };

  let disposed = false;
  const dispose = () => {
    if (disposed) return;
    disposed = true;
    for (const d of disposables) d.dispose();
    if (initialized && ws.readyState === WebSocket.OPEN) {
      notify('textDocument/didClose', { textDocument: { uri } });
    }
    pending.clear();
    if (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING) {
      ws.close();
    }
    const model = editor.getModel();
    if (model) monaco.editor.setModelMarkers(model, markersOwner, []);
  };

  return {
    dispose,
    pendingRequests: () => pending.size,
    ready: () => initialized,
  };
}

/**
 * Build the WebSocket URL for `/lsp` from the same VITE_API_BASE_URL
 * the REST client uses. Empty base (default in dev) returns
 * `ws://<host>/lsp` so Vite's `ws: true` proxy entry can pick it up.
 *
 * In the Tauri desktop shell there is no Vite proxy, so the socket targets
 * the in-process axum sidecar (`tauriStreamWsBase()`) instead.
 */
export function lspWebSocketUrl(): string {
  const tauriBase = tauriStreamWsBase();
  if (tauriBase) return `${tauriBase}/lsp`;

  const env =
    typeof import.meta !== 'undefined'
      ? ((import.meta as unknown as { env?: Record<string, string> }).env)
      : undefined;
  const base = env?.VITE_API_BASE_URL ?? '';
  if (!base) {
    const proto = typeof window !== 'undefined' && window.location.protocol === 'https:' ? 'wss' : 'ws';
    const host = typeof window !== 'undefined' ? window.location.host : 'localhost:3010';
    return `${proto}://${host}/lsp`;
  }
  return `${base.replace(/^http/, 'ws')}/lsp`;
}
