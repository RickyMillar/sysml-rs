//! Completion candidate computation — namespace/type/feature-chain/general
//! routes plus resolve-time enrichment.
//!
//! Replaces the bulk of LSP `completion.rs`. The LSP shell keeps:
//!   - `pending_requests` stale-result handling (transient shell concern),
//!   - `telemetry_events::completion_*`,
//!   - `manifest_language_features::completion_for_manifest`
//!     (sysml.toml is a different file type — early-return at handler entry),
//!   - tree-sitter `CursorSyntaxContext::from_tree` and the CST →
//!     `SyntaxCtxSummary` translation (service stays free of tree-sitter).
//!
//! Position columns follow the LSP convention: UTF-16 code units, 0-indexed
//! line + character. Service has no `tower-lsp` dependency; numeric kinds
//! are encoded as `i32` matching `tower_lsp::lsp_types::CompletionItemKind`
//! variants 1-25, and the LSP shell decodes them at the boundary.

use std::collections::HashMap;
use std::sync::Mutex;

use sysml_core::resolution::{FxHashSet, ResolutionContext};
use sysml_core::{Element, ElementKind, MembershipView, ModelGraph, RelationshipKind};
use sysml_id::ElementId;
use sysml_ide_db::{AnalysisHost, Cancelled, PositionMap};

use crate::goto_definition::find_element_type;
use crate::hover::{element_kind_to_hover_label, extract_doc_comment};
use crate::position::{offset_to_line_col, position_to_offset};

/// Spans for elements that have no source file (e.g. synthetic library defs).
const SYNTHETIC_FILE: &str = "<synthetic>";

const MAX_COMPLETION_ITEMS: usize = 200;
const MIN_NONEMPTY_MATCH_SCORE: u32 = 40;
const MIN_TYPO_QUERY_LEN: usize = 4;

const SOURCE_KEYWORD: u32 = 5;
const SOURCE_SNIPPET: u32 = 6;
const SOURCE_LIBRARY: u32 = 7;
const SOURCE_WORKSPACE: u32 = 8;
const SOURCE_LOCAL: u32 = 9;
const SOURCE_IMPORT_LOCAL: u32 = 10;
const SOURCE_IMPORT_LIBRARY: u32 = 11;
const SOURCE_IMPORT_WORKSPACE: u32 = 13;
const SOURCE_FEATURE_INHERITED: u32 = 7;
const SOURCE_FEATURE_TYPE: u32 = 8;
const SOURCE_FEATURE_OWNED: u32 = 9;

// CompletionItemKind enum values from LSP types (1..=25). Mirrors
// `tower_lsp::lsp_types::CompletionItemKind` so the LSP shell can decode
// directly without a translation table.
const KIND_FUNCTION: i32 = 3;
const KIND_FIELD: i32 = 5;
const KIND_VARIABLE: i32 = 6;
const KIND_CLASS: i32 = 7;
const KIND_INTERFACE: i32 = 8;
const KIND_MODULE: i32 = 9;
const KIND_PROPERTY: i32 = 10;
const KIND_VALUE: i32 = 12;
const KIND_ENUM: i32 = 13;
const KIND_KEYWORD: i32 = 14;
const KIND_SNIPPET: i32 = 15;
const KIND_REFERENCE: i32 = 18;
const KIND_STRUCT: i32 = 22;

/// Lightweight summary of cursor-syntax context used to pick a completion
/// route. The LSP shell builds this from a tree-sitter `Tree` (via
/// `CursorSyntaxContext::from_tree`) and passes it across the wire.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct SyntaxCtxSummary {
    pub in_import: bool,
    pub in_comment_or_string: bool,
    pub in_feature_chain: bool,
    pub in_type_ref: bool,
}

/// A text-edit insertion in line/col coordinates (UTF-16 code units).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct TextEditPos {
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
    pub new_text: String,
}

/// Optional secondary label fields. Mirrors `lsp_types::CompletionItemLabelDetails`.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CompletionLabelDetails {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl CompletionLabelDetails {
    fn is_empty(&self) -> bool {
        self.detail.is_none() && self.description.is_none()
    }
}

/// One completion candidate. Maps 1:1 to `lsp_types::CompletionItem`; the
/// LSP shell converts at the boundary.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletionCandidate {
    pub label: String,
    /// CompletionItemKind enum value (1-25); 0 means unspecified.
    pub kind: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sort_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub insert_text: Option<String>,
    /// `true` → InsertTextFormat::SNIPPET, `false` → PLAIN_TEXT.
    #[serde(default)]
    pub insert_text_is_snippet: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label_details: Option<CompletionLabelDetails>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub additional_text_edits: Vec<TextEditPos>,
    #[serde(default)]
    pub preselect: bool,
    /// ElementId carried for resolve enrichment.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_element_id: Option<String>,
    /// Preferred document URI for resolve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data_document_uri: Option<String>,
}

/// Resolve-time enrichment for a completion item.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletionDetails {
    /// Markdown documentation pulled from a leading doc-comment or block.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub documentation_markdown: Option<String>,
    /// Type-of detail string (e.g. ": Integer").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompletionRoute {
    NamespaceMembers,
    TypeReferences,
    FeatureChain,
    General,
}

#[derive(Debug, Clone, Copy)]
struct NamespaceCtx<'a> {
    namespace_path: &'a str,
    member_query: &'a str,
}

#[derive(Debug, Clone, Copy)]
struct FeatureCtx<'a> {
    base_name: &'a str,
    member_query: &'a str,
}

#[derive(Debug, Clone)]
struct CompletionCtx<'a> {
    route: CompletionRoute,
    query: &'a str,
    namespace: Option<NamespaceCtx<'a>>,
    type_query: Option<&'a str>,
    feature: Option<FeatureCtx<'a>>,
    cursor_offset: usize,
    in_import: bool,
    anchor_element: Option<ElementId>,
}

// ---- pure text helpers -----------------------------------------------------

fn completion_token(text_before: &str) -> &str {
    text_before
        .rsplit(|c: char| {
            c.is_whitespace()
                || c == '{'
                || c == '}'
                || c == ';'
                || c == '('
                || c == ')'
                || c == ','
                || c == '['
                || c == ']'
        })
        .next()
        .unwrap_or("")
}

fn split_namespace_context(token: &str) -> Option<(&str, &str)> {
    token.rsplit_once("::")
}

fn split_import_namespace_context(token: &str) -> Option<(&str, &str)> {
    let path = token.strip_suffix(':')?;
    if path.is_empty() {
        return None;
    }
    Some((path, ""))
}

fn split_feature_chain_context(token: &str) -> Option<(&str, &str)> {
    let (base, query) = token.rsplit_once('.')?;
    if base.is_empty() {
        return None;
    }
    if !base.chars().any(|c| c.is_ascii_alphabetic() || c == '_') {
        return None;
    }
    if !query.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((base, query))
}

