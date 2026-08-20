#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
//! Snapshot tests for diagnostics and semantic tokens.
//!
//! Uses the `insta` crate to maintain human-readable `.snap` files that capture
//! the exact output of the diagnostic pipeline and semantic token builder.
//! Any change in output produces a diff that must be explicitly reviewed and
//! accepted with `cargo insta review`.

use std::collections::HashSet;

use crate::diagnose_source_support::{diagnose_source, DiagnoseOptions};
use crate::types::{
    MOD_ABSTRACT, MOD_DEFINITION, MOD_DERIVED, MOD_READONLY, SEMANTIC_TOKEN_TYPES, SYNTHETIC_FILE,
};
use crate::utils::offset_to_position;
use sysml_span::Severity;
use sysml_parser_incremental::{build_model_graph, TreeSitterParser};

/// Test-local adapter over the production element-kind → LSP token-type
/// mapping: `sysml_ide_db::element_kind_to_category` composed with
/// `semantic_tokens::token_category_to_lsp`. Replaces the retired
/// `kinds::element_kind_to_token_type` duplicate so this render helper
/// mirrors real production token types.
fn element_kind_to_token_type(kind: &sysml_core::ElementKind) -> u32 {
    crate::semantic_tokens::token_category_to_lsp(sysml_ide_db::element_kind_to_category(kind))
}

// ── Fixtures ────────────────────────────────────────────────────────

const CLEAN: &str = include_str!("../fixtures/valid/clean.sysml");
const DUPLICATE_NAMES: &str = include_str!("../fixtures/invalid/duplicate_names.sysml");
const MISSING_SEMICOLON: &str = include_str!("../fixtures/invalid/missing_semicolon.sysml");
const CORRUPTED_LINE: &str = include_str!("../fixtures/invalid/corrupted_line.sysml");
const TYPO_REFERENCE: &str = include_str!("../fixtures/invalid/typo_reference.sysml");
const WRONG_TYPING: &str = include_str!("../fixtures/invalid/wrong_typing.sysml");

// Advanced construct fixtures
const STATE_MACHINE: &str = include_str!("../fixtures/valid/state_machine.sysml");
const ACTIONS_AND_FLOWS: &str = include_str!("../fixtures/valid/actions_and_flows.sysml");
const CONSTRAINTS_AND_REQUIREMENTS: &str =
    include_str!("../fixtures/valid/constraints_and_requirements.sysml");
const PORTS_AND_INTERFACES: &str = include_str!("../fixtures/valid/ports_and_interfaces.sysml");
const ENUMS_AND_CALCULATIONS: &str = include_str!("../fixtures/valid/enums_and_calculations.sysml");
const COMPREHENSIVE: &str = include_str!("../fixtures/valid/comprehensive.sysml");

// Book example fixtures — tests/fixtures/book-examples/
const BOOK_CM_PACKAGE_STRUCTURE: &str =
    include_str!("../../../../tests/fixtures/book-examples/coffee-machine/package-structure.sysml");
const BOOK_CM_DEFINITIONS: &str =
    include_str!("../../../../tests/fixtures/book-examples/coffee-machine/definitions.sysml");
const BOOK_CM_TYPING: &str =
    include_str!("../../../../tests/fixtures/book-examples/coffee-machine/typing-and-specialization.sysml");
const BOOK_CM_PORTS: &str =
    include_str!("../../../../tests/fixtures/book-examples/coffee-machine/ports-and-interfaces.sysml");
const BOOK_CM_CONNECTIONS: &str =
    include_str!("../../../../tests/fixtures/book-examples/coffee-machine/connections.sysml");
const BOOK_CM_FLOWS: &str = include_str!("../../../../tests/fixtures/book-examples/coffee-machine/flows.sysml");
const BOOK_CM_ACTIONS: &str =
    include_str!("../../../../tests/fixtures/book-examples/coffee-machine/actions.sysml");
const BOOK_CM_STATES: &str = include_str!("../../../../tests/fixtures/book-examples/coffee-machine/states.sysml");
const BOOK_CM_CALCULATIONS: &str =
    include_str!("../../../../tests/fixtures/book-examples/coffee-machine/calculations.sysml");
const BOOK_CM_REQUIREMENTS: &str =
    include_str!("../../../../tests/fixtures/book-examples/coffee-machine/requirements.sysml");
