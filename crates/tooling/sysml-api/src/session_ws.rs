//! Session-event WebSocket stream.
//!
//! Endpoint: `GET /api/sessions/:id/events` (WebSocket upgrade).
//!
//! Serves typed [`SessionFrame`]s over the socket:
//! one `Hello` at connect, then a `Tick` per new orchestrator tick,
//! plus `Verdict` / `Completed` / `Error` frames as applicable. See
//!
//! ### Frame format (Stage 4 + Stage 6)
//!
//! Negotiated via the `Sec-WebSocket-Protocol` header:
//!
//! * `sysml-session-v1-json` — JSON text frames (default; Stage 4).
//! * `sysml-session-v1-cbor` — CBOR binary frames (Stage 6).
//!
//! The client offers both; the server picks one. Same `SessionFrame`
//! enum, same external-tag contract — only the outer encoding changes.
//!
//! ### Implementation note — push-based snapshot stream
//!
//! Stage 4 polled the service at [`DEFAULT_POLL_INTERVAL_MS`]. Phase E
//! replaced that with a broadcast channel wired into `Orchestrator::step`
//! via [`SysmlService::subscribe_session_snapshots`]: snapshots flow
//! into a per-session `tokio::sync::broadcast::Sender` and the WS loop
//! `select!`s on the receiver, so there's no poll-interval latency and
//! no wasted peeks when the simulation is paused. When the session
//! predates the broadcast subscription (or the API is called against an
//! older service build), the handler falls back to the original peek
//! loop. Slow subscribers receive `RecvError::Lagged(N)` and recover
//! via a fresh `sessions_info` resync.
//!
//! ### S2.T14 — shaping moved to the service
//!
//! Frame shaping (`normalize`, `verdict_rollup_from_constraints`,
//! `diff`) used to live inline here. As of S2.T14 it lives behind
//! [`sysml_service::session_events::SessionEventShaper`]; this module
//! is the WS transport — socket lifecycle, JSON vs CBOR encoding,
//! and the broadcast/poll switch — and nothing more.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::response::IntoResponse;
use futures::{SinkExt, StreamExt};
use serde::Deserialize;

use sysml_runtime::session_events::{error_codes, SessionFrame};
use sysml_service::session_events::{SessionEventOpen, SessionEventPoll, SessionEventShaper};

use crate::AppState;

/// How often the WS handler peeks the session's tick counter. 30 Hz —
/// matches the plan's "≥30 Hz sustained" target. The peek is a single
/// integer read (`session_pulse`); we only clone the full snapshot
/// when tick has advanced, so faster polling is effectively free.
pub const DEFAULT_POLL_INTERVAL_MS: u64 = 33;

/// Query string for `/api/sessions/:id/events`.
#[derive(Debug, Deserialize, Default)]
pub struct SessionWsQuery {
    /// Optional `?since=<tick>` cursor for reconnect resume. Stage 4
    /// accepts the value but always replies with a fresh `Hello`; the
    /// bounded replay ring lands in a later stage. Kept on the wire so
    /// clients can be written against the final contract today.
    #[serde(default)]
    #[allow(dead_code)]
    pub since: Option<u64>,
}

/// Subprotocol / frame-format identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FrameFormat {
    Json,
    Cbor,
}

/// Subprotocols advertised to the client, in order of server preference.
const SUPPORTED_PROTOCOLS: [&str; 2] =
    ["sysml-session-v1-cbor", "sysml-session-v1-json"];

#[allow(dead_code)]
const PROTO_JSON: &str = "sysml-session-v1-json";
const PROTO_CBOR: &str = "sysml-session-v1-cbor";

fn frame_format_from_protocol(selected: Option<&str>) -> FrameFormat {
    match selected {
        Some(PROTO_CBOR) => FrameFormat::Cbor,
        _ => FrameFormat::Json,
    }
}

