//! Diagram manager: open diagram tracking, expansion state, and graph cache.

use std::collections::HashSet;
use std::sync::Arc;

use dashmap::DashMap;
use sysml_core::ModelGraph;
use sysml_diagram::smodel::ViewType;

/// Owns the open diagrams, their view types, and node expansion state.
pub struct DiagramManager {
    /// Track which documents have open diagrams and their view type.
    pub open_diagrams: DashMap<String, ViewType>,
    /// Track which diagram nodes are expanded per document URI.
    pub expanded_nodes: DashMap<String, HashSet<String>>,
    /// Cached elaborated graph per URI — avoids re-elaboration on expand/collapse.
    /// Invalidated when the file content changes (via `invalidate_graph_cache`).
    ///
    /// Values are `Arc`-wrapped so that retrieval is a cheap pointer clone
    /// instead of a full graph copy.
    pub graph_cache: DashMap<String, Arc<ModelGraph>>,
}

impl DiagramManager {
    pub fn new() -> Self {
        Self {
            open_diagrams: DashMap::new(),
            expanded_nodes: DashMap::new(),
            graph_cache: DashMap::new(),
        }
    }

    /// Invalidate the cached graph for a URI (call when file content changes).
    pub fn invalidate_graph_cache(&self, uri: &str) {
        self.graph_cache.remove(uri);
    }
}

impl Default for DiagramManager {
    fn default() -> Self {
        Self::new()
    }
}
