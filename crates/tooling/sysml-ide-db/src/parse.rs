//! Parse layer: tracked queries that convert source text into parsed results.
//!
//! This is Layer 1 of the salsa query hierarchy. Each query here depends on
//! the source text (Layer 0) and produces parsed artifacts.
//!
//! ## Design decisions
//!
//! - `tree_sitter::Tree` is Send+Sync (as of 0.22.6). It is stored in salsa
//!   via CachedTree (Arc wrapper with pointer-identity Hash/Eq).
//!
//! - We wrap results in `Arc<>` for efficient sharing. `ParseResult` uses a
//!   content fingerprint (graph + diagnostics), so salsa can detect meaningful
//!   parse-result changes while avoiding stale diagnostics reuse.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use sysml_core::ModelGraph;
use sysml_id::ElementId;
use sysml_span::Diagnostic;
use sysml_parser_incremental::{FastParser, OutlineItem, SyntaxNode, TreeSitterParser};

use crate::source::SourceFile;
use crate::Db;

// ---------------------------------------------------------------------------
// Result wrapper types (Arc-based, for salsa compatibility)
// ---------------------------------------------------------------------------

/// Parsed file result: model graph + diagnostics.
///
/// Wrapped in Arc for cheap cloning. PartialEq/Hash use content fingerprinting.
#[derive(Clone, Debug)]
pub struct ParseResult(Arc<ParseResultData>);

#[derive(Debug)]
struct ParseResultData {
    graph: ModelGraph,
    diagnostics: Vec<Diagnostic>,
    element_count: usize,
    has_errors: bool,
    fingerprint: u64,
}

fn hash_diagnostic<H: Hasher>(diag: &Diagnostic, state: &mut H) {
    diag.severity.hash(state);
    diag.code.hash(state);
    diag.message.hash(state);
    diag.span.hash(state);
    diag.notes.hash(state);
    diag.tags.hash(state);
    diag.related.len().hash(state);
    for related in &diag.related {
        related.span.hash(state);
        related.message.hash(state);
    }
}

impl ParseResult {
    fn new(graph: ModelGraph, diagnostics: Vec<Diagnostic>) -> Self {
        let element_count = graph.elements.len();
        let has_errors = diagnostics.iter().any(|d| d.is_error());
        let fingerprint = {
            use std::collections::hash_map::DefaultHasher;
            let mut h = DefaultHasher::new();
            graph.fingerprint().hash(&mut h);
            diagnostics.len().hash(&mut h);
            for diag in &diagnostics {
                hash_diagnostic(diag, &mut h);
            }
            h.finish()
        };
        Self(Arc::new(ParseResultData {
            graph,
            diagnostics,
            element_count,
            has_errors,
            fingerprint,
        }))
    }

    pub fn graph(&self) -> &ModelGraph {
        &self.0.graph
    }

    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.0.diagnostics
    }

    pub fn element_count(&self) -> usize {
        self.0.element_count
    }

    pub fn has_errors(&self) -> bool {
        self.0.has_errors
    }
}

salsa_arc_wrapper!(fingerprint, ParseResult, ParseResultData);

/// File outline result.
#[derive(Clone, Debug)]
pub struct Outline(Arc<Vec<OutlineItem>>);

impl Outline {
    fn new(items: Vec<OutlineItem>) -> Self {
        Self(Arc::new(items))
    }

    pub fn items(&self) -> &[OutlineItem] {
        &self.0
    }
}

salsa_arc_wrapper!(identity, Outline, Vec<OutlineItem>);

/// CST result (our SyntaxNode tree, which IS Send unlike tree_sitter::Tree).
#[derive(Clone, Debug)]
pub struct Cst(Arc<SyntaxNode>);

impl Cst {
    fn new(root: SyntaxNode) -> Self {
        Self(Arc::new(root))
    }

    pub fn root(&self) -> &SyntaxNode {
        &self.0
    }
}

salsa_arc_wrapper!(identity, Cst, SyntaxNode);

/// Tree-sitter Tree wrapped for salsa compatibility.
///
/// tree_sitter::Tree is Send+Sync (as of 0.22.6) but lacks Hash+Eq.
/// We wrap in Arc and use pointer-identity equality, matching the
/// pattern used by ParseResult, Cst, Outline, and PositionMap.
#[derive(Clone, Debug)]
pub struct CachedTree(Arc<tree_sitter::Tree>);