/// Axum handler. Upgrades the HTTP request to a WebSocket and spawns
/// the per-session stream loop.
pub async fn session_ws_handler(
    ws: WebSocketUpgrade,
    Path(session_id): Path<String>,
    Query(params): Query<SessionWsQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.protocols(SUPPORTED_PROTOCOLS)
        .on_upgrade(move |socket| run(socket, session_id, params, state))
}

async fn run(
    socket: WebSocket,
    session_id: String,
    _params: SessionWsQuery,
    state: Arc<AppState>,
) {
    let format = frame_format_from_protocol(
        socket
            .protocol()
            .and_then(|h| h.to_str().ok()),
    );
    let (mut tx, mut rx) = socket.split();

    // Scratch buffer reused across CBOR frame sends for the lifetime of
    // this connection. Idle when the negotiated format is JSON.
    let mut cbor_buf: Vec<u8> = if matches!(format, FrameFormat::Cbor) {
        Vec::with_capacity(1024)
    } else {
        Vec::new()
    };

    // Hello (and optional already-Completed) frames come from the
    // service-side shaper. Drain them in order; on terminal `shaper =
    // None` we close right after.
    let SessionEventOpen { frames, shaper } =
        SessionEventShaper::open(&state.service, &session_id);
    for frame in frames {
        if send_frame(&mut tx, &mut cbor_buf, &frame, format).await.is_err() {
            return;
        }
    }
    let Some(mut shaper) = shaper else {
        let _ = tx.close().await;
        return;
    };

    // Prefer the broadcast subscription (Phase E push path). Fall back to
    // the 33 ms peek loop for the rare case where the session predates
    // the observer (no live receiver handle available).
    if let Some(mut snapshot_rx) = state.service.subscribe_session_snapshots(&session_id) {
        loop {
            tokio::select! {
                recv = snapshot_rx.recv() => {
                    match recv {
                        Ok(snapshot) => {
                            let frames = shaper.on_snapshot(snapshot.as_ref());
                            if forward_frames(&mut tx, &mut cbor_buf, &frames, format)
                                .await
                                .is_err_or_terminal()
                            {
                                return;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                            // Client (or scheduler) fell behind the bounded
                            // fan-out buffer. Resync from the latest
                            // snapshot via the same code path the poll loop
                            // uses — this way the client sees a clean diff
                            // against the shaper's `prev` rather than a
                            // partial delta.
                            if handle_poll(&state, &session_id, &mut shaper, &mut tx, &mut cbor_buf, format).await {
                                return;
                            }
                        }
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                            // Session dropped (stopped / reaped). Report
                            // the same terminal code as the poll path.
                            send_error(&mut tx, &mut cbor_buf, error_codes::SESSION_CLOSED, "session was reaped", format).await;
                            let _ = tx.close().await;
                            return;
                        }
                    }
                }
                client_msg = rx.next() => {
                    match client_msg {
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Err(_)) => break,
                        Some(Ok(_)) => {}
                    }
                }
            }
        }
    } else {
        // Legacy poll fallback. Kept intact so a mismatched deployment
        // (new frontend / old backend) still works while rollout catches up.
        let mut ticker =
            tokio::time::interval(Duration::from_millis(DEFAULT_POLL_INTERVAL_MS));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    if handle_poll(&state, &session_id, &mut shaper, &mut tx, &mut cbor_buf, format).await {
                        return;
                    }
                }
                client_msg = rx.next() => {
                    match client_msg {
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Err(_)) => break,
                        Some(Ok(_)) => {}
                    }
                }
            }
        }
    }

    let _ = tx.close().await;
}

