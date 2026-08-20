/**
 * useSessionStream — opens the session-events WebSocket behind the
 * `VITE_STREAM_V1=1` feature flag, parses JSON frames, and applies them
 * to `sessionLiveStore`.
 *
 * and implemented in `crates/tooling/sysml-api/src/session_ws.rs`.
 *
 * Behaviour:
 *   - mount with a session id → opens `ws://<api>/api/sessions/<id>/events`
 *   - receives one `Hello` frame, then `Tick` / `Verdict` / `Completed` frames
 *   - on socket close that wasn't a Completed, reconnects with `?since=<lastTick>`
 *   - on repeated failures (no frames within `RECONNECT_DEADLINE_MS`), stops
 *     trying and sets `phase = "error"` so the caller can fall back to polling
 *   - when the feature flag is unset, the hook is a no-op (returns `phase = "idle"`)
 *
 * Frame coalescing (live-perf F1): a live run streams ticks far faster
 * than the screen can paint (~2000/sec on a fast-forward run vs ~60fps).
 * `ws.onmessage` no longer applies each frame synchronously — it pushes
 * the decoded frame onto `pendingFramesRef` and schedules one
 * `requestAnimationFrame` flush (a no-op if one is already pending).
 * `flushPending` then drains the whole backlog through the *existing*
 * `dispatchFrame` reducer, in order, inside that single rAF callback.
 * Because every downstream consumer (`sessionLiveStore` for the
 * tree/variables pane, `useTimeSeriesStore` via `useLiveTimeSeriesBridge`
 * for the ring buffer) is updated synchronously within that one
 * callback, React's automatic batching (React 18+, in effect since this
 * app renders via `createRoot`) coalesces all of the resulting state
 * updates into a single re-render — capping the *paint* rate at ~60Hz
 * without dropping any tick: every buffered frame is still folded into
 * the store (and every tick still lands in the time-series ring via the
 * bridge's per-snapshot subscription), just within one batched flush
 * instead of one synchronous reaction per WS message. See
 * `flushPending` (declared per-connection inside the hook) and the
 * module-level `scheduleFlush` / `cancelScheduledFlush` helpers below.
 * A final flush is guaranteed on unmount/reconnect/terminal-frame so
 * buffered-but-not-yet-painted ticks are never silently dropped.
 *
 * Tests live in `useSessionStream.test.ts`. The frame-dispatch logic is
 * exposed as `dispatchFrame` so tests can drive it without mocking the
 * WebSocket global.
 */
import { useEffect, useRef } from 'react';
import { decode as cborDecode } from 'cbor-x';
import { tauriStreamWsBase } from '../../shared/api/transport';
import {
  useSessionLiveStore,
  type DeltaFrame,
  type NormalizedSnapshot,
  type VerdictRollup,
  type StreamPhase,
} from './sessionLiveStore';

// ── Feature flag + config ───────────────────────────────────────────

/** Wire format negotiated via the Sec-WebSocket-Protocol header. */
export type StreamMode = 'json' | 'cbor';

/**
 * The currently-enabled stream mode, or `null` when streaming is off.
 *
 * `VITE_STREAM_V1` values:
 *   - `1`    → JSON (the default; matches Stage 5)
 *   - `json` → JSON
 *   - `cbor` → CBOR binary frames (Stage 6)
 *   - `0` / `off` → streaming disabled, polling owns the path
 *   - unset → JSON (Phase E — broadcast channel is live; default-on)
 *
 * The default flipped from "off" to "json" after Phase E landed the
 * broadcast channel: without a live stream, the Plots picker and
 * time-series bridge stay empty even though the backend is producing
 * snapshots at 10 Hz. Opt-out with `VITE_STREAM_V1=0`.
 */
export function streamMode(): StreamMode | null {
  const env =
    (typeof import.meta !== 'undefined' &&
      (import.meta as Record<string, any>).env) ||
    {};
  const v = env.VITE_STREAM_V1;
  if (v === 'cbor') return 'cbor';
  if (v === '0' || v === 0 || v === 'off') return null;
  // Unset or any truthy value: JSON streaming.
  return 'json';
}

/** Subprotocol string for a given mode — matches the Rust server table. */
export function subprotocolFor(mode: StreamMode): string {
  return mode === 'cbor' ? 'sysml-session-v1-cbor' : 'sysml-session-v1-json';
}