impl CachedTree {
    pub fn new(tree: tree_sitter::Tree) -> Self {
        Self(Arc::new(tree))
    }

    /// Get the underlying tree-sitter Tree reference.
    pub fn tree(&self) -> &tree_sitter::Tree {
        &self.0
    }

    /// Get the root node of the tree.
    pub fn root_node(&self) -> tree_sitter::Node<'_> {
        self.0.root_node()
    }

    /// Create a TreeCursor for walking the tree.
    pub fn walk(&self) -> tree_sitter::TreeCursor<'_> {
        self.0.walk()
    }
}

salsa_arc_wrapper!(identity, CachedTree, tree_sitter::Tree);

/// Position-to-element mapping for a parsed file.
///
/// Maps byte offsets to ElementIds, enabling efficient lookup of which
/// element is at a given cursor position. Used by LSP handlers for
/// hover, goto-definition, and other position-dependent features.
#[derive(Clone, Debug)]
pub struct PositionMap(Arc<PositionMapData>);

#[derive(Debug)]
struct PositionMapData {
    /// Sorted list of position entries (by start offset, then widest first).
    entries: Vec<PositionEntry>,
}

#[derive(Debug, Clone)]
struct PositionEntry {
    start: usize,
    end: usize,
    element_id: ElementId,
    is_definition: bool,
}

impl PositionMap {
    /// Build a position map from a parsed model graph.
    ///
    /// Collects all element spans matching the given file name and sorts
    /// them for efficient lookup.
    pub fn from_graph(graph: &ModelGraph, file_name: &str) -> Self {
        let mut entries = Vec::new();
        for element in graph.elements.values() {
            for (idx, span) in element.spans.iter().enumerate() {
                if span.file == file_name && span.start != span.end {
                    entries.push(PositionEntry {
                        start: span.start,
                        end: span.end,
                        element_id: element.id.clone(),
                        is_definition: idx == 0,
                    });
                }
            }
        }
        entries.sort_by_key(|e| (e.start, std::cmp::Reverse(e.end)));
        Self(Arc::new(PositionMapData { entries }))
    }

    /// Find the element at a given byte offset.
    ///
    /// Returns the innermost (narrowest span) element containing the offset,
    /// along with whether this is a definition site.
    ///
    #[allow(clippy::indexing_slicing)] // partition_point guarantees valid range
    /// Uses binary search to skip entries that start after `offset` (O(log n)
    /// to find the partition point, then backward scan over candidates).
    pub fn element_at(&self, offset: usize) -> Option<(&ElementId, bool)> {
        let entries = &self.0.entries;
        // Entries are sorted by (start, Reverse(end)).
        // Find the first index where start > offset — all candidates are before it.
        let partition = entries.partition_point(|e| e.start <= offset);
        // Scan backwards from partition to find the innermost span containing offset.
        let mut best: Option<&PositionEntry> = None;
        for entry in entries[..partition].iter().rev() {
            if entry.end <= offset {
                // Entries are sorted by start ascending; earlier entries with
                // start < entry.start also have end <= entry.end (widest first),
                // so if this entry doesn't contain offset, no earlier entry
                // starting at the same or lower offset can either — unless a
                // later-starting entry is narrower. We must keep scanning
                // until start changes and the span is clearly too early.
                //
                // Optimization: if we already have a best match and this entry's
                // start is well before the offset, entries even further back
                // can only be wider, so stop early.
                if best.is_some() {
                    break;
                }
                continue;
            }
            // entry.start <= offset && offset < entry.end — this entry contains offset
            match best {
                None => best = Some(entry),
                Some(prev) => {
                    let prev_width = prev.end - prev.start;
                    let entry_width = entry.end - entry.start;
                    if entry_width < prev_width {
                        best = Some(entry);
                    }
                }
            }
        }
        best.map(|e| (&e.element_id, e.is_definition))
    }

    /// Find all spans in this file where the given element appears.
    ///
    /// Returns `(start, end, is_definition)` triples for each occurrence.
    pub fn find_references(&self, element_id: &ElementId) -> Vec<(usize, usize, bool)> {
        self.0
            .entries
            .iter()
            .filter(|e| e.element_id == *element_id)
            .map(|e| (e.start, e.end, e.is_definition))
            .collect()
    }