/// Drive one fallback poll through the service-side shaper and send
/// any resulting frames. Returns `true` when the connection has hit a
/// terminal state and the caller must `return`.
async fn handle_poll<S>(
    state: &AppState,
    session_id: &str,
    shaper: &mut SessionEventShaper,
    tx: &mut S,
    cbor_buf: &mut Vec<u8>,
    format: FrameFormat,
) -> bool
where
    S: SinkExt<Message> + Unpin,
{
    match shaper.poll(&state.service, session_id) {
        SessionEventPoll::Frames(frames) => {
            forward_frames(tx, cbor_buf, &frames, format)
                .await
                .is_err_or_terminal()
        }
        SessionEventPoll::NoChange => false,
        SessionEventPoll::SessionGone => {
            send_error(tx, cbor_buf, error_codes::SESSION_CLOSED, "session was reaped", format).await;
            let _ = tx.close().await;
            true
        }
        SessionEventPoll::Failed(msg) => {
            send_error(tx, cbor_buf, error_codes::PROJECTION_FAILED, msg, format).await;
            let _ = tx.close().await;
            true
        }
    }
}

/// Result of forwarding a slice of frames to the socket. Distinguishes
/// "send error → drop the connection" from "Completed frame → close
/// cleanly" so the call site can collapse both into a single `return`
/// guard via [`ForwardOutcome::is_err_or_terminal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ForwardOutcome {
    Continue,
    Completed,
    SendError,
}

impl ForwardOutcome {
    fn is_err_or_terminal(self) -> bool {
        !matches!(self, ForwardOutcome::Continue)
    }
}

async fn forward_frames<S>(
    tx: &mut S,
    cbor_buf: &mut Vec<u8>,
    frames: &[SessionFrame],
    format: FrameFormat,
) -> ForwardOutcome
where
    S: SinkExt<Message> + Unpin,
{
    for frame in frames {
        let completed = matches!(frame, SessionFrame::Completed { .. });
        if send_frame(tx, cbor_buf, frame, format).await.is_err() {
            return ForwardOutcome::SendError;
        }
        if completed {
            let _ = tx.close().await;
            return ForwardOutcome::Completed;
        }
    }
    ForwardOutcome::Continue
}

async fn send_frame<S>(
    tx: &mut S,
    cbor_buf: &mut Vec<u8>,
    frame: &SessionFrame,
    format: FrameFormat,
) -> Result<(), ()>
where
    S: SinkExt<Message> + Unpin,
{
    let message = match format {
        FrameFormat::Json => {
            let Ok(json) = serde_json::to_string(frame) else {
                return Err(());
            };
            Message::Text(json)
        }
        FrameFormat::Cbor => {
            // Reuse the per-connection scratch buffer for ciborium
            // serialization. `Message::Binary` requires an owned
            // `Vec<u8>` in axum 0.7 so we still have to clone the
            // freshly-written bytes out. The win over the previous
            // `Vec::with_capacity(256)`-per-frame path is that the
            // scratch buffer amortizes capacity growth across the
            // connection — once it's grown to the peak frame size it
            // never reallocates again, and each hand-off clone is
            // sized exactly to the payload (no grow-phase overhead).
            cbor_buf.clear();
            if ciborium::ser::into_writer(frame, &mut *cbor_buf).is_err() {
                return Err(());
            }
            Message::Binary(cbor_buf.clone())
        }
    };
    tx.send(message).await.map_err(|_| ())
}

