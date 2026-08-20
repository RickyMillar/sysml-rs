//! Folding range and selection range handlers.

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

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{FoldingRangeParams, FoldingRange, FoldingRangeKind, SelectionRangeParams, SelectionRange, Range};

use crate::syntax_context::collect_comment_folds;
use crate::types::SYNTHETIC_FILE;
use crate::utils::{offset_to_position, position_to_offset, range_to_lsp_range};
use crate::SysmlLanguageServer;

pub(crate) async fn folding_range(
    server: &SysmlLanguageServer,
    params: FoldingRangeParams,
) -> Result<Option<Vec<FoldingRange>>> {
    let uri = params.text_document.uri.to_string();

    let Some(doc) = server.salsa_parsed_doc(&uri).await else {
        return Ok(None);
    };

    let mut ranges = Vec::new();
    for element in doc.graph.elements.values() {
        if let Some(span) = element.spans.first() {
            if span.file != uri || span.file == SYNTHETIC_FILE {
                continue;
            }
            let start_pos = offset_to_position(span.start, &doc.content);
            let end_pos = offset_to_position(span.end, &doc.content);
            if end_pos.line > start_pos.line {
                ranges.push(FoldingRange {
                    start_line: start_pos.line,
                    start_character: Some(start_pos.character),
                    end_line: end_pos.line,
                    end_character: Some(end_pos.character),
                    kind: Some(FoldingRangeKind::Region),
                    collapsed_text: element.name.clone(),
                });
            }
        }
    }

    // Also fold comments from tree-sitter CST (via salsa)
    if let Some(cached_tree) = server.salsa_tree(&uri).await {
        for fold in collect_comment_folds(cached_tree.tree()) {
            ranges.push(FoldingRange {
                start_line: fold.start.row as u32,
                start_character: Some(fold.start.column as u32),
                end_line: fold.end.row as u32,
                end_character: Some(fold.end.column as u32),
                kind: Some(FoldingRangeKind::Comment),
                collapsed_text: None,
            });
        }
    }

    Ok(Some(ranges))
}

pub(crate) async fn selection_range(
    server: &SysmlLanguageServer,
    params: SelectionRangeParams,
) -> Result<Option<Vec<SelectionRange>>> {
    let uri = params.text_document.uri.to_string();

    let Some(doc) = server.salsa_doc(&uri).await else {
        return Ok(None);
    };

    let mut result = Vec::new();

    for position in &params.positions {
        let offset = position_to_offset(position, &doc.content);

        // Build selection ranges from innermost to outermost
        let mut chain: Vec<Range> = Vec::new();

        // Innermost: element at cursor
        if let Some(element_id) = doc.position_map.element_id_at(offset) {
            if let Some(element) = doc.graph.get_element(&element_id) {
                if let Some(span) = element.spans.iter().find(|s| s.file == uri) {
                    chain.push(range_to_lsp_range(span.start, span.end, &doc.content));
                }

                // Walk up through owners for increasingly larger selections
                let mut current = element.owner.clone();
                while let Some(owner_id) = current {
                    if let Some(owner) = doc.graph.get_element(&owner_id) {
                        if let Some(span) = owner.spans.iter().find(|s| s.file == uri) {
                            chain.push(range_to_lsp_range(span.start, span.end, &doc.content));
                        }
                        current = owner.owner.clone();
                    } else {
                        break;
                    }
                }
            }
        }

        // Build the nested SelectionRange chain (innermost first)
        let selection_range =
            chain
                .into_iter()
                .rev()
                .fold(None::<SelectionRange>, |parent, range| {
                    Some(SelectionRange {
                        range,
                        parent: parent.map(Box::new),
                    })
                });

        result.push(selection_range.unwrap_or(SelectionRange {
            range: Range {
                start: *position,
                end: *position,
            },
            parent: None,
        }));
    }

    Ok(Some(result))
}