fn is_id_like(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !first.is_ascii_alphabetic() && first != '_' {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn quote_name(name: &str) -> String {
    let mut quoted = String::with_capacity(name.len() + 2);
    quoted.push('\'');
    for ch in name.chars() {
        match ch {
            '\\' => {
                quoted.push('\\');
                quoted.push('\\');
            }
            '\'' => {
                quoted.push('\\');
                quoted.push('\'');
            }
            _ => quoted.push(ch),
        }
    }
    quoted.push('\'');
    quoted
}

fn normalize_completion_query(query: &str) -> String {
    let trimmed = query.trim();
    let trimmed = trimmed.trim_start_matches('\'');
    let trimmed = trimmed.trim_end_matches('\'');
    trimmed.to_owned()
}

fn statement_context(text_before: &str) -> &str {
    text_before
        .rsplit(['\n', ';', '{', '}'])
        .next()
        .unwrap_or("")
}

fn type_context_window(text_before: &str) -> &str {
    text_before.rsplit([';', '{', '}']).next().unwrap_or("")
}

fn starts_with_ignore_ascii_case(name: &str, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    name.len() >= query.len()
        && name
            .chars()
            .zip(query.chars())
            .all(|(n, q)| n.eq_ignore_ascii_case(&q))
}

fn lsp_starts_with_ignore_ascii_case(s: &str, prefix: &str) -> bool {
    s.len() >= prefix.len()
        && s.chars()
            .zip(prefix.chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b))
}

fn in_import_context(text_before: &str) -> bool {
    let statement = statement_context(text_before).trim_start();
    lsp_starts_with_ignore_ascii_case(statement, "import ")
        || lsp_starts_with_ignore_ascii_case(statement, "private import ")
        || lsp_starts_with_ignore_ascii_case(statement, "public import ")
        || lsp_starts_with_ignore_ascii_case(statement, "protected import ")
}

fn type_query_from_context(text_before: &str) -> Option<&str> {
    let statement = type_context_window(text_before);
    let bytes = statement.as_bytes();
    let mut colon_idx = None;
    for i in (0..bytes.len()).rev() {
        if bytes[i] != b':' {
            continue;
        }
        let prev_is_colon = i > 0 && bytes[i - 1] == b':';
        let next_is_colon = i + 1 < bytes.len() && bytes[i + 1] == b':';
        if prev_is_colon || next_is_colon {
            continue;
        }
        colon_idx = Some(i);
        break;
    }
    let idx = colon_idx?;
    Some(statement[idx + 1..].trim_start())
}

fn completion_anchor(position_map: &PositionMap, offset: usize) -> Option<ElementId> {
    position_map
        .element_id_at(offset)
        .or_else(|| position_map.nearest_element(offset).map(|(id, _)| id))
}

fn clamp_to_char_boundary(source: &str, offset: usize) -> usize {
    let mut clamped = offset.min(source.len());
    while clamped > 0 && !source.is_char_boundary(clamped) {
        clamped -= 1;
    }
    clamped
}

// ---- scoring ---------------------------------------------------------------

fn fuzzy_match(query: &str, target: &str) -> bool {
    if target.contains(query) {
        return true;
    }
    let mut target_chars = target.chars();
    for qc in query.chars() {
        loop {
            match target_chars.next() {
                Some(tc) if tc == qc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

fn score_completion(query: &str, label: &str) -> u32 {
    if query.is_empty() {
        return 50;
    }
    if label == query {
        return 100;
    }
    let query_lower = query.to_lowercase();
    let label_lower = label.to_lowercase();
    if label_lower == query_lower {
        return 80;
    }
    if label.starts_with(query) {
        return 60;
    }
    if label_lower.starts_with(&query_lower) {
        return 40;
    }
    if fuzzy_match(&query_lower, &label_lower) {
        return 20;
    }
    0
}

fn rank_completion(query: &str, label: &str, source_bucket: u32) -> Option<u32> {
    let mut match_score = score_completion(query, label);
    if match_score == 0 {
        match_score = typo_completion_score(query, label);
    }
    if match_score == 0 {
        return None;
    }
    if !query.is_empty() && match_score < MIN_NONEMPTY_MATCH_SCORE {
        return None;
    }
    Some(source_bucket * 1000 + match_score)
}

fn typo_completion_score(query: &str, label: &str) -> u32 {
    let query = normalize_completion_query(query);
    if query.is_empty() || query.chars().count() < MIN_TYPO_QUERY_LEN {
        return 0;
    }
    let query_lower = query.to_lowercase();
    let label_lower = label.to_lowercase();
    if is_edit_distance_leq_one(&query_lower, &label_lower) {
        MIN_NONEMPTY_MATCH_SCORE
    } else {
        0
    }
}

fn is_edit_distance_leq_one(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let a_len = a_chars.len();
    let b_len = b_chars.len();
    let len_diff = a_len.abs_diff(b_len);
    if len_diff > 1 {
        return false;
    }
    let mut i = 0usize;
    let mut j = 0usize;
    let mut edits = 0usize;
    while i < a_len && j < b_len {
        if a_chars[i] == b_chars[j] {
            i += 1;
            j += 1;
            continue;
        }
        edits += 1;
        if edits > 1 {
            return false;
        }
        if a_len > b_len {
            i += 1;
        } else if b_len > a_len {
            j += 1;
        } else {
            i += 1;
            j += 1;
        }
    }
    if i < a_len || j < b_len {
        edits += 1;
    }
    edits <= 1
}

// ---- candidate post-processing --------------------------------------------

fn sort_scored_items(mut scored: Vec<(u32, CompletionCandidate)>) -> Vec<CompletionCandidate> {
    scored.sort_by(|(score_a, item_a), (score_b, item_b)| {
        score_b
            .cmp(score_a)
            .then_with(|| item_a.label.cmp(&item_b.label))
    });
    scored.into_iter().map(|(_, item)| item).collect()
}

fn dedup_by_label(items: Vec<CompletionCandidate>) -> Vec<CompletionCandidate> {
    let mut seen = FxHashSet::default();
    let mut deduped = Vec::with_capacity(items.len());
    for item in items {
        if seen.insert(item.label.clone()) {
            deduped.push(item);
        }
    }
    deduped
}

fn limit(mut items: Vec<CompletionCandidate>) -> Vec<CompletionCandidate> {
    if items.len() > MAX_COMPLETION_ITEMS {
        items.truncate(MAX_COMPLETION_ITEMS);
    }
    items
}

// ---- ElementKind → CompletionItemKind ------------------------------------

fn element_kind_to_completion_kind(kind: &ElementKind) -> i32 {
    match kind {
        ElementKind::Package | ElementKind::LibraryPackage => KIND_MODULE,
        ElementKind::ActionDefinition | ElementKind::ActionUsage => KIND_FUNCTION,
        ElementKind::StateDefinition => KIND_CLASS,
        ElementKind::PortDefinition | ElementKind::InterfaceDefinition => KIND_INTERFACE,
        ElementKind::EnumerationDefinition | ElementKind::EnumerationUsage => KIND_ENUM,
        ElementKind::RequirementDefinition
        | ElementKind::ConstraintDefinition
        | ElementKind::ConcernDefinition => KIND_STRUCT,
        ElementKind::AttributeUsage => KIND_PROPERTY,
        ElementKind::CalculationDefinition | ElementKind::CalculationUsage => KIND_FUNCTION,
        ElementKind::OwningMembership | ElementKind::Membership => KIND_REFERENCE,
        _ if kind.is_definition() => KIND_CLASS,
        _ if kind.is_usage() => KIND_FIELD,
        _ => KIND_VALUE,
    }
}

// ---- import auto-edit ------------------------------------------------------

fn span_matches_document(span: &sysml_core::Span, doc_uri: &str) -> bool {
    span.file.is_empty() || span.file == doc_uri
}

fn package_span_for_cursor(
    graph: &ModelGraph,
    doc_uri: &str,
    cursor_offset: usize,
    content_len: usize,
) -> (usize, usize) {
    let mut best: Option<(usize, usize)> = None;
    for element in graph.elements.values() {
        if element.kind != ElementKind::Package && element.kind != ElementKind::LibraryPackage {
            continue;
        }
        for span in &element.spans {
            if !span_matches_document(span, doc_uri) {
                continue;
            }
            let start = span.start.min(content_len);
            let end = span.end.min(content_len);
            if start > cursor_offset || cursor_offset > end {
                continue;
            }
            let span_len = end.saturating_sub(start);
            let replace = best
                .as_ref()
                .map(|(best_start, best_end)| span_len < best_end.saturating_sub(*best_start))
                .unwrap_or(true);
            if replace {
                best = Some((start, end));
            }
        }
    }
    best.unwrap_or((0, content_len))
}

fn import_path_from_line(line: &str) -> Option<&str> {
    let mut trimmed = line.trim_start();
    for vis in ["private ", "public ", "protected "] {
        if let Some(rest) = trimmed.strip_prefix(vis) {
            trimmed = rest;
            break;
        }
    }
    let rest = trimmed.strip_prefix("import ")?;
    let semicolon = rest.find(';')?;
    let path = rest[..semicolon].trim();
    if path.is_empty() {
        None
    } else {
        Some(path)
    }
}

fn leading_whitespace(line: &str) -> &str {
    let ws_len = line
        .char_indices()
        .find_map(|(idx, ch)| (!ch.is_whitespace()).then_some(idx))
        .unwrap_or(line.len());
    &line[..ws_len]
}

fn infer_import_indent(package_text: &str) -> String {
    if let Some(open_brace_idx) = package_text.find('{') {
        let after_brace = &package_text[open_brace_idx + 1..];
        for line in after_brace.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let indent = leading_whitespace(line);
            if !indent.is_empty() {
                return indent.to_owned();
            }
        }
    }
    "    ".to_owned()
}

fn build_policy_a_import_edit(
    graph: &ModelGraph,
    uri: &str,
    content: &str,
    cursor_offset: usize,
    root: &str,
) -> Option<TextEditPos> {
    if root.is_empty() {
        return None;
    }
    let (pkg_start_raw, pkg_end_raw) =
        package_span_for_cursor(graph, uri, cursor_offset, content.len());
    let pkg_start = clamp_to_char_boundary(content, pkg_start_raw);
    let pkg_end = clamp_to_char_boundary(content, pkg_end_raw.max(pkg_start));
    let package_text = content.get(pkg_start..pkg_end)?;

    let mut rel_cursor = 0usize;
    let mut last_import_end: Option<usize> = None;
    let mut import_indent: Option<String> = None;

    for line in package_text.split_inclusive('\n') {
        if let Some(path) = import_path_from_line(line) {
            if path == root || path.starts_with(&format!("{root}::")) {
                return None;
            }
            last_import_end = Some(rel_cursor + line.len());
            import_indent = Some(leading_whitespace(line).to_owned());
        }
        rel_cursor += line.len();
    }

    let insert_at = if let Some(rel_end) = last_import_end {
        pkg_start + rel_end
    } else if let Some(open_brace_idx) = package_text.find('{') {
        pkg_start + open_brace_idx + 1
    } else {
        pkg_start
    };

    let indent = import_indent.unwrap_or_else(|| infer_import_indent(package_text));

    let mut new_text = String::new();
    let needs_leading_newline = insert_at > 0
        && content
            .get(..insert_at)
            .map(|s| !s.ends_with('\n'))
            .unwrap_or(false);
    let needs_trailing_newline = content
        .get(insert_at..)
        .map(|s| !s.starts_with('\n'))
        .unwrap_or(true);
    if needs_leading_newline {
        new_text.push('\n');
    }
    new_text.push_str(&format!("{indent}private import {root}::*;"));
    if needs_trailing_newline {
        new_text.push('\n');
    }

    let (line_start, col_start) = offset_to_line_col(insert_at, content);
    Some(TextEditPos {
        line_start,
        col_start,
        line_end: line_start,
        col_end: col_start,
        new_text,
    })
}

// ---- scope resolution helpers (current file) ------------------------------

fn scope_chain_for_completion(
    graph: &ModelGraph,
    doc_uri: &str,
    completion_ctx: &CompletionCtx<'_>,
) -> Vec<ElementId> {
    let mut seed_ids: Vec<ElementId> = Vec::new();

    if let Some(anchor) = completion_ctx.anchor_element.as_ref() {
        seed_ids.push(anchor.clone());
    }

    let mut span_seeds: Vec<(usize, ElementId)> = graph
        .elements
        .values()
        .filter_map(|element| {
            let span = element.spans.first()?;
            if !span_matches_document(span, doc_uri) {
                return None;
            }
            if completion_ctx.cursor_offset < span.start || completion_ctx.cursor_offset > span.end
            {
                return None;
            }
            Some((span.end.saturating_sub(span.start), element.id.clone()))
        })
        .collect();
    span_seeds.sort_by_key(|(len, _)| *len);
    for (_, id) in span_seeds {
        if !seed_ids.contains(&id) {
            seed_ids.push(id);
        }
    }

    let mut scope_ids = Vec::new();
    for seed in seed_ids {
        let mut current = Some(seed);
        while let Some(id) = current {
            if !scope_ids.contains(&id) {
                scope_ids.push(id.clone());
            }
            current = graph.owner_of(&id).map(|owner| owner.id.clone());
        }
    }

    scope_ids
}

fn resolve_name_in_namespace_for_completion(
    graph: &ModelGraph,
    namespace_id: &ElementId,
    name: &str,
    doc_uri: &str,
    cursor_offset: usize,
) -> Option<ElementId> {
    let mut best_before: Option<(usize, ElementId)> = None;
    let mut best_after: Option<(usize, ElementId)> = None;
    let mut best_no_span: Option<ElementId> = None;

    for member in graph.members(namespace_id) {
        if member.name.as_deref() != Some(name) {
            continue;
        }
        match member.spans.first() {
            Some(span) if span_matches_document(span, doc_uri) => {
                if span.start <= cursor_offset {
                    let replace = best_before
                        .as_ref()
                        .map(|(b, _)| span.start >= *b)
                        .unwrap_or(true);
                    if replace {
                        best_before = Some((span.start, member.id.clone()));
                    }
                } else {
                    let replace = best_after
                        .as_ref()
                        .map(|(b, _)| span.start < *b)
                        .unwrap_or(true);
                    if replace {
                        best_after = Some((span.start, member.id.clone()));
                    }
                }
            }
            _ => {
                if best_no_span.is_none() {
                    best_no_span = Some(member.id.clone());
                }
            }
        }
    }

    best_before
        .map(|(_, id)| id)
        .or_else(|| best_after.map(|(_, id)| id))
        .or(best_no_span)
}

fn resolve_simple_name_lexically(
    graph: &ModelGraph,
    scope_id: &ElementId,
    name: &str,
    doc_uri: &str,
    cursor_offset: usize,
) -> Option<ElementId> {
    let mut current = Some(scope_id.clone());
    while let Some(ns_id) = current {
        if let Some(found) =
            resolve_name_in_namespace_for_completion(graph, &ns_id, name, doc_uri, cursor_offset)
        {
            return Some(found);
        }
        current = graph.owner_of(&ns_id).map(|owner| owner.id.clone());
    }
    None
}

fn resolve_import_target_namespace_for_completion(
    resolver: &mut ResolutionContext<'_>,
    namespace_id: &ElementId,
    import_elem: &Element,
) -> Option<ElementId> {
    let ref_name = import_elem
        .get_prop("importedNamespace")
        .and_then(|v| v.as_str())
        .or_else(|| {
            import_elem
                .get_prop("unresolved_importedNamespace")
                .and_then(|v| v.as_str())
        })?;
    let ref_name = ref_name.trim().trim_end_matches("::*");
    if ref_name.is_empty() {
        return None;
    }
    if ref_name.contains("::") {
        return resolver
            .resolve_qualified_name(namespace_id, ref_name)
            .or_else(|| resolver.resolve_qualified_name_global(ref_name));
    }
    resolver
        .resolve_name(namespace_id, ref_name)
        .or_else(|| resolver.resolve_qualified_name_global(ref_name))
}

fn collect_namespace_members_with_imports(
    graph: &ModelGraph,
    namespace_id: &ElementId,
    query: &str,
    source_bucket: u32,
    detail_suffix: &'static str,
    seen_names: &mut FxHashSet<String>,
    scored_items: &mut Vec<(u32, CompletionCandidate)>,
) {
    let mut resolver = ResolutionContext::new(graph);
    let mut visited_namespaces = FxHashSet::default();
    let mut stack = vec![namespace_id.clone()];
    let query_normalized = normalize_completion_query(query);
    let query_has_quote = query.trim_start().starts_with('\'');

    while let Some(current_ns) = stack.pop() {
        if !visited_namespaces.insert(current_ns.clone()) {
            continue;
        }

        for membership in graph.memberships(&current_ns) {
            let Some(membership_view) = MembershipView::try_from_element(membership) else {
                continue;
            };
            if !membership_view.is_public() {
                continue;
            }

            let Some(member_id) = membership_view.member_element() else {
                continue;
            };
            let Some(member) = graph.get_element(member_id) else {
                continue;
            };

            match member.kind {
                ElementKind::NamespaceImport => {
                    if let Some(target_ns) = resolve_import_target_namespace_for_completion(
                        &mut resolver,
                        &current_ns,
                        member,
                    ) {
                        if !visited_namespaces.contains(&target_ns) {
                            stack.push(target_ns);
                        }
                    }
                    continue;
                }
                ElementKind::MembershipImport => {
                    if let Some(target_member_id) = resolve_import_target_namespace_for_completion(
                        &mut resolver,
                        &current_ns,
                        member,
                    ) {
                        if let Some(target_member) = graph.get_element(&target_member_id) {
                            if let Some(name) = target_member.name.as_deref() {
                                let Some(score) =
                                    rank_completion(&query_normalized, name, source_bucket)
                                else {
                                    continue;
                                };
                                if seen_names.insert(name.to_owned()) {
                                    let needs_quotes = !is_id_like(name);
                                    let insert_text = if needs_quotes {
                                        if query_has_quote {
                                            Some(format!("{name}'"))
                                        } else {
                                            Some(quote_name(name))
                                        }
                                    } else {
                                        None
                                    };
                                    scored_items.push((
                                        score,
                                        CompletionCandidate {
                                            label: name.to_owned(),
                                            kind: element_kind_to_completion_kind(
                                                &target_member.kind,
                                            ),
                                            detail: Some(format!(
                                                "{:?} ({detail_suffix})",
                                                target_member.kind
                                            )),
                                            sort_text: Some(format!(
                                                "{source_bucket:02}-1-{name}"
                                            )),
                                            insert_text,
                                            insert_text_is_snippet: false,
                                            label_details: None,
                                            additional_text_edits: Vec::new(),
                                            preselect: false,
                                            data_element_id: None,
                                            data_document_uri: None,
                                        },
                                    ));
                                }
                            }
                        }
                    }
                    continue;
                }
                _ => {}
            }

            let short_name = membership_view.member_short_name();
            let member_name = membership_view.member_name();
            let element_name = member.name.as_deref();
            let mut push_candidate =
                |candidate_name: &str, priority: u8, alt_description: Option<&str>| {
                    let Some(score) =
                        rank_completion(&query_normalized, candidate_name, source_bucket)
                    else {
                        return;
                    };
                    if !seen_names.insert(candidate_name.to_owned()) {
                        return;
                    }
                    let needs_quotes = !is_id_like(candidate_name);
                    let insert_text = if needs_quotes {
                        if query_has_quote {
                            Some(format!("{candidate_name}'"))
                        } else {
                            Some(quote_name(candidate_name))
                        }
                    } else {
                        None
                    };
                    let label_details = alt_description.map(|desc| CompletionLabelDetails {
                        detail: None,
                        description: Some(desc.to_owned()),
                    });
                    let priority_boost = match priority {
                        0 => 2,
                        1 => 1,
                        _ => 0,
                    };
                    scored_items.push((
                        score + priority_boost,
                        CompletionCandidate {
                            label: candidate_name.to_owned(),
                            kind: element_kind_to_completion_kind(&member.kind),
                            detail: Some(format!("{:?} ({detail_suffix})", member.kind)),
                            sort_text: Some(format!(
                                "{source_bucket:02}-{priority}-{candidate_name}"
                            )),
                            label_details,
                            insert_text,
                            insert_text_is_snippet: false,
                            additional_text_edits: Vec::new(),
                            preselect: false,
                            data_element_id: None,
                            data_document_uri: None,
                        },
                    ));
                };

            if let Some(short_name) = short_name {
                let alt = member_name.or(element_name);
                push_candidate(short_name, 0, alt);
            }
            if let Some(member_name) = member_name {
                push_candidate(member_name, 1, None);
            }
            if let Some(element_name) = element_name {
                push_candidate(element_name, 2, None);
            }
        }
    }
}

fn resolve_feature_chain_segment(
    graph: &ModelGraph,
    resolver: &mut ResolutionContext<'_>,
    current_id: &ElementId,
    segment: &str,
) -> Option<ElementId> {
    if segment.contains("::") {
        return resolver.resolve_qualified_name(current_id, segment);
    }
    for member in graph.members(current_id) {
        if member.name.as_deref() == Some(segment) {
            return Some(member.id.clone());
        }
    }
    sysml_core::resolution::scoping::resolve_with_feature_chaining(graph, current_id, segment)
        .element_id()
        .or_else(|| resolver.resolve_name(current_id, segment))
}

fn resolve_feature_chain_path_from_scope(
    graph: &ModelGraph,
    resolver: &mut ResolutionContext<'_>,
    scope_id: &ElementId,
    base_name: &str,
    doc_uri: &str,
    cursor_offset: usize,
) -> Option<ElementId> {
    let mut segments = base_name.split('.').filter(|s| !s.is_empty());
    let first = segments.next()?;
    let mut current = if first.contains("::") {
        resolver.resolve_qualified_name(scope_id, first)?
    } else {
        resolve_simple_name_lexically(graph, scope_id, first, doc_uri, cursor_offset)
            .or_else(|| resolver.resolve_name(scope_id, first))?
    };
    for segment in segments {
        current = resolve_feature_chain_segment(graph, resolver, &current, segment)?;
    }
    Some(current)
}

fn resolve_feature_chain_base_id(
    graph: &ModelGraph,
    doc_uri: &str,
    completion_ctx: &CompletionCtx<'_>,
    base_name: &str,
) -> Option<ElementId> {
    let mut resolver = ResolutionContext::new(graph);
    for scope_id in scope_chain_for_completion(graph, doc_uri, completion_ctx) {
        if let Some(id) = resolve_feature_chain_path_from_scope(
            graph,
            &mut resolver,
            &scope_id,
            base_name,
            doc_uri,
            completion_ctx.cursor_offset,
        ) {
            return Some(id);
        }
    }
    resolver
        .resolve_qualified_name_global(base_name)
        .or_else(|| {
            graph
                .elements
                .values()
                .find(|e| e.name.as_ref().map(|n| n == base_name).unwrap_or(false))
                .map(|e| e.id.clone())
        })
}

// ---- route classification + context build --------------------------------

fn classify_completion_route(
    text_before: &str,
    trigger: Option<&str>,
    syntax_ctx: Option<&SyntaxCtxSummary>,
) -> CompletionRoute {
    let in_import = syntax_ctx.map(|s| s.in_import).unwrap_or(false) || in_import_context(text_before);
    let in_comment_or_string = syntax_ctx.map(|s| s.in_comment_or_string).unwrap_or(false);
    let in_feature_chain = syntax_ctx.map(|s| s.in_feature_chain).unwrap_or(false);
    let in_type_ref = syntax_ctx.map(|s| s.in_type_ref).unwrap_or(false);

    if in_comment_or_string {
        return CompletionRoute::General;
    }
    if matches!(trigger, Some(".")) {
        return CompletionRoute::FeatureChain;
    }
    if matches!(trigger, Some(":")) {
        if text_before.ends_with("::") || in_import {
            return CompletionRoute::NamespaceMembers;
        }
        let before_colon = text_before.strip_suffix(':').unwrap_or(text_before);
        let last_char = before_colon.chars().last();
        if last_char
            .map(|c| c.is_alphanumeric() || c == '_')
            .unwrap_or(false)
        {
            return CompletionRoute::General;
        }
        return CompletionRoute::TypeReferences;
    }
    if matches!(trigger, Some(">"))
        && (text_before.ends_with(":>")
            || text_before.ends_with(":>>")
            || text_before.ends_with("::>"))
    {
        return CompletionRoute::TypeReferences;
    }

    let token = completion_token(text_before);
    if in_feature_chain && split_feature_chain_context(token).is_some() {
        return CompletionRoute::FeatureChain;
    }
    if split_namespace_context(token).is_some() {
        return CompletionRoute::NamespaceMembers;
    }
    if in_import && split_import_namespace_context(token).is_some() {
        return CompletionRoute::NamespaceMembers;
    }
    if split_feature_chain_context(token).is_some() {
        return CompletionRoute::FeatureChain;
    }
    if in_type_ref {
        return CompletionRoute::TypeReferences;
    }
    if type_query_from_context(text_before).is_some() {
        return CompletionRoute::TypeReferences;
    }
    CompletionRoute::General
}

fn build_completion_context<'a>(
    text_before: &'a str,
    trigger: Option<&str>,
    position_map: Option<&PositionMap>,
    offset: usize,
    syntax_ctx: Option<&SyntaxCtxSummary>,
) -> CompletionCtx<'a> {
    let token = completion_token(text_before);
    let in_import = syntax_ctx.map(|s| s.in_import).unwrap_or(false) || in_import_context(text_before);
    let namespace = split_namespace_context(token)
        .or_else(|| {
            if in_import {
                split_import_namespace_context(token)
            } else {
                None
            }
        })
        .map(|(namespace_path, member_query)| NamespaceCtx {
            namespace_path,
            member_query,
        });
    let feature = split_feature_chain_context(token).map(|(base_name, member_query)| FeatureCtx {
        base_name,
        member_query,
    });
    let type_query = type_query_from_context(text_before).map(str::trim);

    CompletionCtx {
        route: classify_completion_route(text_before, trigger, syntax_ctx),
        query: token,
        namespace,
        type_query,
        feature,
        cursor_offset: offset,
        in_import,
        anchor_element: position_map.and_then(|pm| completion_anchor(pm, offset)),
    }
}

// ---- workspace walks ------------------------------------------------------

#[derive(Debug, Clone)]
struct WorkspaceEntry {
    uri: String,
    element_kind: ElementKind,
}

#[derive(Default)]
struct WorkspaceIndex {
    by_name: HashMap<String, Vec<WorkspaceEntry>>,
    by_qname: HashMap<String, WorkspaceEntry>,
}

impl WorkspaceIndex {
    fn for_each_name(&self, mut f: impl FnMut(&str, &[WorkspaceEntry])) {
        for (name, entries) in &self.by_name {
            f(name, entries);
        }
    }

    fn for_each_qname_member(
        &self,
        namespace_path: &str,
        mut f: impl FnMut(&str, &WorkspaceEntry),
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

    fn for_each_root_qname(&self, mut f: impl FnMut(&str, &WorkspaceEntry)) {
        for (qname, entry) in &self.by_qname {
            if qname.is_empty() || qname.contains("::") {
                continue;
            }
            f(qname, entry);
        }
    }
}

/// Build the workspace completion index from a snapshot + a pre-resolved
/// `(uri, SourceFile)` list. Takes the snapshot rather than the host so the
/// per-file parse loop runs with NO host guard held — callers enumerate the
/// file list under their own small guard and drop it first (precedent:
/// `compute_full_diagnostics`).
fn build_workspace_index(
    analysis: &sysml_ide_db::Analysis,
    files: &[(String, sysml_ide_db::SourceFile)],
) -> WorkspaceIndex {
    let mut idx = WorkspaceIndex::default();
    for (uri, sf) in files {
        let graph = match Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            analysis.parse_file(*sf).graph().clone()
        })) {
            Ok(g) => g,
            Err(_) => continue,
        };
        index_file_graph(&mut idx, uri, &graph);
    }
    idx
}

