//! # sysml-ts
//!
//! Tree-sitter based CST (Concrete Syntax Tree) parsing for SysML v2.
//!
//! This crate provides fast, incremental parsing suitable for IDE use cases.
//! It produces a CST rather than a semantic model, making it useful for:
//!
//! - Syntax highlighting
//! - Bracket matching
//! - Code folding
//! - Basic outline view
//! - Error recovery during editing
//!
//! ## Semantic Model Building (optional)
//!
//! With the `semantic` feature enabled, this crate can also build a full
//! ModelGraph from the tree-sitter parse tree. This enables graceful degradation
//! in the LSP: when there are syntax errors, semantic features still work on
//! valid regions of the file.
//!
//! ```ignore
//! use sysml_parser_incremental::{TreeSitterParser, build_model_graph};
//!
//! let parser = TreeSitterParser::new();
//! let tree = parser.parse(source)?;
//! let result = build_model_graph(&tree, source, "file.sysml");
//! // result.graph contains elements from valid regions
//! // result.diagnostics contains syntax errors
//! ```
//!
//! **Note**: Without the `semantic` feature, this crate does NOT depend on sysml-core.
//! It deals only with syntax, not semantics.

use sysml_span::Span;

// Semantic model building (requires sysml-core and sysml-text)
#[cfg(feature = "semantic")]
pub mod ast_builder;

#[cfg(feature = "semantic")]
pub mod expression_elements;

#[cfg(feature = "semantic")]
pub use ast_builder::{build_model_graph, build_model_graph_scoped, ModelGraphResult};

#[cfg(feature = "semantic")]
pub use expression_elements::ExpressionBuilder;

// `SysmlFile` is the canonical wire type from `sysml-parser-trait`.
// When the `semantic` feature is enabled, parser-trait is in scope and we
// re-export its definition. When it is not, we keep a structurally identical
// local copy so the `FastParser` trait remains usable without dragging
// sysml-core into the dependency graph.
#[cfg(feature = "semantic")]
pub use sysml_parser_trait::SysmlFile;

/// A file to be parsed.
#[cfg(not(feature = "semantic"))]
#[derive(Debug, Clone)]
pub struct SysmlFile {
    /// The file path or URI.
    pub path: String,
    /// The file contents.
    pub text: String,
}

#[cfg(not(feature = "semantic"))]
impl SysmlFile {
    /// Create a new SysML file.
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        SysmlFile {
            path: path.into(),
            text: text.into(),
        }
    }
}

/// A node in the concrete syntax tree.
#[derive(Debug, Clone)]
pub struct SyntaxNode {
    /// The kind of this node (e.g., "package_declaration", "identifier").
    pub kind: String,
    /// The source span of this node.
    pub span: Span,
    /// Child nodes.
    pub children: Vec<SyntaxNode>,
    /// Whether this node represents an error.
    pub is_error: bool,
}

impl SyntaxNode {
    /// Create a new syntax node.
    pub fn new(kind: impl Into<String>, span: Span) -> Self {
        SyntaxNode {
            kind: kind.into(),
            span,
            children: Vec::new(),
            is_error: false,
        }
    }

    /// Create an error node.
    pub fn error(span: Span) -> Self {
        SyntaxNode {
            kind: "ERROR".to_owned(),
            span,
            children: Vec::new(),
            is_error: true,
        }
    }

    /// Add a child node.
    pub fn with_child(mut self, child: SyntaxNode) -> Self {
        self.children.push(child);
        self
    }

    /// Add multiple child nodes.
    pub fn with_children(mut self, children: Vec<SyntaxNode>) -> Self {
        self.children = children;
        self
    }