    /// Find the element ID at a given byte offset (without the is_definition flag).
    ///
    /// Convenience wrapper around `element_at()` that returns just the ElementId.
    pub fn element_id_at(&self, offset: usize) -> Option<ElementId> {
        self.element_at(offset).map(|(id, _)| id.clone())
    }

    /// Find the nearest element to a given byte offset.
    ///
    /// When the cursor is in an error region (no element contains the offset),
    /// this finds the closest element by distance. Returns `(ElementId, distance)`
    /// where distance is 0 if the offset is inside an element.
    #[allow(clippy::indexing_slicing)] // partition bounds checked before indexing
    pub fn nearest_element(&self, offset: usize) -> Option<(ElementId, usize)> {
        // First try exact match
        if let Some(id) = self.element_id_at(offset) {
            return Some((id, 0));
        }

        let entries = &self.0.entries;
        if entries.is_empty() {
            return None;
        }

        // Binary search for the insertion point
        let partition = entries.partition_point(|e| e.start <= offset);

        let mut best_id = None;
        let mut best_dist = usize::MAX;

        // Check preceding entry
        if partition > 0 {
            let entry = &entries[partition - 1];
            let dist = if offset >= entry.end {
                offset - entry.end + 1
            } else {
                0
            };
            if dist < best_dist {
                best_dist = dist;
                best_id = Some(&entry.element_id);
            }
        }

        // Check following entry
        if partition < entries.len() {
            let entry = &entries[partition];
            let dist = entry.start - offset;
            if dist < best_dist {
                best_dist = dist;
                best_id = Some(&entry.element_id);
            }
        }

        best_id.map(|id| (id.clone(), best_dist))
    }

    /// Return unique element IDs that have at least one span in this file.
    ///
    /// Since this position map is already scoped to a single file,
    /// this returns all unique element IDs in the map.
    pub fn element_ids(&self) -> Vec<ElementId> {
        let mut seen = std::collections::HashSet::new();
        let mut ids = Vec::new();
        for entry in &self.0.entries {
            if seen.insert(entry.element_id.clone()) {
                ids.push(entry.element_id.clone());
            }
        }
        ids
    }

    /// Number of position entries.
    pub fn len(&self) -> usize {
        self.0.entries.len()
    }

    /// Whether the position map is empty.
    pub fn is_empty(&self) -> bool {
        self.0.entries.is_empty()
    }
}

salsa_arc_wrapper!(identity, PositionMap, PositionMapData);

// ---------------------------------------------------------------------------
// Tracked query functions
//
// ## Why three parse functions?
//
// The pipeline intentionally provides three granularity levels:
//
// 1. `parse_file_cst` → CST only (our Send+Sync `Cst` wrapper). Used by
//    queries that need syntax-tree walking without model semantics (outline,
//    folding ranges, document links).
//
// 2. `parse_file` → full ModelGraph + diagnostics. The main workhorse for
//    semantic features. Diagnostics come from the tree-sitter parser
//    (`sysml-parser-incremental`), the sole parser in the workspace.
//
// 3. `parse_tree` → raw `tree_sitter::Tree` (wrapped in `CachedTree`).
//    Needed by semantic token highlighting and keyword hover docs that
//    require the original tree-sitter node kinds. Cannot share with
//    `parse_file_cst` because the `Cst` type erases tree-sitter specifics.
//
// Each is a separate salsa tracked function, so editing a file only
// recomputes the queries that downstream handlers actually depend on.
// ---------------------------------------------------------------------------

/// Parse a file's source text into a CST.
///
/// This is the lowest-level parse query. Higher-level queries (model graph,
/// outline) can depend on this for CST-only operations.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn parse_file_cst(db: &dyn Db, source_file: SourceFile) -> Cst {
    tracing::debug!(document_uri = source_file.name(db), "starting CST parse");
    let text = source_file.text(db);
    let parser = TreeSitterParser::new();
    let cst = parser.parse_cst(&sysml_parser_incremental::SysmlFile::new(source_file.name(db), text));
    Cst::new(cst)
}

