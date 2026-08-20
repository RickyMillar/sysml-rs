/**
 * Unit tests for useSessionStream pure helpers.
 *
 * The React lifecycle / reconnect loop is covered by e2e — here we
 * exercise `dispatchFrame` + `sessionStreamUrl` directly so the frame
 * reducer and URL builder are correct without mocking WebSocket.
 */
import { describe, it, expect, beforeEach } from 'vitest';
import { encode as cborEncode } from 'cbor-x';
import {
  decodeFrame,
  dispatchFrame,
  sessionStreamUrl,
  subprotocolFor,
  type SessionFrame,
} from './useSessionStream';
import {
  useSessionLiveStore,
  type NormalizedSnapshot,
} from './sessionLiveStore';

function helloFrame(): SessionFrame {
  const base: NormalizedSnapshot = {
    tick: 0,
    time_ms: 0,
    completed: false,
    subsystems: {
      sm1: { current_state: 'Idle', completed: false, kind_label: 'stateMachine' },
    },
    scalar_vars: { x: 1 },
    string_vars: {},
    constraint_results: [],
  };
  return {
    type: 'hello',
    schema_version: 'sysml-session-v1',
    session_id: 'sess-a',
    tick: 0,
    time_ms: 0,
    base,
  };
}

describe('dispatchFrame', () => {
  beforeEach(() => {
    useSessionLiveStore.getState().reset();
  });

  it('hello frame populates the store and is non-terminal', () => {
    const actions = useSessionLiveStore.getState();
    const result = dispatchFrame(helloFrame(), actions);
    expect(result.terminal).toBe(false);
    const s = useSessionLiveStore.getState();
    expect(s.sessionId).toBe('sess-a');
    expect(s.snapshot?.scalar_vars.x).toBe(1);
  });

  it('tick frame applies delta on top of the hello base', () => {
    const actions = useSessionLiveStore.getState();
    dispatchFrame(helloFrame(), actions);
    dispatchFrame(
      {
        type: 'tick',
        delta: { tick: 1, time_ms: 10, completed: false, scalar_changed: { x: 2 } },
      },
      useSessionLiveStore.getState(),
    );
    expect(useSessionLiveStore.getState().snapshot?.scalar_vars.x).toBe(2);
    expect(useSessionLiveStore.getState().lastTick).toBe(1);
  });

  it('verdict frame updates the rollup without touching the snapshot', () => {
    const actions = useSessionLiveStore.getState();
    dispatchFrame(helloFrame(), actions);
    dispatchFrame(
      { type: 'verdict', tick: 1, verdicts: { pass: 3, fail: 1, inconclusive: 0, error: 0 } },
      useSessionLiveStore.getState(),
    );
    const s = useSessionLiveStore.getState();
    expect(s.verdicts).toEqual({ pass: 3, fail: 1, inconclusive: 0, error: 0 });
    expect(s.snapshot?.scalar_vars.x).toBe(1);
  });

  it('completed frame marks terminal + sets phase=closed', () => {
    const actions = useSessionLiveStore.getState();
    dispatchFrame(helloFrame(), actions);
    const result = dispatchFrame(
      { type: 'completed', tick: 5, time_ms: 500 },
      useSessionLiveStore.getState(),
    );
    expect(result.terminal).toBe(true);
    const s = useSessionLiveStore.getState();
    expect(s.snapshot?.completed).toBe(true);
    expect(s.phase).toBe('closed');
  });

  it('error frame is terminal + surfaces a code:message string', () => {
    const actions = useSessionLiveStore.getState();
    const result = dispatchFrame(
      { type: 'error', code: 'not_found', message: 'gone' },
      actions,
    );
    expect(result.terminal).toBe(true);
    const s = useSessionLiveStore.getState();
    expect(s.phase).toBe('error');
    expect(s.lastError).toBe('not_found: gone');
  });
});

describe('sessionStreamUrl', () => {
  it('converts http base to ws', () => {
    const url = sessionStreamUrl('sess-1', { baseUrl: 'http://localhost:8080' });
    expect(url).toBe('ws://localhost:8080/api/sessions/sess-1/events');
  });

  it('converts https base to wss', () => {
    const url = sessionStreamUrl('sess-1', { baseUrl: 'https://api.example.com' });
    expect(url).toBe('wss://api.example.com/api/sessions/sess-1/events');
  });

  it('appends ?since when provided', () => {
    const url = sessionStreamUrl('s', { baseUrl: 'ws://x', sinceTick: 42 });
    expect(url).toBe('ws://x/api/sessions/s/events?since=42');
  });

  it('url-encodes session ids with special characters', () => {
    const url = sessionStreamUrl('sess/with spaces', { baseUrl: 'ws://x' });
    expect(url).toContain('/api/sessions/sess%2Fwith%20spaces/events');
  });

  it('targets the axum sidecar in Tauri mode (no proxy)', () => {
    // Simulate the desktop shell: __TAURI_INTERNALS__ present. With no
    // explicit baseUrl, the builder must point at the sidecar loopback
    // port rather than window.location (there is no Vite proxy in Tauri).
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = { invoke: () => Promise.resolve() };
    try {
      const url = sessionStreamUrl('sess-1');
      expect(url).toBe('ws://localhost:8081/api/sessions/sess-1/events');
    } finally {
      delete w.__TAURI_INTERNALS__;
    }
  });

  it('explicit baseUrl still wins over Tauri detection', () => {
    const w = window as unknown as Record<string, unknown>;
    w.__TAURI_INTERNALS__ = { invoke: () => Promise.resolve() };
    try {
      const url = sessionStreamUrl('sess-1', { baseUrl: 'http://localhost:8080' });
      expect(url).toBe('ws://localhost:8080/api/sessions/sess-1/events');
    } finally {
      delete w.__TAURI_INTERNALS__;
    }
  });
});

describe('subprotocolFor', () => {
  it('maps json ↔ sysml-session-v1-json', () => {
    expect(subprotocolFor('json')).toBe('sysml-session-v1-json');
  });
  it('maps cbor ↔ sysml-session-v1-cbor', () => {
    expect(subprotocolFor('cbor')).toBe('sysml-session-v1-cbor');
  });
});

describe('decodeFrame', () => {
  const jsonFrame: SessionFrame = {
    type: 'completed',
    tick: 42,
    time_ms: 4200,
  };

  it('parses JSON text payloads', () => {
    const decoded = decodeFrame(JSON.stringify(jsonFrame));
    expect(decoded).toEqual(jsonFrame);
  });

  it('parses CBOR ArrayBuffer payloads', () => {
    const bytes = cborEncode(jsonFrame);
    // cbor-x returns a Buffer/Uint8Array; hand decodeFrame an ArrayBuffer.
    const ab = bytes.buffer.slice(
      bytes.byteOffset,
      bytes.byteOffset + bytes.byteLength,
    );
    const decoded = decodeFrame(ab);
    expect(decoded).toEqual(jsonFrame);
  });

  it('parses CBOR Uint8Array payloads', () => {
    const decoded = decodeFrame(cborEncode(jsonFrame));
    expect(decoded).toEqual(jsonFrame);
  });

  it('returns null for malformed JSON', () => {
    expect(decodeFrame('{not-json')).toBeNull();
  });

  it('returns null for non-string non-binary payloads', () => {
    expect(decodeFrame(42 as unknown)).toBeNull();
    expect(decodeFrame(null)).toBeNull();
  });
});
