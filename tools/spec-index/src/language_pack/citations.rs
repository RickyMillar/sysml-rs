//! Clause/anchor resolution against the derived plaintext heading index
//! Anchors are derived deterministically from clause titles, never
//! from raw line numbers.

use std::collections::BTreeMap;

use super::LpError;

/// Index every `## <clause> <title>` heading line in a derived spec-text
/// artifact into `{ clause-number -> title }`.
pub fn heading_index(spec_text: &str) -> BTreeMap<String, String> {
    let mut index = BTreeMap::new();
    for line in spec_text.lines() {
        if let Some(rest) = line.strip_prefix("## ") {
            let mut parts = rest.splitn(2, ' ');
            if let (Some(clause), Some(title)) = (parts.next(), parts.next()) {
                index
                    .entry(clause.to_owned())
                    .or_insert_with(|| title.trim().to_owned());
            }
        }
    }
    index
}

/// Index every `## <clause> <title>` heading into `{ clause-number ->
/// lowercased heading + up to `context_lines` following body lines }`, stopping
/// at the next `## ` heading. Feeds the topical-plausibility citation gate
/// (`report::cards_missing_topical_citation`): a cited clause must actually
/// *discuss* the concept the card governs, so we need the clause's prose, not
/// just its title. First occurrence wins, matching `heading_index` semantics.
pub fn clause_contexts(spec_text: &str, context_lines: usize) -> BTreeMap<String, String> {
    let mut index = BTreeMap::new();
    let lines: Vec<&str> = spec_text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
        let Some(rest) = line.strip_prefix("## ") else { continue };
        let mut parts = rest.splitn(2, ' ');
        let (Some(clause), Some(_title)) = (parts.next(), parts.next()) else { continue };
        if index.contains_key(clause) {
            continue;
        }
        let mut buf = String::from(rest);
        for follow in lines.iter().skip(i + 1).take(context_lines) {
            if follow.starts_with("## ") {
                break;
            }
            buf.push('\n');
            buf.push_str(follow);
        }
        index.insert(clause.to_owned(), buf.to_ascii_lowercase());
    }
    index
}

/// Kebab-case a clause title into an anchor matching the schema pattern
/// `^[a-z0-9]+(-[a-z0-9]+)*$`: lowercase, runs of non-alphanumerics collapse to
/// a single `-`, leading/trailing `-` trimmed. Stable across line renumbering.
pub fn anchor_for_title(title: &str) -> String {
    let mut out = String::new();
    let mut pending_dash = false;
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_dash && !out.is_empty() {
                out.push('-');
            }
            pending_dash = false;
            out.push(c.to_ascii_lowercase());
        } else {
            pending_dash = true;
        }
    }
    out
}

/// Resolve a clause against the heading index, returning `(clause, anchor)`.
/// A clause absent from the index is a hard conflict.
pub fn resolve_clause(
    index: &BTreeMap<String, String>,
    clause: &str,
) -> Result<(String, String), LpError> {
    let title = index
        .get(clause)
        .ok_or_else(|| LpError::ClauseNotFound(clause.to_owned()))?;
    Ok((clause.to_owned(), anchor_for_title(title)))
}

/// Resolve a clause, falling back to its deepest existing ANCESTOR heading when
/// the exact clause is not indexed. The derived spec
/// text extracts headings only to a bounded depth, so a fine library sub-clause
/// like `9.2.12.2.5` has no heading; its deepest ancestor `9.2.12.2` does and is
/// a true containing clause — an honest, coarser locator, never a fabricated
/// one. Returns the resolved ancestor clause + its anchor. `9.9`-style
/// placeholders (`8.3.x`) resolve to no ancestor and stay a hard miss.
/// Returns `(clause, anchor, resolution)` where `resolution` is `"exact"` when
/// the cited clause is itself a heading and `"ancestor"` when it was resolved to
/// its deepest existing ancestor.
pub fn resolve_clause_or_ancestor(
    index: &BTreeMap<String, String>,
    clause: &str,
) -> Result<(String, String, &'static str), LpError> {
    if let Ok((c, a)) = resolve_clause(index, clause) {
        return Ok((c, a, "exact"));
    }
    let mut cur = clause;
    while let Some(pos) = cur.rfind('.') {
        cur = &cur[..pos];
        if let Some(title) = index.get(cur) {
            return Ok((cur.to_owned(), anchor_for_title(title), "ancestor"));
        }
    }
    Err(LpError::ClauseNotFound(clause.to_owned()))
}

/// Extract the leading clause-NUMBER token from a spec-ref string such as
/// `"8.3.21 validateRequirementDefinitionSubjectParameterPosition"` → `8.3.21`.
/// A clause number is a run of digits and dots; anything after the first
/// whitespace (a method name, prose) is dropped. Returns `None` when the token
/// is not a clean dotted number (e.g. the `8.3.x` placeholder).
pub fn clause_number_token(rest: &str) -> Option<&str> {
    let tok = rest.split_whitespace().next()?;
    if !tok.is_empty() && tok.chars().all(|c| c.is_ascii_digit() || c == '.') {
        Some(tok)
    } else {
        None
    }
}
