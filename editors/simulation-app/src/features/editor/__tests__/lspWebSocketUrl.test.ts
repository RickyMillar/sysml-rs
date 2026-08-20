/**
 * Unit tests for `lspWebSocketUrl()` — the `/lsp` WebSocket URL builder.
 *
 * The browser path is exercised against jsdom's default `window.location`
 * (host `localhost`); the Tauri path is exercised by toggling the
 * `__TAURI_INTERNALS__` global the desktop shell injects.
 */
import { describe, it, expect } from 'vitest';
import { lspWebSocketUrl } from '../lspClient';

describe('lspWebSocketUrl', () => {
  it('builds a host-relative ws:// url in the browser (Vite proxy path)', () => {
    // jsdom default origin is http://localhost → ws://localhost/lsp.
    const url = lspWebSocketUrl();
    expect(url.startsWith('ws://')).toBe(true);
    expect(url.endsWith('/lsp')).toBe(true);
  });

  it('targets the axum sidecar in Tauri mode (no proxy)', () => {
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = { invoke: () => Promise.resolve() };
    try {
      expect(lspWebSocketUrl()).toBe('ws://localhost:8081/lsp');
    } finally {
      delete w.__TAURI_INTERNALS__;
    }
  });
});
