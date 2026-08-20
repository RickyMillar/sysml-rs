//! Advanced feature handlers: document links, signature help, call hierarchy.

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
use std::path::PathBuf;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{DocumentLinkParams, DocumentLink, SignatureHelpParams, SignatureHelp, CallHierarchyPrepareParams, CallHierarchyItem, Url, CallHierarchyIncomingCallsParams, CallHierarchyIncomingCall, CallHierarchyOutgoingCallsParams, CallHierarchyOutgoingCall, Range, Position};

use crate::hover::find_element_type;
use crate::kinds::element_kind_to_symbol_kind;
use crate::manifest_language_features;
use crate::syntax_context::{collect_import_declarations, CursorSyntaxContext};
use crate::types::SYNTHETIC_FILE;
use crate::utils::{parse_uri, position_to_offset, range_to_lsp_range};
use crate::workspace_snapshot;
use crate::SysmlLanguageServer;

pub(crate) async fn document_link(
    server: &SysmlLanguageServer,
    params: DocumentLinkParams,
) -> Result<Option<Vec<DocumentLink>>> {
    let uri = params.text_document.uri.to_string();

    let Some(doc) = server.salsa_doc(&uri).await else {
        return Ok(None);
    };

    if uri.ends_with("sysml.toml") {
        let links = manifest_language_features::document_links_for_manifest(&uri, &doc.content);
        return Ok(if links.is_empty() { None } else { Some(links) });
    }

    let Some(cached_tree) = server.salsa_tree(&uri).await else {
        return Ok(None);
    };

    let workspace_roots = server.workspace_index.workspace_roots.read().await.clone();
    let ws = server.workspace_snapshot().await;
    let links = SysmlLanguageServer::collect_import_links(
        &uri,
        &doc.content,
        cached_tree.tree(),
        &ws,
        &workspace_roots,
    );

    if links.is_empty() {
        Ok(None)
    } else {
        Ok(Some(links))
    }
}

pub(crate) async fn signature_help(
    server: &SysmlLanguageServer,
    params: SignatureHelpParams,
) -> Result<Option<SignatureHelp>> {
    let uri = params
        .text_document_position_params
        .text_document
        .uri
        .to_string();
    let position = params.text_document_position_params.position;

    let Some(doc) = server.salsa_doc(&uri).await else {
        return Ok(None);
    };

    let offset = position_to_offset(&position, &doc.content).min(doc.content.len());
    let text_before = &doc.content[..offset];
    let cached_tree = server.salsa_tree(&uri).await;
    let syntax_ctx = cached_tree
        .as_ref()
        .map(|ct| CursorSyntaxContext::from_tree(ct.tree(), &doc.content, offset));

    let signatures = SysmlLanguageServer::build_signature_help(text_before, syntax_ctx.as_ref());
    if signatures.is_empty() {
        return Ok(None);
    }

    Ok(Some(SignatureHelp {
        signatures,
        active_signature: Some(0),
        active_parameter: None,
    }))
}

pub(crate) async fn prepare_call_hierarchy(
    server: &SysmlLanguageServer,
    params: CallHierarchyPrepareParams,
) -> Result<Option<Vec<CallHierarchyItem>>> {
    let uri = params
        .text_document_position_params
        .text_document
        .uri
        .to_string();
    let position = params.text_document_position_params.position;

    let Some(doc) = server.salsa_doc(&uri).await else {
        return Ok(None);
    };

    let offset = position_to_offset(&position, &doc.content);

    let Some(element_id) = doc.position_map.element_id_at(offset) else {
        return Ok(None);
    };

    let Some(element) = doc.graph.get_element(&element_id) else {
        return Ok(None);
    };

    // Call hierarchy applies to action definitions and action usages
    let is_action = matches!(
        element.kind,
        sysml_core::ElementKind::ActionDefinition
            | sysml_core::ElementKind::ActionUsage
            | sysml_core::ElementKind::StateDefinition
            | sysml_core::ElementKind::StateUsage
    );
    if !is_action {
        return Ok(None);
    }

    let Some(name) = element.name.clone() else {
        return Ok(None);
    };

    let span = match element.spans.first() {
        Some(s) if s.file != SYNTHETIC_FILE => s,
        _ => return Ok(None),
    };

    let range = range_to_lsp_range(span.start, span.end, &doc.content);

    Ok(Some(vec![CallHierarchyItem {
        name,
        kind: element_kind_to_symbol_kind(&element.kind),
        tags: None,
        detail: element.qname.as_ref().map(|q| q.to_string()),
        uri: Url::parse(&uri).unwrap_or_else(|_| Url::parse("file:///unknown").unwrap()),
        range,
        selection_range: range,
        data: Some(serde_json::json!({
            "elementId": element_id.to_string(),
            "uri": uri,
        })),
    }]))
}

