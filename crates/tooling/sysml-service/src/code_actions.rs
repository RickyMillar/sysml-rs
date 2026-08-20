//! Code action (quick-fix, refactoring, source action) generation.
//!
//! Replaces the LSP-side `generate_code_actions` body. Replicates the full
//! surface from `sysml_lsp_server::code_actions`:
//! - Quick-fixes: auto-import, use qualified name, create missing definition
//!   stub, rename duplicate, insert missing semicolon, insert missing brace,
//!   replace `Real` with ISQ type.
//! - Refactorings: expand definition body, keyword form toggles, toggle
//!   abstract, add doc comment.
//! - Source actions: organize imports.
//!
//! All edits use line/column coordinates (UTF-16 code units, 0-indexed —
//! matches LSP convention) so the LSP shim is a 1:1 translation. The service
//! has no `tower-lsp` dependency.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use sysml_core::ModelGraph;
use sysml_id::ElementId;
use sysml_ide_db::{AnalysisHost, Cancelled};

use crate::error::ServiceError;
use crate::position::{offset_to_line_col, position_to_offset};
use crate::text_edit::TextEdit;

// ─── Wire types ───────────────────────────────────────────────────────────────

/// One diagnostic carried into / out of code-action computation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeActionDiagnostic {
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
    pub code: Option<String>,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<u32>,
}

/// One code action returned to the caller.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeAction {
    pub title: String,
    /// LSP `CodeActionKind` — `quickfix`, `refactor.rewrite`,
    /// `source.organizeImports`, etc.
    pub kind: String,
    pub edits: Vec<CodeActionFileEdits>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<CodeActionDiagnostic>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_preferred: Option<bool>,
    /// Optional server-side command to invoke when this action is selected.
    /// Used by command-only actions (e.g. "Create sysml.toml", "Open this
    /// folder") that have no text-edit form. Maps to LSP `CodeAction.command`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<CodeActionCommand>,
}

/// A command attached to a CodeAction. Maps to LSP `Command`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeActionCommand {
    /// Human-readable title for the command (LSP `Command.title`).
    pub title: String,
    /// Server command identifier registered in `command_dispatch.rs`
    /// (e.g. `"sysml.init"`, `"sysml.workspace.open_folder"`).
    pub command: String,
    /// JSON arguments passed verbatim to the command handler.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<serde_json::Value>,
}

/// All edits for a single file in a code action.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodeActionFileEdits {
    pub uri: String,
    pub edits: Vec<TextEdit>,
}

// ─── Public entry point ───────────────────────────────────────────────────────

/// Compute every code action that applies at `(uri, range)` given the supplied
/// diagnostics.
pub fn compute_code_actions(
    host: &Mutex<AnalysisHost>,
    uri: &str,
    range_start_line: u32,
    range_start_col: u32,
    range_end_line: u32,
    range_end_col: u32,
    diagnostics: &[CodeActionDiagnostic],
) -> Result<Vec<CodeAction>, ServiceError> {
    // Resolve every host-keyed handle under a SMALL guard, then run the
    // salsa queries lock-free — this path elaborates the whole workspace
    // (`elaborate_workspace_best`) and parses every file
    // (`WorkspaceSnapshot`); doing that under the guard serializes every
    // other host user (precedent: `compute_full_diagnostics`).
    let (analysis, sf, project_id, all_files) = {
        let guard = host.lock().unwrap();
        let Some(file_id) = guard.file_id(uri) else {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        };
        let Some(sf) = guard.source_file(file_id) else {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        };
        let project_id = guard.files().project_id(file_id);
        let all_files: Vec<(String, sysml_ide_db::SourceFile)> = guard
            .files()
            .file_ids()
            .filter_map(|fid| Some((guard.files().uri(fid)?.to_string(), guard.source_file(fid)?)))
            .collect();
        (guard.analysis(), sf, project_id, all_files)
    };

    let content = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
        analysis.file_text(sf).to_owned()
    }))
    .unwrap_or_default();
    let library_graph_arc = analysis
        .library_graph()
        .map(|lg| std::sync::Arc::new(lg.data(analysis.db()).graph().clone()));
    let ws = WorkspaceSnapshot::from_snapshot(&analysis, &all_files);

    // Phase 4: workspace-aware auto-import enumeration. The legacy
    // WorkspaceSnapshot walk parses per-file graphs and indexes by element
    // name; its URI filter is fragile under canonicalization mismatches and
    // the parse graph misses cross-file resolution updates. Use the
    // salsa-cached workspace_name_index + the elaborated workspace graph
    // instead — same data, salsa-memoized, and built from the merged +
    // resolved graph the rest of the IDE features already trust.
    let workspace_merged_graph_arc = analysis
        .elaborate_workspace_best(project_id)
        .map(|elab| elab.graph().clone());
    let workspace_name_index_arc: Option<std::sync::Arc<HashMap<String, Vec<ElementId>>>> =
        analysis
            .workspace_name_index_best(project_id)
            .map(|idx| idx.arc());

    drop(analysis);

    let library_graph: Option<&ModelGraph> = library_graph_arc.as_deref();
    let workspace_merged_graph: Option<&ModelGraph> = workspace_merged_graph_arc.as_deref();

    let mut actions: Vec<CodeAction> = Vec::new();

    // 1. Diagnostic-based quick-fixes.
    actions.extend(diagnostic_actions(
        uri,
        diagnostics,
        &ws,
        library_graph,
        workspace_merged_graph,
        workspace_name_index_arc.as_deref(),
        &content,
    ));

    // 2. Cursor-position refactorings + 3. Source actions (require tree).
    let parser = sysml_parser_incremental::TreeSitterParser::new();
    if let Some(tree) = parser.parse_tree(&content) {
        actions.extend(cursor_refactorings(
            uri,
            &content,
            range_start_line,
            range_start_col,
            range_end_line,
            range_end_col,
            &tree,
        ));
        actions.extend(source_actions(uri, &content, &tree));
    }

    Ok(actions)
}

