//! Type hierarchy handlers (prepare, supertypes, subtypes).

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
use tower_lsp::lsp_types::{TypeHierarchyPrepareParams, TypeHierarchyItem, Url, TypeHierarchySupertypesParams, TypeHierarchySubtypesParams};

use crate::kinds::element_kind_to_symbol_kind;
use crate::read_file_content_for_uri;
use crate::types::SYNTHETIC_FILE;
use crate::utils::{parse_uri, position_to_offset, range_to_lsp_range};
use crate::SysmlLanguageServer;

pub(crate) async fn prepare_type_hierarchy(
    server: &SysmlLanguageServer,
    params: TypeHierarchyPrepareParams,
) -> Result<Option<Vec<TypeHierarchyItem>>> {
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

    // Type hierarchy only applies to definitions
    if !element.kind.is_definition() && element.kind != sysml_core::ElementKind::Package {
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

    Ok(Some(vec![TypeHierarchyItem {
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

pub(crate) async fn supertypes(
    server: &SysmlLanguageServer,
    params: TypeHierarchySupertypesParams,
) -> Result<Option<Vec<TypeHierarchyItem>>> {
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
                "invalid type hierarchy element id"
            );
            return Ok(None);
        }
    };

    let mut items = Vec::new();

    // Cache for cross-file content reads
    let mut cross_file_cache: HashMap<String, String> = HashMap::new();

    // Find Specialize relationships where this element is the source (specific)
    for rel in doc.graph.outgoing(&element_id) {
        if rel.kind == sysml_core::RelationshipKind::Specialize {
            if let Some(supertype) = doc.graph.get_element(&rel.target) {
                if let Some(name) = &supertype.name {
                    let span = match supertype.spans.first() {
                        Some(s) if s.file != SYNTHETIC_FILE => s,
                        _ => continue,
                    };
                    let span_file = span.file.clone();
                    let file_url = parse_uri(&span_file)
                        .unwrap_or_else(|| Url::parse("file:///unknown").unwrap());
                    let content: &str = if span_file == uri {
                        &doc.content
                    } else {
                        // Cross-file: read the source file for accurate range calc
                        if !cross_file_cache.contains_key(&span_file) {
                            if let Some(cross_doc) = server.salsa_doc(&span_file).await {
                                cross_file_cache.insert(span_file.clone(), cross_doc.content);
                            } else if let Some(text) = read_file_content_for_uri(&span_file) {
                                cross_file_cache.insert(span_file.clone(), text);
                            }
                        }
                        match cross_file_cache.get(&span_file) {
                            Some(c) => c.as_str(),
                            None => continue,
                        }
                    };
                    let range = range_to_lsp_range(span.start, span.end, content);

                    items.push(TypeHierarchyItem {
                        name: name.clone(),
                        kind: element_kind_to_symbol_kind(&supertype.kind),
                        tags: None,
                        detail: supertype.qname.as_ref().map(|q| q.to_string()),
                        uri: file_url,
                        range,
                        selection_range: range,
                        data: Some(serde_json::json!({
                            "elementId": supertype.id.to_string(),
                            "uri": span_file,
                        })),
                    });
                }
            }
        }
    }

    if items.is_empty() {
        Ok(None)
    } else {
        Ok(Some(items))
    }
}

pub(crate) async fn subtypes(
    server: &SysmlLanguageServer,
    params: TypeHierarchySubtypesParams,
) -> Result<Option<Vec<TypeHierarchyItem>>> {
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
                "invalid type hierarchy element id"
            );
            return Ok(None);
        }
    };

    let mut items = Vec::new();

    // Cache for cross-file content reads
    let mut cross_file_cache: HashMap<String, String> = HashMap::new();

    // Find Specialize relationships where this element is the target (general)
    for rel in doc.graph.incoming(&element_id) {
        if rel.kind == sysml_core::RelationshipKind::Specialize {
            if let Some(subtype) = doc.graph.get_element(&rel.source) {
                if let Some(name) = &subtype.name {
                    let span = match subtype.spans.first() {
                        Some(s) if s.file != SYNTHETIC_FILE => s,
                        _ => continue,
                    };
                    let span_file = span.file.clone();
                    let file_url = parse_uri(&span_file)
                        .unwrap_or_else(|| Url::parse("file:///unknown").unwrap());
                    let content: &str = if span_file == uri {
                        &doc.content
                    } else {
                        // Cross-file: read the source file for accurate range calc
                        if !cross_file_cache.contains_key(&span_file) {
                            if let Some(cross_doc) = server.salsa_doc(&span_file).await {
                                cross_file_cache.insert(span_file.clone(), cross_doc.content);
                            } else if let Some(text) = read_file_content_for_uri(&span_file) {
                                cross_file_cache.insert(span_file.clone(), text);
                            }
                        }
                        match cross_file_cache.get(&span_file) {
                            Some(c) => c.as_str(),
                            None => continue,
                        }
                    };
                    let range = range_to_lsp_range(span.start, span.end, content);

                    items.push(TypeHierarchyItem {
                        name: name.clone(),
                        kind: element_kind_to_symbol_kind(&subtype.kind),
                        tags: None,
                        detail: subtype.qname.as_ref().map(|q| q.to_string()),
                        uri: file_url,
                        range,
                        selection_range: range,
                        data: Some(serde_json::json!({
                            "elementId": subtype.id.to_string(),
                            "uri": span_file,
                        })),
                    });
                }
            }
        }
    }

    if items.is_empty() {
        Ok(None)
    } else {
        Ok(Some(items))
    }
}
