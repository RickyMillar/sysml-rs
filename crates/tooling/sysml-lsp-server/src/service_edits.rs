//! ONE home for reshaping the service's shared [`sysml_service::text_edit::TextEdit`]
//! into an LSP [`lsp_types::TextEdit`]. Rename, formatting, and code actions all
//! route here — the three per-feature copies were collapsed with the service-side
//! struct collapse (workbench design §7.2).

use tower_lsp::lsp_types::{Position, Range, TextEdit};

pub(crate) fn to_lsp_text_edit(edit: sysml_service::text_edit::TextEdit) -> TextEdit {
    TextEdit {
        range: Range {
            start: Position {
                line: edit.line_start,
                character: edit.col_start,
            },
            end: Position {
                line: edit.line_end,
                character: edit.col_end,
            },
        },
        new_text: edit.new_text,
    }
}