// ─── Diagnostic-based quick-fixes ─────────────────────────────────────────────

fn diagnostic_actions(
    uri: &str,
    diagnostics: &[CodeActionDiagnostic],
    ws: &WorkspaceSnapshot,
    library_graph: Option<&ModelGraph>,
    workspace_merged_graph: Option<&ModelGraph>,
    workspace_name_index: Option<&HashMap<String, Vec<ElementId>>>,
    content: &str,
) -> Vec<CodeAction> {
    let mut actions: Vec<CodeAction> = Vec::new();

    for diag in diagnostics {
        let code = diag.code.as_deref();
        // IM010 is the actionable replacement for the lenient-fallback path;
        // E200 stays for true unresolved errors. Both should drive the same
        // auto-import / use-qualified / create-definition quick-fix set.
        let is_unresolved = code == Some("E200")
            || code == Some("IM010")
            || looks_like_unresolved_reference(&diag.message);
        if is_unresolved {
            actions.extend(auto_import_actions(
                uri,
                diag,
                ws,
                library_graph,
                workspace_merged_graph,
                workspace_name_index,
                content,
            ));
            if let Some(action) = use_qualified_name_action(uri, diag, library_graph, content) {
                actions.push(action);
            }
            if let Some(action) = create_definition_action(uri, diag, content) {
                actions.push(action);
            }
        }

        if code == Some("IM012") {
            actions.extend(strict_mode_actions(uri, diag));
        }

        if code == Some("S001") {
            if let Some(action) = rename_duplicate_action(uri, diag) {
                actions.push(action);
            }
        }

        if code == Some("PH006") {
            if let Some(action) = replace_real_with_isq_action(uri, diag, content) {
                actions.push(action);
            }
        }

        let msg_lower = diag.message.to_lowercase();
        if msg_lower.contains("expected")
            && (msg_lower.contains("\";\"")
                || msg_lower.contains("semicolon")
                || msg_lower.contains("';'"))
        {
            if let Some(action) = insert_semicolon_action(uri, diag) {
                actions.push(action);
            }
        }
        if msg_lower.contains("expected")
            && (msg_lower.contains("\"}\"")
                || msg_lower.contains("'}'")
                || msg_lower.contains("closing brace"))
        {
            if let Some(action) = insert_closing_brace_action(uri, diag) {
                actions.push(action);
            }
        }
    }

    actions
}

fn collect_workspace_import_qnames_best(
    workspace_merged_graph: Option<&ModelGraph>,
    workspace_name_index: Option<&HashMap<String, Vec<ElementId>>>,
    name: &str,
    cursor_uri: &str,
) -> Vec<String> {
    match (workspace_merged_graph, workspace_name_index) {
        (Some(graph), Some(index)) => {
            collect_workspace_import_qnames(graph, index, name, cursor_uri)
        }
        _ => Vec::new(),
    }
}

fn auto_import_actions(
    uri: &str,
    diag: &CodeActionDiagnostic,
    ws: &WorkspaceSnapshot,
    library_graph: Option<&ModelGraph>,
    workspace_merged_graph: Option<&ModelGraph>,
    workspace_name_index: Option<&HashMap<String, Vec<ElementId>>>,
    content: &str,
) -> Vec<CodeAction> {
    let mut actions: Vec<CodeAction> = Vec::new();

    let Some(name) = unresolved_name_for_diagnostic(diag, content) else {
        return actions;
    };

    // Phase 4: workspace-aware auto-import enumeration via the salsa-cached
    // name index + elaborated workspace graph. The merged graph carries the
    // resolved qnames and the user-package element ids; the name index is the
    // fast lookup. Library elements stay handled by the library_graph branch
    // below; here we enumerate cross-file user-package definitions whose
    // simple name matches the unresolved identifier.
    let workspace_qnames: Vec<String> = collect_workspace_import_qnames_best(
        workspace_merged_graph,
        workspace_name_index,
        &name,
        uri,
    );

    let entries = ws.find_by_name(&name);
    let mut seen_qnames: HashSet<String> = HashSet::new();

    // First pass: dedupe synthetic qnames so the loop below doesn't double up.
    for entry in entries {
        if entry.uri == uri {
            continue;
        }
        let qname = format!(
            "{}::{}",
            entry
                .uri
                .rsplit('/')
                .next()
                .unwrap_or("")
                .trim_end_matches(".sysml"),
            name
        );
        let _ = seen_qnames.insert(qname);
    }
    // Reset — the original code kept the first pass as a no-op. Match its
    // observable behavior: only the second pass actually emits actions.
    seen_qnames.clear();

    // Workspace user-package candidates first (preferred over library and
    // over the legacy WorkspaceSnapshot path, which can miss cross-file
    // names under URI canonicalization mismatches).
    for qname_str in &workspace_qnames {
        if seen_qnames.insert(qname_str.clone()) {
            actions.push(make_import_action(uri, diag, &name, qname_str, content));
        }
    }

    // Library-graph candidates next.
    if let Some(graph) = library_graph {
        for element in graph.elements.values() {
            if element.name.as_deref() == Some(&name) {
                if let Some(qname) = &element.qname {
                    let qname_str = qname.to_string();
                    if seen_qnames.insert(qname_str.clone()) {
                        actions.push(make_import_action(uri, diag, &name, &qname_str, content));
                    }
                }
            }
        }
    }

    // Cross-file definitions — prefer the actual qualified name from the
    // workspace index; fall back to a synthetic file_stem::name.
    for entry in entries {
        if entry.uri == uri {
            continue;
        }
        let qname = {
            let mut found: Option<String> = None;
            ws.for_each_qname_in_file(&entry.uri, |qn, e| {
                if e.element_id == entry.element_id && found.is_none() {
                    found = Some(qn.to_owned());
                }
            });
            found
        }
        .unwrap_or_else(|| {
            let file_stem = entry
                .uri
                .rsplit('/')
                .next()
                .unwrap_or("")
                .trim_end_matches(".sysml");
            format!("{}::{}", file_stem, name)
        });
        if seen_qnames.insert(qname.clone()) {
            actions.push(make_import_action(uri, diag, &name, &qname, content));
        }
    }

    // Well-known library-types fallback.
    if actions.is_empty() {
        if let Some(qname) = suggest_library_import(&name) {
            actions.push(make_import_action(uri, diag, &name, qname, content));
        }
    }

    if let Some(first) = actions.first_mut() {
        first.is_preferred = Some(true);
    }

    actions
}

