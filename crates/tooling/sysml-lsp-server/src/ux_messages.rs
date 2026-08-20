//! User-facing client message helpers.
//!
//! Policy:
//! - Use these helpers only for user-visible status or actionable warnings/errors.
//! - Do not use this module for operational telemetry; emit telemetry via `tracing::*`.

// LSP server: tower-lsp patterns use unwrap/expect for client sends,
// indexing is bounds-checked by protocol invariants, Arc cloning is intentional.
#![allow(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::manual_let_else,
    clippy::arc_with_non_send_sync,
    clippy::clone_on_ref_ptr,
    clippy::map_err_ignore,
    clippy::needless_pass_by_value,
    clippy::panic
)]

use tower_lsp::lsp_types::MessageType;
use tower_lsp::Client;

/// Emit an informational user-facing message.
pub(crate) async fn info(client: &Client, message: impl Into<String>) {
    let message = message.into();
    tracing::info!(ux_message = %message, "client info");
    client.log_message(MessageType::INFO, message).await;
}

/// Emit a warning user-facing message.
pub(crate) async fn warn(client: &Client, message: impl Into<String>) {
    let message = message.into();
    tracing::warn!(ux_message = %message, "client warning");
    client.log_message(MessageType::WARNING, message).await;
}

/// Show a user-visible info popup (more prominent than log_message).
pub(crate) async fn show_info(client: &Client, message: impl Into<String>) {
    let message = message.into();
    tracing::info!(ux_show_message = %message, "client show info");
    client.show_message(MessageType::INFO, message).await;
}

/// Show a user-visible warning popup (more prominent than log_message).
pub(crate) async fn show_warn(client: &Client, message: impl Into<String>) {
    let message = message.into();
    tracing::warn!(ux_show_message = %message, "client show warning");
    client.show_message(MessageType::WARNING, message).await;
}

/// Show a user-visible error popup (more prominent than log_message).
pub(crate) async fn show_error(client: &Client, message: impl Into<String>) {
    let message = message.into();
    tracing::error!(ux_show_message = %message, "client show error");
    client.show_message(MessageType::ERROR, message).await;
}