/// Parse a file and build its model graph.
///
/// This is the main entry point for semantic analysis. It uses tree-sitter
/// to parse the source, then builds a ModelGraph from the CST.
///
/// Depends on: `source_file.text()` (Layer 0 input)
/// Depended on by: resolution queries (Layer 3, future)
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn parse_file(db: &dyn Db, source_file: SourceFile) -> ParseResult {
    let file_name = source_file.name(db);
    tracing::debug!(document_uri = file_name, "starting parse");
    let text = source_file.text(db);

    // Tree-sitter is the canonical parser per ADR-014: it owns both the
    // ModelGraph (with `name_span` per element used by semantic-tokens /
    // hover / rename) and the strict-syntax diagnostic stream. ERROR and
    // MISSING node handling in `ast_builder/dispatch.rs` emits actionable
    // "expected `;` after `X`" diagnostics, which is what Pest's
    // `diagnose()` used to provide. The Pest enricher was retired in
    // TS-3.3 once the diagnostic_ux_tests suite confirmed parity.
    let ts_parser = TreeSitterParser::new();
    let tree = ts_parser.parse_tree(text);

    // Root canonical keys derive from a checkout-independent scope (ADR-009)
    // so element IDs / content_digest / CommitId are machine-independent. The
    // loader sets `root_scope` from the owning project root; when it's empty
    // (tests, synthetic loads with no project) fall back to `file_name`, the
    // pre-fix behaviour. `file_name` is still used verbatim for spans.
    let scope = source_file.root_scope(db);
    let root_scope: &str = if scope.is_empty() { file_name } else { scope };

    let (graph, diagnostics): (ModelGraph, Vec<Diagnostic>) = match tree.as_ref() {
        Some(tree) => {
            let r = sysml_parser_incremental::build_model_graph_scoped(
                tree, text, file_name, root_scope,
            );
            (r.graph, r.diagnostics)
        }
        None => (ModelGraph::default(), vec![]),
    };

    ParseResult::new(graph, diagnostics)
}

/// Parse a file and return the tree-sitter Tree, memoized by salsa.
///
/// Unlike parse_file (which extracts a ModelGraph), this returns the
/// raw CST for use by semantic tokens, folding, hover keyword
/// detection, and document links.
///
/// Depends on: source_file.text() (Layer 0 input)
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn parse_tree(db: &dyn Db, source_file: SourceFile) -> Option<CachedTree> {
    tracing::debug!(document_uri = source_file.name(db), "starting tree parse");
    let text = source_file.text(db);
    let parser = TreeSitterParser::new();
    parser.parse_tree(text).map(CachedTree::new)
}

/// Extract outline items from a file's CST.
///
/// Lightweight query for document symbols. Only needs CST, not the full
/// model graph.
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_outline(db: &dyn Db, source_file: SourceFile) -> Outline {
    tracing::debug!(
        document_uri = source_file.name(db),
        "starting outline extraction"
    );
    let text = source_file.text(db);
    let cst = parse_file_cst(db, source_file);
    let items = sysml_parser_incremental::extract_outline(cst.root(), text);
    Outline::new(items)
}