fn span_belongs_to_uri(span_file: &str, uri: &str) -> bool {
    span_file.is_empty() || span_file == uri
}

fn index_file_graph(idx: &mut WorkspaceIndex, uri: &str, graph: &ModelGraph) {
    for element in graph.elements.values() {
        if let Some(span) = element.spans.first() {
            if span.file == SYNTHETIC_FILE {
                continue;
            }
            if !span_belongs_to_uri(&span.file, uri) {
                continue;
            }
        }
        if let Some(name) = &element.name {
            let entry = WorkspaceEntry {
                uri: uri.to_owned(),
                element_kind: element.kind.clone(),
            };
            idx.by_name
                .entry(name.clone())
                .or_default()
                .push(entry.clone());
            let qname = element.qname.as_ref().map(|q| q.to_string()).or_else(|| {
                graph
                    .build_qualified_name(&element.id)
                    .map(|q| q.to_string())
            });
            if let Some(qname_str) = qname {
                idx.by_qname.insert(qname_str, entry);
            }
        }
    }
}

// ---- the four route impls -------------------------------------------------

fn complete_import_roots(
    graph: &ModelGraph,
    library: Option<&ModelGraph>,
    library_element_ids: Option<&FxHashSet<ElementId>>,
    workspace: &WorkspaceIndex,
    query: &str,
) -> Vec<(u32, CompletionCandidate)> {
    let mut scored: Vec<(u32, CompletionCandidate)> = Vec::new();
    let mut seen_names: FxHashSet<String> = FxHashSet::default();

    let mut push_candidate =
        |scored: &mut Vec<(u32, CompletionCandidate)>,
         seen: &mut FxHashSet<String>,
         name: &str,
         kind: i32,
         detail: Option<String>,
         source_bucket: u32| {
            let Some(score) = rank_completion(query, name, source_bucket) else {
                return;
            };
            if !seen.insert(name.to_owned()) {
                return;
            }
            let source_sort = "00";
            scored.push((
                score,
                CompletionCandidate {
                    label: name.to_owned(),
                    kind,
                    detail,
                    sort_text: Some(format!("{source_sort}-{name}")),
                    insert_text: None,
                    insert_text_is_snippet: false,
                    label_details: None,
                    additional_text_edits: Vec::new(),
                    preselect: false,
                    data_element_id: None,
                    data_document_uri: None,
                },
            ));
        };

    workspace.for_each_root_qname(|name, entry| {
        push_candidate(
            &mut scored,
            &mut seen_names,
            name,
            element_kind_to_completion_kind(&entry.element_kind),
            Some(format!("{:?} (workspace)", entry.element_kind)),
            SOURCE_IMPORT_WORKSPACE,
        );
    });
    workspace.for_each_name(|name, entries| {
        if !entries
            .iter()
            .any(|entry| entry.element_kind == ElementKind::Package)
        {
            return;
        }
        push_candidate(
            &mut scored,
            &mut seen_names,
            name,
            KIND_MODULE,
            Some("Package (workspace)".to_owned()),
            SOURCE_IMPORT_WORKSPACE,
        );
    });

    if let Some(lib) = library {
        for root in lib.roots() {
            let Some(name) = root.name.as_deref() else {
                continue;
            };
            push_candidate(
                &mut scored,
                &mut seen_names,
                name,
                element_kind_to_completion_kind(&root.kind),
                Some(format!("{:?} (standard library)", root.kind)),
                SOURCE_IMPORT_LIBRARY,
            );
        }
    }

    for root in graph.roots() {
        let Some(name) = root.name.as_deref() else {
            continue;
        };
        if library_element_ids
            .map(|ids| ids.contains(&root.id))
            .unwrap_or(false)
        {
            continue;
        }
        push_candidate(
            &mut scored,
            &mut seen_names,
            name,
            element_kind_to_completion_kind(&root.kind),
            Some(format!("{:?} (current file)", root.kind)),
            SOURCE_IMPORT_LOCAL,
        );
    }

    scored
}