const BOOK_CM_VIEWS: &str = include_str!("../../../../tests/fixtures/book-examples/coffee-machine/views.sysml");
const BOOK_CM_METADATA: &str =
    include_str!("../../../../tests/fixtures/book-examples/coffee-machine/metadata.sysml");
const BOOK_BW_TYPES: &str =
    include_str!("../../../../tests/fixtures/book-examples/beverage-workspace/beverage-types/src/types.sysml");
const BOOK_BW_PARTS: &str =
    include_str!("../../../../tests/fixtures/book-examples/beverage-workspace/coffee-machine/src/parts.sysml");

const URI: &str = "file:///test.sysml";

// ── Render helpers ──────────────────────────────────────────────────

/// Strip UUID-like patterns (8-4-4-4-12 hex) for deterministic snapshots.
fn sanitize_uuids(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        // Look for UUID pattern: 8 hex chars, then "-", 4 hex, "-", 4 hex, "-", 4 hex, "-", 12 hex
        if i + 36 <= chars.len()
            && chars[i + 8] == '-'
            && chars[i + 13] == '-'
            && chars[i + 18] == '-'
            && chars[i + 23] == '-'
        {
            let candidate: String = chars[i..i + 36].iter().collect();
            if candidate.chars().enumerate().all(|(j, c)| {
                if j == 8 || j == 13 || j == 18 || j == 23 {
                    c == '-'
                } else {
                    c.is_ascii_hexdigit()
                }
            }) {
                result.push_str("<UUID>");
                i += 36;
                continue;
            }
        }
        result.push(chars[i]);
        i += 1;
    }
    result
}

/// Convert byte offset to 1-indexed `line:col` string.
fn offset_to_line_col(offset: usize, source: &str) -> String {
    let pos = offset_to_position(offset, source);
    format!("{}:{}", pos.line + 1, pos.character + 1)
}

/// Render diagnostics as stable, diffable text.
///
/// Format per diagnostic:
/// ```text
/// [CODE] severity @ line:col..line:col
///   message text
///   note: each note on its own line
/// ```
fn render_diagnostics(source: &str, uri: &str) -> String {
    let opts = DiagnoseOptions {
        resolution: true,
        validation: true,
    };
    let result = diagnose_source(source, uri, &opts);

    // Render each diagnostic as a self-contained block, then group by
    // (severity, code, message) and pick the block with the smallest span
    // from each group. This produces fully deterministic output regardless
    // of HashMap iteration order in checks like S001 (duplicate name diagnostic
    // can attach to either of two identical declarations across runs).
    struct Block {
        sort_key: String,
        span_start: usize,
        rendered: String,
    }
    let mut blocks: Vec<Block> = Vec::new();
    for diag in &result.diagnostics {
        let code = diag.code.as_deref().unwrap_or("----");
        let severity = match diag.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        let span_info = if let Some(ref span) = diag.span {
            let text = source
                .get(span.start..span.end)
                .unwrap_or("<out-of-bounds>")
                .replace('\n', "\\n");
            let display_text = if text.chars().count() > 60 {
                let truncated: String = text.chars().take(57).collect();
                format!("{}...", truncated)
            } else {
                text
            };
            format!("{} @ \"{}\"", severity, display_text)
        } else {
            format!("{} @ <no span>", severity)
        };

        let mut rendered = String::new();
        rendered.push_str(&format!("[{}] {}\n", code, span_info));
        rendered.push_str(&format!("  {}\n", sanitize_uuids(&diag.message)));
        for note in &diag.notes {
            rendered.push_str(&format!("  note: {}\n", sanitize_uuids(note)));
        }

        // Sort key: (severity_rank, code, message) — excludes position for determinism
        let sev_rank = match diag.severity {
            Severity::Error => 0,
            Severity::Warning => 1,
            Severity::Info => 2,
        };
        let sort_key = format!("{}:{}:{}", sev_rank, code, diag.message);
        let span_start = diag.span.as_ref().map(|s| s.start).unwrap_or(usize::MAX);
        blocks.push(Block {
            sort_key,
            span_start,
            rendered,
        });
    }
    // Sort by sort_key, then by span_start for determinism within same key
    blocks.sort_by(|a, b| {
        a.sort_key
            .cmp(&b.sort_key)
            .then(a.span_start.cmp(&b.span_start))
    });
    // Dedup: when multiple blocks share a sort_key, keep only the one with
    // the smallest span_start (already first after sort).
    blocks.dedup_by(|b, a| a.sort_key == b.sort_key);

    let mut out: String = blocks.into_iter().map(|b| b.rendered).collect();
    if out.is_empty() {
        out.push_str("(no diagnostics)\n");
    }
    out
}

