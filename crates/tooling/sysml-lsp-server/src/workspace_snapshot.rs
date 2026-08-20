//! On-demand workspace snapshot derived from salsa queries.
//!
//! Replaces the old `CrossFileIndex` which was dual-written alongside salsa.
//! This struct is built from salsa's memoized per-file queries, ensuring
//! the workspace data always reflects the latest state without manual
//! synchronization.

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

use sysml_core::ModelGraph;
use sysml_ide_db::Cancelled;

use crate::background::CrossFileEntry;
use crate::types::SYNTHETIC_FILE;
use crate::utils::parse_uri;

/// On-demand snapshot of workspace-wide element data.
///
/// Built by iterating all files in the `AnalysisHost` and collecting
/// element names and qualified names from their parse trees (cached by salsa).
pub(crate) struct WorkspaceSnapshot {
    by_name: HashMap<String, Vec<CrossFileEntry>>,
    by_qname: HashMap<String, CrossFileEntry>,
}

impl WorkspaceSnapshot {
    /// Build from all files in the AnalysisHost.
    ///
    /// Per-file parse queries are memoized by salsa, so this is cheap
    /// when files haven't changed since the last query.
    pub fn from_host(host: &sysml_ide_db::AnalysisHost) -> Self {
        let analysis = host.analysis();
        let mut by_name: HashMap<String, Vec<CrossFileEntry>> = HashMap::new();
        let mut by_qname: HashMap<String, CrossFileEntry> = HashMap::new();

        for file_id in host.files().file_ids() {
            let Some(uri) = host.files().uri(file_id) else {
                continue;
            };
            let Some(sf) = host.source_file(file_id) else {
                continue;
            };

            let graph = match Cancelled::catch(std::panic::AssertUnwindSafe(|| {
                analysis.parse_file(sf).graph().clone()
            })) {
                Ok(g) => g,
                Err(_) => continue,
            };

            index_file_graph(&mut by_name, &mut by_qname, uri, &graph);
        }

        // Drop the Analysis snapshot before returning so the host isn't blocked.
        drop(analysis);

        WorkspaceSnapshot { by_name, by_qname }
    }

    /// Find element locations by name.
    pub fn find_by_name(&self, name: &str) -> &[CrossFileEntry] {
        self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Find element location by qualified name.
    pub fn find_by_qname(&self, qname: &str) -> Option<&CrossFileEntry> {
        self.by_qname.get(qname)
    }

    /// Iterate over all names, calling the callback with each name and its locations.
    pub fn for_each_name(&self, mut f: impl FnMut(&str, &[CrossFileEntry])) {
        for (name, entries) in &self.by_name {
            f(name, entries);
        }
    }

    /// Iterate direct namespace members by qualified-name prefix.
    pub fn for_each_qname_member(
        &self,
        namespace_path: &str,
        mut f: impl FnMut(&str, &CrossFileEntry),
    ) {
        let prefix = format!("{namespace_path}::");
        for (qname, entry) in &self.by_qname {
            let Some(remainder) = qname.strip_prefix(&prefix) else {
                continue;
            };
            if remainder.is_empty() || remainder.contains("::") {
                continue;
            }
            f(remainder, entry);
        }
    }

}

fn index_file_graph(
    by_name: &mut HashMap<String, Vec<CrossFileEntry>>,
    by_qname: &mut HashMap<String, CrossFileEntry>,
    uri: &str,
    graph: &ModelGraph,
) {
    for element in graph.elements.values() {
        // Skip synthetic and foreign elements.
        if let Some(span) = element.spans.first() {
            if span.file == SYNTHETIC_FILE {
                continue;
            }
            if !span_belongs_to_uri(&span.file, uri) {
                continue;
            }
        }

        let (span_start, span_end) = element
            .spans
            .first()
            .map(|s| (s.start, s.end))
            .unwrap_or((0, 0));

        if let Some(name) = &element.name {
            let entry = CrossFileEntry {
                uri: uri.to_owned(),
                element_id: element.id.clone(),
                element_kind: element.kind.clone(),
                span_start,
                span_end,
            };
            by_name.entry(name.clone()).or_default().push(entry.clone());

            // Compute qname on-the-fly: element.qname is only set after
            // resolution, but the snapshot uses pre-resolution graphs.
            // Fall back to build_qualified_name() which walks the ownership
            // hierarchy to construct the qname from ancestor names.
            let qname = element.qname.as_ref().map(|q| q.to_string()).or_else(|| {
                graph
                    .build_qualified_name(&element.id)
                    .map(|q| q.to_string())
            });
            if let Some(qname_str) = qname {
                by_qname.insert(qname_str, entry);
            }
        }
    }
}

fn span_belongs_to_uri(span_file: &str, uri: &str) -> bool {
    if span_file.is_empty() || span_file == uri {
        return true;
    }
    match (parse_uri(span_file), parse_uri(uri)) {
        (Some(span_uri), Some(doc_uri)) => span_uri == doc_uri,
        _ => false,
    }
}