fn complete_namespace_members(
    graph: &ModelGraph,
    library: Option<&ModelGraph>,
    library_element_ids: Option<&FxHashSet<ElementId>>,
    workspace: &WorkspaceIndex,
    completion_ctx: &CompletionCtx<'_>,
) -> Option<Vec<CompletionCandidate>> {
    let NamespaceCtx {
        namespace_path: ns_path,
        member_query,
    } = completion_ctx.namespace?;
    if ns_path.is_empty() {
        return Some(complete_roots(graph, member_query));
    }

    let mut scored: Vec<(u32, CompletionCandidate)> = Vec::new();
    let mut seen_names: FxHashSet<String> = FxHashSet::default();

    if let Some(namespace) = graph.resolve_qname(ns_path) {
        let is_library_ns = library_element_ids
            .map(|ids| ids.contains(&namespace.id))
            .unwrap_or(false);
        if !is_library_ns {
            collect_namespace_members_with_imports(
                graph,
                &namespace.id,
                member_query,
                SOURCE_LOCAL,
                "current file",
                &mut seen_names,
                &mut scored,
            );
        }
    }

    if let Some(lib) = library {
        if let Some(namespace) = lib.resolve_qname(ns_path) {
            collect_namespace_members_with_imports(
                lib,
                &namespace.id,
                member_query,
                SOURCE_LIBRARY,
                "standard library",
                &mut seen_names,
                &mut scored,
            );
        }
    }

    workspace.for_each_qname_member(ns_path, |name, entry| {
        if seen_names.contains(name) {
            return;
        }
        if let Some(score) = rank_completion(member_query, name, SOURCE_WORKSPACE) {
            if seen_names.insert(name.to_owned()) {
                scored.push((
                    score,
                    CompletionCandidate {
                        label: name.to_owned(),
                        kind: element_kind_to_completion_kind(&entry.element_kind),
                        detail: Some(format!("{:?} (from other file)", entry.element_kind)),
                        sort_text: None,
                        insert_text: None,
                        insert_text_is_snippet: false,
                        label_details: None,
                        additional_text_edits: Vec::new(),
                        preselect: false,
                        data_element_id: None,
                        data_document_uri: None,
                    },
                ));
            }
        }
    });

    if scored.is_empty() {
        return None;
    }
    Some(limit(dedup_by_label(sort_scored_items(scored))))
}