fn use_qualified_name_action(
    uri: &str,
    diag: &CodeActionDiagnostic,
    library_graph: Option<&ModelGraph>,
    content: &str,
) -> Option<CodeAction> {
    let name = unresolved_name_for_diagnostic(diag, content)?;
    let qname = library_graph
        .and_then(|graph| {
            graph
                .elements
                .values()
                .find(|e| e.name.as_deref() == Some(&name))
                .and_then(|e| e.qname.as_ref())
                .map(|q| q.to_string())
        })
        .or_else(|| suggest_library_import(&name).map(|s| s.to_owned()))?;

    let edit = TextEdit {
        expected_old_text: None,
        line_start: diag.line_start,
        col_start: diag.col_start,
        line_end: diag.line_end,
        col_end: diag.col_end,
        new_text: qname.clone(),
    };
    Some(make_action(
        uri,
        format!("Use fully qualified name '{}'", qname),
        "quickfix",
        vec![edit],
        Some(vec![diag.clone()]),
        None,
    ))
}

fn create_definition_action(
    uri: &str,
    diag: &CodeActionDiagnostic,
    content: &str,
) -> Option<CodeAction> {
    let name = unresolved_name_for_diagnostic(diag, content)?;
    let keyword = guess_definition_keyword(&diag.message);
    let insert_pos = find_definition_insertion_point(content, diag.line_start);
    let indent = get_indent_at_line(content, insert_pos.0);
    let stub_text = format!("\n{}{} def {} {{}}\n", indent, keyword, name);
    let edit = TextEdit {
        expected_old_text: None,
        line_start: insert_pos.0,
        col_start: insert_pos.1,
        line_end: insert_pos.0,
        col_end: insert_pos.1,
        new_text: stub_text,
    };
    Some(make_action(
        uri,
        format!("Create {} definition '{}'", keyword, name),
        "quickfix",
        vec![edit],
        Some(vec![diag.clone()]),
        None,
    ))
}

fn rename_duplicate_action(uri: &str, diag: &CodeActionDiagnostic) -> Option<CodeAction> {
    let name = extract_quoted_name(&diag.message)?;
    let new_name = format!("{}_2", name);
    let edit = TextEdit {
        expected_old_text: None,
        line_start: diag.line_start,
        col_start: diag.col_start,
        line_end: diag.line_end,
        col_end: diag.col_end,
        new_text: new_name.clone(),
    };
    Some(make_action(
        uri,
        format!("Rename to '{}'", new_name),
        "quickfix",
        vec![edit],
        Some(vec![diag.clone()]),
        None,
    ))
}

fn insert_semicolon_action(uri: &str, diag: &CodeActionDiagnostic) -> Option<CodeAction> {
    let edit = TextEdit {
        expected_old_text: None,
        line_start: diag.line_end,
        col_start: diag.col_end,
        line_end: diag.line_end,
        col_end: diag.col_end,
        new_text: ";".to_owned(),
    };
    Some(make_action(
        uri,
        "Insert missing semicolon".to_owned(),
        "quickfix",
        vec![edit],
        Some(vec![diag.clone()]),
        Some(true),
    ))
}

fn insert_closing_brace_action(uri: &str, diag: &CodeActionDiagnostic) -> Option<CodeAction> {
    let edit = TextEdit {
        expected_old_text: None,
        line_start: diag.line_end,
        col_start: diag.col_end,
        line_end: diag.line_end,
        col_end: diag.col_end,
        new_text: "}".to_owned(),
    };
    Some(make_action(
        uri,
        "Insert missing closing brace".to_owned(),
        "quickfix",
        vec![edit],
        Some(vec![diag.clone()]),
        Some(true),
    ))
}

fn replace_real_with_isq_action(
    uri: &str,
    diag: &CodeActionDiagnostic,
    content: &str,
) -> Option<CodeAction> {
    let msg = &diag.message;
    let isq_start = msg.find('`')? + 1;
    let isq_end = msg[isq_start..].find('`')? + isq_start;
    let isq_type = &msg[isq_start..isq_end];

    let line_idx = diag.line_start as usize;
    let line_text = content.lines().nth(line_idx)?;
    let real_patterns = [": Real", ":Real", " Real;", " Real "];
    let mut real_col: Option<u32> = None;
    let real_len = 4u32;
    for pattern in &real_patterns {
        if let Some(pos) = line_text.find(pattern) {
            let real_offset = pattern.find("Real").unwrap();
            real_col = Some((pos + real_offset) as u32);
            break;
        }
    }
    let col = real_col?;

    let edit = TextEdit {
        expected_old_text: None,
        line_start: diag.line_start,
        col_start: col,
        line_end: diag.line_start,
        col_end: col + real_len,
        new_text: isq_type.to_owned(),
    };
    Some(make_action(
        uri,
        format!("Replace Real with {}", isq_type),
        "quickfix",
        vec![edit],
        Some(vec![diag.clone()]),
        Some(true),
    ))
}

