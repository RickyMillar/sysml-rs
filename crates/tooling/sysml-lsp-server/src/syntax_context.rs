//! Shared tree-sitter cursor-context helpers for position-sensitive LSP features.

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

use tower_lsp::lsp_types::{Position, Range};
use tree_sitter::{Node, Tree};

use crate::utils::position_to_offset;

#[derive(Debug, Clone)]
pub(crate) struct ImportDecl {
    pub path_text: Option<String>,
    pub path_start: Option<tree_sitter::Point>,
    pub path_end: Option<tree_sitter::Point>,
}

#[derive(Debug, Clone)]
pub(crate) struct CommentFoldSpan {
    pub start: tree_sitter::Point,
    pub end: tree_sitter::Point,
}

/// Lightweight syntax context for a single cursor offset.
#[derive(Debug, Clone)]
pub(crate) struct CursorSyntaxContext {
    // Closest node first, root last.
    ancestor_kinds: Vec<String>,
    lexical_in_line_comment: bool,
    lexical_in_string: bool,
}

impl CursorSyntaxContext {
    pub(crate) fn from_tree(tree: &Tree, content: &str, offset: usize) -> Self {
        let clamped_offset = offset.min(content.len());
        let node = node_at_offset(tree, content, clamped_offset);
        let (lexical_in_line_comment, lexical_in_string) =
            lexical_context_before_offset(content, clamped_offset);

        let mut ancestor_kinds = Vec::new();

        if let Some(mut current) = node {
            loop {
                ancestor_kinds.push(current.kind().to_owned());
                match current.parent() {
                    Some(parent) => current = parent,
                    None => break,
                }
            }
        }

        Self {
            ancestor_kinds,
            lexical_in_line_comment,
            lexical_in_string,
        }
    }

    pub(crate) fn has_ancestor(&self, kind: &str) -> bool {
        self.ancestor_kinds.iter().any(|k| k == kind)
    }

    pub(crate) fn has_ancestor_where<F>(&self, mut predicate: F) -> bool
    where
        F: FnMut(&str) -> bool,
    {
        self.ancestor_kinds.iter().any(|k| predicate(k))
    }

    pub(crate) fn in_import_decl(&self) -> bool {
        self.has_ancestor("import_decl")
    }

    pub(crate) fn in_type_ref(&self) -> bool {
        self.has_ancestor("type_ref") || self.has_ancestor("typing")
    }

    pub(crate) fn in_feature_chain(&self) -> bool {
        self.has_ancestor("feature_chain") || self.has_ancestor("member_access")
    }

    pub(crate) fn in_comment_or_string(&self) -> bool {
        self.has_ancestor_where(|kind| kind.contains("comment") || kind == "string_literal")
            || self.lexical_in_line_comment
            || self.lexical_in_string
    }
}

pub(crate) fn node_at_lsp_position<'a>(
    tree: &'a Tree,
    content: &str,
    position: &Position,
) -> Option<Node<'a>> {
    let offset = position_to_offset(position, content).min(content.len());
    node_at_offset(tree, content, offset)
}

pub(crate) fn node_range_to_lsp(node: &Node<'_>) -> Range {
    Range {
        start: Position {
            line: node.start_position().row as u32,
            character: node.start_position().column as u32,
        },
        end: Position {
            line: node.end_position().row as u32,
            character: node.end_position().column as u32,
        },
    }
}

pub(crate) fn node_at_offset<'a>(tree: &'a Tree, content: &str, offset: usize) -> Option<Node<'a>> {
    let root = tree.root_node();
    let byte = offset.min(content.len());

    root.descendant_for_byte_range(byte, byte).or_else(|| {
        if byte > 0 {
            root.descendant_for_byte_range(byte - 1, byte - 1)
        } else {
            None
        }
    })
}

pub(crate) fn collect_import_declarations(tree: &Tree, content: &str) -> Vec<ImportDecl> {
    let mut imports = Vec::new();
    let mut cursor = tree.walk();
    collect_import_declarations_cursor(&mut cursor, content, &mut imports);
    imports
}

pub(crate) fn collect_comment_folds(tree: &Tree) -> Vec<CommentFoldSpan> {
    let mut ranges = Vec::new();
    let mut cursor = tree.walk();
    collect_comment_folds_cursor(&mut cursor, &mut ranges);
    ranges
}

