//! Prepare-rename and rename handlers — thin shims over `sysml.rename`.
//!
//! All resolution and cross-file scanning lives in the service
//! (`sysml_service::rename`). The LSP layer here just converts
//! `lsp_types::Position` to a `(line, col)` pair, dispatches, and reshapes the
//! response into LSP types.

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

use std::collections::HashMap;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    Position, PrepareRenameResponse, Range, RenameParams, TextDocumentPositionParams, TextEdit,
    Url, WorkspaceEdit,
};

use sysml_service::rename::RenameResponse;

use crate::service_edits::to_lsp_text_edit;

use crate::utils::parse_uri;
use crate::SysmlLanguageServer;

pub(crate) async fn prepare_rename(
    server: &SysmlLanguageServer,
    params: TextDocumentPositionParams,
) -> Result<Option<PrepareRenameResponse>> {
    let uri = params.text_document.uri.to_string();
    let position = params.position;

    let response = match dispatch_rename(server, &uri, position, None).await {
        Some(r) => r,
        None => return Ok(None),
    };

    let Some(prepare) = response.prepare else {
        return Ok(None);
    };

    Ok(Some(PrepareRenameResponse::RangeWithPlaceholder {
        range: lsp_range(
            prepare.line_start,
            prepare.col_start,
            prepare.line_end,
            prepare.col_end,
        ),
        placeholder: prepare.placeholder,
    }))
}

pub(crate) async fn rename(
    server: &SysmlLanguageServer,
    params: RenameParams,
) -> Result<Option<WorkspaceEdit>> {
    let uri = params.text_document_position.text_document.uri.to_string();
    let position = params.text_document_position.position;
    let new_name = params.new_name;

    let result = server.service.rename(
        &uri,
        position.line,
        position.character,
        Some(new_name.as_str()),
    );
    let response = match result {
        Ok(r) => r,
        Err(sysml_service::ServiceError::InvalidInput(msg)) => {
            return Err(tower_lsp::jsonrpc::Error {
                code: tower_lsp::jsonrpc::ErrorCode::InvalidParams,
                message: msg.into(),
                data: None,
            });
        }
        Err(_) => return Ok(None),
    };

    let Some(workspace_edit) = response.apply else {
        return Ok(None);
    };

    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for file_edits in workspace_edit.changes {
        let Some(url) = parse_uri(&file_edits.uri) else {
            continue;
        };
        let edits: Vec<TextEdit> = file_edits.edits.into_iter().map(to_lsp_text_edit).collect();
        changes.insert(url, edits);
    }

    Ok(Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    }))
}

async fn dispatch_rename(
    server: &SysmlLanguageServer,
    uri: &str,
    position: Position,
    new_name: Option<&str>,
) -> Option<RenameResponse> {
    let result = server
        .service
        .rename(uri, position.line, position.character, new_name);
    match result {
        Ok(r) => Some(r),
        Err(_) => None,
    }
}

fn lsp_range(line_start: u32, col_start: u32, line_end: u32, col_end: u32) -> Range {
    Range {
        start: Position {
            line: line_start,
            character: col_start,
        },
        end: Position {
            line: line_end,
            character: col_end,
        },
    }
}