fn complete_type_references(
    graph: &ModelGraph,
    position_map: &PositionMap,
    library: Option<&ModelGraph>,
    workspace: &WorkspaceIndex,
    uri: &str,
    content: &str,
    completion_ctx: &CompletionCtx<'_>,
) -> Vec<CompletionCandidate> {
    let query = completion_ctx.type_query.unwrap_or("");
    let mut scored: Vec<(u32, CompletionCandidate)> = Vec::new();
    let mut seen_names: FxHashSet<String> = FxHashSet::default();
    let mut import_edit_cache: HashMap<String, Option<TextEditPos>> = HashMap::new();

    for element_id in position_map.element_ids() {
        let Some(def) = graph.get_element(&element_id) else {
            continue;
        };
        if !def.kind.is_definition() || def.name.is_none() {
            continue;
        }
        let name = def.name.clone().unwrap();
        let Some(score) = rank_completion(query, &name, SOURCE_LOCAL) else {
            continue;
        };
        let qname = def
            .qname
            .as_ref()
            .map(|q| q.to_string())
            .unwrap_or_else(|| name.clone());
        seen_names.insert(name.clone());
        scored.push((
            score,
            CompletionCandidate {
                label: name,
                kind: element_kind_to_completion_kind(&def.kind),
                detail: Some(format!("{:?}", def.kind)),
                sort_text: None,
                insert_text: None,
                insert_text_is_snippet: false,
                label_details: Some(CompletionLabelDetails {
                    detail: None,
                    description: Some(qname),
                }),
                additional_text_edits: Vec::new(),
                preselect: false,
                data_element_id: None,
                data_document_uri: None,
            },
        ));
    }

    workspace.for_each_name(|name, entries| {
        if seen_names.contains(name) {
            return;
        }
        let Some(score) = rank_completion(query, name, SOURCE_WORKSPACE) else {
            return;
        };
        for entry in entries {
            if entry.element_kind.is_definition() {
                seen_names.insert(name.to_owned());
                scored.push((
                    score,
                    CompletionCandidate {
                        label: name.to_owned(),
                        kind: element_kind_to_completion_kind(&entry.element_kind),
                        detail: Some(format!("{:?} (from other file)", entry.element_kind)),
                        sort_text: None,
                        insert_text: None,
                        insert_text_is_snippet: false,
                        label_details: None,
                        additional_text_edits: Vec::new(),
                        preselect: false,
                        data_element_id: None,
                        data_document_uri: None,
                    },
                ));
                break;
            }
        }
    });

    if let Some(lib) = library {
        for def in lib.elements.values() {
            if def.kind.is_definition() {
                if let Some(name) = &def.name {
                    let Some(score) = rank_completion(query, name, SOURCE_LIBRARY) else {
                        continue;
                    };
                    if seen_names.insert(name.clone()) {
                        let root = def
                            .qname
                            .as_ref()
                            .and_then(|q| q.to_string().split("::").next().map(str::to_string))
                            .filter(|s| !s.is_empty());
                        let import_edit = root.and_then(|root| {
                            import_edit_cache
                                .entry(root.clone())
                                .or_insert_with(|| {
                                    build_policy_a_import_edit(
                                        graph,
                                        uri,
                                        content,
                                        completion_ctx.cursor_offset,
                                        &root,
                                    )
                                })
                                .clone()
                        });
                        let kind_label = element_kind_to_hover_label(&def.kind);
                        let qname_str = def.qname.as_ref().map(|q| q.to_string());
                        scored.push((
                            score,
                            CompletionCandidate {
                                label: name.clone(),
                                kind: element_kind_to_completion_kind(&def.kind),
                                detail: Some(format!("{} (library)", kind_label)),
                                sort_text: None,
                                insert_text: None,
                                insert_text_is_snippet: false,
                                label_details: Some(CompletionLabelDetails {
                                    detail: None,
                                    description: qname_str,
                                }),
                                additional_text_edits: import_edit.into_iter().collect(),
                                preselect: false,
                                data_element_id: None,
                                data_document_uri: None,
                            },
                        ));
                    }
                }
            }
        }
    }

    limit(dedup_by_label(sort_scored_items(scored)))
}

