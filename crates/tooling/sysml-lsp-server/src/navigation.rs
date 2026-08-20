//! Navigation handlers for goto-definition, references, goto-type-definition,
//! and goto-implementation.
//!
//! The relationship-following ladder that resolves a clicked element to its
//! meaningful target lives in `sysml_service::goto_definition` (the one home);
//! `goto_definition` below calls `service.goto_definition(...)` for that and
//! falls back to a word-based workspace-snapshot lookup.

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

use std::path::{Path, PathBuf};

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::request;
use tower_lsp::lsp_types::{GotoDefinitionParams, GotoDefinitionResponse, ReferenceParams, Location, Url, Range, Position};

use sysml_project::ProjectHandle;

use crate::hover::find_element_type;
use crate::telemetry_control;
use crate::types::SYNTHETIC_FILE;
use crate::utils::{parse_uri, position_to_offset, range_to_lsp_range};
use crate::SysmlLanguageServer;

pub(crate) async fn goto_definition(
    server: &SysmlLanguageServer,
    params: GotoDefinitionParams,
) -> Result<Option<GotoDefinitionResponse>> {
    let uri = params
        .text_document_position_params
        .text_document
        .uri
        .to_string();
    let position = params.text_document_position_params.position;
    let dep_trace = telemetry_control::dependency_trace_enabled();
    if dep_trace {
        tracing::info!(
            request_uri = %uri,
            line = position.line,
            character = position.character,
            "dependency trace: goto-definition request"
        );
    }

    // Primary path: ask the service. It owns the relationship-following
    // ladder + typed-usage type-def lookup and returns a span (line/col).
    if let Ok(Some(target)) = server.service.goto_definition(&uri, position.line, position.character) {
        // `project://N/...` URI rewriting stays LSP-side (the service emits
        // raw `file://` paths from element spans).
        if let Some(target_uri) = resolve_navigation_uri(server, &target.uri).await {
            if dep_trace {
                tracing::info!(
                    request_uri = %uri,
                    target_uri = %target_uri,
                    "dependency trace: goto-definition resolved via service"
                );
            }
            return Ok(Some(GotoDefinitionResponse::Scalar(Location {
                uri: target_uri,
                range: tower_lsp::lsp_types::Range {
                    start: Position { line: target.line_start, character: target.col_start },
                    end: Position { line: target.line_end, character: target.col_end },
                },
            })));
        }
    }

    // Fallback path: word-based workspace lookup (used when the cursor is
    // not over an identifiable element, e.g. inside an unresolved name).
    let doc = match server.salsa_doc(&uri).await {
        Some(d) => d,
        None => {
            if dep_trace {
                tracing::info!(
                    request_uri = %uri,
                    "dependency trace: goto-definition skipped (document not in salsa)"
                );
            }
            return Ok(None);
        }
    };
    let offset = position_to_offset(&position, &doc.content);
    let word = extract_word_at_offset(&doc.content, offset);
    if !word.is_empty() {
        if let Some(response) = workspace_goto_definition(server, word, &uri).await {
            return Ok(Some(response));
        }
    }
    if dep_trace {
        tracing::info!(
            request_uri = %uri,
            "dependency trace: no goto-definition target found"
        );
    }
    Ok(None)
}

pub(crate) async fn references(
    server: &SysmlLanguageServer,
    params: ReferenceParams,
) -> Result<Option<Vec<Location>>> {
    let uri = params.text_document_position.text_document.uri.to_string();
    let position = params.text_document_position.position;

    // Service owns position resolution + workspace walk; LSP shell shapes
    // the result into LSP `Location`s.
    let hits = match server.service.references(&uri, position.line, position.character) {
        Ok(hits) => hits,
        Err(_) => return Ok(None),
    };

    if hits.is_empty() {
        return Ok(None);
    }

    let mut locations: Vec<Location> = Vec::with_capacity(hits.len());
    for hit in &hits {
        let Some(target_uri) = parse_uri(&hit.uri) else {
            continue;
        };
        locations.push(Location {
            uri: target_uri,
            range: tower_lsp::lsp_types::Range {
                start: tower_lsp::lsp_types::Position {
                    line: hit.line_start,
                    character: hit.col_start,
                },
                end: tower_lsp::lsp_types::Position {
                    line: hit.line_end,
                    character: hit.col_end,
                },
            },
        });
    }

    if locations.is_empty() {
        Ok(None)
    } else {
        Ok(Some(locations))
    }
}

