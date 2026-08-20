//! Background task coordination for the LSP server.
//!
//! This module provides infrastructure for cross-file element entries and
//! resolution tier tracking.
//!
//! ## Resolution Tiers
//!
//! The LSP uses a tiered approach to resolution:
//!
//! - **T1 (Syntax)**: < 50ms - Highlighting, outline, syntax errors
//!   Runs synchronously on every edit.
//!
//! - **T2 (Local)**: < 200ms - Same-file go-to-def, completion
//!   Runs after a short debounce.
//!
//! - **T3 (Full)**: Background - Cross-file, library types, validation
//!   Runs when the user is idle.

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

use sysml_core::ElementKind;
use sysml_id::ElementId;

/// Resolution tier.
// Vestigial production type — only exercised by tests today (the standalone
// diagnostic pipeline that used it now lives under `#[cfg(test)]`, RSC-6.6).
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ResolutionTier {
    /// Syntax only - runs synchronously.
    T1Syntax,
    /// Local resolution - same-file definitions.
    T2Local,
}

/// Entry representing a cross-file element with location information.
///
/// Used by [`WorkspaceSnapshot`](crate::workspace_snapshot::WorkspaceSnapshot)
/// to track element definitions across workspace files.
#[derive(Debug, Clone)]
pub struct CrossFileEntry {
    /// File URI where the element is defined.
    pub uri: String,
    /// Unique element identifier.
    pub element_id: ElementId,
    /// The kind of element (Package, PartDefinition, etc.).
    pub element_kind: ElementKind,
    /// Byte offset of element start in the source file.
    pub span_start: usize,
    /// Byte offset of element end in the source file.
    pub span_end: usize,
}
