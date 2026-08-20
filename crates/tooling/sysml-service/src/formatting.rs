//! Document formatting — whitespace-only edits driven by tree-sitter.
//!
//! Provides `compute_format_edits(content, options)` which produces a list of
//! line/column-based text edits:
//! - Consistent indentation (4 spaces per nesting level by default)
//! - Trailing whitespace removal
//! - Blank line collapse (3+ consecutive → 2)
//! - Final newline enforcement
//! - Space-before-semicolon removal
//!
//! Replaces the LSP-side `format_document` body. The LSP shell shrinks to a
//! shim that converts `lsp_types::FormattingOptions` to `FormatOptions` and
//! reshapes `TextEdit` into `lsp_types::TextEdit`.

use crate::text_edit::TextEdit;
use sysml_parser_incremental::TreeSitterParser;

/// Formatting options exposed to transports.
///
/// Mirrors the relevant subset of `lsp_types::FormattingOptions`. `tab_size`
/// is the indent width in characters; `insert_spaces` selects spaces vs
/// tabs.
#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct FormatOptions {
    pub tab_size: u32,
    pub insert_spaces: bool,
}

impl Default for FormatOptions {
    fn default() -> Self {
        FormatOptions {
            tab_size: 4,
            insert_spaces: true,
        }
    }
}

/// Tree-sitter node kinds that increase nesting depth. Mirrors the original
/// LSP-side list verbatim — same set, same indent semantics.
const NESTING_KINDS: &[&str] = &[
    "source_file", // depth 0 (not counted)
    "package_declaration",
    "namespace_declaration",
    "element_body",
    "part_definition",
    "part_usage",
    "attribute_definition",
    "attribute_usage",
    "action_definition",
    "action_usage",
    "state_definition",
    "state_usage",
    "requirement_definition",
    "requirement_usage",
    "constraint_definition",
    "constraint_usage",
    "item_definition",
    "item_usage",
    "port_definition",
    "port_usage",
    "connection_definition",
    "connection_usage",
    "interface_definition",
    "interface_usage",
    "allocation_definition",
    "allocation_usage",
    "enum_definition",
    "enum_usage",
    "occurrence_definition",
    "occurrence_usage",
    "case_definition",
    "case_usage",
    "analysis_case_definition",
    "analysis_case_usage",
    "verification_case_definition",
    "verification_case_usage",
    "use_case_definition",
    "use_case_usage",
    "view_definition",
    "view_usage",
    "viewpoint_definition",
    "viewpoint_usage",
    "rendering_definition",
    "rendering_usage",
    "concern_definition",
    "concern_usage",
    "stakeholder_membership",
    "transition_usage",
    "calculation_definition",
    "calculation_usage",
    "flow_connection_usage",
    "succession_flow_connection_usage",
    "metadata_definition",
    "metadata_usage",
];