/// Render semantic tokens as stable, diffable text.
///
/// Format per token:
/// ```text
/// line:col..line:col  "text_slice"  TOKEN_TYPE  [mod1, mod2]
/// ```
fn render_tokens(source: &str, uri: &str) -> String {
    let ts_parser = TreeSitterParser::new();
    let Some(tree) = ts_parser.parse_tree(source) else {
        return "(parse failed)\n".to_string();
    };

    let graph_result = build_model_graph(&tree, source, uri);
    let graph = graph_result.graph;

    // Collect raw token tuples by walking model graph elements (same logic as add_model_tokens)
    let mut tokens: Vec<(usize, usize, u32, u32)> = Vec::new();

    for element in graph.elements.values() {
        if matches!(
            element.kind,
            sysml_core::ElementKind::Membership | sysml_core::ElementKind::OwningMembership
        ) {
            continue;
        }
        for (idx, span) in element.spans.iter().enumerate() {
            if span.file != uri {
                continue;
            }
            if span.file == SYNTHETIC_FILE || span.start == span.end {
                continue;
            }
            let token_type = element_kind_to_token_type(&element.kind);
            let mut modifiers = 0u32;
            if idx == 0 && !element.kind.is_relationship() {
                modifiers |= MOD_DEFINITION;
            }
            if element
                .props
                .get("isAbstract")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                modifiers |= MOD_ABSTRACT;
            }
            if element
                .props
                .get("isReadOnly")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                modifiers |= MOD_READONLY;
            }
            if element
                .props
                .get("isDerived")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                modifiers |= MOD_DERIVED;
            }
            tokens.push((span.start, span.end, token_type, modifiers));
        }
    }

    // Collect CST tokens. This test renders tokens via its own walker
    // (`collect_cst_tokens` below) rather than the production
    // `file_semantic_tokens` salsa path — a divergent path tracked in
    // sysml-ide-db/src/tokens.rs's module docs, not fixed here.
    collect_cst_tokens(&tree, &mut tokens);

    // Sort by start position, then by end position, then by token type for determinism
    tokens.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then(a.2.cmp(&b.2))
            .then(a.3.cmp(&b.3))
    });

    let mut out = String::new();
    for (start, end, type_idx, mod_bits) in &tokens {
        let start_lc = offset_to_line_col(*start, source);
        let end_lc = offset_to_line_col(*end, source);
        let text = source
            .get(*start..*end)
            .unwrap_or("<out-of-bounds>")
            .replace('\n', "\\n");
        // Truncate long text for readability (char-boundary safe)
        let display_text = if text.chars().count() > 40 {
            let truncated: String = text.chars().take(37).collect();
            format!("{}...", truncated)
        } else {
            text
        };
        let type_name = token_type_name(*type_idx);
        let mods = modifier_names(*mod_bits);
        out.push_str(&format!(
            "{}..{}  \"{}\"  {}  [{}]\n",
            start_lc, end_lc, display_text, type_name, mods
        ));
    }
    if out.is_empty() {
        out.push_str("(no tokens)\n");
    }
    out
}

/// Walk the CST and collect raw token tuples (same logic as walk_cst_tokens).
fn collect_cst_tokens(tree: &tree_sitter::Tree, tokens: &mut Vec<(usize, usize, u32, u32)>) {
    let mut cursor = tree.walk();
    collect_cst_tokens_recursive(&mut cursor, tokens);
}