// ─── Cursor-position refactorings ─────────────────────────────────────────────

fn cursor_refactorings(
    uri: &str,
    content: &str,
    range_start_line: u32,
    range_start_col: u32,
    _range_end_line: u32,
    _range_end_col: u32,
    tree: &tree_sitter::Tree,
) -> Vec<CodeAction> {
    let mut actions: Vec<CodeAction> = Vec::new();
    let point = tree_sitter::Point {
        row: range_start_line as usize,
        column: range_start_col as usize,
    };
    let Some(node) = tree.root_node().descendant_for_point_range(point, point) else {
        return actions;
    };
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = range_start_line as usize;
    let line = lines.get(line_idx).copied().unwrap_or("");

    if let Some(action) = expand_definition_body(uri, content, &node) {
        actions.push(action);
    }
    actions.extend(keyword_toggle_actions(uri, line, line_idx));
    if let Some(action) = toggle_abstract_action(uri, content, &node) {
        actions.push(action);
    }
    if let Some(action) = add_doc_comment_action(uri, &node, line, line_idx) {
        actions.push(action);
    }
    actions
}

fn expand_definition_body(
    uri: &str,
    content: &str,
    node: &tree_sitter::Node,
) -> Option<CodeAction> {
    let def_node = find_ancestor_definition(node)?;
    let def_text = def_node.utf8_text(content.as_bytes()).ok()?;
    let trimmed = def_text.trim();
    if !trimmed.ends_with(';') {
        return None;
    }
    let def_end = def_node.end_byte();
    let semi_byte = def_end - 1;
    let (semi_line, semi_col) = offset_to_line_col(semi_byte, content);
    let (after_line, after_col) = offset_to_line_col(def_end, content);

    let edit = TextEdit {
        expected_old_text: None,
        line_start: semi_line,
        col_start: semi_col,
        line_end: after_line,
        col_end: after_col,
        new_text: " {\n}\n".to_owned(),
    };
    let kind_name = def_node.kind().replace('_', " ");
    Some(make_action(
        uri,
        format!("Expand {} body", kind_name),
        "refactor.rewrite",
        vec![edit],
        None,
        None,
    ))
}

fn keyword_toggle_actions(uri: &str, line: &str, line_idx: usize) -> Vec<CodeAction> {
    let mut actions: Vec<CodeAction> = Vec::new();
    let trimmed = line.trim();

    let toggles: &[(&str, &str, &str)] = &[
        (":>", "specializes", "specialization"),
        (":>>", "redefines", "redefinition"),
        (":", "defined by", "type annotation"),
        (":>", "subsets", "subsetting"),
    ];

    for &(short, verbose, _desc) in toggles {
        // Verbose → short.
        if trimmed.contains(&format!(" {} ", verbose)) {
            if let Some(col) = line.find(&format!(" {} ", verbose)) {
                let match_start = col + 1;
                let match_end = match_start + verbose.len();
                let edit = TextEdit {
                    expected_old_text: None,
                    line_start: line_idx as u32,
                    col_start: match_start as u32,
                    line_end: line_idx as u32,
                    col_end: match_end as u32,
                    new_text: short.to_owned(),
                };
                actions.push(make_action(
                    uri,
                    format!("Convert '{}' to '{}'", verbose, short),
                    "refactor.rewrite",
                    vec![edit],
                    None,
                    None,
                ));
            }
        }

        // Short → verbose (longest first).
        if short == ":>>" {
            if let Some(col) = find_operator_in_line(line, ":>>") {
                let edit = TextEdit {
                    expected_old_text: None,
                    line_start: line_idx as u32,
                    col_start: col as u32,
                    line_end: line_idx as u32,
                    col_end: (col + 3) as u32,
                    new_text: verbose.to_owned(),
                };
                actions.push(make_action(
                    uri,
                    format!("Convert '{}' to '{}'", short, verbose),
                    "refactor.rewrite",
                    vec![edit],
                    None,
                    None,
                ));
            }
        } else if short == ":>" {
            if let Some(col) = find_operator_in_line(line, ":>") {
                let after = line.as_bytes().get(col + 2);
                if after != Some(&b'>') {
                    let edit = TextEdit {
                        expected_old_text: None,
                        line_start: line_idx as u32,
                        col_start: col as u32,
                        line_end: line_idx as u32,
                        col_end: (col + 2) as u32,
                        new_text: verbose.to_owned(),
                    };
                    actions.push(make_action(
                        uri,
                        format!("Convert '{}' to '{}'", short, verbose),
                        "refactor.rewrite",
                        vec![edit],
                        None,
                        None,
                    ));
                }
            }
        } else if short == ":" {
            if let Some(col) = find_type_colon_in_line(line) {
                let edit = TextEdit {
                    expected_old_text: None,
                    line_start: line_idx as u32,
                    col_start: col as u32,
                    line_end: line_idx as u32,
                    col_end: (col + 1) as u32,
                    new_text: verbose.to_owned(),
                };
                actions.push(make_action(
                    uri,
                    format!("Convert '{}' to '{}'", short, verbose),
                    "refactor.rewrite",
                    vec![edit],
                    None,
                    None,
                ));
            }
        }
    }

    actions
}