/** Backward-compatible check used by mounted hooks. */
export function isStreamV1Enabled(): boolean {
  return streamMode() !== null;
}

/** Initial retry delay (doubled each failure, capped at `RECONNECT_MAX_MS`). */
export const RECONNECT_BASE_MS = 500;
export const RECONNECT_MAX_MS = 10_000;
/** If no frames arrive within this window we stop retrying. */
export const RECONNECT_DEADLINE_MS = 60_000;

// ── Frame coalescing (live-perf F1) ─────────────────────────────────

/**
 * Schedule `cb` on the next paint. Uses `requestAnimationFrame` in the
 * browser; falls back to a ~60Hz `setTimeout` in non-browser test/SSR
 * environments where `requestAnimationFrame` may be unavailable.
 */
function scheduleFlush(cb: () => void): number {
  if (typeof requestAnimationFrame === 'function') {
    return requestAnimationFrame(cb);
  }
  return setTimeout(cb, 16) as unknown as number;
}

/** Cancel a handle returned by `scheduleFlush`. */
function cancelScheduledFlush(handle: number): void {
  if (typeof cancelAnimationFrame === 'function') {
    cancelAnimationFrame(handle);
  } else {
    clearTimeout(handle);
  }
}

// ── Frame wire types (match `SessionFrame` in Rust) ─────────────────

export type SessionFrame =
  | { type: 'hello'; schema_version: string; session_id: string; tick: number; time_ms: number; base: NormalizedSnapshot }
  | { type: 'tick'; delta: DeltaFrame }
  | { type: 'verdict'; tick: number; verdicts: VerdictRollup }
  | { type: 'completed'; tick: number; time_ms: number }
  | { type: 'error'; code: string; message: string };

// ── URL builder ─────────────────────────────────────────────────────

/**
 * Derive the WebSocket URL for a given session id.
 *
 * Uses `VITE_API_BASE_URL` when set, else same-origin. Converts
 * `http:` → `ws:` / `https:` → `wss:` automatically.
 */
export function sessionStreamUrl(
  sessionId: string,
  opts: { sinceTick?: number | null; baseUrl?: string } = {},
): string {
  const envBase =
    (typeof import.meta !== 'undefined' &&
      ((import.meta as Record<string, any>).env?.VITE_API_BASE_URL as
        | string
        | undefined)) ||
    '';
  let base =
    opts.baseUrl ??
    // In the Tauri desktop shell there is no Vite proxy — target the
    // in-process axum sidecar instead of window.location.
    tauriStreamWsBase() ??
    (envBase ||
      (typeof window !== 'undefined' ? window.location.origin : ''));

  if (base.startsWith('http://')) base = 'ws://' + base.slice(7);
  else if (base.startsWith('https://')) base = 'wss://' + base.slice(8);
  else if (!base.startsWith('ws://') && !base.startsWith('wss://')) {
    base = 'ws://' + base.replace(/^\/+/, '');
  }

  const path = `/api/sessions/${encodeURIComponent(sessionId)}/events`;
  const qs =
    typeof opts.sinceTick === 'number'
      ? `?since=${opts.sinceTick}`
      : '';
  return `${base.replace(/\/+$/, '')}${path}${qs}`;
}

// ── Frame decode (shared by text + binary paths) ────────────────────

/**
 * Decode a WS payload into a `SessionFrame`. `string` payloads are
 * JSON; `ArrayBuffer` / `Uint8Array` payloads are CBOR. Returns
 * `null` on a malformed frame — callers ignore those.
 */
export function decodeFrame(data: unknown): SessionFrame | null {
  try {
    if (typeof data === 'string') {
      return JSON.parse(data) as SessionFrame;
    }
    // `instanceof` fails across realms (vitest + cbor-x cross their
    // ArrayBuffer globals), so we use realm-safe checks: `ArrayBuffer.isView`
    // for TypedArray / DataView / Node Buffer, and `[object ArrayBuffer]`
    // for raw ArrayBuffers.
    if (ArrayBuffer.isView(data)) {
      const view = data as ArrayBufferView;
      const bytes = new Uint8Array(view.buffer, view.byteOffset, view.byteLength);
      return cborDecode(bytes) as SessionFrame;
    }
    if (
      data != null &&
      Object.prototype.toString.call(data) === '[object ArrayBuffer]'
    ) {
      return cborDecode(new Uint8Array(data as ArrayBuffer)) as SessionFrame;
    }
    return null;
  } catch {
    return null;
  }
}