fn complete_feature_chain(
    graph: &ModelGraph,
    uri: &str,
    completion_ctx: &CompletionCtx<'_>,
) -> Vec<CompletionCandidate> {
    let FeatureCtx {
        base_name,
        member_query,
    } = match completion_ctx.feature {
        Some(ctx) => ctx,
        None => return Vec::new(),
    };
    if base_name.is_empty() {
        return Vec::new();
    }

    let element_id = match resolve_feature_chain_base_id(graph, uri, completion_ctx, base_name) {
        Some(id) => id,
        None => return Vec::new(),
    };
    let Some(element) = graph.get_element(&element_id) else {
        return Vec::new();
    };

    let mut scored: Vec<(u32, CompletionCandidate)> = Vec::new();
    let mut seen_names: FxHashSet<String> = FxHashSet::default();

    for member in graph.owned_members(&element.id) {
        if let Some(name) = &member.name {
            let Some(score) = rank_completion(member_query, name, SOURCE_FEATURE_OWNED) else {
                continue;
            };
            if seen_names.insert(name.clone()) {
                let type_info = find_element_type(member, graph);
                scored.push((
                    score,
                    CompletionCandidate {
                        label: name.clone(),
                        kind: element_kind_to_completion_kind(&member.kind),
                        detail: type_info.or_else(|| Some(format!("{:?}", member.kind))),
                        sort_text: None,
                        insert_text: None,
                        insert_text_is_snippet: false,
                        label_details: member.qname.as_ref().map(|q| CompletionLabelDetails {
                            detail: None,
                            description: Some(q.to_string()),
                        }),
                        additional_text_edits: Vec::new(),
                        preselect: false,
                        data_element_id: None,
                        data_document_uri: None,
                    },
                ));
            }
        }
    }

    if let Some(type_name) = find_element_type(element, graph) {
        let type_def = graph
            .elements
            .values()
            .find(|e| e.kind.is_definition() && e.name.as_deref() == Some(type_name.as_str()));
        if let Some(def) = type_def {
            for member in graph.owned_members(&def.id) {
                if let Some(name) = &member.name {
                    let Some(score) = rank_completion(member_query, name, SOURCE_FEATURE_TYPE)
                    else {
                        continue;
                    };
                    if seen_names.insert(name.clone()) {
                        let type_info = find_element_type(member, graph);
                        scored.push((
                            score,
                            CompletionCandidate {
                                label: name.clone(),
                                kind: element_kind_to_completion_kind(&member.kind),
                                detail: type_info.or_else(|| Some(format!("{:?}", member.kind))),
                                sort_text: None,
                                insert_text: None,
                                insert_text_is_snippet: false,
                                label_details: Some(CompletionLabelDetails {
                                    detail: Some(format!(" (from {})", type_name)),
                                    description: None,
                                }),
                                additional_text_edits: Vec::new(),
                                preselect: false,
                                data_element_id: None,
                                data_document_uri: None,
                            },
                        ));
                    }
                }
            }

            let mut visited: FxHashSet<ElementId> = FxHashSet::default();
            visited.insert(def.id.clone());
            let mut stack: Vec<ElementId> = vec![def.id.clone()];
            let max_depth = 10;
            let mut depth = 0;

            while let Some(current_id) = stack.pop() {
                depth += 1;
                if depth > max_depth {
                    break;
                }
                for rel in graph.outgoing(&current_id) {
                    if rel.kind == RelationshipKind::Specialize
                        && visited.insert(rel.target.clone())
                    {
                        if let Some(super_def) = graph.get_element(&rel.target) {
                            let super_name =
                                super_def.name.clone().unwrap_or_else(|| "?".to_owned());
                            for member in graph.owned_members(&super_def.id) {
                                if let Some(name) = &member.name {
                                    let Some(score) = rank_completion(
                                        member_query,
                                        name,
                                        SOURCE_FEATURE_INHERITED,
                                    ) else {
                                        continue;
                                    };
                                    if seen_names.insert(name.clone()) {
                                        let type_info = find_element_type(member, graph);
                                        scored.push((
                                            score,
                                            CompletionCandidate {
                                                label: name.clone(),
                                                kind: element_kind_to_completion_kind(
                                                    &member.kind,
                                                ),
                                                detail: type_info
                                                    .or_else(|| Some(format!("{:?}", member.kind))),
                                                sort_text: None,
                                                insert_text: None,
                                                insert_text_is_snippet: false,
                                                label_details: Some(CompletionLabelDetails {
                                                    detail: Some(format!(
                                                        " (inherited from {})",
                                                        super_name
                                                    )),
                                                    description: None,
                                                }),
                                                additional_text_edits: Vec::new(),
                                                preselect: false,
                                                data_element_id: None,
                                                data_document_uri: None,
                                            },
                                        ));
                                    }
                                }
                            }
                            stack.push(super_def.id.clone());
                        }
                    }
                }
            }
        }
    }

    limit(dedup_by_label(sort_scored_items(scored)))
}