fn toggle_abstract_action(
    uri: &str,
    content: &str,
    node: &tree_sitter::Node,
) -> Option<CodeAction> {
    let def_node = find_ancestor_definition(node)?;
    let def_line = def_node.start_position().row;
    let def_line_text = content.lines().nth(def_line)?;
    let trimmed = def_line_text.trim_start();
    if trimmed.starts_with("abstract ") {
        let indent_len = def_line_text.len() - trimmed.len();
        let abstract_start = indent_len;
        let abstract_end = indent_len + "abstract ".len();
        let edit = TextEdit {
            expected_old_text: None,
            line_start: def_line as u32,
            col_start: abstract_start as u32,
            line_end: def_line as u32,
            col_end: abstract_end as u32,
            new_text: String::new(),
        };
        Some(make_action(
            uri,
            "Remove 'abstract' modifier".to_owned(),
            "refactor.rewrite",
            vec![edit],
            None,
            None,
        ))
    } else if is_definition_keyword(trimmed) {
        let indent_len = def_line_text.len() - trimmed.len();
        let edit = TextEdit {
            expected_old_text: None,
            line_start: def_line as u32,
            col_start: indent_len as u32,
            line_end: def_line as u32,
            col_end: indent_len as u32,
            new_text: "abstract ".to_owned(),
        };
        Some(make_action(
            uri,
            "Add 'abstract' modifier".to_owned(),
            "refactor.rewrite",
            vec![edit],
            None,
            None,
        ))
    } else {
        None
    }
}

fn add_doc_comment_action(
    uri: &str,
    node: &tree_sitter::Node,
    line: &str,
    line_idx: usize,
) -> Option<CodeAction> {
    let def_node = find_ancestor_definition(node).or_else(|| find_ancestor_usage(node))?;
    let def_line = def_node.start_position().row;
    if def_line != line_idx {
        return None;
    }
    let indent = get_indent_str(line);
    let comment_text = format!("{}doc /* TODO: Add documentation */\n", indent);
    let edit = TextEdit {
        expected_old_text: None,
        line_start: def_line as u32,
        col_start: 0,
        line_end: def_line as u32,
        col_end: 0,
        new_text: comment_text,
    };
    Some(make_action(
        uri,
        "Add documentation comment".to_owned(),
        "refactor.rewrite",
        vec![edit],
        None,
        None,
    ))
}

// ─── Source actions ───────────────────────────────────────────────────────────

fn source_actions(uri: &str, content: &str, tree: &tree_sitter::Tree) -> Vec<CodeAction> {
    let mut actions: Vec<CodeAction> = Vec::new();
    if let Some(action) = organize_imports_action(uri, content, tree) {
        actions.push(action);
    }
    actions
}

