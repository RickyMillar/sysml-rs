//! LSP completion shell — delegates the heavy lifting to
//! `sysml_service::completion::compute_completion` and
//! `compute_completion_resolve`. This shell keeps:
//!
//!   - `pending_requests` stale-result handling (transient shell concern),
//!   - telemetry events (`completion_latency`, `completion_phases`),
//!   - `manifest_language_features::completion_for_manifest` short-circuit
//!     (sysml.toml — handled at the caller in `lib.rs`),
//!   - tree-sitter `CursorSyntaxContext::from_tree` and the CST →
//!     `SyntaxCtxSummary` translation (service stays free of tree-sitter).
//!
//! Everything else (route classification, candidate enumeration, scoring,
//! dedupe, item construction, resolve enrichment) lives on the service.

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

use std::time::Instant;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionItemLabelDetails, CompletionParams,
    CompletionResponse, Documentation, InsertTextFormat, MarkupContent, MarkupKind, Position,
    Range, TextEdit,
};

use sysml_service::completion::{
    CompletionCandidate, CompletionLabelDetails, SyntaxCtxSummary, TextEditPos,
};

use crate::syntax_context::CursorSyntaxContext;
use crate::telemetry_events;
use crate::utils::position_to_offset;
use crate::SysmlLanguageServer;

/// Translate a tree-sitter `CursorSyntaxContext` into the service-side
/// `SyntaxCtxSummary` (only the fields the route classifier needs).
fn syntax_summary_from_ctx(ctx: Option<&CursorSyntaxContext>) -> SyntaxCtxSummary {
    match ctx {
        Some(c) => SyntaxCtxSummary {
            in_import: c.in_import_decl(),
            in_comment_or_string: c.in_comment_or_string(),
            in_feature_chain: c.in_feature_chain(),
            in_type_ref: c.in_type_ref(),
        },
        None => SyntaxCtxSummary::default(),
    }
}

fn completion_kind_from_i32(value: i32) -> Option<CompletionItemKind> {
    if (1..=25).contains(&value) {
        // CompletionItemKind variants 1..=25 map directly via the LSP types
        // numeric encoding. Reach for the canonical From<i32> if the variant
        // exists; otherwise fall through to None.
        match value {
            1 => Some(CompletionItemKind::TEXT),
            2 => Some(CompletionItemKind::METHOD),
            3 => Some(CompletionItemKind::FUNCTION),
            4 => Some(CompletionItemKind::CONSTRUCTOR),
            5 => Some(CompletionItemKind::FIELD),
            6 => Some(CompletionItemKind::VARIABLE),
            7 => Some(CompletionItemKind::CLASS),
            8 => Some(CompletionItemKind::INTERFACE),
            9 => Some(CompletionItemKind::MODULE),
            10 => Some(CompletionItemKind::PROPERTY),
            11 => Some(CompletionItemKind::UNIT),
            12 => Some(CompletionItemKind::VALUE),
            13 => Some(CompletionItemKind::ENUM),
            14 => Some(CompletionItemKind::KEYWORD),
            15 => Some(CompletionItemKind::SNIPPET),
            16 => Some(CompletionItemKind::COLOR),
            17 => Some(CompletionItemKind::FILE),
            18 => Some(CompletionItemKind::REFERENCE),
            19 => Some(CompletionItemKind::FOLDER),
            20 => Some(CompletionItemKind::ENUM_MEMBER),
            21 => Some(CompletionItemKind::CONSTANT),
            22 => Some(CompletionItemKind::STRUCT),
            23 => Some(CompletionItemKind::EVENT),
            24 => Some(CompletionItemKind::OPERATOR),
            25 => Some(CompletionItemKind::TYPE_PARAMETER),
            _ => None,
        }
    } else {
        None
    }
}

fn text_edit_from_pos(edit: TextEditPos) -> TextEdit {
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

fn label_details_from(details: CompletionLabelDetails) -> CompletionItemLabelDetails {
    CompletionItemLabelDetails {
        detail: details.detail,
        description: details.description,
    }
}

fn candidate_to_item(candidate: CompletionCandidate) -> CompletionItem {
    let data = match (
        candidate.data_element_id.as_ref(),
        candidate.data_document_uri.as_ref(),
    ) {
        (Some(eid), Some(uri)) => Some(serde_json::json!({
            "element_id": eid,
            "document_uri": uri,
        })),
        (Some(eid), None) => Some(serde_json::json!({
            "element_id": eid,
        })),
        _ => None,
    };
    CompletionItem {
        label: candidate.label,
        kind: completion_kind_from_i32(candidate.kind),
        detail: candidate.detail,
        sort_text: candidate.sort_text,
        insert_text: candidate.insert_text,
        insert_text_format: if candidate.insert_text_is_snippet {
            Some(InsertTextFormat::SNIPPET)
        } else {
            None
        },
        label_details: candidate.label_details.map(label_details_from),
        additional_text_edits: if candidate.additional_text_edits.is_empty() {
            None
        } else {
            Some(
                candidate
                    .additional_text_edits
                    .into_iter()
                    .map(text_edit_from_pos)
                    .collect(),
            )
        },
        preselect: if candidate.preselect { Some(true) } else { None },
        data,
        ..Default::default()
    }
}

fn route_label(syntax_ctx: Option<&CursorSyntaxContext>, trigger: Option<&str>) -> &'static str {
    // Mirrors the previous LSP-side `completion_route_label`. We can't ask
    // the service for the route directly without doing a round-trip, so the
    // label is a best-effort summary for telemetry.
    let in_import = syntax_ctx.map(|c| c.in_import_decl()).unwrap_or(false);
    let in_feature_chain = syntax_ctx.map(|c| c.in_feature_chain()).unwrap_or(false);
    let in_type_ref = syntax_ctx.map(|c| c.in_type_ref()).unwrap_or(false);
    let in_comment_or_string = syntax_ctx.map(|c| c.in_comment_or_string()).unwrap_or(false);

    if in_comment_or_string {
        return "general";
    }
    if matches!(trigger, Some(".")) || in_feature_chain {
        return "feature_chain";
    }
    if matches!(trigger, Some(":")) || in_import {
        return "namespace_members";
    }
    if in_type_ref {
        return "type_references";
    }
    "general"
}

