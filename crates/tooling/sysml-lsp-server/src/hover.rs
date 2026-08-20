//! Hover handler — thin LSP transport over `sysml_service::hover`.
//!
//! The model-element hover content (signature, supertypes, doc comments,
//! physics classification, evaluated value, expression AST) lives in
//! `sysml_service::hover`. This module keeps:
//!
//!   - the async LSP `hover` handler entry,
//!   - import-segment hover (depends on the tree-sitter Tree from
//!     `salsa_tree` and on workspace/library lookup),
//!   - the keyword fallback (CST node lookup via `salsa_tree`),
//!   - the external library hover-source disk loader
//!     (`load_external_hover_source` lives on `SysmlLanguageServer` and
//!     is unchanged).

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
    Hover, HoverContents, HoverParams, MarkupContent, MarkupKind, Position, Range,
};

use sysml_core::ModelGraph;
use sysml_service::hover::{
    append_package_members_preview, build_hover_content, keyword_documentation,
};

// Re-exports so the existing LSP-internal callers (navigation, inlay_hints,
// advanced_features) keep importing from `crate::hover::*`.
pub(crate) use sysml_service::goto_definition::find_element_type;

use crate::syntax_context::{node_at_lsp_position, node_range_to_lsp};
use crate::telemetry_events;
use crate::utils::{parse_uri, position_to_offset, range_to_lsp_range};
use crate::SysmlLanguageServer;

#[derive(Debug, Clone)]
struct ImportSegment {
    name: String,
    qname: String,
    start: usize,
    end: usize,
}

fn starts_with_ignore_ascii_case(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len())
        .map(|head| head.eq_ignore_ascii_case(prefix))
        .unwrap_or(false)
}

fn strip_prefix_ignore_ascii_case<'a>(text: &'a str, prefix: &str) -> Option<&'a str> {
    if starts_with_ignore_ascii_case(text, prefix) {
        text.get(prefix.len()..)
    } else {
        None
    }
}

fn parse_import_path_from_decl_text(text: &str) -> Option<String> {
    let mut s = text.trim().trim_end_matches(';').trim();

    for vis in ["public ", "private ", "protected "] {
        if let Some(rest) = strip_prefix_ignore_ascii_case(s, vis) {
            s = rest.trim_start();
            break;
        }
    }

    s = strip_prefix_ignore_ascii_case(s, "import ")?;

    if let Some(rest) = strip_prefix_ignore_ascii_case(s, "all ") {
        s = rest.trim_start();
    }

    if let Some(idx) = s.find('[') {
        s = s[..idx].trim_end();
    }

    if let Some(prefix) = s.strip_suffix("::**") {
        s = prefix.trim_end();
    } else if let Some(prefix) = s.strip_suffix("::*") {
        s = prefix.trim_end();
    }

    let path = s.trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_owned())
    }
}

