//! SSE endpoint that forwards [`ProgressEvent`]s to HTTP clients.
//!
//! `GET /v1/progress` subscribes to [`SysmlService::subscribe_progress`]
//! and streams each [`ProgressEvent`] as an [`axum::response::sse::Event`].
//!
//! The SSE `event` name is the [`ProgressEvent`] discriminant
//! (`library_load`, `workspace_index`, `dependency_fetch`, `refresh`,
//! `ready`), matching the `tag = "kind"` rename used in the service-layer
//! serde derive. The `data` payload is the full event as JSON.
//!
//! Lagged subscribers receive a synthetic `lagged` event with `data` =
//! the dropped-event count, then keep streaming. The publisher is
//! non-blocking (capacity 256 on the underlying broadcast channel; see
//! `sysml-service/src/progress.rs`).
//!
//! Wired into the REST router via `/v1/progress` in
//! [`crate::create_router`].

use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
};
use futures::stream::Stream;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};
use tokio_stream::StreamExt;

use sysml_service::progress::ProgressEvent;

use crate::AppState;

/// Discriminant string for an event, mirroring `#[serde(tag = "kind", rename_all = "snake_case")]`.
fn event_kind(ev: &ProgressEvent) -> &'static str {
    match ev {
        ProgressEvent::LibraryLoad { .. } => "library_load",
        ProgressEvent::WorkspaceIndex { .. } => "workspace_index",
        ProgressEvent::DependencyFetch { .. } => "dependency_fetch",
        ProgressEvent::Refresh { .. } => "refresh",
        ProgressEvent::Ready => "ready",
    }
}

/// Axum handler for `GET /v1/progress`. Subscribes to the service's
/// progress bus and returns an SSE stream that the client keeps open.
pub async fn progress_sse_handler(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.service.subscribe_progress();
    let stream = BroadcastStream::new(rx).map(|item| {
        let event = match item {
            Ok(ev) => {
                let kind = event_kind(&ev);
                // serde_json::to_value on ProgressEvent is infallible
                // (plain derive(Serialize)). If it ever fails, fall
                // through to a minimal placeholder so we don't drop the
                // event silently.
                let data = serde_json::to_string(&ev)
                    .unwrap_or_else(|_| String::from("{\"kind\":\"unknown\"}"));
                Event::default().event(kind).data(data)
            }
            Err(BroadcastStreamRecvError::Lagged(n)) => {
                // Tell the client they missed N events; keep streaming.
                Event::default()
                    .event("lagged")
                    .data(n.to_string())
            }
        };
        Ok::<_, Infallible>(event)
    });

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}