fn complete_general(
    graph: &ModelGraph,
    position_map: &PositionMap,
    library: Option<&ModelGraph>,
    library_element_ids: Option<&FxHashSet<ElementId>>,
    workspace: &WorkspaceIndex,
    uri: &str,
    content: &str,
    completion_ctx: &CompletionCtx<'_>,
) -> Vec<CompletionCandidate> {
    let query = completion_ctx.query;
    let mut scored: Vec<(u32, CompletionCandidate)> = Vec::new();

    if completion_ctx.in_import {
        scored.extend(complete_import_roots(
            graph,
            library,
            library_element_ids,
            workspace,
            query,
        ));
    }

    let keywords = [
        ("package", "Package declaration"),
        ("part", "Part usage"),
        ("part def", "Part definition"),
        ("attribute", "Attribute usage"),
        ("attribute def", "Attribute definition"),
        ("action", "Action usage"),
        ("action def", "Action definition"),
        ("state", "State usage"),
        ("state def", "State definition"),
        ("port", "Port usage"),
        ("port def", "Port definition"),
        ("connection", "Connection usage"),
        ("connection def", "Connection definition"),
        ("interface", "Interface usage"),
        ("interface def", "Interface definition"),
        ("item", "Item usage"),
        ("item def", "Item definition"),
        ("requirement", "Requirement usage"),
        ("requirement def", "Requirement definition"),
        ("constraint", "Constraint usage"),
        ("constraint def", "Constraint definition"),
        ("allocation", "Allocation usage"),
        ("allocation def", "Allocation definition"),
        ("import", "Import declaration"),
        ("alias", "Alias declaration"),
        ("ref", "Reference usage"),
        ("in", "Input feature"),
        ("out", "Output feature"),
        ("inout", "Input/output feature"),
        ("private", "Private visibility"),
        ("protected", "Protected visibility"),
        ("public", "Public visibility"),
        ("abstract", "Abstract modifier"),
        ("readonly", "Readonly modifier"),
        ("derived", "Derived modifier"),
        ("end", "End modifier"),
        ("redefines", "Redefines relationship"),
        ("subsets", "Subsets relationship"),
        ("specializes", "Specializes relationship"),
        ("entry", "Entry action"),
        ("exit", "Exit action"),
        ("do", "Do action"),
        ("transition", "Transition"),
        ("accept", "Accept action"),
        ("send", "Send action"),
        ("if", "If action"),
        ("then", "Then clause"),
        ("else", "Else clause"),
        ("while", "While action"),
        ("for", "For action"),
        ("return", "Return action"),
    ];
    for (kw, desc) in keywords {
        if let Some(score) = rank_completion(query, kw, SOURCE_KEYWORD) {
            scored.push((
                score,
                CompletionCandidate {
                    label: kw.to_owned(),
                    kind: KIND_KEYWORD,
                    detail: Some(desc.to_owned()),
                    sort_text: None,
                    insert_text: None,
                    insert_text_is_snippet: false,
                    label_details: None,
                    additional_text_edits: Vec::new(),
                    preselect: false,
                    data_element_id: None,
                    data_document_uri: None,
                },
            ));
        }
    }

    let snippets = [
        ("part def", "part def ${1:Name} {\n\t$0\n}", "Part definition (snippet)"),
        ("action def", "action def ${1:Name} {\n\t$0\n}", "Action definition (snippet)"),
        ("state def", "state def ${1:Name} {\n\t$0\n}", "State definition (snippet)"),
        ("port def", "port def ${1:Name} {\n\t$0\n}", "Port definition (snippet)"),
        ("item def", "item def ${1:Name} {\n\t$0\n}", "Item definition (snippet)"),
        (
            "requirement def",
            "requirement def ${1:Name} {\n\t$0\n}",
            "Requirement definition (snippet)",
        ),
        (
            "constraint def",
            "constraint def ${1:Name} {\n\t$0\n}",
            "Constraint definition (snippet)",
        ),
        ("package", "package ${1:Name} {\n\t$0\n}", "Package (snippet)"),
        (
            "attribute def",
            "attribute def ${1:Name} {\n\t$0\n}",
            "Attribute definition (snippet)",
        ),
    ];
    for (label, snippet, desc) in snippets {
        if let Some(score) = rank_completion(query, label, SOURCE_SNIPPET) {
            scored.push((
                score.saturating_sub(1),
                CompletionCandidate {
                    label: format!("{} {{ }}", label),
                    kind: KIND_SNIPPET,
                    detail: Some(desc.to_owned()),
                    sort_text: None,
                    insert_text: Some(snippet.to_owned()),
                    insert_text_is_snippet: true,
                    label_details: None,
                    additional_text_edits: Vec::new(),
                    preselect: false,
                    data_element_id: None,
                    data_document_uri: None,
                },
            ));
        }
    }

    for element_id in position_map.element_ids() {
        let Some(element) = graph.get_element(&element_id) else {
            continue;
        };
        if let Some(name) = &element.name {
            if let Some(score) = rank_completion(query, name, SOURCE_LOCAL) {
                scored.push((
                    score,
                    CompletionCandidate {
                        label: name.clone(),
                        kind: element_kind_to_completion_kind(&element.kind),
                        detail: Some(format!("{:?}", element.kind)),
                        sort_text: None,
                        insert_text: None,
                        insert_text_is_snippet: false,
                        label_details: None,
                        additional_text_edits: Vec::new(),
                        preselect: false,
                        data_element_id: Some(element.id.to_string()),
                        data_document_uri: Some(uri.to_owned()),
                    },
                ));
            }
        }
    }

    if !query.is_empty() {
        let local_names: FxHashSet<String> = scored
            .iter()
            .map(|(_, item)| item.label.clone())
            .collect();
        let mut import_edit_cache: HashMap<String, Option<TextEditPos>> = HashMap::new();
        let mut workspace_count = 0usize;
        workspace.for_each_name(|name, entries| {
            if workspace_count >= 20 || local_names.contains(name) {
                return;
            }
            let Some(score) = rank_completion(query, name, SOURCE_WORKSPACE) else {
                return;
            };
            for entry in entries {
                if entry.uri == uri {
                    continue;
                }
                let file_stem = file_stem_from_uri(&entry.uri).unwrap_or_else(|| "other file".to_owned());
                let import_root = file_stem.clone();
                let import_edit = import_edit_cache
                    .entry(import_root.clone())
                    .or_insert_with(|| {
                        build_policy_a_import_edit(
                            graph,
                            uri,
                            content,
                            completion_ctx.cursor_offset,
                            &import_root,
                        )
                    })
                    .clone();
                scored.push((
                    score,
                    CompletionCandidate {
                        label: name.to_owned(),
                        kind: element_kind_to_completion_kind(&entry.element_kind),
                        detail: Some(format!(
                            "{:?} (auto-import from {})",
                            entry.element_kind, file_stem
                        )),
                        sort_text: Some(format!("08-2-{}", name)),
                        insert_text: None,
                        insert_text_is_snippet: false,
                        label_details: None,
                        additional_text_edits: import_edit.into_iter().collect(),
                        preselect: false,
                        data_element_id: None,
                        data_document_uri: None,
                    },
                ));
                workspace_count += 1;
                break;
            }
        });
    }

    let mut items = limit(dedup_by_label(sort_scored_items(scored)));
    if let Some(first) = items.first_mut() {
        if !query.is_empty() {
            first.preselect = true;
        }
    }
    items
}

fn complete_roots(graph: &ModelGraph, query: &str) -> Vec<CompletionCandidate> {
    graph
        .roots()
        .filter_map(|root| {
            root.name.as_ref().map(|name| CompletionCandidate {
                label: name.clone(),
                kind: element_kind_to_completion_kind(&root.kind),
                detail: Some(format!("{:?}", root.kind)),
                sort_text: None,
                insert_text: None,
                insert_text_is_snippet: false,
                label_details: None,
                additional_text_edits: Vec::new(),
                preselect: false,
                data_element_id: None,
                data_document_uri: None,
            })
        })
        .filter(|item| starts_with_ignore_ascii_case(&item.label, query))
        .collect()
}

fn file_stem_from_uri(uri: &str) -> Option<String> {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    let path = path.split('?').next().unwrap_or(path);
    let path = path.split('#').next().unwrap_or(path);
    let last_segment = path.rsplit('/').next().unwrap_or(path);
    let stem = last_segment.split('.').next().unwrap_or(last_segment);
    if stem.is_empty() {
        None
    } else {
        Some(stem.to_owned())
    }
}

// ---- top-level entry points -----------------------------------------------

/// Compute completion candidates for the cursor at `(uri, line, col)`.
pub fn compute_completion(
    host: &Mutex<AnalysisHost>,
    uri: &str,
    line: u32,
    col: u32,
    trigger: Option<&str>,
    syntax_ctx: Option<&SyntaxCtxSummary>,
) -> Vec<CompletionCandidate> {
    // Resolve every host-keyed handle under a SMALL guard, then drop the
    // guard before running any salsa query — the closure below parses the
    // cursor file AND every user file (workspace index); doing that under
    // the guard serializes every other host user (precedent:
    // `compute_full_diagnostics`).
    let (analysis, sf_opt, user_files) = {
        let guard = host.lock().unwrap();
        let sf_opt = guard.file_id(uri).and_then(|id| guard.source_file(id));
        let user_files: Vec<(String, sysml_ide_db::SourceFile)> = guard
            .files()
            .user_file_ids()
            .filter_map(|fid| {
                Some((guard.files().uri(fid)?.to_string(), guard.source_file(fid)?))
            })
            .collect();
        (guard.analysis(), sf_opt, user_files)
    };
    let Some(sf) = sf_opt else {
        return Vec::new();
    };

    let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
        let content = analysis.file_text(sf).to_owned();
        let offset = position_to_offset(line, col, &content).min(content.len());
        let text_before = &content[..offset];

        let position_map = analysis.position_map(sf);
        let parsed = analysis.parse_file(sf);
        let graph = parsed.graph().clone();

        let owned_ctx = {
            let completion_ctx = build_completion_context(
                text_before,
                trigger,
                Some(&position_map),
                offset,
                syntax_ctx,
            );
            completion_ctx_owned(&completion_ctx)
        };
        let route = owned_ctx.route;

        let library_data = analysis
            .library_graph()
            .map(|lg| lg.data(analysis.db()).clone());
        let library_graph_owned: Option<ModelGraph> =
            library_data.as_ref().map(|d| d.graph().clone());
        let library_element_ids: Option<FxHashSet<ElementId>> =
            library_data.as_ref().map(|d| d.element_ids().clone());

        let workspace = build_workspace_index(&analysis, &user_files);

        Some((
            content,
            offset,
            graph,
            position_map,
            library_graph_owned,
            library_element_ids,
            workspace,
            owned_ctx,
            route,
        ))
    }));

    drop(analysis);

    let Ok(Some((
        content,
        _offset,
        graph,
        position_map,
        library_graph,
        library_element_ids,
        workspace,
        owned_ctx,
        route,
    ))) = result
    else {
        return Vec::new();
    };

    let completion_ctx = ctx_from_owned(&owned_ctx);

    match route {
        CompletionRoute::NamespaceMembers => complete_namespace_members(
            &graph,
            library_graph.as_ref(),
            library_element_ids.as_ref(),
            &workspace,
            &completion_ctx,
        )
        .unwrap_or_default(),
        CompletionRoute::TypeReferences => complete_type_references(
            &graph,
            &position_map,
            library_graph.as_ref(),
            &workspace,
            uri,
            &content,
            &completion_ctx,
        ),
        CompletionRoute::FeatureChain => complete_feature_chain(&graph, uri, &completion_ctx),
        CompletionRoute::General => complete_general(
            &graph,
            &position_map,
            library_graph.as_ref(),
            library_element_ids.as_ref(),
            &workspace,
            uri,
            &content,
            &completion_ctx,
        ),
    }
}

