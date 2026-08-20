//! Document outline (symbol tree) — graph-based, salsa-memoized.
//!
//! Replaces the LSP-side `symbols::document_symbol` body. The LSP shell keeps
//! byte-offset → LSP `Range` conversion (needs file content) and the rare
//! CST-only fallback for files where the graph builder produced no roots.

use std::sync::Mutex;

use sysml_core::ElementKind;
use sysml_ide_db::{AnalysisHost, SymbolNode};

/// Service-side outline node. Mirrors `sysml_ide_db::SymbolNode` but is
/// `serde`-friendly so transports can serialize it as JSON.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OutlineNode {
    pub name: String,
    pub detail: Option<String>,
    pub kind: ElementKind,
    pub range_start: usize,
    pub range_end: usize,
    pub selection_start: usize,
    pub selection_end: usize,
    pub children: Vec<OutlineNode>,
}

impl From<&SymbolNode> for OutlineNode {
    fn from(node: &SymbolNode) -> Self {
        OutlineNode {
            name: node.name.clone(),
            detail: node.detail.clone(),
            kind: node.kind.clone(),
            range_start: node.range_start,
            range_end: node.range_end,
            selection_start: node.selection_start,
            selection_end: node.selection_end,
            children: node.children.iter().map(OutlineNode::from).collect(),
        }
    }
}

/// Compute the document outline for a loaded URI.
///
/// Lock the host briefly to resolve the file id and grab an `Analysis`
/// snapshot, then drop the lock before driving the salsa query — keeps
/// concurrent edits unblocked.
pub fn compute_outline(host: &Mutex<AnalysisHost>, uri: &str) -> Vec<OutlineNode> {
    let (analysis, source_file) = {
        let guard = host.lock().unwrap();
        let Some(file_id) = guard.file_id(uri) else {
            return Vec::new();
        };
        let Some(sf) = guard.source_file(file_id) else {
            return Vec::new();
        };
        (guard.analysis(), sf)
    };

    let tree = analysis.document_symbols(source_file);
    tree.symbols().iter().map(OutlineNode::from).collect()
}