fn collect_cst_tokens_recursive(
    cursor: &mut tree_sitter::TreeCursor<'_>,
    tokens: &mut Vec<(usize, usize, u32, u32)>,
) {
    loop {
        let node = cursor.node();
        let start = node.start_byte();
        let end = node.end_byte();

        if start < end {
            let kind = node.kind();
            let token_type = match kind {
                "comment" | "doc_comment" | "doc_string" | "comment_element" => Some(9u32), // COMMENT
                "string_literal" => Some(10),                                // STRING
                "integer_literal" | "real_literal" => Some(11),              // NUMBER
                _ if !node.is_named() => {
                    let text = node.kind();
                    if text.len() >= 2 && text.chars().all(|c| c.is_ascii_alphabetic() || c == '_')
                    {
                        Some(8) // KEYWORD
                    } else if text
                        .chars()
                        .all(|c| !c.is_alphanumeric() && !c.is_whitespace())
                        && !text.is_empty()
                    {
                        match text {
                            "{" | "}" | "(" | ")" | "[" | "]" => None,
                            _ => Some(12), // OPERATOR
                        }
                    } else {
                        None
                    }
                }
                _ if node.is_named() && node.kind() == "identifier" => {
                    if let Some(parent) = node.parent() {
                        match parent.kind() {
                            "type_ref" | "qualified_name" | "feature_chain" => Some(1u32), // TYPE
                            "import_single_name" | "import_qualified_name" => Some(0u32), // NAMESPACE
                            _ => None,
                        }
                    } else {
                        None
                    }
                }
                _ => None,
            };

            if let Some(tt) = token_type {
                tokens.push((start, end, tt, 0));
                if !cursor.goto_next_sibling() {
                    return;
                }
                continue;
            }
        }

        if cursor.goto_first_child() {
            collect_cst_tokens_recursive(cursor, tokens);
            cursor.goto_parent();
        }

        if !cursor.goto_next_sibling() {
            return;
        }
    }
}

/// Map token type index to name from SEMANTIC_TOKEN_TYPES.
fn token_type_name(idx: u32) -> &'static str {
    let names = [
        "NAMESPACE",
        "TYPE",
        "CLASS",
        "STRUCT",
        "PROPERTY",
        "VARIABLE",
        "PARAMETER",
        "FUNCTION",
        "KEYWORD",
        "COMMENT",
        "STRING",
        "NUMBER",
        "OPERATOR",
        "INTERFACE",
        "ENUM",
    ];
    names.get(idx as usize).unwrap_or(&"UNKNOWN")
}

/// Map modifier bit flags to comma-separated names.
fn modifier_names(bits: u32) -> String {
    let names = [
        "definition",
        "declaration",
        "readonly",
        "static",
        "abstract",
        "deprecated",
    ];
    let mut result = Vec::new();
    for (i, name) in names.iter().enumerate() {
        if bits & (1 << i) != 0 {
            result.push(*name);
        }
    }
    result.join(", ")
}

// ── Phase 1: Diagnostic snapshot tests ──────────────────────────────