// ── Frame dispatch (pure; testable without a real WS) ───────────────

/**
 * Apply a single frame to the live store. Returns `true` when the server
 * signalled the end of the stream (`completed` / `error`), so the caller
 * can close the socket instead of reconnecting.
 */
export function dispatchFrame(
  frame: SessionFrame,
  actions: Pick<
    ReturnType<typeof useSessionLiveStore.getState>,
    'applyHello' | 'applyTick' | 'applyVerdict' | 'markCompleted' | 'setError' | 'setPhase'
  >,
): { terminal: boolean } {
  switch (frame.type) {
    case 'hello':
      actions.applyHello(frame.session_id, frame.base);
      return { terminal: false };
    case 'tick':
      actions.applyTick(frame.delta);
      return { terminal: false };
    case 'verdict':
      actions.applyVerdict(frame.tick, frame.verdicts);
      return { terminal: false };
    case 'completed':
      actions.markCompleted(frame.tick, frame.time_ms);
      actions.setPhase('closed');
      return { terminal: true };
    case 'error':
      actions.setError(`${frame.code}: ${frame.message}`);
      actions.setPhase('error');
      return { terminal: true };
  }
}

// ── Single-owner guard (ADR-004 / ninebar Phase 1) ──────────────────

/**
 * The stream has exactly one owner: `SessionStreamProvider`, mounted at
 * the layout gate. Two live mounts of this hook mean two WebSockets
 * racing writes into one `sessionLiveStore` (interleaved deltas, double
 * resets). Counted per hook mount — not per connection — so the guard
 * fires even when both mounts share a session id.
 */
let activeStreamOwners = 0;

function useSingleOwnerGuard(): void {
  useEffect(() => {
    activeStreamOwners += 1;
    if (activeStreamOwners > 1 && (import.meta as Record<string, any>).env?.DEV) {
      // eslint-disable-next-line no-console
      console.error(
        `useSessionStream has ${activeStreamOwners} concurrent mounts — ` +
          'the stream must have exactly one owner (SessionStreamProvider). ' +
          'A second mount races two sockets against sessionLiveStore.',
      );
    }
    return () => {
      activeStreamOwners -= 1;
    };
  }, []);
}

// ── Hook ────────────────────────────────────────────────────────────

export interface UseSessionStreamResult {
  phase: StreamPhase;
  error: string | null;
  /** Whether the feature flag + connection attempt are active. */
  enabled: boolean;
}

/**
 * Opens a WebSocket against the session-events endpoint and keeps
 * `sessionLiveStore` in sync. Pass `null` to disable; pass a session id
 * to (re)connect when `VITE_STREAM_V1=1`.
 *
 * The hook reconnects with `?since=<lastTick>` on unexpected close,
 * using exponential backoff capped at `RECONNECT_MAX_MS`. It gives up
 * after `RECONNECT_DEADLINE_MS` without receiving any frames so the
 * caller can fall back to polling.
 */