    /// Get the text content of this node given the source.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        if self.span.start <= source.len() && self.span.end <= source.len() {
            &source[self.span.start..self.span.end]
        } else {
            ""
        }
    }

    /// Find all nodes of a given kind in the tree.
    pub fn find_by_kind(&self, kind: &str) -> Vec<&SyntaxNode> {
        let mut result = Vec::new();
        self.find_by_kind_recursive(kind, &mut result);
        result
    }

    fn find_by_kind_recursive<'a>(&'a self, kind: &str, result: &mut Vec<&'a SyntaxNode>) {
        if self.kind == kind {
            result.push(self);
        }
        for child in &self.children {
            child.find_by_kind_recursive(kind, result);
        }
    }

    /// Find the first child of a given kind.
    pub fn child_by_kind(&self, kind: &str) -> Option<&SyntaxNode> {
        self.children.iter().find(|c| c.kind == kind)
    }

    /// Check if this node or any descendant has errors.
    pub fn has_errors(&self) -> bool {
        if self.is_error {
            return true;
        }
        self.children.iter().any(|c| c.has_errors())
    }

    /// Get all error nodes.
    pub fn errors(&self) -> Vec<&SyntaxNode> {
        let mut result = Vec::new();
        self.collect_errors(&mut result);
        result
    }

    fn collect_errors<'a>(&'a self, result: &mut Vec<&'a SyntaxNode>) {
        if self.is_error {
            result.push(self);
        }
        for child in &self.children {
            child.collect_errors(result);
        }
    }
}

/// Trait for fast CST parsers.
pub trait FastParser {
    /// Parse a file and return its CST.
    fn parse_cst(&self, file: &SysmlFile) -> SyntaxNode;

    /// Check if incremental parsing is supported.
    fn supports_incremental(&self) -> bool {
        false
    }
}

/// A real tree-sitter parser that uses the tree-sitter-sysml grammar.
#[derive(Debug, Clone, Default)]
pub struct TreeSitterParser {
    // Note: tree_sitter::Parser is not Clone, so we create it on demand
}

impl TreeSitterParser {
    /// Create a new tree-sitter parser.
    pub fn new() -> Self {
        TreeSitterParser {}
    }

    /// Parse source code and return the tree-sitter Tree.
    ///
    /// This is useful when you need the raw tree-sitter tree for incremental
    /// parsing or other advanced use cases.
    pub fn parse_tree(&self, source: &str) -> Option<tree_sitter::Tree> {
        #[cfg(feature = "tracing")]
        let start = std::time::Instant::now();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_sysml::language()).ok()?;
        let tree = parser.parse(source, None);
        #[cfg(feature = "tracing")]
        tracing::trace!(
            bytes = source.len(),
            parsed = tree.is_some(),
            elapsed_ms = start.elapsed().as_millis(),
            "tree-sitter full parse"
        );
        tree
    }

    /// Parse with an existing tree for incremental updates.
    ///
    /// This is more efficient when editing: pass the previous tree and the
    /// new source, and tree-sitter will reuse unchanged portions.
    pub fn parse_tree_incremental(
        &self,
        source: &str,
        old_tree: Option<&tree_sitter::Tree>,
    ) -> Option<tree_sitter::Tree> {
        #[cfg(feature = "tracing")]
        let start = std::time::Instant::now();
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&tree_sitter_sysml::language()).ok()?;
        let tree = parser.parse(source, old_tree);
        #[cfg(feature = "tracing")]
        tracing::trace!(
            bytes = source.len(),
            has_old_tree = old_tree.is_some(),
            parsed = tree.is_some(),
            elapsed_ms = start.elapsed().as_millis(),
            "tree-sitter incremental parse"
        );
        tree
    }
}

impl FastParser for TreeSitterParser {
    fn parse_cst(&self, file: &SysmlFile) -> SyntaxNode {
        match self.parse_tree(&file.text) {
            Some(tree) => convert_tree_sitter_node(tree.root_node(), &file.path, &file.text),
            None => {
                // Return an error node if parsing failed
                let span = Span::new(&file.path, 0, file.text.len());
                SyntaxNode::error(span)
            }
        }
    }

