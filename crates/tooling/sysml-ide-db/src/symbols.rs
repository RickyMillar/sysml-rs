//! Document symbol tree: tracked query for nested symbol extraction.
//!
//! Builds a tree of `SymbolNode` from the parsed model graph, representing
//! the document's structure for IDE features like "Go to Symbol" and the
//! outline panel.
//!
//! The output type `DocumentSymbolTree` is a crate-local type (not LSP types)
//! so that sysml-ide-db has no dependency on tower-lsp.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::ElementKind;

use crate::parse;
use crate::source::SourceFile;
use crate::Db;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A node in the document symbol tree.
///
/// Mirrors the ownership hierarchy of the model graph. Contains byte-offset
/// ranges (not line/column) — the LSP layer converts to LSP positions.
#[derive(Debug, Clone)]
pub struct SymbolNode {
    /// Display name (element name or "<unnamed>").
    pub name: String,
    /// Optional detail (e.g., qualified name).
    pub detail: Option<String>,
    /// Element kind for icon/classification in the IDE.
    pub kind: ElementKind,
    /// Start byte offset of the full element span.
    pub range_start: usize,
    /// End byte offset of the full element span.
    pub range_end: usize,
    /// Start byte offset of the name/selection span.
    pub selection_start: usize,
    /// End byte offset of the name/selection span.
    pub selection_end: usize,
    /// Child symbols (owned members).
    pub children: Vec<SymbolNode>,
}

/// Document symbol tree for a file.
///
/// Arc-wrapped for cheap cloning in salsa. Uses pointer-identity equality.
#[derive(Clone, Debug)]
pub struct DocumentSymbolTree(Arc<Vec<SymbolNode>>);

impl DocumentSymbolTree {
    fn new(symbols: Vec<SymbolNode>) -> Self {
        Self(Arc::new(symbols))
    }

    /// Top-level symbols in the document.
    pub fn symbols(&self) -> &[SymbolNode] {
        &self.0
    }

    /// Whether the document has no symbols.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

salsa_arc_wrapper!(identity, DocumentSymbolTree, Vec<SymbolNode>);

// ---------------------------------------------------------------------------
// Query
// ---------------------------------------------------------------------------

/// Build a document symbol tree from the parsed model graph.
///
/// Walks root elements (no owner) and recursively builds child symbols
/// from owned members. Only includes elements with spans matching the
/// file's URI.
///
/// Depends on: `parse_file()` (Layer 1)
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_document_symbols(db: &dyn Db, source_file: SourceFile) -> DocumentSymbolTree {
    tracing::debug!(
        document_uri = source_file.name(db),
        "starting document symbol extraction"
    );
    let parsed = parse::parse_file(db, source_file);
    let graph = parsed.graph();
    let uri = source_file.name(db);

    let roots: Vec<_> = graph
        .elements
        .values()
        .filter(|e| e.owner.is_none() && e.spans.iter().any(|s| s.file == *uri))
        .collect();

    let symbols: Vec<SymbolNode> = roots
        .into_iter()
        .filter_map(|e| build_symbol_node(graph, &e.id, uri))
        .collect();

    DocumentSymbolTree::new(symbols)
}

/// Recursively build a SymbolNode from an element and its owned members.
fn build_symbol_node(
    graph: &sysml_core::ModelGraph,
    element_id: &sysml_id::ElementId,
    uri: &str,
) -> Option<SymbolNode> {
    let element = graph.get_element(element_id)?;

    let name = element
        .name
        .clone()
        .unwrap_or_else(|| "<unnamed>".to_owned());

    let span = element.spans.iter().find(|s| s.file == uri)?;

    // Selection range: name portion only (or full span if unnamed)
    let name_len = element.name.as_ref().map_or(0, |n| n.len());
    let name_end = (span.start + name_len).min(span.end);
    let (selection_start, selection_end) = if name_len > 0 {
        (span.start, name_end)
    } else {
        (span.start, span.end)
    };

    let children: Vec<SymbolNode> = graph
        .owned_members(element_id)
        .filter_map(|child| build_symbol_node(graph, &child.id, uri))
        .collect();

    let detail = element.qname.as_ref().map(|q| q.to_string());

    Some(SymbolNode {
        name,
        detail,
        kind: element.kind.clone(),
        range_start: span.start,
        range_end: span.end,
        selection_start,
        selection_end,
        children,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootDatabase;

    #[test]
    fn symbols_simple_package() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let tree = file_document_symbols(&db, sf);
        assert!(!tree.is_empty());
        assert_eq!(tree.symbols()[0].name, "Foo");
    }

    #[test]
    fn symbols_nested_elements() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(
            &db,
            "test.sysml".to_string(),
            "package Vehicle {\n  part def Engine;\n  part engine : Engine;\n}".to_string(),
        );
        let tree = file_document_symbols(&db, sf);
        assert!(!tree.is_empty());
        let vehicle = &tree.symbols()[0];
        assert_eq!(vehicle.name, "Vehicle");
        // Should have children (Engine def and engine usage)
        assert!(
            !vehicle.children.is_empty(),
            "Vehicle should have child symbols"
        );
    }

    #[test]
    fn symbols_empty_file() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), String::new());
        let tree = file_document_symbols(&db, sf);
        assert!(tree.is_empty());
    }

    #[test]
    fn symbols_memoization() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let t1 = file_document_symbols(&db, sf);
        let t2 = file_document_symbols(&db, sf);
        assert_eq!(t1, t2, "Memoized results should be pointer-equal");
    }
}