pub(crate) async fn incoming_calls(
    server: &SysmlLanguageServer,
    params: CallHierarchyIncomingCallsParams,
) -> Result<Option<Vec<CallHierarchyIncomingCall>>> {
    let Some(data) = &params.item.data else {
        return Ok(None);
    };

    let uri = data.get("uri").and_then(|v| v.as_str()).unwrap_or("");
    let element_id_str = data.get("elementId").and_then(|v| v.as_str()).unwrap_or("");

    let Some(doc) = server.salsa_doc(uri).await else {
        return Ok(None);
    };

    let element_id: sysml_id::ElementId = match element_id_str.parse() {
        Ok(id) => id,
        Err(e) => {
            tracing::debug!(
                uri,
                element_id = element_id_str,
                error = %e,
                "invalid call hierarchy element id"
            );
            return Ok(None);
        }
    };

    let Some(element) = doc.graph.get_element(&element_id) else {
        return Ok(None);
    };

    let Some(target_name) = element.name.clone() else {
        return Ok(None);
    };

    // Find action usages that reference this element's type
    // (i.e., actions that "call" this action definition)
    let mut items = Vec::new();
    for caller in doc.graph.elements.values() {
        if !matches!(
            caller.kind,
            sysml_core::ElementKind::ActionUsage | sysml_core::ElementKind::StateUsage
        ) {
            continue;
        }

        // Check if this usage is typed by our target
        let type_name = find_element_type(caller, &doc.graph);
        if type_name.as_deref() != Some(target_name.as_str()) {
            continue;
        }

        let caller_name = match &caller.name {
            Some(n) => n.clone(),
            None => continue,
        };

        let span = match caller.spans.first() {
            Some(s) if s.file != SYNTHETIC_FILE => s,
            _ => continue,
        };

        let file_url =
            parse_uri(&span.file).unwrap_or_else(|| Url::parse("file:///unknown").unwrap());
        let content = if span.file == uri {
            &doc.content
        } else {
            continue; // Only search current file for now
        };
        let range = range_to_lsp_range(span.start, span.end, content);

        items.push(CallHierarchyIncomingCall {
            from: CallHierarchyItem {
                name: caller_name,
                kind: element_kind_to_symbol_kind(&caller.kind),
                tags: None,
                detail: caller.qname.as_ref().map(|q| q.to_string()),
                uri: file_url,
                range,
                selection_range: range,
                data: Some(serde_json::json!({
                    "elementId": caller.id.to_string(),
                    "uri": uri,
                })),
            },
            from_ranges: vec![range],
        });
    }

    if items.is_empty() {
        Ok(None)
    } else {
        Ok(Some(items))
    }
}

