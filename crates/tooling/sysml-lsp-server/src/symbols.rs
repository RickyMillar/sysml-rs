//! Document/workspace symbol building.
//!
//! Converts model graph elements into nested LSP DocumentSymbol trees.
//! Also contains the extracted `document_symbol` and `symbol` handler implementations.

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

use std::path::PathBuf;

use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::{DocumentSymbolParams, DocumentSymbolResponse, DocumentSymbol, WorkspaceSymbolParams, SymbolInformation, Location, Range};

#[cfg(test)]
use sysml_core::ModelGraph;
use sysml_ide_db::Cancelled;
#[cfg(test)]
use sysml_id::ElementId;
use sysml_service::outline::OutlineNode;

use crate::kinds::element_kind_to_symbol_kind;
use crate::types::SYNTHETIC_FILE;
use crate::utils::{fuzzy_match, parse_uri, range_to_lsp_range, score_completion};
use crate::{outline_item_to_document_symbol, SysmlLanguageServer};

/// Build nested document symbols from the graph.
#[cfg(test)]
#[tracing::instrument(level = "debug", skip(graph, content))]
pub(crate) fn build_nested_symbols(
    graph: &ModelGraph,
    uri: &str,
    content: &str,
) -> Vec<DocumentSymbol> {
    // Find root elements (no owner) in this file
    let roots: Vec<_> = graph
        .elements
        .values()
        .filter(|e| {
            e.owner.is_none()
                && e.spans
                    .iter()
                    .any(|s| s.file == uri && s.file != SYNTHETIC_FILE)
        })
        .collect();

    // Build symbols recursively
    fn build_symbol(
        graph: &ModelGraph,
        element_id: &ElementId,
        uri: &str,
        content: &str,
    ) -> Option<DocumentSymbol> {
        let element = graph.get_element(element_id)?;

        let name = element
            .name
            .clone()
            .unwrap_or_else(|| "<unnamed>".to_string());

        let span = element
            .spans
            .iter()
            .find(|s| s.file == uri && s.file != SYNTHETIC_FILE)?;

        let range = range_to_lsp_range(span.start, span.end, content);

        // selection_range should be the name portion only, not the full element
        let name_len = element.name.as_ref().map_or(0, |n| n.len());
        let name_end = (span.start + name_len).min(span.end);
        let selection_range = if name_len > 0 {
            range_to_lsp_range(span.start, name_end, content)
        } else {
            range
        };

        // Build children
        let children: Vec<DocumentSymbol> = graph
            .owned_members(element_id)
            .filter_map(|child| build_symbol(graph, &child.id, uri, content))
            .collect();

        let detail = element.qname.as_ref().map(|q| q.to_string());

        #[allow(deprecated)]
        Some(DocumentSymbol {
            name,
            detail,
            kind: element_kind_to_symbol_kind(&element.kind),
            tags: None,
            deprecated: None,
            range,
            selection_range,
            children: if children.is_empty() {
                None
            } else {
                Some(children)
            },
        })
    }

    roots
        .into_iter()
        .filter_map(|e| build_symbol(graph, &e.id, uri, content))
        .collect()
}

pub(crate) async fn document_symbol(
    server: &SysmlLanguageServer,
    params: DocumentSymbolParams,
) -> Result<Option<DocumentSymbolResponse>> {
    let uri = params.text_document.uri.to_string();

    // Extract the owned content, then DROP the snapshot before calling the
    // service: `service.outline` re-locks the shared host, and holding an
    // `Analysis` (salsa db clone) across that re-lock deadlocks against a
    // concurrent mutation (did_change / load_workspace parks in a salsa
    // setter holding the Mutex until every clone drops — the 2026-07-17
    // wedge; see the lock-order invariant on `SysmlService::host_analysis`
    // and the fixed precedent in diagnostics.rs).
    let (sf, analysis) = match server.salsa_file_context(&uri).await {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let content = match Cancelled::catch(std::panic::AssertUnwindSafe(|| {
        analysis.file_text(sf).to_owned()
    })) {
        Ok(c) => c,
        Err(_) => return Ok(None),
    };
    drop(analysis);

    // Graph-based outline lives in the service (salsa-memoized).
    let nodes = server.service.outline(&uri).unwrap_or_default();
    if !nodes.is_empty() {
        let symbols = nodes
            .iter()
            .map(|node| outline_node_to_document_symbol(node, &content))
            .collect();
        return Ok(Some(DocumentSymbolResponse::Nested(symbols)));
    }

    // Fall back to CST-based outline when the model graph produced no
    // roots — e.g. the file is severely malformed but tree-sitter still
    // recovered a tree. Service has no CST surface, so this stays in
    // the LSP shell. Re-snapshot: the earlier one was dropped before the
    // service call (see above).
    let (sf, analysis) = match server.salsa_file_context(&uri).await {
        Some(ctx) => ctx,
        None => return Ok(None),
    };
    let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
        let outline = analysis.outline(sf);
        let symbols: Vec<DocumentSymbol> = outline
            .items()
            .iter()
            .map(|item| outline_item_to_document_symbol(item, &content))
            .collect();
        DocumentSymbolResponse::Nested(symbols)
    }));

    Ok(result.ok())
}

