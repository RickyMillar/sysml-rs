//! Document formatting — thin shim over `sysml.format.document`.
//!
//! All whitespace edit computation lives in `sysml_service::formatting`. The
//! LSP layer just converts `lsp_types::FormattingOptions` to `FormatOptions`,
//! dispatches, and reshapes the shared service `TextEdit` into `lsp_types::TextEdit`.

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

use tower_lsp::lsp_types::{FormattingOptions, TextEdit};

use crate::service_edits::to_lsp_text_edit;
use sysml_service::SysmlService;

/// Format a document, returning text edits that only modify whitespace.
///
/// Returns an empty vec when the document isn't loaded into the service host
/// or when the tree-sitter parse fails (no nesting context to drive
/// indentation).
pub(crate) fn format_document(
    service: &SysmlService,
    uri: &str,
    options: &FormattingOptions,
) -> Vec<TextEdit> {
    let result = service.format_document(uri, Some(options.tab_size), Some(options.insert_spaces));
    let edits = match result {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };
    edits.into_iter().map(to_lsp_text_edit).collect()
}