    fn supports_incremental(&self) -> bool {
        true
    }
}

#[cfg(feature = "semantic")]
impl sysml_parser_trait::Parser for TreeSitterParser {
    fn parse(&self, inputs: &[SysmlFile]) -> sysml_parser_trait::ParseResult {
        let mut combined = sysml_core::ModelGraph::new();
        let mut all_diagnostics: Vec<sysml_span::Diagnostic> = Vec::new();

        for file in inputs {
            let result = match self.parse_tree(&file.text) {
                Some(tree) => build_model_graph(&tree, &file.text, &file.path),
                None => {
                    let mut r = ModelGraphResult::new();
                    r.diagnostics.push(sysml_span::Diagnostic::error(format!(
                        "tree-sitter failed to parse {}",
                        file.path
                    )));
                    r
                }
            };
            for (_id, element) in result.graph.elements {
                combined.add_element(element);
            }
            for (_id, rel) in result.graph.relationships {
                combined.add_relationship(rel);
            }
            all_diagnostics.extend(result.diagnostics);
        }

        combined.rebuild_indexes();
        sysml_parser_trait::ParseResult::new(combined, all_diagnostics)
    }

    fn name(&self) -> &str {
        "tree-sitter"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }
}

/// Convert a tree-sitter node to our SyntaxNode representation.
#[allow(clippy::only_used_in_recursion)]
fn convert_tree_sitter_node(
    ts_node: tree_sitter::Node,
    file_path: &str,
    source: &str,
) -> SyntaxNode {
    let mut span = Span::new(file_path, ts_node.start_byte(), ts_node.end_byte());
    let pos = ts_node.start_position();
    span.line = Some(pos.row as u32 + 1); // tree-sitter is 0-indexed, Span is 1-indexed
    span.col = Some(pos.column as u32);
    let kind = ts_node.kind().to_owned();
    let is_error = ts_node.is_error();

    let mut node = if is_error {
        SyntaxNode::error(span)
    } else {
        SyntaxNode::new(kind, span)
    };

    // Convert children
    let child_count = ts_node.child_count();
    let mut children = Vec::with_capacity(child_count);
    for i in 0..child_count {
        if let Some(child) = ts_node.child(i) {
            // Skip anonymous nodes (punctuation, keywords) for a cleaner tree
            if child.is_named() {
                children.push(convert_tree_sitter_node(child, file_path, source));
            }
        }
    }
    node.children = children;

    node
}

/// A stub tree-sitter parser that returns a minimal CST (for testing/fallback).
#[derive(Debug, Clone, Default)]
pub struct StubTreeSitterParser;

impl StubTreeSitterParser {
    /// Create a new stub parser.
    pub fn new() -> Self {
        StubTreeSitterParser
    }
}

impl FastParser for StubTreeSitterParser {
    fn parse_cst(&self, file: &SysmlFile) -> SyntaxNode {
        // Return a simple root node representing the entire file
        let span = Span::new(&file.path, 0, file.text.len());

        // Try to extract some basic structure
        let mut root = SyntaxNode::new("source_file", span);

        // Very basic: look for "package" keyword
        if let Some(pkg_start) = file.text.find("package") {
            // Find the package name (simplified)
            let after_pkg = &file.text[pkg_start + 7..];
            let name_start =
                pkg_start + 7 + after_pkg.chars().take_while(|c| c.is_whitespace()).count();

            if let Some(name_end_offset) = after_pkg
                .trim_start()
                .find(|c: char| !c.is_alphanumeric() && c != '_')
            {
                let name_end = name_start + name_end_offset;

                let pkg_node = SyntaxNode::new(
                    "package_declaration",
                    Span::new(&file.path, pkg_start, file.text.len()),
                )
                .with_child(SyntaxNode::new(
                    "identifier",
                    Span::new(&file.path, name_start, name_end),
                ));

                root = root.with_child(pkg_node);
            }
        }

        root
    }