async fn send_error<S>(
    tx: &mut S,
    cbor_buf: &mut Vec<u8>,
    code: &str,
    message: impl Into<String>,
    format: FrameFormat,
) where
    S: SinkExt<Message> + Unpin,
{
    let frame = SessionFrame::error(code, message);
    let _ = send_frame(tx, cbor_buf, &frame, format).await;
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use sysml_runtime::snapshot_diff::diff;
    use sysml_runtime::snapshot_view::NormalizedSnapshot;

    fn make_snapshot(tick: u64, time_ms: f64, scalars: &[(&str, f64)]) -> NormalizedSnapshot {
        let mut s = NormalizedSnapshot {
            tick,
            time_ms,
            ..Default::default()
        };
        for (k, v) in scalars {
            s.scalar_vars.insert((*k).into(), *v);
        }
        s
    }

    #[test]
    fn hello_frame_serializes_with_schema_version() {
        let snap = make_snapshot(0, 0.0, &[("x", 1.0)]);
        let frame = SessionFrame::hello("sess-a", snap);
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["type"], "hello");
        assert_eq!(
            json["schema_version"],
            sysml_runtime::session_events::SCHEMA_VERSION,
        );
        assert_eq!(json["session_id"], "sess-a");
    }

    #[test]
    fn tick_frame_carries_only_changes() {
        let a = make_snapshot(1, 100.0, &[("x", 1.0), ("y", 2.0)]);
        let mut b = a.clone();
        b.tick = 2;
        b.time_ms = 200.0;
        b.scalar_vars.insert("y".into(), 3.0);
        let delta = diff(Some(&a), &b);
        let frame = SessionFrame::tick(delta);
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["type"], "tick");
        assert_eq!(json["delta"]["tick"], 2);
        assert_eq!(json["delta"]["scalar_changed"]["y"].as_f64().unwrap(), 3.0);
        assert!(json["delta"]["scalar_changed"].get("x").is_none());
    }

    #[test]
    fn error_frame_uses_stable_code() {
        let frame = SessionFrame::error(error_codes::NOT_FOUND, "x");
        let json = serde_json::to_value(&frame).unwrap();
        assert_eq!(json["type"], "error");
        assert_eq!(json["code"], "not_found");
    }

    #[test]
    fn frame_format_defaults_to_json_when_no_protocol_selected() {
        assert_eq!(frame_format_from_protocol(None), FrameFormat::Json);
        assert_eq!(frame_format_from_protocol(Some("something-else")), FrameFormat::Json);
    }

    #[test]
    fn frame_format_picks_cbor_when_selected() {
        assert_eq!(
            frame_format_from_protocol(Some(PROTO_CBOR)),
            FrameFormat::Cbor,
        );
        assert_eq!(
            frame_format_from_protocol(Some(PROTO_JSON)),
            FrameFormat::Json,
        );
    }

    #[test]
    fn cbor_round_trips_every_frame_variant() {
        // Every variant encodes + decodes identically under ciborium.
        let variants = vec![
            SessionFrame::hello("s", make_snapshot(0, 0.0, &[("x", 1.0)])),
            SessionFrame::tick({
                let a = make_snapshot(0, 0.0, &[("x", 1.0)]);
                let mut b = a.clone();
                b.tick = 1;
                b.time_ms = 10.0;
                b.scalar_vars.insert("x".into(), 2.0);
                diff(Some(&a), &b)
            }),
            SessionFrame::verdict(
                5,
                sysml_runtime::aggregates::VerdictRollup {
                    pass: 2,
                    fail: 1,
                    inconclusive: 0,
                    error: 0,
                },
            ),
            SessionFrame::completed(9, 900.0),
            SessionFrame::error(error_codes::NOT_FOUND, "gone"),
        ];
        for original in variants {
            let mut buf = Vec::<u8>::new();
            ciborium::ser::into_writer(&original, &mut buf).unwrap();
            let decoded: SessionFrame =
                ciborium::de::from_reader(buf.as_slice()).unwrap();
            assert_eq!(decoded, original);
        }
    }

    #[test]
    fn cbor_payload_smaller_than_json_for_realistic_hello() {
        // Not a contract test (CBOR always is smaller here), but catches
        // regressions in the frame shape that would accidentally inflate
        // the wire payload.
        let mut snap = make_snapshot(0, 0.0, &[]);
        for i in 0..200 {
            snap.scalar_vars.insert(format!("var_{i}"), i as f64);
        }
        let frame = SessionFrame::hello("sess", snap);

        let json_bytes = serde_json::to_vec(&frame).unwrap();
        let mut cbor_bytes = Vec::<u8>::new();
        ciborium::ser::into_writer(&frame, &mut cbor_bytes).unwrap();

        assert!(
            cbor_bytes.len() < json_bytes.len(),
            "expected CBOR ({}) < JSON ({}) for a 200-variable hello",
            cbor_bytes.len(),
            json_bytes.len(),
        );
    }
}