pub(crate) async fn outgoing_calls(
    server: &SysmlLanguageServer,
    params: CallHierarchyOutgoingCallsParams,
) -> Result<Option<Vec<CallHierarchyOutgoingCall>>> {
    let Some(data) = &params.item.data else {
        return Ok(None);
    };

    let uri = data.get("uri").and_then(|v| v.as_str()).unwrap_or("");
    let element_id_str = data.get("elementId").and_then(|v| v.as_str()).unwrap_or("");

    let Some(doc) = server.salsa_doc(uri).await else {
        return Ok(None);
    };

    let element_id: sysml_id::ElementId = match element_id_str.parse() {
        Ok(id) => id,
        Err(e) => {
            tracing::debug!(
                uri,
                element_id = element_id_str,
                error = %e,
                "invalid call hierarchy element id"
            );
            return Ok(None);
        }
    };

    // Find owned action usages (these are the "outgoing calls")
    let mut items = Vec::new();
    for member in doc.graph.owned_members(&element_id) {
        if !matches!(
            member.kind,
            sysml_core::ElementKind::ActionUsage | sysml_core::ElementKind::StateUsage
        ) {
            continue;
        }

        let member_name = match &member.name {
            Some(n) => n.clone(),
            None => continue,
        };

        let span = match member.spans.first() {
            Some(s) if s.file != SYNTHETIC_FILE => s,
            _ => continue,
        };

        let file_url =
            parse_uri(&span.file).unwrap_or_else(|| Url::parse("file:///unknown").unwrap());
        let content = if span.file == uri {
            &doc.content
        } else {
            continue;
        };
        let range = range_to_lsp_range(span.start, span.end, content);

        // Try to resolve to the definition for richer info
        let detail = find_element_type(member, &doc.graph).map(|t| format!("calls {}", t));

        items.push(CallHierarchyOutgoingCall {
            to: CallHierarchyItem {
                name: member_name,
                kind: element_kind_to_symbol_kind(&member.kind),
                tags: None,
                detail,
                uri: file_url,
                range,
                selection_range: range,
                data: Some(serde_json::json!({
                    "elementId": member.id.to_string(),
                    "uri": uri,
                })),
            },
            from_ranges: vec![range],
        });
    }

    if items.is_empty() {
        Ok(None)
    } else {
        Ok(Some(items))
    }
}

// --- Import link helpers ---

impl SysmlLanguageServer {
    /// Collect DocumentLinks for import statements using shared syntax extraction.
    pub(crate) fn collect_import_links(
        source_uri: &str,
        content: &str,
        tree: &tree_sitter::Tree,
        ws: &workspace_snapshot::WorkspaceSnapshot,
        workspace_roots: &[String],
    ) -> Vec<DocumentLink> {
        let mut links = Vec::new();
        for import in collect_import_declarations(tree, content) {
            let Some(path_text) = import.path_text else {
                continue;
            };
            let (Some(path_start), Some(path_end)) = (import.path_start, import.path_end) else {
                continue;
            };

            let target_uri =
                Self::resolve_import_target(&path_text, source_uri, ws, workspace_roots);
            let range = Range {
                start: Position {
                    line: path_start.row as u32,
                    character: path_start.column as u32,
                },
                end: Position {
                    line: path_end.row as u32,
                    character: path_end.column as u32,
                },
            };

            links.push(DocumentLink {
                range,
                target: target_uri,
                tooltip: Some(format!("Import: {}", path_text)),
                data: None,
            });
        }
        links
    }

