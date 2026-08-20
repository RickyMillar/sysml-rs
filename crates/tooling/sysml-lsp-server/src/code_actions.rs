//! Code-action handler — thin shim over `sysml.code_action.list`.
//!
//! All quick-fix / refactoring / source-action computation lives in
//! `sysml_service::code_actions`. The LSP layer here just converts
//! `lsp_types::Diagnostic` to the service-side wire shape, dispatches, and
//! reshapes `CodeAction` into `lsp_types::CodeActionOrCommand`.

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

use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Command, Diagnostic, NumberOrString, Position,
    Range, TextEdit, Url, WorkspaceEdit,
};

use crate::service_edits::to_lsp_text_edit;
use sysml_service::code_actions::{
    CodeAction as ServiceCodeAction, CodeActionCommand as ServiceCodeActionCommand,
    CodeActionDiagnostic,
};
use sysml_service::SysmlService;

/// Generate all code actions for the given context.
pub(crate) fn generate_code_actions(
    service: &SysmlService,
    uri: &str,
    diagnostics: &[Diagnostic],
    range: &Range,
) -> Vec<CodeActionOrCommand> {
    let wire_diags = diagnostics
        .iter()
        .map(diagnostic_to_wire)
        .collect::<Vec<_>>();
    let wire_diags_json = match serde_json::to_value(&wire_diags) {
        Ok(v) => v,
        Err(_) => return Vec::new(),
    };
    let actions = match service.code_action_list(
        uri,
        range.start.line,
        range.start.character,
        range.end.line,
        range.end.character,
        &wire_diags_json,
    ) {
        Ok(a) => a,
        Err(_) => return Vec::new(),
    };
    actions.into_iter().map(action_from_service).collect()
}

fn diagnostic_to_wire(d: &Diagnostic) -> CodeActionDiagnostic {
    let code = d.code.as_ref().and_then(|c| match c {
        NumberOrString::String(s) => Some(s.clone()),
        NumberOrString::Number(n) => Some(n.to_string()),
    });
    CodeActionDiagnostic {
        line_start: d.range.start.line,
        col_start: d.range.start.character,
        line_end: d.range.end.line,
        col_end: d.range.end.character,
        code,
        message: d.message.clone(),
        source: d.source.clone(),
        severity: d.severity.map(|s| match s {
            tower_lsp::lsp_types::DiagnosticSeverity::ERROR => 1,
            tower_lsp::lsp_types::DiagnosticSeverity::WARNING => 2,
            tower_lsp::lsp_types::DiagnosticSeverity::INFORMATION => 3,
            tower_lsp::lsp_types::DiagnosticSeverity::HINT => 4,
            _ => 1,
        }),
    }
}

fn action_from_service(svc: ServiceCodeAction) -> CodeActionOrCommand {
    let mut changes: HashMap<Url, Vec<TextEdit>> = HashMap::new();
    for file_edits in svc.edits {
        let Some(url) = parse_uri(&file_edits.uri) else {
            continue;
        };
        let edits: Vec<TextEdit> = file_edits.edits.into_iter().map(to_lsp_text_edit).collect();
        changes.insert(url, edits);
    }
    let kind = lsp_kind_from_str(&svc.kind);
    let diagnostics = if svc.diagnostics.is_empty() {
        None
    } else {
        Some(svc.diagnostics.into_iter().map(wire_to_diag).collect())
    };
    let edit = if changes.is_empty() {
        // Command-only actions (e.g. IM012 strict-mode quick-fixes) have no
        // text edits — leave `edit` unset so the client treats them as
        // command-only.
        None
    } else {
        Some(WorkspaceEdit {
            changes: Some(changes),
            ..Default::default()
        })
    };
    let command = svc.command.map(service_command_to_lsp);
    CodeActionOrCommand::CodeAction(CodeAction {
        title: svc.title,
        kind: Some(kind),
        diagnostics,
        edit,
        is_preferred: svc.is_preferred,
        command,
        ..Default::default()
    })
}

fn service_command_to_lsp(cmd: ServiceCodeActionCommand) -> Command {
    Command {
        title: cmd.title,
        command: cmd.command,
        arguments: if cmd.arguments.is_empty() {
            None
        } else {
            Some(cmd.arguments)
        },
    }
}

fn wire_to_diag(d: CodeActionDiagnostic) -> Diagnostic {
    Diagnostic {
        range: Range {
            start: Position {
                line: d.line_start,
                character: d.col_start,
            },
            end: Position {
                line: d.line_end,
                character: d.col_end,
            },
        },
        severity: d.severity.and_then(|s| match s {
            1 => Some(tower_lsp::lsp_types::DiagnosticSeverity::ERROR),
            2 => Some(tower_lsp::lsp_types::DiagnosticSeverity::WARNING),
            3 => Some(tower_lsp::lsp_types::DiagnosticSeverity::INFORMATION),
            4 => Some(tower_lsp::lsp_types::DiagnosticSeverity::HINT),
            _ => None,
        }),
        code: d.code.map(NumberOrString::String),
        source: d.source,
        message: d.message,
        ..Default::default()
    }
}

fn parse_uri(uri: &str) -> Option<Url> {
    Url::parse(uri)
        .ok()
        .or_else(|| Url::from_file_path(uri).ok())
}

fn lsp_kind_from_str(kind: &str) -> CodeActionKind {
    match kind {
        "quickfix" => CodeActionKind::QUICKFIX,
        "refactor" => CodeActionKind::REFACTOR,
        "refactor.rewrite" => CodeActionKind::REFACTOR_REWRITE,
        "refactor.extract" => CodeActionKind::REFACTOR_EXTRACT,
        "refactor.inline" => CodeActionKind::REFACTOR_INLINE,
        "source" => CodeActionKind::SOURCE,
        "source.organizeImports" => CodeActionKind::SOURCE_ORGANIZE_IMPORTS,
        "source.fixAll" => CodeActionKind::SOURCE_FIX_ALL,
        // Fall back to a `String`-backed kind so the lifetime escapes cleanly.
        // `CodeActionKind::new` takes `&'static str`, so we Box-leak (rare path).
        other => CodeActionKind::from(other.to_owned()),
    }
}