/// Convert a service-side `OutlineNode` to an LSP `DocumentSymbol`.
fn outline_node_to_document_symbol(node: &OutlineNode, content: &str) -> DocumentSymbol {
    let range = range_to_lsp_range(node.range_start, node.range_end, content);
    let selection_range = range_to_lsp_range(node.selection_start, node.selection_end, content);
    let children = if node.children.is_empty() {
        None
    } else {
        Some(
            node.children
                .iter()
                .map(|child| outline_node_to_document_symbol(child, content))
                .collect(),
        )
    };
    #[allow(deprecated)]
    DocumentSymbol {
        name: node.name.clone(),
        detail: node.detail.clone(),
        kind: element_kind_to_symbol_kind(&node.kind),
        tags: None,
        deprecated: None,
        range,
        selection_range,
        children,
    }
}

pub(crate) async fn symbol(
    server: &SysmlLanguageServer,
    params: WorkspaceSymbolParams,
) -> Result<Option<Vec<SymbolInformation>>> {
    let query = params.query.to_lowercase();

    let mut symbols = Vec::new();
    let mut seen_ids = std::collections::HashSet::new();

    // Search open documents — delegate per-URI element enumeration to the
    // service. The LSP shell still owns fuzzy matching, scoring, and the
    // span→LSP-range conversion (which needs file content).
    //
    // Extract every file's owned content FIRST, then drop the snapshot
    // before the service loop: `service.find` re-locks the shared host,
    // and holding an `Analysis` across that re-lock deadlocks against a
    // concurrent mutation (the 2026-07-17 wedge; lock-order invariant on
    // `SysmlService::host_analysis`).
    let (files, analysis) = server.salsa_all_files().await;
    let files_with_content: Vec<(String, String)> = files
        .iter()
        .filter_map(|(file_uri, sf)| {
            Cancelled::catch(std::panic::AssertUnwindSafe(|| {
                analysis.file_text(*sf).to_owned()
            }))
            .ok()
            .map(|content| (file_uri.clone(), content))
        })
        .collect();
    drop(analysis);
    for (file_uri, content) in &files_with_content {
        let elements = match server.service.find(file_uri, "", None) {
            Ok(els) => els,
            Err(_) => continue,
        };

        for element in &elements {
            let Some(name) = &element.name else {
                continue;
            };

            if !query.is_empty() && !fuzzy_match(&query, &name.to_lowercase()) {
                continue;
            }

            let span = match element.spans.first() {
                Some(s) if s.file != SYNTHETIC_FILE => s,
                _ => continue,
            };

            let Some(file_url) = parse_uri(&span.file) else {
                continue;
            };

            if span.file != *file_uri {
                continue;
            }

            let range = range_to_lsp_range(span.start, span.end, content);

            seen_ids.insert(element.id.clone());

            #[allow(deprecated)]
            let symbol = SymbolInformation {
                name: name.clone(),
                kind: element_kind_to_symbol_kind(&element.kind),
                tags: None,
                deprecated: None,
                location: Location {
                    uri: file_url,
                    range,
                },
                container_name: element.qname.as_ref().and_then(|q| {
                    let parts = q.segments();
                    if parts.len() > 1 {
                        Some(parts[..parts.len() - 1].join("::"))
                    } else {
                        None
                    }
                }),
            };

            symbols.push(symbol);
        }
    }

    // Also search workspace snapshot for elements not in open documents
    let ws = server.workspace_snapshot().await;
    ws.for_each_name(|name, entries| {
        if !query.is_empty() && !fuzzy_match(&query, &name.to_lowercase()) {
            return;
        }
        for entry in entries {
            if seen_ids.contains(&entry.element_id) {
                continue;
            }
            seen_ids.insert(entry.element_id.clone());

            let Some(file_url) = parse_uri(&entry.uri) else {
                continue;
            };

            // Read file content for accurate position conversion.
            // Note: blocking read inside sync closure — acceptable because
            // workspace symbol is a cold-path handler (user-triggered search).
            let range = match std::fs::read_to_string(
                file_url
                    .to_file_path()
                    .unwrap_or_else(|_| PathBuf::from(&entry.uri)),
            ) {
                Ok(content) => range_to_lsp_range(entry.span_start, entry.span_end, &content),
                Err(e) => {
                    tracing::debug!(
                        path = %entry.uri,
                        file_url = %file_url,
                        error = %e,
                        "failed to read cross-file symbol content"
                    );
                    Range::default()
                }
            };

            #[allow(deprecated)]
            let symbol = SymbolInformation {
                name: name.to_owned(),
                kind: element_kind_to_symbol_kind(&entry.element_kind),
                tags: None,
                deprecated: None,
                location: Location {
                    uri: file_url,
                    range,
                },
                container_name: None,
            };

            symbols.push(symbol);
        }
    });

    if symbols.is_empty() {
        return Ok(None);
    }

    // Score and rank symbols
    if !query.is_empty() {
        symbols.sort_by(|a, b| {
            let score_a = score_completion(&query, &a.name.to_lowercase());
            let score_b = score_completion(&query, &b.name.to_lowercase());
            score_b.cmp(&score_a)
        });
    }

    // Cap results to 100
    symbols.truncate(100);

    Ok(Some(symbols))
}