    /// Attempt to resolve an import path to a file URI.
    fn resolve_import_target(
        path: &str,
        source_uri: &str,
        ws: &workspace_snapshot::WorkspaceSnapshot,
        workspace_roots: &[String],
    ) -> Option<Url> {
        let normalized = Self::normalize_import_path(path)?;

        // Prefer fully-qualified import matches first.
        if let Some(entry) = ws.find_by_qname(&normalized) {
            if let Some(url) = parse_uri(&entry.uri) {
                return Some(url);
            }
        }

        let segments: Vec<&str> = normalized.split("::").filter(|s| !s.is_empty()).collect();
        if segments.is_empty() {
            return None;
        }

        let source_path = parse_uri(source_uri).and_then(|url| url.to_file_path().ok());
        let workspace_root_paths: Vec<PathBuf> =
            workspace_roots.iter().map(PathBuf::from).collect();

        #[derive(Clone)]
        struct RankedImportCandidate {
            uri: String,
            segment_rank: usize,
            workspace_rank: usize,
            distance_rank: usize,
        }

        let mut candidates: HashMap<(String, sysml_id::ElementId), RankedImportCandidate> =
            HashMap::new();
        for (segment_rank, segment) in segments.iter().rev().enumerate() {
            for entry in ws.find_by_name(segment) {
                let candidate_path = parse_uri(&entry.uri).and_then(|url| url.to_file_path().ok());
                let workspace_rank = Self::workspace_rank(
                    source_path.as_deref(),
                    candidate_path.as_deref(),
                    &workspace_root_paths,
                );
                let distance_rank =
                    Self::path_distance_rank(source_path.as_deref(), candidate_path.as_deref());
                let key = (entry.uri.clone(), entry.element_id.clone());
                let candidate = RankedImportCandidate {
                    uri: entry.uri.clone(),
                    segment_rank,
                    workspace_rank,
                    distance_rank,
                };
                candidates
                    .entry(key)
                    .and_modify(|existing| {
                        if (
                            candidate.segment_rank,
                            candidate.workspace_rank,
                            candidate.distance_rank,
                            &candidate.uri,
                        ) < (
                            existing.segment_rank,
                            existing.workspace_rank,
                            existing.distance_rank,
                            &existing.uri,
                        ) {
                            *existing = candidate.clone();
                        }
                    })
                    .or_insert(candidate);
            }
        }

        candidates
            .into_values()
            .min_by(|left, right| {
                (
                    left.segment_rank,
                    left.workspace_rank,
                    left.distance_rank,
                    &left.uri,
                )
                    .cmp(&(
                        right.segment_rank,
                        right.workspace_rank,
                        right.distance_rank,
                        &right.uri,
                    ))
            })
            .and_then(|best| parse_uri(&best.uri))
    }

    pub(crate) fn normalize_import_path(path: &str) -> Option<String> {
        let mut trimmed = path.trim().trim_end_matches(';').trim();
        if let Some(prefix) = trimmed.strip_suffix("::**") {
            trimmed = prefix.trim_end();
        } else if let Some(prefix) = trimmed.strip_suffix("::*") {
            trimmed = prefix.trim_end();
        }
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_owned())
        }
    }

    fn workspace_rank(
        source_path: Option<&std::path::Path>,
        candidate_path: Option<&std::path::Path>,
        workspace_roots: &[PathBuf],
    ) -> usize {
        let Some(candidate_path) = candidate_path else {
            return usize::MAX / 2;
        };

        let source_root =
            source_path.and_then(|path| Self::workspace_root_index(path, workspace_roots));
        let candidate_root = Self::workspace_root_index(candidate_path, workspace_roots);

        match (source_root, candidate_root) {
            (Some(source), Some(candidate)) if source == candidate => 0,
            (_, Some(_)) => 1,
            _ => 2,
        }
    }

    fn workspace_root_index(path: &std::path::Path, workspace_roots: &[PathBuf]) -> Option<usize> {
        workspace_roots
            .iter()
            .enumerate()
            .filter_map(|(idx, root)| {
                if path.starts_with(root) {
                    Some((root.components().count(), idx))
                } else {
                    None
                }
            })
            .max_by_key(|(depth, _)| *depth)
            .map(|(_, idx)| idx)
    }

    fn path_distance_rank(
        source_path: Option<&std::path::Path>,
        candidate_path: Option<&std::path::Path>,
    ) -> usize {
        let (Some(source), Some(candidate)) = (source_path, candidate_path) else {
            return usize::MAX / 2;
        };

        let source_components: Vec<_> = source.components().collect();
        let candidate_components: Vec<_> = candidate.components().collect();
        let common_prefix_len = source_components
            .iter()
            .zip(candidate_components.iter())
            .take_while(|(left, right)| left == right)
            .count();
        (source_components.len().saturating_sub(common_prefix_len))
            + (candidate_components.len().saturating_sub(common_prefix_len))
    }
}
