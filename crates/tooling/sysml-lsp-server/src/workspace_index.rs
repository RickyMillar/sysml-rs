//! Workspace index: workspace root tracking.

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

use std::sync::Arc;

use tokio::sync::RwLock;

/// Owns workspace root folders.
pub struct WorkspaceIndex {
    /// Workspace root folders (set on initialize).
    pub workspace_roots: Arc<RwLock<Vec<String>>>,
}

impl WorkspaceIndex {
    pub fn new() -> Self {
        Self {
            workspace_roots: Arc::new(RwLock::new(Vec::new())),
        }
    }
}