fn organize_imports_action(
    uri: &str,
    content: &str,
    tree: &tree_sitter::Tree,
) -> Option<CodeAction> {
    let mut imports: Vec<ImportInfo> = Vec::new();
    let mut cursor = tree.walk();
    collect_imports(&mut cursor, content, &mut imports);
    if imports.len() < 2 {
        return None;
    }
    let original_texts: Vec<&str> = imports.iter().map(|i| i.text.as_str()).collect();
    let mut sorted_texts: Vec<String> = imports.iter().map(|i| i.text.clone()).collect();
    sorted_texts.sort();
    sorted_texts.dedup();
    let already_organized = sorted_texts.len() == original_texts.len()
        && sorted_texts
            .iter()
            .zip(original_texts.iter())
            .all(|(a, b)| a == *b);
    if already_organized {
        return None;
    }
    let first_import = &imports[0];
    let last_import = &imports[imports.len() - 1];
    let sorted_block = sorted_texts.join("\n") + "\n";
    let edit = TextEdit {
        expected_old_text: None,
        line_start: first_import.line as u32,
        col_start: 0,
        line_end: (last_import.line + 1) as u32,
        col_end: 0,
        new_text: sorted_block,
    };
    let removed = original_texts.len() - sorted_texts.len();
    let title = if removed > 0 {
        format!(
            "Organize imports (sort and remove {} duplicate{})",
            removed,
            if removed == 1 { "" } else { "s" }
        )
    } else {
        "Organize imports (sort alphabetically)".to_owned()
    };
    Some(make_action(
        uri,
        title,
        "source.organizeImports",
        vec![edit],
        None,
        None,
    ))
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

struct ImportInfo {
    line: usize,
    text: String,
}

fn collect_imports(
    cursor: &mut tree_sitter::TreeCursor,
    content: &str,
    imports: &mut Vec<ImportInfo>,
) {
    loop {
        let node = cursor.node();
        let kind = node.kind();
        // Only collect the statement-level `import_decl` node. The grammar's
        // import sub-parts (`import_target`, `import_qualified_name`,
        // `import_single_name`) also contain "import" in their kind name, so a
        // `contains("import")` match double-counts each import and emits bare
        // target names (e.g. `SI`, `ScalarValues`) into the organize-imports
        // edit text.
        if kind == "import_decl" {
            let start_line = node.start_position().row;
            let text = node
                .utf8_text(content.as_bytes())
                .unwrap_or("")
                .trim()
                .to_owned();
            if !text.is_empty() {
                imports.push(ImportInfo {
                    line: start_line,
                    text,
                });
            }
        }
        if cursor.goto_first_child() {
            collect_imports(cursor, content, imports);
            cursor.goto_parent();
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
}

fn make_action(
    uri: &str,
    title: String,
    kind: &str,
    edits: Vec<TextEdit>,
    diagnostics: Option<Vec<CodeActionDiagnostic>>,
    is_preferred: Option<bool>,
) -> CodeAction {
    CodeAction {
        title,
        kind: kind.to_owned(),
        edits: vec![CodeActionFileEdits {
            uri: uri.to_owned(),
            edits,
        }],
        diagnostics: diagnostics.unwrap_or_default(),
        is_preferred,
        command: None,
    }
}

/// Strict-mode quick-fix actions attached to IM012.
///
/// When a single file is opened without folder context, two paths get the
/// user un-stuck:
/// 1. Drop a `sysml.toml` next to the file so the next open promotes the
///    project to `DiscoveredViaManifest`.
/// 2. Open the parent folder in the editor so it becomes Discovered.
///
/// Both actions are command-only (no text edits) — the actual handlers
/// live in the LSP shell's `command_dispatch.rs` (`sysml.init` and
/// `sysml.workspace.open_folder`).
fn strict_mode_actions(uri: &str, diag: &CodeActionDiagnostic) -> Vec<CodeAction> {
    let path = match uri.strip_prefix("file://") {
        Some(p) => std::path::PathBuf::from(p),
        None => return Vec::new(),
    };
    let Some(parent) = path.parent() else {
        return Vec::new();
    };
    let parent_str = parent.to_string_lossy().into_owned();

    let mut out = Vec::with_capacity(2);
    out.push(make_command_action(
        "Create sysml.toml here (enables cross-file imports)".to_string(),
        "quickfix",
        vec![diag.clone()],
        CodeActionCommand {
            title: "Create sysml.toml".to_string(),
            command: "sysml.init".to_string(),
            arguments: vec![serde_json::Value::String(parent_str.clone())],
        },
    ));
    out.push(make_command_action(
        "Open the parent folder in your editor".to_string(),
        "quickfix",
        vec![diag.clone()],
        CodeActionCommand {
            title: "Open folder".to_string(),
            command: "sysml.workspace.open_folder".to_string(),
            arguments: vec![serde_json::Value::String(parent_str)],
        },
    ));
    out
}

/// Build a command-only code action (no text edits). Used for actions
/// that must be handled by a server-side command, e.g. "Create sysml.toml"
/// or "Open this folder in your editor".
fn make_command_action(
    title: String,
    kind: &str,
    diagnostics: Vec<CodeActionDiagnostic>,
    command: CodeActionCommand,
) -> CodeAction {
    CodeAction {
        title,
        kind: kind.to_owned(),
        edits: Vec::new(),
        diagnostics,
        is_preferred: None,
        command: Some(command),
    }
}

fn make_import_action(
    uri: &str,
    diag: &CodeActionDiagnostic,
    name: &str,
    qualified_name: &str,
    content: &str,
) -> CodeAction {
    let (line, col) = find_import_insertion_point(content);
    let import_text = format!("import {};\n", qualified_name);
    let edit = TextEdit {
        expected_old_text: None,
        line_start: line,
        col_start: col,
        line_end: line,
        col_end: col,
        new_text: import_text,
    };
    make_action(
        uri,
        format!("Import {} (from {})", name, qualified_name),
        "quickfix",
        vec![edit],
        Some(vec![diag.clone()]),
        None,
    )
}

fn find_import_insertion_point(content: &str) -> (u32, u32) {
    for (line_idx, line) in content.lines().enumerate() {
        let trimmed = line.trim();
        if (trimmed.starts_with("package") || trimmed.starts_with("namespace"))
            && trimmed.contains('{')
        {
            return ((line_idx + 1) as u32, 0);
        }
    }
    (0, 0)
}

fn find_definition_insertion_point(content: &str, diag_line: u32) -> (u32, u32) {
    let lines: Vec<&str> = content.lines().collect();
    for i in (diag_line as usize + 1)..lines.len() {
        let trimmed = lines[i].trim();
        if trimmed == "}" || trimmed.starts_with('}') {
            return (i as u32, 0);
        }
    }
    (lines.len() as u32, 0)
}

pub fn extract_quoted_name(message: &str) -> Option<String> {
    let start = message.find('\'')?;
    let rest = &message[start + 1..];
    let end = rest.find('\'')?;
    Some(rest[..end].to_string())
}

fn looks_like_unresolved_reference(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    (lower.contains("no definition '") && lower.contains("found in scope"))
        || lower.contains("unresolved reference")
}

/// Enumerate qualified names that an unresolved bare-name reference could
/// resolve to in the workspace-merged graph (Phase 4 of
///
/// Returns one qname per **definition** in the workspace whose simple name
/// matches `name` AND whose decl-site span isn't in `cursor_uri` (we're
/// looking for cross-file imports; in-file names don't need importing).
/// Library elements are excluded — the library is its own action path.
///
/// Replaces the parse-graph walk in [`WorkspaceSnapshot::from_snapshot`] for the
/// auto-import code path. Uses the salsa-cached
/// [`workspace_name_index`](sysml_ide_db::workspace_name_index) + the
/// elaborated workspace graph, both of which the rest of the IDE features
/// already trust for cross-file lookups.
fn collect_workspace_import_qnames(
    workspace_merged_graph: &ModelGraph,
    workspace_name_index: &HashMap<String, Vec<ElementId>>,
    name: &str,
    cursor_uri: &str,
) -> Vec<String> {
    let mut qnames: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let cursor_path = cursor_uri.strip_prefix("file://").unwrap_or(cursor_uri);

    let Some(ids) = workspace_name_index.get(name) else {
        return qnames;
    };
    for id in ids {
        let Some(element) = workspace_merged_graph.get_element(id) else {
            continue;
        };
        // Only emit imports for definitions — usages/relationships are never
        // valid import targets.
        if !element.kind.is_definition() {
            continue;
        }
        // Skip elements whose decl-site is the cursor's file — in-file refs
        // don't need an import. URI normalization is handled with the same
        // `strip_prefix("file://")` shape as index_file_graph elsewhere.
        let in_cursor_file = element.spans.first().is_some_and(|span| {
            let span_path = span.file.strip_prefix("file://").unwrap_or(&span.file);
            span_path == cursor_path
        });
        if in_cursor_file {
            continue;
        }
        // Prefer the element's pre-built qname; fall back to walking the
        // graph's qname builder.
        let qname = element.qname.as_ref().map(|q| q.to_string()).or_else(|| {
            workspace_merged_graph
                .build_qualified_name(&element.id)
                .map(|q| q.to_string())
        });
        if let Some(qname_str) = qname {
            if seen.insert(qname_str.clone()) {
                qnames.push(qname_str);
            }
        }
    }
    qnames
}

fn unresolved_name_for_diagnostic(diag: &CodeActionDiagnostic, content: &str) -> Option<String> {
    let from_message = extract_quoted_name(&diag.message)?;
    if from_message.chars().count() > 1 {
        return Some(from_message);
    }
    identifier_around_range(
        content,
        diag.line_start,
        diag.col_start,
        diag.line_end,
        diag.col_end,
    )
    .or(Some(from_message))
}

fn identifier_around_range(
    content: &str,
    line_start: u32,
    col_start: u32,
    line_end: u32,
    col_end: u32,
) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let start_raw = position_to_offset(line_start, col_start, content);
    let end_raw = position_to_offset(line_end, col_end, content);
    let mut start = clamp_to_char_boundary(content, start_raw.min(content.len()));
    let mut end = clamp_to_char_boundary(content, end_raw.min(content.len()));
    if end < start {
        std::mem::swap(&mut start, &mut end);
    }
    while start > 0 {
        let mut iter = content[..start].char_indices();
        let Some((prev_start, ch)) = iter.next_back() else {
            break;
        };
        if is_identifier_char(ch) {
            start = prev_start;
        } else {
            break;
        }
    }
    while end < content.len() {
        let mut iter = content[end..].chars();
        let Some(ch) = iter.next() else {
            break;
        };
        if is_identifier_char(ch) {
            end += ch.len_utf8();
        } else {
            break;
        }
    }
    if start >= end {
        return None;
    }
    let candidate = &content[start..end];
    if candidate.chars().all(is_identifier_char) {
        Some(candidate.to_owned())
    } else {
        None
    }
}

fn clamp_to_char_boundary(source: &str, offset: usize) -> usize {
    let mut clamped = offset.min(source.len());
    while clamped > 0 && !source.is_char_boundary(clamped) {
        clamped -= 1;
    }
    clamped
}

fn is_identifier_char(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == '_'
}

fn suggest_library_import(name: &str) -> Option<&'static str> {
    const LIBRARY_IMPORTS: &[(&str, &str)] = &[
        ("Real", "ScalarValues::Real"),
        ("Integer", "ScalarValues::Integer"),
        ("String", "ScalarValues::String"),
        ("Boolean", "ScalarValues::Boolean"),
        ("Natural", "ScalarValues::Natural"),
        ("Positive", "ScalarValues::Positive"),
        ("Complex", "ScalarValues::Complex"),
        ("Number", "ScalarValues::Number"),
        ("Anything", "Base::Anything"),
        ("DataValue", "Base::DataValue"),
    ];
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some((_, qname)) = LIBRARY_IMPORTS
        .iter()
        .find(|(type_name, _)| type_name.eq_ignore_ascii_case(trimmed))
    {
        return Some(*qname);
    }
    let normalized = trimmed.to_ascii_lowercase();
    let max_distance = if normalized.len() <= 4 { 1 } else { 2 };
    let mut best: Option<(usize, &'static str)> = None;
    for (type_name, qname) in LIBRARY_IMPORTS {
        let candidate = type_name.to_ascii_lowercase();
        let distance = levenshtein_distance(&normalized, &candidate);
        if distance > max_distance {
            continue;
        }
        if let Some((best_distance, _)) = best {
            if distance >= best_distance {
                continue;
            }
        }
        best = Some((distance, *qname));
    }
    best.map(|(_, qname)| qname)
}

fn levenshtein_distance(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr = vec![0usize; b_chars.len() + 1];
    for (i, a_char) in a.chars().enumerate() {
        curr[0] = i + 1;
        for (j, b_char) in b_chars.iter().enumerate() {
            let replace_cost = usize::from(a_char != *b_char);
            let insert_cost = curr[j] + 1;
            let delete_cost = prev[j + 1] + 1;
            let replace_total = prev[j] + replace_cost;
            curr[j + 1] = insert_cost.min(delete_cost).min(replace_total);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[b_chars.len()]
}

fn guess_definition_keyword(message: &str) -> &'static str {
    let msg_lower = message.to_lowercase();
    if msg_lower.contains("attribute") {
        "attribute"
    } else if msg_lower.contains("action") {
        "action"
    } else if msg_lower.contains("state") {
        "state"
    } else if msg_lower.contains("requirement") {
        "requirement"
    } else if msg_lower.contains("constraint") {
        "constraint"
    } else {
        "part"
    }
}

fn get_indent_str(line: &str) -> &str {
    let non_ws = line.len() - line.trim_start().len();
    &line[..non_ws]
}

fn get_indent_at_line(content: &str, line: u32) -> String {
    content
        .lines()
        .nth(line as usize)
        .map(|l| get_indent_str(l).to_owned())
        .unwrap_or_default()
}

fn find_ancestor_definition<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut current = *node;
    loop {
        if is_definition_node(current.kind()) {
            return Some(current);
        }
        current = current.parent()?;
    }
}

fn find_ancestor_usage<'a>(node: &'a tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
    let mut current = *node;
    loop {
        if is_usage_node(current.kind()) {
            return Some(current);
        }
        current = current.parent()?;
    }
}

fn is_definition_node(kind: &str) -> bool {
    kind.ends_with("_definition") || kind == "package_declaration"
}

fn is_usage_node(kind: &str) -> bool {
    kind.ends_with("_usage")
}

fn is_definition_keyword(trimmed: &str) -> bool {
    const DEF_KEYWORDS: &[&str] = &[
        "part def",
        "attribute def",
        "item def",
        "action def",
        "state def",
        "requirement def",
        "constraint def",
        "port def",
        "connection def",
        "interface def",
        "allocation def",
        "enum def",
        "occurrence def",
        "case def",
        "analysis case def",
        "verification case def",
        "use case def",
        "view def",
        "viewpoint def",
        "rendering def",
        "concern def",
        "calculation def",
        "metadata def",
        "package",
    ];
    DEF_KEYWORDS.iter().any(|kw| trimmed.starts_with(kw))
}

fn find_operator_in_line(line: &str, op: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let op_bytes = op.as_bytes();
    let mut in_string = false;
    for i in 0..bytes.len() {
        if bytes[i] == b'\'' || bytes[i] == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if i + op_bytes.len() <= bytes.len() && &bytes[i..i + op_bytes.len()] == op_bytes {
            return Some(i);
        }
    }
    None
}

fn find_type_colon_in_line(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    let mut in_string = false;
    for i in 0..bytes.len() {
        if bytes[i] == b'\'' || bytes[i] == b'"' {
            in_string = !in_string;
            continue;
        }
        if in_string {
            continue;
        }
        if bytes[i] == b':' {
            let next = bytes.get(i + 1);
            if next == Some(&b'>') || next == Some(&b':') {
                continue;
            }
            if i > 0 && bytes[i - 1] == b':' {
                continue;
            }
            return Some(i);
        }
    }
    None
}

// ─── WorkspaceSnapshot (service-side; mirrors the LSP version) ────────────────

#[derive(Debug, Clone)]
struct CrossFileEntry {
    uri: String,
    element_id: ElementId,
}

struct WorkspaceSnapshot {
    by_name: HashMap<String, Vec<CrossFileEntry>>,
    by_qname: BTreeMap<String, CrossFileEntry>,
    file_qnames: HashMap<String, Vec<(String, CrossFileEntry)>>,
}

impl WorkspaceSnapshot {
    /// Build from a snapshot + a pre-resolved `(uri, SourceFile)` list so
    /// the per-file parse walk runs with NO host guard held — the caller
    /// enumerates the file list under its own small guard and drops it
    /// first (precedent: `compute_full_diagnostics`).
    fn from_snapshot(
        analysis: &sysml_ide_db::Analysis,
        files: &[(String, sysml_ide_db::SourceFile)],
    ) -> Self {
        let mut by_name: HashMap<String, Vec<CrossFileEntry>> = HashMap::new();
        let mut by_qname: BTreeMap<String, CrossFileEntry> = BTreeMap::new();
        let mut file_qnames: HashMap<String, Vec<(String, CrossFileEntry)>> = HashMap::new();

        for (uri, sf) in files {
            let graph = match Cancelled::catch(std::panic::AssertUnwindSafe(|| {
                analysis.parse_file(*sf).graph().clone()
            })) {
                Ok(g) => g,
                Err(_) => continue,
            };
            index_file_graph(&mut by_name, &mut by_qname, &mut file_qnames, uri, &graph);
        }
        WorkspaceSnapshot {
            by_name,
            by_qname,
            file_qnames,
        }
    }

    fn find_by_name(&self, name: &str) -> &[CrossFileEntry] {
        self.by_name.get(name).map(|v| v.as_slice()).unwrap_or(&[])
    }

    fn for_each_qname_in_file(&self, uri: &str, mut f: impl FnMut(&str, &CrossFileEntry)) {
        if let Some(entries) = self.file_qnames.get(uri) {
            for (qname, entry) in entries {
                f(qname, entry);
            }
        }
    }
}

fn index_file_graph(
    by_name: &mut HashMap<String, Vec<CrossFileEntry>>,
    by_qname: &mut BTreeMap<String, CrossFileEntry>,
    file_qnames: &mut HashMap<String, Vec<(String, CrossFileEntry)>>,
    uri: &str,
    graph: &ModelGraph,
) {
    for element in graph.elements.values() {
        if let Some(span) = element.spans.first() {
            if span.file != uri && !span.file.is_empty() && {
                let span_path = span.file.strip_prefix("file://").unwrap_or(&span.file);
                let uri_path = uri.strip_prefix("file://").unwrap_or(uri);
                span_path != uri_path
            } {
                continue;
            }
        }
        if let Some(name) = &element.name {
            let entry = CrossFileEntry {
                uri: uri.to_owned(),
                element_id: element.id.clone(),
            };
            by_name.entry(name.clone()).or_default().push(entry.clone());
            let qname = element.qname.as_ref().map(|q| q.to_string()).or_else(|| {
                graph
                    .build_qualified_name(&element.id)
                    .map(|q| q.to_string())
            });
            if let Some(qname_str) = qname {
                let qentry = entry.clone();
                file_qnames
                    .entry(uri.to_owned())
                    .or_default()
                    .push((qname_str.clone(), qentry.clone()));
                by_qname.insert(qname_str, qentry);
            }
        }
    }
}