#[cfg(test)]
fn find_ancestor<'a, F>(node: &'a Node<'a>, mut predicate: F) -> Option<Node<'a>>
where
    F: FnMut(&str) -> bool,
{
    let mut current = *node;
    loop {
        if predicate(current.kind()) {
            return Some(current);
        }
        current = current.parent()?;
    }
}

fn collect_import_declarations_cursor(
    cursor: &mut tree_sitter::TreeCursor,
    content: &str,
    imports: &mut Vec<ImportDecl>,
) {
    loop {
        let node = cursor.node();
        if node.kind() == "import_decl" {
            imports.push(build_import_decl(node, content));
        }

        if cursor.goto_first_child() {
            collect_import_declarations_cursor(cursor, content, imports);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn collect_comment_folds_cursor(
    cursor: &mut tree_sitter::TreeCursor,
    ranges: &mut Vec<CommentFoldSpan>,
) {
    loop {
        let node = cursor.node();
        if matches!(node.kind(), "comment" | "doc_comment" | "doc_string" | "comment_element")
            && node.end_position().row > node.start_position().row
        {
            ranges.push(CommentFoldSpan {
                start: node.start_position(),
                end: node.end_position(),
            });
        }

        if cursor.goto_first_child() {
            collect_comment_folds_cursor(cursor, ranges);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn build_import_decl(node: Node<'_>, content: &str) -> ImportDecl {
    let raw_text = node.utf8_text(content.as_bytes()).unwrap_or("").to_owned();
    let trimmed_text = raw_text.trim().to_owned();

    let (path_text, path_start, path_end) =
        if let Some(path_node) = node.child_by_field_name("path") {
            let path_text = safe_slice(content, path_node.start_byte(), path_node.end_byte())
                .map(|s| s.to_owned());
            let path_start = Some(path_node.start_position());
            let path_end = Some(path_node.end_position());
            (path_text, path_start, path_end)
        } else {
            let path_text = parse_import_path_from_decl_text(&trimmed_text);
            if let Some(ref path) = path_text {
                if let Some(local_idx) = raw_text.find(path) {
                    let start_byte = node.start_byte() + local_idx;
                    let end_byte = start_byte + path.len();
                    (
                        path_text,
                        Some(point_for_byte(content, start_byte)),
                        Some(point_for_byte(content, end_byte)),
                    )
                } else {
                    (path_text, None, None)
                }
            } else {
                (None, None, None)
            }
        };

    ImportDecl {
        path_text,
        path_start,
        path_end,
    }
}

fn safe_slice(content: &str, start: usize, end: usize) -> Option<&str> {
    if start > end || end > content.len() {
        return None;
    }
    if !content.is_char_boundary(start) || !content.is_char_boundary(end) {
        return None;
    }
    Some(&content[start..end])
}

fn point_for_byte(content: &str, byte: usize) -> tree_sitter::Point {
    let clamped = byte.min(content.len());
    let mut row = 0usize;
    let mut line_start = 0usize;

    for (idx, ch) in content.char_indices() {
        if idx >= clamped {
            break;
        }
        if ch == '\n' {
            row += 1;
            line_start = idx + 1;
        }
    }

    tree_sitter::Point {
        row,
        column: clamped.saturating_sub(line_start),
    }
}

fn parse_import_path_from_decl_text(text: &str) -> Option<String> {
    let mut s = text.trim().trim_end_matches(';').trim();

    for vis in ["public ", "private ", "protected "] {
        if let Some(rest) = s.strip_prefix(vis) {
            s = rest.trim_start();
            break;
        }
    }

    s = s.strip_prefix("import ")?;

    if let Some(rest) = s.strip_prefix("all ") {
        s = rest.trim_start();
    }

    if let Some(idx) = s.find('[') {
        s = s[..idx].trim_end();
    }

    if let Some(prefix) = s.strip_suffix("::**") {
        s = prefix.trim_end();
    } else if let Some(prefix) = s.strip_suffix("::*") {
        s = prefix.trim_end();
    }

    let path = s.trim();
    if path.is_empty() {
        None
    } else {
        Some(path.to_owned())
    }
}

fn lexical_context_before_offset(content: &str, offset: usize) -> (bool, bool) {
    let clamped = offset.min(content.len());
    let line_start = content[..clamped].rfind('\n').map(|i| i + 1).unwrap_or(0);
    let mut in_string = false;
    let mut escaped = false;
    let bytes = content.as_bytes();
    let mut i = line_start;

    while i < clamped {
        let b = bytes[i];
        if !in_string && b == b'/' && i + 1 < clamped && bytes[i + 1] == b'/' {
            return (true, in_string);
        }

        if b == b'"' && !escaped {
            in_string = !in_string;
        }

        escaped = in_string && b == b'\\' && !escaped;

        i += 1;
    }

    (false, in_string)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::{
        collect_comment_folds, collect_import_declarations, find_ancestor, node_range_to_lsp,
        CursorSyntaxContext,
    };
    use sysml_parser_incremental::TreeSitterParser;

    fn parse_tree(content: &str) -> tree_sitter::Tree {
        let parser = TreeSitterParser::new();
        parser.parse_tree(content).expect("tree-sitter parse")
    }

    #[test]
    fn detects_import_context() {
        let content = "import ScalarValues::Integer;";
        let tree = parse_tree(content);
        let offset = content.find("Integer").unwrap() + 2;
        let ctx = CursorSyntaxContext::from_tree(&tree, content, offset);
        assert!(ctx.in_import_decl());
    }

    #[test]
    fn detects_type_ref_context() {
        let content = "part engine : Integer;";
        let tree = parse_tree(content);
        let offset = content.find("Integer").unwrap() + 2;
        let ctx = CursorSyntaxContext::from_tree(&tree, content, offset);
        assert!(ctx.in_type_ref());
    }

    #[test]
    fn detects_feature_chain_context() {
        let content = "part engine;\nattribute ref = engine.owner.name;";
        let tree = parse_tree(content);
        let offset = content.find("owner").unwrap() + 2;
        let ctx = CursorSyntaxContext::from_tree(&tree, content, offset);
        assert!(ctx.in_feature_chain());
    }

    #[test]
    fn detects_comment_context_lexically() {
        let content = "// part x : Integer";
        let tree = parse_tree(content);
        let offset = content.len();
        let ctx = CursorSyntaxContext::from_tree(&tree, content, offset);
        assert!(ctx.in_comment_or_string());
    }

    #[test]
    fn detects_string_context_lexically() {
        let content = "attribute note = \"part def \"";
        let tree = parse_tree(content);
        let offset = content.find("def").unwrap() + 2;
        let ctx = CursorSyntaxContext::from_tree(&tree, content, offset);
        assert!(ctx.in_comment_or_string());
    }

    #[test]
    fn collects_import_declaration_text_and_path() {
        let content = "import ScalarValues::Integer;\npart def A;";
        let tree = parse_tree(content);
        let imports = collect_import_declarations(&tree, content);
        assert_eq!(imports.len(), 1);
        assert_eq!(
            imports[0].path_text.as_deref(),
            Some("ScalarValues::Integer")
        );
        assert!(imports[0].path_start.is_some());
        assert!(imports[0].path_end.is_some());
    }

    #[test]
    fn finds_definition_ancestor_for_child_node() {
        let content = "part def Vehicle;";
        let tree = parse_tree(content);
        let root = tree.root_node();
        let child = root
            .descendant_for_byte_range(
                content.find("Vehicle").unwrap(),
                content.find("Vehicle").unwrap(),
            )
            .expect("child node");
        let def = find_ancestor(&child, |kind| {
            kind.ends_with("_definition")
                || kind.ends_with("_def")
                || kind == "definition"
                || kind == "package_declaration"
        })
        .expect("definition ancestor");
        assert!(def.kind().contains("_def") || def.kind().contains("definition"));
    }

    #[test]
    fn converts_node_range_to_lsp() {
        let content = "part def Vehicle;";
        let tree = parse_tree(content);
        let root = tree.root_node();
        let node = root
            .descendant_for_byte_range(
                content.find("Vehicle").unwrap(),
                content.find("Vehicle").unwrap(),
            )
            .expect("vehicle node");
        let range = node_range_to_lsp(&node);
        assert_eq!(range.start.line, 0);
        assert!(range.end.character >= range.start.character);
    }

    #[test]
    fn collects_multiline_comment_folds() {
        let content = "/* a\nb */\npart def A;";
        let tree = parse_tree(content);
        let folds = collect_comment_folds(&tree);
        assert_eq!(folds.len(), 1);
        assert!(folds[0].end.row > folds[0].start.row);
    }
}