pub(crate) async fn goto_type_definition(
    server: &SysmlLanguageServer,
    params: request::GotoTypeDefinitionParams,
) -> Result<Option<request::GotoTypeDefinitionResponse>> {
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

    // Find the type definition via FeatureTyping relationships
    let Some(type_name) = find_element_type(element, &doc.graph) else {
        return Ok(None);
    };

    // Find the type definition element
    let type_def = doc
        .graph
        .elements
        .values()
        .find(|e| e.kind.is_definition() && e.name.as_deref() == Some(type_name.as_str()));

    let Some(type_def) = type_def else {
        return Ok(None);
    };

    let span = match type_def.spans.first() {
        Some(s) if s.file != SYNTHETIC_FILE => s,
        _ => return Ok(None),
    };

    let target_uri =
        parse_uri(&span.file).unwrap_or_else(|| Url::parse("file:///unknown").unwrap());

    let target_content = if span.file == uri {
        doc.content.clone()
    } else {
        match tokio::fs::read_to_string(
            target_uri
                .to_file_path()
                .unwrap_or_else(|_| PathBuf::from(&span.file)),
        )
        .await
        {
            Ok(content) => content,
            Err(e) => {
                tracing::debug!(
                    path = %span.file,
                    target_uri = %target_uri,
                    error = %e,
                    "failed to read type-definition target file"
                );
                String::new()
            }
        }
    };

    let range = if !target_content.is_empty() {
        range_to_lsp_range(span.start, span.end, &target_content)
    } else {
        Range::default()
    };

    Ok(Some(request::GotoTypeDefinitionResponse::Scalar(
        Location {
            uri: target_uri,
            range,
        },
    )))
}

pub(crate) async fn goto_implementation(
    server: &SysmlLanguageServer,
    params: request::GotoImplementationParams,
) -> Result<Option<request::GotoImplementationResponse>> {
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

    // Find all elements that specialize this element (incoming Specialize relationships)
    let mut locations = Vec::new();
    for rel in doc.graph.incoming(&element_id) {
        if rel.kind == sysml_core::RelationshipKind::Specialize {
            if let Some(impl_elem) = doc.graph.get_element(&rel.source) {
                if let Some(span) = impl_elem.spans.first() {
                    if span.file == SYNTHETIC_FILE {
                        continue;
                    }
                    let Some(file_url) = parse_uri(&span.file) else {
                        continue;
                    };
                    if span.file == uri {
                        locations.push(Location {
                            uri: file_url,
                            range: range_to_lsp_range(span.start, span.end, &doc.content),
                        });
                    } else {
                        // Read cross-file content for accurate positions
                        match tokio::fs::read_to_string(
                            file_url
                                .to_file_path()
                                .unwrap_or_else(|_| PathBuf::from(&span.file)),
                        )
                        .await
                        {
                            Ok(content) => {
                                locations.push(Location {
                                    uri: file_url,
                                    range: range_to_lsp_range(span.start, span.end, &content),
                                });
                            }
                            Err(e) => {
                                tracing::debug!(
                                    path = %span.file,
                                    file_url = %file_url,
                                    error = %e,
                                    "failed to read implementation target file"
                                );
                                continue;
                            }
                        }
                    }
                }
            }
        }
    }

    if locations.is_empty() {
        Ok(None)
    } else {
        Ok(Some(request::GotoImplementationResponse::Array(locations)))
    }
}

async fn resolve_navigation_uri(server: &SysmlLanguageServer, span_file: &str) -> Option<Url> {
    let parsed = parse_uri(span_file)?;
    if parsed.scheme() != "project" {
        if telemetry_control::dependency_trace_enabled() {
            tracing::info!(
                span_file,
                resolved_uri = %parsed,
                "dependency trace: navigation URI resolved directly"
            );
        }
        return Some(parsed);
    }

    let project_id = parsed.host_str()?.parse::<u32>().ok().map(ProjectHandle)?;
    let relative_path = parsed.path().trim_start_matches('/');
    if relative_path.is_empty() {
        return None;
    }

    {
        let registry = server.service.project_registry().read().unwrap();
        if let Some(path) = registry.source_path(project_id, relative_path) {
            let resolved = Url::from_file_path(path).ok();
            if telemetry_control::dependency_trace_enabled() {
                if let Some(uri) = &resolved {
                    tracing::info!(
                        project_id = project_id.0,
                        relative_path,
                        resolved_uri = %uri,
                        "dependency trace: navigation URI resolved via project registry"
                    );
                }
            }
            return resolved;
        }
    }

    if let Some(fallback_uri) =
        resolve_project_relative_uri_from_tracked_files(server, relative_path).await
    {
        if telemetry_control::dependency_trace_enabled() {
            tracing::info!(
                project_id = project_id.0,
                relative_path,
                resolved_uri = %fallback_uri,
                "dependency trace: navigation URI resolved via tracked-file fallback"
            );
        } else {
            tracing::debug!(
                project_id = project_id.0,
                relative_path,
                resolved_uri = %fallback_uri,
                "resolved project URI via tracked-file fallback"
            );
        }
        return Some(fallback_uri);
    }

    if telemetry_control::dependency_trace_enabled() {
        tracing::info!(
            project_id = project_id.0,
            relative_path,
            span_file,
            "dependency trace: project URI unresolved"
        );
    }

    None
}

