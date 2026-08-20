//! WebSocket-to-LSP bridge.
//!
//! Each WebSocket connection on `/lsp` gets its own `tower-lsp`
//! `LspService`, but all share the host process's `Arc<SysmlService>`
//! (and therefore the same salsa `AnalysisHost`). That means LSP
//! `did_change` notifications land in the same store the REST
//! `sysml.*` commands read from — the FE doesn't have to dual-write
//! source edits.
//!
//! The bridge translates between WebSocket text frames (raw JSON-RPC)
//! and the LSP base protocol (`Content-Length: N\r\n\r\n{json}`).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::response::IntoResponse;
use futures::stream::StreamExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower_lsp::Server;

use crate::AppState;

/// Axum handler that upgrades an HTTP request to a WebSocket and
/// spawns the LSP bridge sharing the host's `SysmlService`.
pub async fn lsp_ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    let service = state.service.clone();
    ws.on_upgrade(move |socket| handle_lsp_socket(socket, service))
}

/// Core bridge: splits the WebSocket, creates duplex pipes, and
/// spawns three concurrent tasks (tower-lsp server, WS->LSP writer,
/// LSP->WS reader).
///
/// All three tasks are owned by a `JoinSet`; if the bridge future is
/// dropped before completion (client goes away, request cancelled),
/// `JoinSet::drop` aborts any still-running handles so we don't leak
/// orphan tower-lsp instances or duplex pipe pumps.
async fn handle_lsp_socket(socket: WebSocket, sysml_service: Arc<sysml_service::SysmlService>) {
    let (mut ws_sink, mut ws_stream) = socket.split();

    // Duplex pair 1: tower-lsp reads from server_read; WS bridge writes to client_write
    let (server_read, mut client_write) = tokio::io::duplex(8192);
    // Duplex pair 2: tower-lsp writes to server_write; WS bridge reads from client_read
    let (mut client_read, server_write) = tokio::io::duplex(8192);

    // Reuse the host process's SysmlService so this connection's salsa
    // store is the same one the REST sysml.* commands read from.
    let (service, socket) = sysml_lsp_server::create_service_with(sysml_service);

    let mut tasks: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

    // Task 1: tower-lsp server — runs until the pipes close.
    tasks.spawn(async move {
        Server::new(server_read, server_write, socket)
            .serve(service)
            .await;
    });

    // Task 2: WS -> LSP — read WebSocket text frames, wrap with
    // Content-Length header, and write into the pipe that tower-lsp reads.
    tasks.spawn(async move {
        while let Some(Ok(msg)) = ws_stream.next().await {
            match msg {
                Message::Text(text) => {
                    let body = text.as_bytes();
                    let header = format!("Content-Length: {}\r\n\r\n", body.len());
                    if client_write.write_all(header.as_bytes()).await.is_err() {
                        break;
                    }
                    if client_write.write_all(body).await.is_err() {
                        break;
                    }
                    if client_write.flush().await.is_err() {
                        break;
                    }
                }
                Message::Close(_) => break,
                // Ignore binary/ping/pong frames.
                _ => {}
            }
        }
        // Dropping client_write closes the pipe, signaling EOF to tower-lsp.
        drop(client_write);
    });

    // Task 3: LSP -> WS — read Content-Length-framed messages from the
    // pipe that tower-lsp writes to, then send the JSON body as a
    // WebSocket text frame.
    tasks.spawn(async move {
        use futures::SinkExt;

        let mut header_buf = Vec::with_capacity(128);

        loop {
            // Read the header section byte-by-byte until we find \r\n\r\n.
            header_buf.clear();
            let Some(content_length) =
                read_lsp_header(&mut client_read, &mut header_buf).await
            else {
                break; // EOF or malformed — server shut down.
            };

            // Read exactly `content_length` bytes of body.
            let mut body = vec![0u8; content_length];
            if client_read.read_exact(&mut body).await.is_err() {
                break;
            }

            let Ok(text) = String::from_utf8(body) else {
                break;
            };

            if ws_sink.send(Message::Text(text)).await.is_err() {
                break;
            }
        }

        let _ = ws_sink.close().await;
    });

    // Drive all tasks to completion. `JoinSet` will abort any stragglers
    // if this future is dropped before `join_next` returns `None`.
    while let Some(_res) = tasks.join_next().await {}
}

/// Parse LSP base protocol headers from an `AsyncRead`, returning the
/// `Content-Length` value. Returns `None` on EOF or parse failure.
async fn read_lsp_header<R: tokio::io::AsyncRead + Unpin>(
    reader: &mut R,
    buf: &mut Vec<u8>,
) -> Option<usize> {
    // Headers end with \r\n\r\n. Read byte-by-byte (headers are small).
    loop {
        let mut byte = [0u8; 1];
        if reader.read_exact(&mut byte).await.is_err() {
            return None;
        }
        buf.push(byte[0]);

        // Check for the \r\n\r\n terminator.
        if buf.len() >= 4 && &buf[buf.len() - 4..] == b"\r\n\r\n" {
            break;
        }
    }

    // Parse headers (there may be multiple, but we only need Content-Length).
    let header_str = std::str::from_utf8(buf).ok()?;
    for line in header_str.split("\r\n") {
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            return value.trim().parse::<usize>().ok();
        }
        // Case-insensitive fallback.
        if let Some(value) = line.strip_prefix("content-length: ") {
            return value.trim().parse::<usize>().ok();
        }
    }

    None
}