    fn supports_incremental(&self) -> bool {
        false
    }
}

/// An outline item for IDE navigation.
#[derive(Debug, Clone)]
pub struct OutlineItem {
    /// The name/label of this item.
    pub name: String,
    /// The kind of this item (e.g., "package", "part").
    pub kind: String,
    /// The source span.
    pub span: Span,
    /// Child items.
    pub children: Vec<OutlineItem>,
}

impl OutlineItem {
    /// Create a new outline item.
    pub fn new(name: impl Into<String>, kind: impl Into<String>, span: Span) -> Self {
        OutlineItem {
            name: name.into(),
            kind: kind.into(),
            span,
            children: Vec::new(),
        }
    }

    /// Add a child item.
    pub fn with_child(mut self, child: OutlineItem) -> Self {
        self.children.push(child);
        self
    }
}

/// Extract an outline from a CST.
pub fn extract_outline(root: &SyntaxNode, source: &str) -> Vec<OutlineItem> {
    let mut items = Vec::new();

    for pkg in root.find_by_kind("package_declaration") {
        if let Some(id) = pkg.child_by_kind("identifier") {
            let name = id.text(source).to_owned();
            items.push(OutlineItem::new(name, "package", pkg.span.clone()));
        }
    }

    items
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn syntax_node_creation() {
        let span = Span::new("test.sysml", 0, 10);
        let node = SyntaxNode::new("identifier", span);
        assert_eq!(node.kind, "identifier");
        assert!(!node.is_error);
    }

    #[test]
    fn syntax_node_children() {
        let span = Span::new("test.sysml", 0, 20);
        let child = SyntaxNode::new("identifier", Span::new("test.sysml", 8, 12));
        let parent = SyntaxNode::new("package_declaration", span).with_child(child);

        assert_eq!(parent.children.len(), 1);
        assert_eq!(parent.children[0].kind, "identifier");
    }

    #[test]
    fn syntax_node_text() {
        let source = "package Test {}";
        let span = Span::new("test.sysml", 8, 12);
        let node = SyntaxNode::new("identifier", span);
        assert_eq!(node.text(source), "Test");
    }

    #[test]
    fn syntax_node_text_start_out_of_bounds() {
        let source = "short";
        // span.start > source.len() - must return "" not panic
        let span = Span::new("test.sysml", 100, 105);
        let node = SyntaxNode::new("identifier", span);
        assert_eq!(node.text(source), "");
    }

    #[test]
    fn syntax_node_text_end_out_of_bounds() {
        let source = "short";
        // span.end > source.len() - must return "" not panic
        let span = Span::new("test.sysml", 2, 100);
        let node = SyntaxNode::new("identifier", span);
        assert_eq!(node.text(source), "");
    }

    #[test]
    fn find_by_kind() {
        let span = Span::new("test.sysml", 0, 20);
        let id1 = SyntaxNode::new("identifier", Span::new("test.sysml", 8, 12));
        let id2 = SyntaxNode::new("identifier", Span::new("test.sysml", 14, 18));
        let root = SyntaxNode::new("source_file", span)
            .with_child(id1)
            .with_child(id2);

        let found = root.find_by_kind("identifier");
        assert_eq!(found.len(), 2);
    }

    #[test]
    fn error_nodes() {
        let span = Span::new("test.sysml", 0, 20);
        let error = SyntaxNode::error(Span::new("test.sysml", 5, 10));
        let root = SyntaxNode::new("source_file", span).with_child(error);

        assert!(root.has_errors());
        assert_eq!(root.errors().len(), 1);
    }

    #[test]
    fn stub_parser() {
        let parser = StubTreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", "package Test {}");
        let cst = parser.parse_cst(&file);

        assert_eq!(cst.kind, "source_file");
        assert!(!parser.supports_incremental());
    }

    #[test]
    fn stub_parser_extracts_package() {
        let parser = StubTreeSitterParser::new();
        let file = SysmlFile::new("test.sysml", "package MyPackage { }");
        let cst = parser.parse_cst(&file);

        let packages = cst.find_by_kind("package_declaration");
        assert!(!packages.is_empty());
    }

    #[test]
    fn outline_extraction() {
        let parser = StubTreeSitterParser::new();
        let source = "package MyPackage { }";
        let file = SysmlFile::new("test.sysml", source);
        let cst = parser.parse_cst(&file);
        let outline = extract_outline(&cst, source);

        assert!(!outline.is_empty());
        assert_eq!(outline[0].kind, "package");
    }

    /// Embedded stage gate: 13 library syntax patterns that must parse
    /// without ERROR/MISSING nodes in tree-sitter. No env var needed.
    #[test]
    fn treesitter_library_syntax_stage_gate() {
        let parser = TreeSitterParser::new();
        let patterns: &[(&str, &str)] = &[
            (
                "struct_with_multiplicity",
                "package T { struct Life[1] :> Clock; }",
            ),
            (
                "constant_keyword",
                "package T { constant ref occurrence x[1..*] :>> y :> z; }",
            ),
            (
                "member_feature",
                "package T { datatype E { member feature 'v1' : E[1]; } }",
            ),
            (
                "comment_about_multiple",
                "package T { part def A; part def B; comment about A, B /* text */ }",
            ),
            (
                "succession_all",
                "package T { succession all [*] a then [*] b; }",
            ),
            (
                "connector_multiplicity_first",
                "package T { connector [0..1] link to [1..*] target; }",
            ),
            (
                "return_feature",
                "package T { function F { return feature r : Integer[1]; } }",
            ),
            (
                "meta_cast_expression",
                "package T { metadata def M :> SemanticMetadata { ref :>> baseType = x meta SysML::Usage; } }",
            ),
            (
                "named_arguments",
                "package T { attribute x = new Pair(a = 1, b = 2); }",
            ),
            (
                "feature_chain_in_primary_expr",
                "package T { constraint c { edges#(1).vertices->size() } }",
            ),
            (
                "end_bool_feature",
                "package T { assoc struct S { end bool g; } }",
            ),
            (
                "succession_with_multiplicity",
                "package T { succession [*] guard then [1] exit; }",
            ),
            (
                "connector_all_from_to",
                "package T { connector all gc: Type[*] from [0..1] link to [*] guard; }",
            ),
        ];

        let mut failures: Vec<(&str, String)> = Vec::new();
        for (name, src) in patterns {
            let tree = parser.parse_tree(src);
            match tree {
                Some(tree) => {
                    if tree.root_node().has_error() {
                        let root = tree.root_node();
                        let err_text = find_first_ts_error(root, src);
                        failures.push((name, err_text));
                    }
                }
                None => {
                    failures.push((name, "parse returned None".to_owned()));
                }
            }
        }

        if !failures.is_empty() {
            let msgs: Vec<String> = failures
                .iter()
                .map(|(name, err)| format!("  {name}: {err}"))
                .collect();
            panic!(
                "Tree-sitter library syntax stage gate FAILED ({}/{} patterns):\n{}",
                failures.len(),
                patterns.len(),
                msgs.join("\n")
            );
        }
    }

    /// Find the first ERROR or MISSING node and return a diagnostic string.
    fn find_first_ts_error(node: tree_sitter::Node, source: &str) -> String {
        if node.is_error() || node.is_missing() {
            let start = node.start_position();
            let byte_start = node.start_byte();
            let byte_end = node.end_byte().min(byte_start + 60).min(source.len());
            let text = &source[byte_start..byte_end];
            return format!("line {}:{} {:?}", start.row + 1, start.column, text);
        }
        let child_count = node.child_count();
        for i in 0..child_count {
            if let Some(child) = node.child(i) {
                let result = find_first_ts_error(child, source);
                if !result.is_empty() {
                    return result;
                }
            }
        }
        String::new()
    }
}
