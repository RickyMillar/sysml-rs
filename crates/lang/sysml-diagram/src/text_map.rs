//! `ElementId ↔ Span` text-map (Bucket 1.6) — the bidirectional text↔diagram link.
//!
//! The text-map projects each model element's primary source span into a compact,
//! serializable form keyed by the **same string id the scene nodes use**
//! (`ElementId::to_string()`), so a frontend can join `DiagramNode::element_id`
//! directly against it:
//!
//! - **diagram → text** (go-to-source): given a clicked node's `element_id`,
//!   [`TextMap::span_for`] returns its source location.
//! - **text → diagram** (cursor-follows): given an editor `(file, offset)`,
//!   [`TextMap::element_at`] returns the innermost element whose span contains it.
//!
//! This is the typed replacement for the `source_uri` / `source_range` strings
//! that Bucket 1.2 left on `DiagramNode`. It is built once per graph (a separate,
//! cheaper salsa query than the per-view diagram) and the [`crate::ViewModel`]
//! holds a shared `Arc` to it.

use std::collections::HashMap;

use sysml_core::ModelGraph;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A source span for one element, in the compact wire form the frontend needs.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextSpan {
    /// Source file URI/path.
    pub file: String,
    /// Byte offset of the span start.
    pub start: u32,
    /// Byte offset of the span end (exclusive).
    pub end: u32,
    /// 1-based start line, when known.
    pub line: u32,
    /// Start column, when known.
    pub col: u32,
}

/// Map from a scene node id (`ElementId::to_string()`) to its source span.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TextMap {
    spans: HashMap<String, TextSpan>,
}

impl TextMap {
    /// Forward lookup: the source span for a scene node id (diagram → text).
    pub fn span_for(&self, element_id: &str) -> Option<&TextSpan> {
        self.spans.get(element_id)
    }

    /// Reverse lookup: the **innermost** element whose span contains
    /// `(file, offset)` (text → diagram). Ties broken toward the smallest span.
    pub fn element_at(&self, file: &str, offset: u32) -> Option<&str> {
        self.spans
            .iter()
            .filter(|(_, s)| s.file == file && offset >= s.start && offset < s.end)
            .min_by_key(|(_, s)| s.end.saturating_sub(s.start))
            .map(|(id, _)| id.as_str())
    }

    /// Number of mapped elements.
    pub fn len(&self) -> usize {
        self.spans.len()
    }

    /// Whether the map is empty.
    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Iterate `(node_id, span)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&String, &TextSpan)> {
        self.spans.iter()
    }

    /// A copy of this map retaining only the entries whose id is in `keep`.
    ///
    /// The map is built once per graph and carries a span for **every** element
    /// in the workspace; export paths that serialize one view's `ViewModel`
    /// scope it down to the ids the scene / non-graph payload actually
    /// references (see `ViewModel::pruned_to_referenced`).
    /// Rewrite every span whose `file` is a `file://` URI (or absolute path)
    /// under one of the labeled `roots` to `label + relative-path` (first
    /// match wins). An absolute path matching no root is reduced to
    /// `<external>/<file-name>` — a serialized text map is an export
    /// artifact and must never carry a machine's directory layout.
    /// Already-relative files are left untouched.
    pub fn relativize_files(&mut self, roots: &[(&std::path::Path, &str)]) {
        for span in self.spans.values_mut() {
            let fs = span.file.strip_prefix("file://").unwrap_or(&span.file);
            let path = std::path::Path::new(fs);
            if path.is_relative() {
                continue;
            }
            if let Some(rewritten) = roots.iter().find_map(|(root, label)| {
                path.strip_prefix(root)
                    .ok()
                    .map(|rel| format!("{label}{}", rel.to_string_lossy()))
            }) {
                span.file = rewritten;
            } else {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();
                span.file = format!("<external>/{name}");
            }
        }
    }

    pub fn retained(&self, keep: &std::collections::HashSet<String>) -> TextMap {
        TextMap {
            spans: self
                .spans
                .iter()
                .filter(|(id, _)| keep.contains(*id))
                .map(|(id, span)| (id.clone(), span.clone()))
                .collect(),
        }
    }
}

/// Build the [`TextMap`] for a model graph. Pure function of the graph — keyed by
/// `ElementId::to_string()` to match scene node ids. Elements without a source
/// span are omitted (fail-soft only on genuinely span-less synthetic elements).
pub fn build_text_map(graph: &ModelGraph) -> TextMap {
    let mut spans = HashMap::new();
    for (id, element) in &graph.elements {
        let Some(span) = element.spans.first() else {
            continue;
        };
        spans.insert(
            id.to_string(),
            TextSpan {
                file: span.file.clone(),
                start: span.start as u32,
                end: span.end as u32,
                line: span.line.unwrap_or(0),
                col: span.col.unwrap_or(0),
            },
        );
    }
    TextMap { spans }
}
