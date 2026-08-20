/**
 * Tauri invoke transport.
 *
 * Active when `window.__TAURI_INTERNALS__` is present (Tauri v2 context with
 * `withGlobalTauri: true` in tauri.conf.json).  Selected by `transport.ts`
 * at module init — do not import this module directly from feature code.
 *
 * Routing rules (steward-approved, 2026-06-30):
 *   POST /api/command  →  invoke('sysml_command', { command, params })
 *   Everything else    →  fetch() against the axum sidecar on TAURI_SIDECAR_PORT
 *
 * The axum sidecar handles WebSocket session events (/api/sessions/:id/events),
 * SSE progress (/v1/progress), and the Monaco LSP WebSocket (/lsp) — channels
 * that Tauri invoke cannot serve.
 */

import { ApiError } from './rest';

/** Must match `SIDECAR_PORT` in `src-tauri/src/lib.rs`. */
export const TAURI_SIDECAR_PORT = 8081;

type TauriInternals = {
  invoke: <T>(cmd: string, args?: Record<string, unknown>) => Promise<T>;
};

function getInternals(): TauriInternals {
  const internals = (window as unknown as Record<string, unknown>)
    .__TAURI_INTERNALS__ as TauriInternals | undefined;
  if (!internals || typeof internals.invoke !== 'function') {
    throw new Error(
      'TauriTransport: __TAURI_INTERNALS__.invoke unavailable. ' +
        'Ensure withGlobalTauri is set in tauri.conf.json.',
    );
  }
  return internals;
}

/** Build a full sidecar URL for non-command requests. */
function sidecarUrl(path: string): string {
  return `http://localhost:${TAURI_SIDECAR_PORT}${path}`;
}

async function sidecarFetch(url: string, options?: RequestInit): Promise<Response> {
  const res = await fetch(url, options);
  if (!res.ok) {
    let msg: string;
    try {
      const body = (await res.json()) as Record<string, unknown>;
      msg = typeof body.error === 'string' ? body.error : res.statusText;
    } catch {
      msg = res.statusText;
    }
    // Reuse ApiError so consumers that check instanceof still work.
    throw new ApiError(res.status, msg, url);
  }
  return res;
}

export function createTauriTransport() {
  const internals = getInternals();

  return {
    async post<T>(path: string, body?: unknown): Promise<T> {
      if (path === '/api/command' && body !== null && typeof body === 'object') {
        const req = body as { command?: unknown; params?: unknown };
        if (typeof req.command === 'string') {
          return internals.invoke<T>('sysml_command', {
            command: req.command,
            params: req.params ?? null,
          });
        }
      }
      // Non-command POSTs (e.g. /sources, /files, /sessions/*/start) go to the sidecar.
      const res = await sidecarFetch(sidecarUrl(path), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: body !== undefined ? JSON.stringify(body) : undefined,
      });
      return res.json() as Promise<T>;
    },

    async get<T>(path: string): Promise<T> {
      const res = await sidecarFetch(sidecarUrl(path));
      return res.json() as Promise<T>;
    },

    async postText(path: string, body?: unknown): Promise<string> {
      const res = await sidecarFetch(sidecarUrl(path), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: body !== undefined ? JSON.stringify(body) : undefined,
      });
      return res.text();
    },

    async del(path: string): Promise<void> {
      await sidecarFetch(sidecarUrl(path), { method: 'DELETE' });
    },
  };
}