pub(crate) async fn completion(
    server: &SysmlLanguageServer,
    params: CompletionParams,
) -> Result<Option<CompletionResponse>> {
    let started_at = Instant::now();
    let uri = params.text_document_position.text_document.uri.to_string();
    let guard = server.pending_requests.begin(uri.clone(), "completion");
    let position = params.text_document_position.position;

    let trigger = params
        .context
        .as_ref()
        .and_then(|c| c.trigger_character.as_deref());
    let trigger_label = trigger.unwrap_or("manual");

    // Build the syntax context from the cached tree-sitter Tree (if any).
    // The service stays free of tree-sitter — we summarize into bools.
    let context_started = Instant::now();
    let cached_tree = server.salsa_tree(&uri).await;
    let syntax_ctx = if let Some(ct) = cached_tree.as_ref() {
        if let Some(doc) = server.salsa_doc(&uri).await {
            let offset = position_to_offset(&position, &doc.content).min(doc.content.len());
            Some(CursorSyntaxContext::from_tree(ct.tree(), &doc.content, offset))
        } else {
            None
        }
    } else {
        None
    };
    let summary = syntax_summary_from_ctx(syntax_ctx.as_ref());
    let context_us = context_started.elapsed().as_micros();
    let route_lbl = route_label(syntax_ctx.as_ref(), trigger);

    // Route through the service. If the service can't see the URI yet (no
    // graph loaded), we degrade silently — same shape as the previous
    // `salsa_doc → None` early-return.
    let provider_started = Instant::now();
    let result = server.service.completion(
        &uri,
        position.line,
        position.character,
        trigger,
        summary.in_import,
        summary.in_comment_or_string,
        summary.in_feature_chain,
        summary.in_type_ref,
    );
    let provider_us = provider_started.elapsed().as_micros();

    let (result_label, item_count, response) = match result {
        Ok(candidates) => {
            let n = candidates.len();
            let items: Vec<CompletionItem> =
                candidates.into_iter().map(candidate_to_item).collect();
            let label = if n == 0 { "none" } else { "ok" };
            (label, n, Some(CompletionResponse::Array(items)))
        }
        Err(_) => {
            // Missing graph — same behaviour as the old missing_document branch.
            let elapsed_ms = started_at.elapsed().as_millis();
            telemetry_events::completion_latency(
                &uri,
                "unknown",
                trigger_label,
                "missing_document",
                0,
                false,
                elapsed_ms,
            );
            telemetry_events::completion_phases(
                &uri,
                "unknown",
                trigger_label,
                "missing_document",
                0,
                false,
                context_us,
                provider_us,
                0,
                elapsed_ms,
            );
            server.pending_requests.end(&guard);
            return Ok(None);
        }
    };

    // Stale-result handling: if a newer completion request landed, drop this
    // one. The service has already done the work but we don't ship the result.
    let finalize_started = Instant::now();
    if !server.pending_requests.is_current(&guard) {
        let finalize_us = finalize_started.elapsed().as_micros();
        let elapsed_ms = started_at.elapsed().as_millis();
        telemetry_events::completion_latency(
            &uri,
            route_lbl,
            trigger_label,
            "stale_discarded",
            item_count,
            true,
            elapsed_ms,
        );
        telemetry_events::completion_phases(
            &uri,
            route_lbl,
            trigger_label,
            "stale_discarded",
            item_count,
            true,
            context_us,
            provider_us,
            finalize_us,
            elapsed_ms,
        );
        server.pending_requests.end(&guard);
        return Ok(None);
    }
    server.pending_requests.end(&guard);
    let finalize_us = finalize_started.elapsed().as_micros();
    let elapsed_ms = started_at.elapsed().as_millis();
    telemetry_events::completion_latency(
        &uri,
        route_lbl,
        trigger_label,
        result_label,
        item_count,
        false,
        elapsed_ms,
    );
    telemetry_events::completion_phases(
        &uri,
        route_lbl,
        trigger_label,
        result_label,
        item_count,
        false,
        context_us,
        provider_us,
        finalize_us,
        elapsed_ms,
    );
    Ok(response)
}

pub(crate) async fn completion_resolve(
    server: &SysmlLanguageServer,
    mut item: CompletionItem,
) -> Result<CompletionItem> {
    let Some(data) = item.data.as_ref() else {
        return Ok(item);
    };
    let obj = match data.as_object() {
        Some(o) => o,
        None => return Ok(item),
    };
    let element_id_str = match obj.get("element_id").and_then(|v| v.as_str()) {
        Some(s) => s,
        None => return Ok(item),
    };
    let element_id = match element_id_str.parse::<sysml_id::ElementId>() {
        Ok(id) => id,
        Err(_) => return Ok(item),
    };
    let preferred_uri = obj.get("document_uri").and_then(|v| v.as_str());

    let details = match server
        .service
        .completion_resolve(preferred_uri, &element_id)
    {
        Ok(Some(d)) => d,
        _ => return Ok(item),
    };

    if item.documentation.is_none() {
        if let Some(markdown) = details.documentation_markdown {
            item.documentation = Some(Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value: markdown,
            }));
        }
    }
    if item.detail.is_none() {
        if let Some(detail) = details.detail {
            item.detail = Some(detail);
        }
    }
    Ok(item)
}