#[test]
fn snapshot_diagnostics_clean() {
    let rendered = render_diagnostics(CLEAN, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_duplicate_names() {
    let rendered = render_diagnostics(DUPLICATE_NAMES, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_typo_reference() {
    let rendered = render_diagnostics(TYPO_REFERENCE, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_missing_semicolon() {
    let rendered = render_diagnostics(MISSING_SEMICOLON, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_corrupted_line() {
    let rendered = render_diagnostics(CORRUPTED_LINE, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_wrong_typing() {
    let rendered = render_diagnostics(WRONG_TYPING, URI);
    insta::assert_snapshot!(rendered);
}

// ── Phase 2: Semantic token snapshot tests ──────────────────────────

#[test]
fn snapshot_tokens_clean() {
    let rendered = render_tokens(CLEAN, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_duplicate_names() {
    let rendered = render_tokens(DUPLICATE_NAMES, URI);
    insta::assert_snapshot!(rendered);
}

// ── Phase 3: Advanced construct diagnostic snapshots ─────────────────

#[test]
fn snapshot_diagnostics_state_machine() {
    let rendered = render_diagnostics(STATE_MACHINE, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_actions_and_flows() {
    let rendered = render_diagnostics(ACTIONS_AND_FLOWS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_constraints_and_requirements() {
    let rendered = render_diagnostics(CONSTRAINTS_AND_REQUIREMENTS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_ports_and_interfaces() {
    let rendered = render_diagnostics(PORTS_AND_INTERFACES, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_enums_and_calculations() {
    let rendered = render_diagnostics(ENUMS_AND_CALCULATIONS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_comprehensive() {
    let rendered = render_diagnostics(COMPREHENSIVE, URI);
    insta::assert_snapshot!(rendered);
}

// ── Phase 4: Advanced construct token snapshots ──────────────────────

#[test]
fn snapshot_tokens_state_machine() {
    let rendered = render_tokens(STATE_MACHINE, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_actions_and_flows() {
    let rendered = render_tokens(ACTIONS_AND_FLOWS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_constraints_and_requirements() {
    let rendered = render_tokens(CONSTRAINTS_AND_REQUIREMENTS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_ports_and_interfaces() {
    let rendered = render_tokens(PORTS_AND_INTERFACES, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_enums_and_calculations() {
    let rendered = render_tokens(ENUMS_AND_CALCULATIONS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_comprehensive() {
    let rendered = render_tokens(COMPREHENSIVE, URI);
    insta::assert_snapshot!(rendered);
}

// ── Phase 5: Quality gate tests ─────────────────────────────────────

#[test]
fn snapshot_token_type_coverage() {
    // Parse the comprehensive fixture and check that we see all 15 token types.
    let ts_parser = TreeSitterParser::new();
    let source = COMPREHENSIVE;
    let tree = ts_parser.parse_tree(source).expect("parse should succeed");
    let graph_result = build_model_graph(&tree, source, URI);
    let graph = graph_result.graph;

    let mut seen_types: HashSet<u32> = HashSet::new();

    // Model tokens
    for element in graph.elements.values() {
        for span in &element.spans {
            if span.file != URI || span.file == SYNTHETIC_FILE || span.start == span.end {
                continue;
            }
            seen_types.insert(element_kind_to_token_type(&element.kind));
        }
    }

    // CST tokens
    let mut cst_tokens = Vec::new();
    collect_cst_tokens(&tree, &mut cst_tokens);
    for (_, _, type_idx, _) in &cst_tokens {
        seen_types.insert(*type_idx);
    }

    // Document which types appeared
    let mut coverage_report = String::new();
    coverage_report.push_str("Token type coverage from comprehensive.sysml:\n\n");
    for (idx, _tt) in SEMANTIC_TOKEN_TYPES.iter().enumerate() {
        let status = if seen_types.contains(&(idx as u32)) {
            "SEEN"
        } else {
            "----"
        };
        coverage_report.push_str(&format!(
            "  {:>2}  {:12}  {}\n",
            idx,
            token_type_name(idx as u32),
            status
        ));
    }
    coverage_report.push_str(&format!(
        "\nTotal: {}/{} types seen\n",
        seen_types.len(),
        SEMANTIC_TOKEN_TYPES.len()
    ));

    insta::assert_snapshot!(coverage_report);
}

#[test]
fn snapshot_unused_types_and_modifiers_audit() {
    // Parse multiple fixtures and audit which token types and modifiers are never produced.
    let fixtures: &[(&str, &str)] = &[
        ("clean", CLEAN),
        ("duplicate_names", DUPLICATE_NAMES),
        ("typo_reference", TYPO_REFERENCE),
        ("wrong_typing", WRONG_TYPING),
        ("state_machine", STATE_MACHINE),
        ("actions_and_flows", ACTIONS_AND_FLOWS),
        ("constraints_and_requirements", CONSTRAINTS_AND_REQUIREMENTS),
        ("ports_and_interfaces", PORTS_AND_INTERFACES),
        ("enums_and_calculations", ENUMS_AND_CALCULATIONS),
        ("comprehensive", COMPREHENSIVE),
    ];

    let mut all_types: HashSet<u32> = HashSet::new();
    let mut all_mods: HashSet<u32> = HashSet::new();
    let ts_parser = TreeSitterParser::new();

    for (_name, source) in fixtures {
        if let Some(tree) = ts_parser.parse_tree(source) {
            let graph_result = build_model_graph(&tree, source, URI);
            let graph = graph_result.graph;

            for element in graph.elements.values() {
                if matches!(
                    element.kind,
                    sysml_core::ElementKind::Membership | sysml_core::ElementKind::OwningMembership
                ) {
                    continue;
                }
                for (idx, span) in element.spans.iter().enumerate() {
                    if span.file != URI || span.file == SYNTHETIC_FILE || span.start == span.end {
                        continue;
                    }
                    all_types.insert(element_kind_to_token_type(&element.kind));
                    let mut mods = 0u32;
                    if idx == 0 && !element.kind.is_relationship() {
                        mods |= MOD_DEFINITION;
                    }
                    if element
                        .props
                        .get("isAbstract")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        mods |= MOD_ABSTRACT;
                    }
                    if element
                        .props
                        .get("isReadOnly")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        mods |= MOD_READONLY;
                    }
                    if element
                        .props
                        .get("isDerived")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                    {
                        mods |= MOD_DERIVED;
                    }
                    for bit in 0..6 {
                        if mods & (1 << bit) != 0 {
                            all_mods.insert(bit);
                        }
                    }
                }
            }

            let mut cst_tokens = Vec::new();
            collect_cst_tokens(&tree, &mut cst_tokens);
            for (_, _, type_idx, mod_bits) in &cst_tokens {
                all_types.insert(*type_idx);
                for bit in 0..6 {
                    if mod_bits & (1 << bit) != 0 {
                        all_mods.insert(bit);
                    }
                }
            }
        }
    }

    let mod_names = [
        "DEFINITION",
        "DECLARATION",
        "READONLY",
        "STATIC",
        "ABSTRACT",
        "DEPRECATED",
    ];

    let mut report = String::new();
    report.push_str("Unused token types (across 10 fixtures):\n");
    let mut any_unused_type = false;
    for (idx, _tt) in SEMANTIC_TOKEN_TYPES.iter().enumerate() {
        if !all_types.contains(&(idx as u32)) {
            report.push_str(&format!("  {:>2}  {}\n", idx, token_type_name(idx as u32)));
            any_unused_type = true;
        }
    }
    if !any_unused_type {
        report.push_str("  (none)\n");
    }

    report.push_str("\nUnused token modifiers (across 10 fixtures):\n");
    let mut any_unused_mod = false;
    for (idx, name) in mod_names.iter().enumerate() {
        if !all_mods.contains(&(idx as u32)) {
            report.push_str(&format!("  {:>2}  {}\n", idx, name));
            any_unused_mod = true;
        }
    }
    if !any_unused_mod {
        report.push_str("  (none)\n");
    }

    insta::assert_snapshot!(report);
}

// ── Phase 6: Book example diagnostic snapshots ──────────────────────

#[test]
fn snapshot_diagnostics_book_cm_package_structure() {
    let rendered = render_diagnostics(BOOK_CM_PACKAGE_STRUCTURE, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_definitions() {
    let rendered = render_diagnostics(BOOK_CM_DEFINITIONS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_typing() {
    let rendered = render_diagnostics(BOOK_CM_TYPING, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_ports() {
    let rendered = render_diagnostics(BOOK_CM_PORTS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_connections() {
    let rendered = render_diagnostics(BOOK_CM_CONNECTIONS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_flows() {
    let rendered = render_diagnostics(BOOK_CM_FLOWS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_actions() {
    let rendered = render_diagnostics(BOOK_CM_ACTIONS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_states() {
    let rendered = render_diagnostics(BOOK_CM_STATES, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_calculations() {
    let rendered = render_diagnostics(BOOK_CM_CALCULATIONS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_requirements() {
    let rendered = render_diagnostics(BOOK_CM_REQUIREMENTS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_views() {
    let rendered = render_diagnostics(BOOK_CM_VIEWS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_cm_metadata() {
    let rendered = render_diagnostics(BOOK_CM_METADATA, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_bw_types() {
    let rendered = render_diagnostics(BOOK_BW_TYPES, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_diagnostics_book_bw_parts() {
    let rendered = render_diagnostics(BOOK_BW_PARTS, URI);
    insta::assert_snapshot!(rendered);
}

// ── Phase 7: Book example token snapshots ────────────────────────────

#[test]
fn snapshot_tokens_book_cm_package_structure() {
    let rendered = render_tokens(BOOK_CM_PACKAGE_STRUCTURE, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_definitions() {
    let rendered = render_tokens(BOOK_CM_DEFINITIONS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_typing() {
    let rendered = render_tokens(BOOK_CM_TYPING, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_ports() {
    let rendered = render_tokens(BOOK_CM_PORTS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_connections() {
    let rendered = render_tokens(BOOK_CM_CONNECTIONS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_flows() {
    let rendered = render_tokens(BOOK_CM_FLOWS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_actions() {
    let rendered = render_tokens(BOOK_CM_ACTIONS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_states() {
    let rendered = render_tokens(BOOK_CM_STATES, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_calculations() {
    let rendered = render_tokens(BOOK_CM_CALCULATIONS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_requirements() {
    let rendered = render_tokens(BOOK_CM_REQUIREMENTS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_views() {
    let rendered = render_tokens(BOOK_CM_VIEWS, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_cm_metadata() {
    let rendered = render_tokens(BOOK_CM_METADATA, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_bw_types() {
    let rendered = render_tokens(BOOK_BW_TYPES, URI);
    insta::assert_snapshot!(rendered);
}

#[test]
fn snapshot_tokens_book_bw_parts() {
    let rendered = render_tokens(BOOK_BW_PARTS, URI);
    insta::assert_snapshot!(rendered);
}
