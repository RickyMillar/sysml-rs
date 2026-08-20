/**
 * Active transport singleton for all backend communication.
 *
 * Detection runs once at module init:
 *   - Tauri context (`window.__TAURI_INTERNALS__` present) → TauriTransport
 *   - Browser / Vite dev-server context                    → RestTransport
 *
 * Fail-hard contract (steward, 2026-06-30):
 *   If __TAURI_INTERNALS__ is present but `invoke` is missing, we throw
 *   immediately — we never silently fall back to fetch() in Tauri mode.
 *
 * All feature code should import httpGet / httpPost / etc. from this module
 * (or the backward-compat re-export in `./http`).  Do not import from
 * `./rest` or `./tauri-transport` directly.
 */

import {
  ApiError,
  httpGetRest,
  httpPostRest,
  httpPostTextRest,
  httpDeleteRest,
} from './rest';
import { createTauriTransport, TAURI_SIDECAR_PORT } from './tauri-transport';

export { ApiError, TAURI_SIDECAR_PORT };

// ── Transport interface ──────────────────────────────────────────────

export interface Transport {
  get<T>(path: string): Promise<T>;
  post<T>(path: string, body?: unknown): Promise<T>;
  postText(path: string, body?: unknown): Promise<string>;
  del(path: string): Promise<void>;
}

// ── Detection + singleton ────────────────────────────────────────────

function isTauriContext(): boolean {
  return (
    typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window
  );
}

/** True when running inside the Tauri desktop shell. */
export function isTauri(): boolean {
  return isTauriContext();
}

/**
 * WebSocket base origin for streaming transports (LSP, session events) when
 * running in Tauri.
 *
 * The desktop shell loads the webview from a custom protocol
 * (`tauri://localhost`) with no Vite dev-proxy, so relative `/lsp` and
 * `/api/...` socket URLs have nothing to forward them. In Tauri mode these
 * sockets must target the in-process axum sidecar on the fixed loopback port
 * (see `SIDECAR_PORT` in `src-tauri/src/lib.rs`).
 *
 * Returns `ws://localhost:<port>` in the desktop shell, or `null` in the
 * browser — browser callers keep their existing `window.location` logic.
 */
export function tauriStreamWsBase(): string | null {
  return isTauriContext() ? `ws://localhost:${TAURI_SIDECAR_PORT}` : null;
}

function selectTransport(): Transport {
  if (isTauriContext()) {
    // Fail hard if the IPC bridge is absent — no silent REST fallback.
    const internals = (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    if (
      !internals ||
      typeof (internals as Record<string, unknown>).invoke !== 'function'
    ) {
      throw new Error(
        'Tauri context detected (__TAURI_INTERNALS__) but the IPC bridge is ' +
          'unavailable (invoke is not a function). ' +
          'Check withGlobalTauri in tauri.conf.json.',
      );
    }
    return createTauriTransport();
  }
  return {
    get: httpGetRest,
    post: httpPostRest,
    postText: httpPostTextRest,
    del: httpDeleteRest,
  };
}

const _active: Transport = selectTransport();

// ── Public API (same surface as the former http.ts) ──────────────────

export async function httpGet<T>(path: string): Promise<T> {
  return _active.get<T>(path);
}

export async function httpPost<T>(path: string, body?: unknown): Promise<T> {
  return _active.post<T>(path, body);
}

export async function httpPostText(path: string, body?: unknown): Promise<string> {
  return _active.postText(path, body);
}

export async function httpDelete(path: string): Promise<void> {
  return _active.del(path);
}