fn find_import_decl_ancestor(mut node: tree_sitter::Node<'_>) -> Option<tree_sitter::Node<'_>> {
    loop {
        if node.kind() == "import_decl" {
            return Some(node);
        }
        node = node.parent()?;
    }
}

fn import_path_from_decl_node(
    import_decl: tree_sitter::Node<'_>,
    content: &str,
) -> Option<(String, usize)> {
    if let Some(path_node) = import_decl.child_by_field_name("path") {
        let start = path_node.start_byte();
        let end = path_node.end_byte().min(content.len());
        let text = content.get(start..end)?.to_owned();
        return Some((text, start));
    }

    let start = import_decl.start_byte();
    let end = import_decl.end_byte().min(content.len());
    let decl_text = content.get(start..end)?;
    let parsed = parse_import_path_from_decl_text(decl_text)?;
    let local = decl_text.find(&parsed)?;
    Some((parsed, start + local))
}

fn split_import_segments(path_text: &str, path_abs_start: usize) -> Vec<ImportSegment> {
    let trimmed = path_text.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    let lead = path_text.find(trimmed).unwrap_or(0);
    let mut core = trimmed;
    if let Some(prefix) = core.strip_suffix("::**") {
        core = prefix.trim_end();
    } else if let Some(prefix) = core.strip_suffix("::*") {
        core = prefix.trim_end();
    }
    if core.is_empty() {
        return Vec::new();
    }

    let core_abs_start = path_abs_start + lead;
    let mut idx = 0usize;
    let mut segments = Vec::new();
    let mut qname_parts: Vec<String> = Vec::new();

    while idx < core.len() {
        if core[idx..].starts_with("::") {
            idx += 2;
            continue;
        }

        let next = core[idx..]
            .find("::")
            .map(|offset| idx + offset)
            .unwrap_or(core.len());
        let raw_segment = &core[idx..next];
        let segment = raw_segment.trim();
        if !segment.is_empty() && segment != "*" && segment != "**" {
            let local_trim_start = raw_segment.find(segment).unwrap_or(0);
            let rel_start = idx + local_trim_start;
            let rel_end = rel_start + segment.len();
            qname_parts.push(segment.to_owned());
            segments.push(ImportSegment {
                name: segment.to_owned(),
                qname: qname_parts.join("::"),
                start: core_abs_start + rel_start,
                end: core_abs_start + rel_end,
            });
        }

        idx = if next < core.len() {
            next + 2
        } else {
            core.len()
        };
    }

    segments
}

async fn resolve_workspace_entry_hover(
    server: &SysmlLanguageServer,
    entry: &crate::background::CrossFileEntry,
) -> Option<(sysml_core::Element, ModelGraph, String)> {
    let target_doc = if let Some(doc) = server.salsa_doc(&entry.uri).await {
        Some(doc)
    } else if let Some(uri) = parse_uri(&entry.uri).map(|u| u.to_string()) {
        server.salsa_doc(&uri).await
    } else {
        None
    }?;

    let raw = target_doc.graph.get_element(&entry.element_id)?;
    let element = sysml_service::goto_definition::resolve_goto_target(raw, &target_doc.graph).clone();
    Some((element, target_doc.graph, target_doc.content))
}

async fn try_import_segment_hover(
    server: &SysmlLanguageServer,
    doc: &crate::SalsaDoc,
    uri: &str,
    position: &Position,
    offset: usize,
) -> Option<Hover> {
    let cached_tree = server.salsa_tree(uri).await?;
    let node = node_at_lsp_position(cached_tree.tree(), &doc.content, position)?;
    let import_decl = find_import_decl_ancestor(node)?;
    let (path_text, path_start) = import_path_from_decl_node(import_decl, &doc.content)?;
    let segment = split_import_segments(&path_text, path_start)
        .into_iter()
        .find(|segment| segment.start <= offset && offset < segment.end)?;

    let ws = server.workspace_snapshot().await;

    let mut resolved: Option<(sysml_core::Element, ModelGraph, String)> = None;

    if let Some(element) = doc.graph.resolve_qname(&segment.qname) {
        resolved = Some((element.clone(), doc.graph.clone(), doc.content.clone()));
    }

    if resolved.is_none() {
        if let Some(entry) = ws.find_by_qname(&segment.qname) {
            resolved = resolve_workspace_entry_hover(server, entry).await;
        }
    }

    if resolved.is_none() {
        if let Some((parent_qname, member_name)) = segment.qname.rsplit_once("::") {
            let mut matched: Option<crate::background::CrossFileEntry> = None;
            ws.for_each_qname_member(parent_qname, |name, entry| {
                if matched.is_none() && name == member_name {
                    matched = Some(entry.clone());
                }
            });
            if let Some(entry) = matched {
                resolved = resolve_workspace_entry_hover(server, &entry).await;
            }
        }
    }

    if resolved.is_none() && !segment.qname.contains("::") {
        let root_entry = ws
            .find_by_name(&segment.name)
            .iter()
            .find(|entry| {
                matches!(
                    entry.element_kind,
                    sysml_core::ElementKind::Package | sysml_core::ElementKind::LibraryPackage
                )
            })
            .cloned()
            .or_else(|| ws.find_by_name(&segment.name).first().cloned());
        if let Some(entry) = root_entry {
            resolved = resolve_workspace_entry_hover(server, &entry).await;
        }
    }

    if resolved.is_none() {
        if let Some((library, _)) = server.get_library().await {
            if let Some(raw_element) = library.resolve_qname(&segment.qname) {
                let element =
                    sysml_service::goto_definition::resolve_goto_target(raw_element, &library)
                        .clone();
                let source = server
                    .load_external_hover_source(&element, uri)
                    .await
                    .map(|s| s.as_str().to_owned())
                    .unwrap_or_default();
                resolved = Some((element, (*library).clone(), source));
            }
        }
    }

    let (element, graph, source) = resolved?;
    let hover_source = if source.is_empty() {
        &doc.content
    } else {
        &source
    };
    let registry = sysml_core::physics::PhysicsDomainRegistry::new();
    let mut hover_content =
        build_hover_content(&element, &graph, hover_source, false, false, &registry);
    append_package_members_preview(&mut hover_content, &element, &graph);
    let hover_range = range_to_lsp_range(segment.start, segment.end, &doc.content);

    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: hover_content,
        }),
        range: Some(hover_range),
    })
}