/// Build a position map from a file's parsed model graph.
///
/// Maps byte offsets to ElementIds for efficient position-based lookups.
/// Used by LSP handlers for hover, goto-definition, etc.
///
/// Depends on: `parse_file()` (Layer 1)
#[tracing::instrument(level = "debug", skip(db))]
#[salsa::tracked]
pub fn file_position_map(db: &dyn Db, source_file: SourceFile) -> PositionMap {
    tracing::debug!(
        document_uri = source_file.name(db),
        "starting position map build"
    );
    let parsed = parse_file(db, source_file);
    let name = source_file.name(db);
    PositionMap::from_graph(parsed.graph(), name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RootDatabase;

    #[test]
    fn parse_simple_package() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let parsed = parse_file(&db, sf);

        assert!(!parsed.has_errors());
        assert!(parsed.element_count() > 0);
        assert!(!parsed.graph().elements.is_empty());
    }

    #[test]
    fn parse_empty_file() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), String::new());
        let parsed = parse_file(&db, sf);

        assert_eq!(parsed.element_count(), 0);
    }

    #[test]
    fn parse_with_syntax_error() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(
            &db,
            "test.sysml".to_string(),
            "package { invalid syntax }}}".to_string(),
        );
        let parsed = parse_file(&db, sf);

        // tree-sitter is error-tolerant, so it produces a partial parse
        // We just verify it doesn't panic
        let _count = parsed.element_count();
        let _diags = parsed.diagnostics();
    }

    #[test]
    fn parse_file_cst_roundtrip() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(
            &db,
            "test.sysml".to_string(),
            "package TestPkg {}".to_string(),
        );
        let cst = parse_file_cst(&db, sf);

        let root = cst.root();
        assert_eq!(root.kind, "source_file");
        assert!(!root.children.is_empty());
    }

    #[test]
    fn outline_extraction() {
        let db = RootDatabase::default();
        // Use a richer example that produces outline items
        let sf = SourceFile::new(
            &db,
            "test.sysml".to_string(),
            "package MyPackage {\n  part myPart : Base;\n}".to_string(),
        );
        let outline = file_outline(&db, sf);
        let items = outline.items();
        // Outline may or may not find items depending on tree-sitter grammar
        // Just verify it doesn't panic
        let _ = items;
    }

    #[test]
    fn incremental_reparse() {
        let mut db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package A {}".to_string());

        // First parse
        let parsed1 = parse_file(&db, sf);
        let count1 = parsed1.element_count();

        // Update source text
        use salsa::Setter;
        sf.set_text(&mut db)
            .to("package A {} package B {}".to_string());

        // Re-parse (salsa detects the input changed)
        let parsed2 = parse_file(&db, sf);
        let count2 = parsed2.element_count();

        // Should have more elements now
        assert!(
            count2 > count1,
            "Expected more elements after adding package B: {} vs {}",
            count2,
            count1
        );
    }

    #[test]
    fn memoization_returns_same_result() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());

        // Parse twice without changing input
        let result1 = parse_file(&db, sf);
        let result2 = parse_file(&db, sf);

        // Salsa should return the same memoized result (same Arc)
        assert_eq!(result1, result2, "Memoized results should be pointer-equal");
    }

    /// Content-based equality across ALLOCATIONS (not across files):
    /// the same file re-parsed in a fresh database yields a different
    /// Arc but an equal ParseResult via the content-true fingerprint.
    ///
    /// Re-blessed 2026-07-16 (content-true fingerprint): the old
    /// assertion compared parses of two DIFFERENT files with identical
    /// text as equal — a false equality (their spans and canonical
    /// element ids genuinely differ, and span consumers can't
    /// interchange them). That same false-equality class made
    /// doc-only edits invisible to salsa backdating (live staleness
    /// bug, workspace-scope plan §W6).
    #[test]
    fn content_based_equality() {
        let db1 = RootDatabase::default();
        let db2 = RootDatabase::default();
        let sf1 = SourceFile::new(&db1, "a.sysml".to_string(), "package Foo {}".to_string());
        let sf2 = SourceFile::new(&db2, "a.sysml".to_string(), "package Foo {}".to_string());
        let r1 = parse_file(&db1, sf1);
        let r2 = parse_file(&db2, sf2);
        // Different Arc pointers, same file+content -> equal via fingerprint
        assert_eq!(r1, r2);
    }

    /// Two different files with identical text are NOT equal: spans and
    /// canonical element ids carry the file, and downstream consumers
    /// (goto-def, requirement-row source_span) must not interchange them.
    #[test]
    fn same_text_different_file_is_not_equal() {
        let db = RootDatabase::default();
        let sf1 = SourceFile::new(&db, "a.sysml".to_string(), "package Foo {}".to_string());
        let sf2 = SourceFile::new(&db, "b.sysml".to_string(), "package Foo {}".to_string());
        let r1 = parse_file(&db, sf1);
        let r2 = parse_file(&db, sf2);
        assert_ne!(r1, r2);
    }

    #[test]
    fn diagnostics_affect_parse_result_fingerprint() {
        let graph = ModelGraph::new();
        let without_diag = ParseResult::new(graph.clone(), Vec::new());
        let with_diag = ParseResult::new(graph, vec![Diagnostic::warning("parse warning")]);

        assert_ne!(
            without_diag, with_diag,
            "Diagnostic changes must invalidate parse result identity"
        );
    }

    #[test]
    fn position_map_finds_element() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let pm = file_position_map(&db, sf);
        // "Foo" starts around offset 8
        let result = pm.element_at(9);
        assert!(
            result.is_some(),
            "Should find element at offset within 'Foo'"
        );
    }

    #[test]
    fn position_map_empty_file() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), String::new());
        let pm = file_position_map(&db, sf);
        assert!(pm.is_empty());
        assert!(pm.element_at(0).is_none());
    }

    #[test]
    fn position_map_nested_spans_binary_search() {
        // Test that the binary-search element_at finds the innermost span
        // when multiple elements overlap at a given offset.
        //
        // Layout: "package Outer { part inner : Integer; }"
        //          0       8      16   21
        let db = RootDatabase::default();
        let sf = SourceFile::new(
            &db,
            "test.sysml".to_string(),
            "package Outer { part inner : Integer; }".to_string(),
        );
        let pm = file_position_map(&db, sf);

        // "inner" is around offset 21-26 — should be found inside Outer
        let at_inner = pm.element_at(22);
        assert!(
            at_inner.is_some(),
            "Should find element at offset within 'inner'"
        );

        // Offset 0 — before "package" keyword, may or may not have an element
        // Just verify it doesn't panic
        let _ = pm.element_at(0);

        // Past end of file — should return None
        assert!(pm.element_at(1000).is_none());
    }

    // --- CachedTree / parse_tree tests ---

    /// Compile-time proof that CachedTree is Send.
    const _: () = {
        fn assert_send<T: Send>() {}
        fn check() {
            assert_send::<CachedTree>();
        }
    };

    /// Compile-time proof that CachedTree is Sync.
    const _: () = {
        fn assert_sync<T: Sync>() {}
        fn check() {
            assert_sync::<CachedTree>();
        }
    };

    #[test]
    fn parse_tree_basic() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let tree = parse_tree(&db, sf);
        assert!(
            tree.is_some(),
            "parse_tree should return Some for valid input"
        );
        let cached = tree.unwrap();
        assert_eq!(cached.root_node().kind(), "source_file");
    }

    #[test]
    fn parse_tree_empty_input() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), String::new());
        let tree = parse_tree(&db, sf);
        // tree-sitter parses empty input as an empty source_file
        assert!(
            tree.is_some(),
            "parse_tree returns Some even for empty input"
        );
    }

    #[test]
    fn parse_tree_memoization() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let t1 = parse_tree(&db, sf);
        let t2 = parse_tree(&db, sf);
        assert!(t1.is_some());
        assert!(t2.is_some());
        // Salsa memoization: same Arc pointer
        assert_eq!(
            t1, t2,
            "Memoized parse_tree should return pointer-equal results"
        );
    }

    #[test]
    fn parse_tree_invalidation() {
        let mut db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package A {}".to_string());
        let t1 = parse_tree(&db, sf);
        assert!(t1.is_some());

        use salsa::Setter;
        sf.set_text(&mut db).to("package B {}".to_string());
        let t2 = parse_tree(&db, sf);
        assert!(t2.is_some());
        // Different content → different Arc (pointer inequality)
        assert_ne!(t1, t2, "Changed input should produce different tree");
    }

    #[test]
    fn parse_tree_walk() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(
            &db,
            "test.sysml".to_string(),
            "package Foo { part def Engine; }".to_string(),
        );
        let cached = parse_tree(&db, sf).unwrap();
        let mut cursor = cached.walk();
        assert!(cursor.goto_first_child(), "Tree should have children");
    }

    #[test]
    fn parse_tree_tree_accessor() {
        let db = RootDatabase::default();
        let sf = SourceFile::new(&db, "test.sysml".to_string(), "package Foo {}".to_string());
        let cached = parse_tree(&db, sf).unwrap();
        let tree: &tree_sitter::Tree = cached.tree();
        assert_eq!(tree.root_node().kind(), "source_file");
    }

    #[test]
    fn parse_tree_latency_sanity() {
        let db = RootDatabase::default();
        let source = "package Foo {\n".to_string() + &"  part def Engine;\n".repeat(100) + "}";
        let sf = SourceFile::new(&db, "big.sysml".to_string(), source);
        let start = std::time::Instant::now();
        let tree = parse_tree(&db, sf);
        let elapsed = start.elapsed();
        assert!(tree.is_some());
        assert!(
            elapsed.as_millis() < 50,
            "parse_tree should be <50ms for 100-element file, was {}ms",
            elapsed.as_millis()
        );
    }
}