async fn resolve_project_relative_uri_from_tracked_files(
    server: &SysmlLanguageServer,
    relative_path: &str,
) -> Option<Url> {
    let relative_path = Path::new(relative_path);
    if relative_path.as_os_str().is_empty() {
        return None;
    }

    // Bind the snapshot to `_` so it drops IMMEDIATELY (a `_analysis`
    // binding lives to end of scope and blocks host mutations for the
    // whole walk — workspace.rs precedent).
    let (files, _) = server.salsa_all_files().await;
    let mut matches: Vec<String> = files
        .into_iter()
        .filter_map(|(uri, _)| {
            let parsed = Url::parse(&uri).ok()?;
            if parsed.scheme() != "file" {
                return None;
            }
            let file_path = parsed.to_file_path().ok()?;
            if file_path.ends_with(relative_path) {
                Some(uri)
            } else {
                None
            }
        })
        .collect();

    matches.sort();
    matches.dedup();
    if matches.len() == 1 {
        return Url::parse(&matches[0]).ok();
    }

    None
}

/// Extract the identifier word at the given byte offset in the source text.
fn extract_word_at_offset(content: &str, offset: usize) -> &str {
    let bytes = content.as_bytes();
    if offset >= bytes.len() {
        return "";
    }
    let start = (0..offset)
        .rev()
        .find(|&i| !bytes[i].is_ascii_alphanumeric() && bytes[i] != b'_')
        .map(|i| i + 1)
        .unwrap_or(0);
    let end = (offset..bytes.len())
        .find(|&i| !bytes[i].is_ascii_alphanumeric() && bytes[i] != b'_')
        .unwrap_or(bytes.len());
    &content[start..end]
}

/// Try to find a goto-definition target via the workspace snapshot index.
async fn workspace_goto_definition(
    server: &SysmlLanguageServer,
    word: &str,
    current_uri: &str,
) -> Option<GotoDefinitionResponse> {
    let ws = server.workspace_snapshot().await;
    let entries = ws.find_by_name(word);
    for entry in entries {
        // Skip entries from the current file (already searched above)
        if entry.uri == current_uri {
            continue;
        }
        let Some(target_uri) = resolve_navigation_uri(server, &entry.uri)
            .await
            .or_else(|| parse_uri(&entry.uri))
        else {
            tracing::debug!(
                entry_uri = %entry.uri,
                name = %word,
                "skipping workspace goto-definition candidate with invalid URI"
            );
            continue;
        };

        if target_uri.scheme() != "file" {
            tracing::debug!(
                entry_uri = %entry.uri,
                resolved_uri = %target_uri,
                name = %word,
                "skipping workspace goto-definition candidate with non-file URI"
            );
            continue;
        }

        let target_path = target_uri
            .to_file_path()
            .unwrap_or_else(|_| PathBuf::from(&entry.uri));
        if let Ok(content) = tokio::fs::read_to_string(&target_path).await {
            if telemetry_control::dependency_trace_enabled() {
                tracing::info!(
                    word,
                    current_uri,
                    candidate_uri = %entry.uri,
                    resolved_uri = %target_uri,
                    target_path = %target_path.display(),
                    "dependency trace: workspace goto-definition selected candidate"
                );
            }
            let range = range_to_lsp_range(entry.span_start, entry.span_end, &content);
            return Some(GotoDefinitionResponse::Scalar(Location {
                uri: target_uri,
                range,
            }));
        }
    }
    if telemetry_control::dependency_trace_enabled() {
        tracing::info!(
            word,
            current_uri,
            "dependency trace: workspace goto-definition found no candidate"
        );
    }
    None
}