export function useSessionStream(sessionId: string | null): UseSessionStreamResult {
  useSingleOwnerGuard();
  const mode = streamMode();
  const enabled = mode !== null && !!sessionId;
  const phase = useSessionLiveStore((s) => s.phase);
  const error = useSessionLiveStore((s) => s.lastError);

  // Stable refs for reconnect bookkeeping — we don't want renders to
  // cancel the in-flight socket.
  const socketRef = useRef<WebSocket | null>(null);
  const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const connectDeadlineRef = useRef<number | null>(null);
  const backoffRef = useRef(RECONNECT_BASE_MS);

  // F1 coalescing: frames decoded off the wire land here first; a single
  // rAF-scheduled flush drains the whole backlog. `rafHandleRef` guards
  // against scheduling more than one flush per frame.
  const pendingFramesRef = useRef<SessionFrame[]>([]);
  const rafHandleRef = useRef<number | null>(null);
  /** Latest connection's flush fn, so cleanup can force a final drain. */
  const pendingFlushRef = useRef<(() => void) | null>(null);

  useEffect(() => {
    if (!enabled) {
      useSessionLiveStore.getState().reset();
      return;
    }

    let cancelled = false;
    backoffRef.current = RECONNECT_BASE_MS;
    connectDeadlineRef.current = Date.now() + RECONNECT_DEADLINE_MS;
    pendingFramesRef.current = [];

    const connect = () => {
      if (cancelled) return;
      const store = useSessionLiveStore.getState();
      store.setPhase('connecting');

      const url = sessionStreamUrl(sessionId!, {
        sinceTick: store.lastTick ?? undefined,
      });

      let ws: WebSocket;
      try {
        ws = new WebSocket(url, [subprotocolFor(mode!)]);
      } catch (e) {
        store.setError(`websocket open failed: ${String(e)}`);
        store.setPhase('error');
        return;
      }
      // CBOR frames arrive as ArrayBuffer — declare it here so Safari
      // and Chromium hand us the expected payload shape in onmessage.
      ws.binaryType = 'arraybuffer';
      socketRef.current = ws;

      // Drain every frame buffered since the last flush through the
      // existing `dispatchFrame` reducer, in arrival order, inside one
      // synchronous burst — React batches the resulting store updates
      // into a single re-render (see the module-level doc comment).
      // Safe to call from the rAF callback, from `onclose` (to catch
      // frames that arrived just before an unexpected drop), and from
      // the effect cleanup (final flush on unmount/session switch).
      const flushPending = () => {
        const frames = pendingFramesRef.current;
        if (frames.length === 0) return;
        pendingFramesRef.current = [];
        for (const frame of frames) {
          const storeNow = useSessionLiveStore.getState();
          const { terminal } = dispatchFrame(frame, storeNow);
          if (terminal) {
            cancelled = true;
            if (ws.readyState <= WebSocket.OPEN) ws.close();
            break; // frames after a terminal frame would be spurious
          }
        }
      };

      ws.onopen = () => {
        if (cancelled) return;
        useSessionLiveStore.getState().setPhase('open');
      };

      ws.onmessage = (evt) => {
        if (cancelled) return;
        const frame = decodeFrame(evt.data);
        if (!frame) return;
        // Reset the deadline + backoff immediately on arrival (not
        // gated on the deferred flush) — the frame itself is the
        // liveness signal, whether or not it's been painted yet.
        connectDeadlineRef.current = Date.now() + RECONNECT_DEADLINE_MS;
        backoffRef.current = RECONNECT_BASE_MS;
        pendingFramesRef.current.push(frame);
        if (rafHandleRef.current == null) {
          rafHandleRef.current = scheduleFlush(() => {
            rafHandleRef.current = null;
            flushPending();
          });
        }
      };

      ws.onerror = () => {
        // Browsers don't give us detail — leave the message to onclose.
      };

      ws.onclose = () => {
        // An unexpected close shouldn't silently drop ticks that arrived
        // but hadn't been flushed to a paint yet — drain them first.
        if (rafHandleRef.current != null) {
          cancelScheduledFlush(rafHandleRef.current);
          rafHandleRef.current = null;
        }
        flushPending();
        if (cancelled) return;
        // Exponential backoff reconnect, bounded by the global deadline.
        const now = Date.now();
        if (connectDeadlineRef.current && now > connectDeadlineRef.current) {
          useSessionLiveStore.getState().setError(
            'session stream unavailable; falling back to polling',
          );
          useSessionLiveStore.getState().setPhase('error');
          return;
        }
        const delay = Math.min(backoffRef.current, RECONNECT_MAX_MS);
        backoffRef.current = Math.min(
          backoffRef.current * 2,
          RECONNECT_MAX_MS,
        );
        useSessionLiveStore.getState().setPhase('closed');
        reconnectTimerRef.current = setTimeout(connect, delay);
      };

      // Stash the flush so the effect cleanup (below, out of this
      // connection's closure) can force a final drain on unmount.
      pendingFlushRef.current = flushPending;
    };

    connect();

    return () => {
      cancelled = true;
      if (rafHandleRef.current != null) {
        cancelScheduledFlush(rafHandleRef.current);
        rafHandleRef.current = null;
      }
      // Final flush: apply any frames that arrived but hadn't been
      // painted yet so the last ticks before unmount/session-switch
      // aren't silently lost.
      pendingFlushRef.current?.();
      pendingFlushRef.current = null;
      if (reconnectTimerRef.current) {
        clearTimeout(reconnectTimerRef.current);
        reconnectTimerRef.current = null;
      }
      const sock = socketRef.current;
      socketRef.current = null;
      if (sock && sock.readyState <= WebSocket.OPEN) {
        sock.close();
      }
      useSessionLiveStore.getState().reset();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [enabled, sessionId]);

  return { phase, error, enabled };
}