/// Format a document's content into whitespace-only edits.
///
/// Returns an empty vec when the tree-sitter parse fails (no edits can be
/// computed without nesting context).
pub fn compute_format_edits(content: &str, options: &FormatOptions) -> Vec<TextEdit> {
    let indent_size = if options.insert_spaces {
        options.tab_size as usize
    } else {
        1 // tab counts as 1
    };
    let indent_char = if options.insert_spaces { ' ' } else { '\t' };

    let parser = TreeSitterParser::new();
    let Some(tree) = parser.parse_tree(content) else {
        return Vec::new();
    };

    let mut edits: Vec<TextEdit> = Vec::new();
    let lines: Vec<&str> = content.lines().collect();
    let has_final_newline = content.ends_with('\n');

    // Pass 1: indentation and trailing whitespace per line.
    let mut consecutive_blank = 0u32;
    for (line_idx, line) in lines.iter().enumerate() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            consecutive_blank += 1;
            if consecutive_blank > 2 {
                let line_u32 = line_idx as u32;
                let (le, ce) = if line_idx + 1 < lines.len() {
                    ((line_idx + 1) as u32, 0u32)
                } else {
                    (line_u32, line.len() as u32)
                };
                edits.push(TextEdit {
                    expected_old_text: None,
                    line_start: line_u32,
                    col_start: 0,
                    line_end: le,
                    col_end: ce,
                    new_text: String::new(),
                });
            } else if !line.is_empty() {
                edits.push(TextEdit {
                    expected_old_text: None,
                    line_start: line_idx as u32,
                    col_start: 0,
                    line_end: line_idx as u32,
                    col_end: line.len() as u32,
                    new_text: String::new(),
                });
            }
            continue;
        }

        consecutive_blank = 0;

        let line_start_byte = byte_offset_of_line(content, line_idx);
        let first_non_ws = line.len() - trimmed.len();
        let node_byte = line_start_byte + first_non_ws;

        let depth = compute_nesting_depth(&tree, node_byte);
        let effective_depth = if trimmed.starts_with('}') {
            depth.saturating_sub(1)
        } else {
            depth
        };

        let expected_indent_len = effective_depth * indent_size;
        let expected_indent: String =
            std::iter::repeat_n(indent_char, expected_indent_len).collect();

        let actual_indent = &line[..first_non_ws];
        if actual_indent != expected_indent {
            edits.push(TextEdit {
                expected_old_text: None,
                line_start: line_idx as u32,
                col_start: 0,
                line_end: line_idx as u32,
                col_end: first_non_ws as u32,
                new_text: expected_indent,
            });
        }

        let content_end = line.trim_end().len();
        if content_end < line.len() {
            edits.push(TextEdit {
                expected_old_text: None,
                line_start: line_idx as u32,
                col_start: content_end as u32,
                line_end: line_idx as u32,
                col_end: line.len() as u32,
                new_text: String::new(),
            });
        }

        if let Some(semi_pos) = trimmed.rfind(';') {
            let abs_semi = first_non_ws + semi_pos;
            if abs_semi > 0 && line.as_bytes().get(abs_semi - 1) == Some(&b' ') {
                let mut space_start = abs_semi - 1;
                while space_start > 0 && line.as_bytes().get(space_start - 1) == Some(&b' ') {
                    space_start -= 1;
                }
                if space_start < abs_semi {
                    edits.push(TextEdit {
                        expected_old_text: None,
                        line_start: line_idx as u32,
                        col_start: space_start as u32,
                        line_end: line_idx as u32,
                        col_end: abs_semi as u32,
                        new_text: String::new(),
                    });
                }
            }
        }
    }

    // Pass 2: ensure exactly one trailing newline at EOF.
    if !has_final_newline && !content.is_empty() {
        let last_line = lines.len().saturating_sub(1);
        let last_char = lines.last().map_or(0, |l| l.len());
        edits.push(TextEdit {
            expected_old_text: None,
            line_start: last_line as u32,
            col_start: last_char as u32,
            line_end: last_line as u32,
            col_end: last_char as u32,
            new_text: "\n".to_owned(),
        });
    }

    edits
}

fn byte_offset_of_line(content: &str, line: usize) -> usize {
    let mut offset = 0;
    for (idx, l) in content.lines().enumerate() {
        if idx == line {
            return offset;
        }
        offset += l.len() + 1;
    }
    offset
}

fn compute_nesting_depth(tree: &tree_sitter::Tree, byte_offset: usize) -> usize {
    let root = tree.root_node();
    let Some(node) = root.descendant_for_byte_range(byte_offset, byte_offset) else {
        return 0;
    };

    let mut depth = 0;
    let mut current = node;
    loop {
        let kind = current.kind();
        if kind != "source_file" && NESTING_KINDS.contains(&kind) {
            depth += 1;
        }
        match current.parent() {
            Some(parent) => current = parent,
            None => break,
        }
    }
    depth
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_content_no_edits() {
        let edits = compute_format_edits("", &FormatOptions::default());
        assert!(edits.is_empty());
    }

    #[test]
    fn missing_final_newline_added() {
        let edits = compute_format_edits("package P {}", &FormatOptions::default());
        let nl = edits.iter().find(|e| e.new_text == "\n");
        assert!(
            nl.is_some(),
            "expected trailing newline edit, got {:?}",
            edits
        );
    }

    #[test]
    fn trailing_whitespace_removed() {
        let edits = compute_format_edits("package P {   \n}\n", &FormatOptions::default());
        let trailing = edits
            .iter()
            .find(|e| e.line_start == 0 && e.new_text.is_empty() && e.col_start < e.col_end);
        assert!(
            trailing.is_some(),
            "expected trailing-ws removal, got {:?}",
            edits
        );
    }

    #[test]
    fn byte_offset_of_line_works() {
        let content = "line0\nline1\nline2\n";
        assert_eq!(byte_offset_of_line(content, 0), 0);
        assert_eq!(byte_offset_of_line(content, 1), 6);
        assert_eq!(byte_offset_of_line(content, 2), 12);
    }
}