/// Compute resolve-time enrichment (doc-comment + type detail) for a previously-emitted
/// completion item. Walks salsa-loaded files first, then falls back to the library.
pub fn compute_completion_resolve(
    host: &Mutex<AnalysisHost>,
    preferred_uri: Option<&str>,
    element_id: &ElementId,
) -> Option<CompletionDetails> {
    // Resolve every host-keyed handle under a SMALL guard, then drop the
    // guard before the per-file parse loop — parsing every user file under
    // the guard serializes every other host user (precedent:
    // `compute_full_diagnostics`).
    let (analysis, preferred_sf, user_files) = {
        let guard = host.lock().unwrap();
        let preferred_sf = preferred_uri
            .and_then(|uri| guard.file_id(uri))
            .and_then(|id| guard.source_file(id));
        let user_files: Vec<sysml_ide_db::SourceFile> = guard
            .files()
            .user_file_ids()
            .filter_map(|fid| guard.source_file(fid))
            .collect();
        (guard.analysis(), preferred_sf, user_files)
    };

    let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
        if let Some(sf) = preferred_sf {
            let content = analysis.file_text(sf).to_owned();
            let parsed = analysis.parse_file(sf);
            let graph = parsed.graph();
            if let Some(element) = graph.get_element(element_id) {
                return Some(build_completion_details(element, graph, Some(&content)));
            }
        }
        for sf in &user_files {
            let content = analysis.file_text(*sf).to_owned();
            let parsed = analysis.parse_file(*sf);
            let graph = parsed.graph();
            if let Some(element) = graph.get_element(element_id) {
                return Some(build_completion_details(element, graph, Some(&content)));
            }
        }
        if let Some(lg) = analysis.library_graph() {
            let data = lg.data(analysis.db()).clone();
            let graph = data.graph();
            if let Some(element) = graph.get_element(element_id) {
                return Some(build_completion_details(element, graph, None));
            }
        }
        None
    }));

    drop(analysis);
    result.ok().flatten()
}

fn build_completion_details(
    element: &Element,
    graph: &ModelGraph,
    source: Option<&str>,
) -> CompletionDetails {
    let documentation_markdown =
        source.and_then(|s| extract_doc_comment(element, s));
    let detail = find_element_type(element, graph).map(|t| format!(": {}", t));
    CompletionDetails {
        documentation_markdown,
        detail,
    }
}

// ---- owned ctx (string-backed copy of CompletionCtx for cross-snapshot usage)

#[derive(Debug, Clone)]
struct OwnedCompletionCtx {
    route: CompletionRoute,
    query: String,
    namespace: Option<(String, String)>,
    type_query: Option<String>,
    feature: Option<(String, String)>,
    cursor_offset: usize,
    in_import: bool,
    anchor_element: Option<ElementId>,
}

fn completion_ctx_owned(ctx: &CompletionCtx<'_>) -> OwnedCompletionCtx {
    OwnedCompletionCtx {
        route: ctx.route,
        query: ctx.query.to_owned(),
        namespace: ctx
            .namespace
            .map(|n| (n.namespace_path.to_owned(), n.member_query.to_owned())),
        type_query: ctx.type_query.map(|s| s.to_owned()),
        feature: ctx
            .feature
            .map(|f| (f.base_name.to_owned(), f.member_query.to_owned())),
        cursor_offset: ctx.cursor_offset,
        in_import: ctx.in_import,
        anchor_element: ctx.anchor_element.clone(),
    }
}

fn ctx_from_owned(owned: &OwnedCompletionCtx) -> CompletionCtx<'_> {
    CompletionCtx {
        route: owned.route,
        query: owned.query.as_str(),
        namespace: owned.namespace.as_ref().map(|(p, q)| NamespaceCtx {
            namespace_path: p.as_str(),
            member_query: q.as_str(),
        }),
        type_query: owned.type_query.as_deref(),
        feature: owned.feature.as_ref().map(|(b, q)| FeatureCtx {
            base_name: b.as_str(),
            member_query: q.as_str(),
        }),
        cursor_offset: owned.cursor_offset,
        in_import: owned.in_import,
        anchor_element: owned.anchor_element.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_namespace_completion_without_trigger() {
        let route = classify_completion_route("import ScalarValues::Int", None, None);
        assert_eq!(route, CompletionRoute::NamespaceMembers);
    }

    #[test]
    fn classify_type_completion_without_trigger() {
        let route = classify_completion_route("part x : Int", None, None);
        assert_eq!(route, CompletionRoute::TypeReferences);
    }

    #[test]
    fn classify_feature_chain_completion_without_trigger() {
        let route = classify_completion_route("engine.rp", None, None);
        assert_eq!(route, CompletionRoute::FeatureChain);
    }

    #[test]
    fn classify_numeric_literal_as_general_completion() {
        let route = classify_completion_route("attribute x = 3.14", None, None);
        assert_eq!(route, CompletionRoute::General);
    }

    #[test]
    fn classify_import_single_colon_trigger_as_namespace_completion() {
        let route = classify_completion_route("import ScalarValues:", Some(":"), None);
        assert_eq!(route, CompletionRoute::NamespaceMembers);
    }

    #[test]
    fn classify_import_context_case_insensitive() {
        let route = classify_completion_route("Import ScalarValues:", Some(":"), None);
        assert_eq!(route, CompletionRoute::NamespaceMembers);
    }

    #[test]
    fn classify_specializes_operator_trigger() {
        let route = classify_completion_route("part def Car :>", Some(">"), None);
        assert_eq!(route, CompletionRoute::TypeReferences);
    }

    #[test]
    fn classify_redefines_operator_trigger() {
        let route = classify_completion_route("part engine :>>", Some(">"), None);
        assert_eq!(route, CompletionRoute::TypeReferences);
    }

    #[test]
    fn classify_references_operator_trigger() {
        let route = classify_completion_route("ref item ::>", Some(">"), None);
        assert_eq!(route, CompletionRoute::TypeReferences);
    }

    #[test]
    fn rank_completion_accepts_single_edit_typo_for_long_queries() {
        let score = rank_completion("Intager", "Integer", SOURCE_LIBRARY);
        assert!(score.is_some());
    }

    #[test]
    fn rank_completion_rejects_multi_edit_typos() {
        let score = rank_completion("Xntagar", "Integer", SOURCE_LIBRARY);
        assert!(score.is_none());
    }

    #[test]
    fn file_stem_extracts_basename_without_extension() {
        assert_eq!(file_stem_from_uri("file:///a/b/foo.sysml"), Some("foo".to_owned()));
        assert_eq!(file_stem_from_uri("/foo/bar/baz.kerml"), Some("baz".to_owned()));
        assert_eq!(file_stem_from_uri("plain"), Some("plain".to_owned()));
    }

    #[test]
    fn context_extracts_namespace_query() {
        let ctx = build_completion_context("import ScalarValues::Int", None, None, 0, None);
        assert_eq!(ctx.route, CompletionRoute::NamespaceMembers);
        let ns = ctx.namespace.expect("namespace");
        assert_eq!(ns.namespace_path, "ScalarValues");
        assert_eq!(ns.member_query, "Int");
    }
}