pub(crate) async fn hover(
    server: &SysmlLanguageServer,
    params: HoverParams,
) -> Result<Option<Hover>> {
    let started_at = Instant::now();
    let uri = params
        .text_document_position_params
        .text_document
        .uri
        .to_string();
    let position = params.text_document_position_params.position;

    let doc = match server.salsa_doc(&uri).await {
        Some(d) => d,
        None => {
            telemetry_events::hover_latency(
                &uri,
                "none",
                "missing_document",
                false,
                started_at.elapsed().as_millis(),
            );
            return Ok(None);
        }
    };

    let offset = position_to_offset(&position, &doc.content);

    // Import declarations cover the full statement span; provide segment-specific
    // hover first so `Definitions` and `CoffeeMachine` can resolve independently.
    let is_import_element = doc
        .position_map
        .element_id_at(offset)
        .and_then(|id| doc.graph.get_element(&id))
        .map(|element| {
            matches!(
                element.kind,
                sysml_core::ElementKind::MembershipImport
                    | sysml_core::ElementKind::NamespaceImport
                    | sysml_core::ElementKind::MembershipExpose
                    | sysml_core::ElementKind::NamespaceExpose
            )
        })
        .unwrap_or(false);
    if is_import_element {
        if let Some(import_hover) =
            try_import_segment_hover(server, &doc, &uri, &position, offset).await
        {
            telemetry_events::hover_latency(
                &uri,
                "import",
                "ok",
                false,
                started_at.elapsed().as_millis(),
            );
            return Ok(Some(import_hover));
        }
    }

    // Model-element hover via the unified service path.
    if let Ok(Some(info)) = server.service.hover(&uri, position.line, position.character) {
        let range = Range {
            start: Position {
                line: info.line_start,
                character: info.col_start,
            },
            end: Position {
                line: info.line_end,
                character: info.col_end,
            },
        };
        telemetry_events::hover_latency(
            &uri,
            "model",
            "ok",
            false,
            started_at.elapsed().as_millis(),
        );
        return Ok(Some(Hover {
            contents: HoverContents::Markup(MarkupContent {
                kind: MarkupKind::Markdown,
                value: info.markdown,
            }),
            range: Some(range),
        }));
    }

    // No element found — try keyword hover from tree-sitter CST.
    if let Some(cached_tree) = server.salsa_tree(&uri).await {
        let node = node_at_lsp_position(cached_tree.tree(), &doc.content, &position);
        if let Some(node) = node {
            if let Some(keyword_doc) = keyword_documentation(node.kind()) {
                let range = node_range_to_lsp(&node);
                telemetry_events::hover_latency(
                    &uri,
                    "keyword",
                    "ok",
                    false,
                    started_at.elapsed().as_millis(),
                );
                return Ok(Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: keyword_doc.to_owned(),
                    }),
                    range: Some(range),
                }));
            }
        }
    }

    telemetry_events::hover_latency(
        &uri,
        "none",
        "no_context",
        false,
        started_at.elapsed().as_millis(),
    );
    Ok(None)
}
