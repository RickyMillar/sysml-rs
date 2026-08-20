//! Prepare-rename and apply-rename — resolve cursor → element, then either
//! report whether a rename is allowed (and the placeholder + range) or emit a
//! workspace-wide set of text edits.
//!
//! Replaces the LSP-side `rename::{prepare_rename, rename}` bodies. The LSP
//! shell shrinks to URI parsing + LSP `WorkspaceEdit` / `PrepareRenameResponse`
//! construction.
//!
//! Position columns follow the LSP convention: UTF-16 code units, 0-indexed
//! line + character. Service has no `tower-lsp` dependency; the fields are
//! plain `u32` so any transport can shape them as needed.

use std::collections::BTreeMap;
use std::sync::Mutex;

use sysml_id::ElementId;
use sysml_ide_db::{AnalysisHost, Cancelled};

use crate::error::ServiceError;
use crate::position::{offset_to_line_col, position_to_offset};
use crate::text_edit::TextEdit;

/// Response shape for `sysml.rename`.
///
/// Either `prepare` or `apply` is set, never both. The mode is selected by
/// whether the caller passed `new_name` to the command.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenameResponse {
    /// Set when caller did not provide `new_name` — the cursor is over a
    /// renameable element and this is the placeholder + range to seed the
    /// editor's rename popup.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prepare: Option<RenamePrepare>,
    /// Set when caller provided `new_name` — every text edit needed to
    /// rename the element across the workspace, grouped by URI.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apply: Option<RenameWorkspaceEdit>,
}

/// Prepare-rename payload: identifier text + its range.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenamePrepare {
    pub placeholder: String,
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
}

/// Workspace-wide rename edit set, sorted by URI for stable serialization.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenameWorkspaceEdit {
    pub changes: Vec<RenameFileEdits>,
}

/// All edits for a single file, sorted by `(line_start, col_start)` and
/// deduplicated.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RenameFileEdits {
    pub uri: String,
    pub edits: Vec<TextEdit>,
}

/// Compute prepare-rename info or apply-rename edits at a cursor position.
///
/// When `new_name` is `None`, returns `RenameResponse { prepare: Some(_), apply: None }`
/// if the cursor is on a renameable element, or both fields `None` when it is
/// not.
///
/// When `new_name` is `Some(_)`, validates the identifier, then returns
/// `RenameResponse { prepare: None, apply: Some(_) }`. The error type is
/// `ServiceError::InvalidInput` for an invalid identifier.
pub fn compute_rename(
    host: &Mutex<AnalysisHost>,
    uri: &str,
    line: u32,
    col: u32,
    new_name: Option<&str>,
) -> Result<RenameResponse, ServiceError> {
    if let Some(name) = new_name {
        if let Err(msg) = validate_identifier(name) {
            return Err(ServiceError::InvalidInput(msg));
        }
    }

    // Phase 1: locate the element + collect in-file refs against an Analysis
    // snapshot. We drop the snapshot before the cross-file walk so concurrent
    // edits aren't blocked.
    let phase1 = {
        // Resolve (SourceFile, project, snapshot) under a SMALL guard, then
        // drop the guard before running any salsa query — a query under the
        // guard serializes every other host user (precedent:
        // `compute_full_diagnostics`).
        let (analysis, sf, file_id, project_id) = {
            let guard = host.lock().unwrap();
            let Some(file_id) = guard.file_id(uri) else {
                return Ok(empty_response());
            };
            let Some(sf) = guard.source_file(file_id) else {
                return Ok(empty_response());
            };
            let project_id = guard.files().project_id(file_id);
            (guard.analysis(), sf, file_id, project_id)
        };

        let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            let content = analysis.file_text(sf).to_owned();
            let offset = position_to_offset(line, col, &content);
            let position_map = analysis.position_map(sf);
            let Some(id) = position_map.element_id_at(offset) else {
                return None;
            };
            let parsed = analysis.parse_file(sf);
            let element_name = parsed.graph().get_element(&id).and_then(|e| e.name.clone());
            let in_file_refs = position_map.find_references(&id);
            let identifier_span = parsed.graph().get_element(&id).and_then(|el| {
                el.spans
                    .iter()
                    .find(|s| s.file == uri && s.start <= offset && offset < s.end)
                    .map(|s| (s.start, s.end))
            });
            Some(Phase1 {
                id,
                element_name,
                in_file_refs,
                content,
                identifier_span,
                offset,
                cursor_file_id: file_id,
                project_id,
            })
        }));

        drop(analysis);

        match result {
            Ok(Some(p)) => p,
            _ => return Ok(empty_response()),
        }
    };

    // Prepare-mode: report the identifier span + name as a placeholder.
    if new_name.is_none() {
        let Some(name) = phase1.element_name else {
            return Ok(empty_response());
        };
        let (start, end) = match phase1.identifier_span {
            Some(span) => span,
            None => (phase1.offset, phase1.offset.saturating_add(name.len())),
        };
        let (line_start, col_start) = offset_to_line_col(start, &phase1.content);
        let (line_end, col_end) = offset_to_line_col(end, &phase1.content);
        return Ok(RenameResponse {
            prepare: Some(RenamePrepare {
                placeholder: name,
                line_start,
                col_start,
                line_end,
                col_end,
            }),
            apply: None,
        });
    }

    // Apply-mode: aggregate edits per URI.
    let new_text = new_name.unwrap_or_default().to_owned();
    let mut by_uri: BTreeMap<String, Vec<TextEdit>> = BTreeMap::new();

    // In-file refs from position_map.
    for (start, end, _is_def) in &phase1.in_file_refs {
        let (line_start, col_start) = offset_to_line_col(*start, &phase1.content);
        let (line_end, col_end) = offset_to_line_col(*end, &phase1.content);
        by_uri.entry(uri.to_owned()).or_default().push(TextEdit {
            expected_old_text: None,
            line_start,
            col_start,
            line_end,
            col_end,
            new_text: new_text.clone(),
        });
    }

    // Cross-file rename via the salsa-cached id-reverse index. Replaces the
    // legacy name-based walk that was both unsound (over-edited homonyms in
    // unrelated packages) and incomplete (missed redefinition chains and
    // relationship targets resolved cross-file). Phase 3b of
    if let Some(name) = phase1.element_name.as_deref() {
        let cross_file = collect_cross_file_edits(
            host,
            phase1.cursor_file_id,
            phase1.project_id,
            &phase1.id,
            name,
            &new_text,
        );
        for (other_uri, edits) in cross_file {
            by_uri.entry(other_uri).or_default().extend(edits);
        }
    }

    // Sort + dedup edits per file by (line_start, col_start, line_end, col_end).
    for edits in by_uri.values_mut() {
        edits.sort_by_key(|e| (e.line_start, e.col_start, e.line_end, e.col_end));
        edits.dedup_by(|a, b| {
            a.line_start == b.line_start
                && a.col_start == b.col_start
                && a.line_end == b.line_end
                && a.col_end == b.col_end
        });
    }

    let changes: Vec<RenameFileEdits> = by_uri
        .into_iter()
        .map(|(uri, edits)| RenameFileEdits { uri, edits })
        .collect();

    Ok(RenameResponse {
        prepare: None,
        apply: Some(RenameWorkspaceEdit { changes }),
    })
}

struct Phase1 {
    id: ElementId,
    element_name: Option<String>,
    in_file_refs: Vec<(usize, usize, bool)>,
    content: String,
    identifier_span: Option<(usize, usize)>,
    offset: usize,
    cursor_file_id: sysml_ide_db::source::FileId,
    project_id: Option<sysml_project::ProjectHandle>,
}

fn empty_response() -> RenameResponse {
    RenameResponse {
        prepare: None,
        apply: None,
    }
}

fn collect_cross_file_edits(
    host: &Mutex<AnalysisHost>,
    cursor_file_id: sysml_ide_db::source::FileId,
    project_id: Option<sysml_project::ProjectHandle>,
    self_id: &ElementId,
    name: &str,
    new_text: &str,
) -> Vec<(String, Vec<TextEdit>)> {
    // Snapshot under a SMALL guard, then run the (potentially expensive)
    // workspace ref-index query lock-free — a query under the guard
    // serializes every other host user (precedent:
    // `compute_full_diagnostics`).
    let analysis = {
        let guard = host.lock().unwrap();
        guard.analysis()
    };

    // Without a workspace project, there's no cross-file scope to rename.
    let Some(ref_idx) = analysis.ref_index_best(project_id) else {
        return Vec::new();
    };

    let sites = ref_idx.get(self_id);
    if sites.is_empty() {
        return Vec::new();
    }

    // Resolving each site's uri→SourceFile needs the host (file-id compare
    // is canonicalization-safe, unlike a `site.file == uri` string compare).
    // Drop the snapshot FIRST — re-locking the host with an `Analysis`
    // alive on this thread is the 2026-07-17 wedge (lock-order invariant on
    // `SysmlService::host_analysis`) — then resolve under one small guard
    // and fetch contents lock-free on a fresh snapshot. Library-provenance
    // sites are skipped: rename-across-project edits user code only (Task
    // #194 sweep). Sites in the cursor's own file are skipped — in-file
    // edits come from position_map.
    drop(analysis);
    let (analysis, resolved_sites) = {
        let guard = host.lock().unwrap();
        let resolved: Vec<(&sysml_ide_db::ref_index::RefSite, sysml_ide_db::SourceFile)> = sites
            .iter()
            .filter(|site| {
                !matches!(
                    site.provenance,
                    sysml_ide_db::ref_index::Provenance::Library
                )
            })
            .filter_map(|site| {
                let site_file_id = guard.file_id(&site.file)?;
                if site_file_id == cursor_file_id {
                    return None;
                }
                Some((site, guard.source_file(site_file_id)?))
            })
            .collect();
        (guard.analysis(), resolved)
    };

    // (uri, [(start, end)]) keyed by SITE.file so we group edits per file.
    let mut by_uri: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    let mut content_by_uri: BTreeMap<String, String> = BTreeMap::new();

    for (site, other_sf) in resolved_sites {
        // Ensure content cached for this file.
        if !content_by_uri.contains_key(&site.file) {
            let Ok(text) = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
                analysis.file_text(other_sf).to_owned()
            })) else {
                continue;
            };
            content_by_uri.insert(site.file.clone(), text);
        }
        let content = match content_by_uri.get(&site.file) {
            Some(c) => c,
            None => continue,
        };

        // Only emit an edit when the site's span text matches the identifier
        // being renamed. RelationshipTarget sites carry the relationship's
        // span — which IS the type-ref or feature-chain text in source for
        // FeatureTyping / Subsetting / Redefinition / Specialization. But a
        // relationship span can cover a qualified path like `Pkg::Name` or a
        // chained ref `engine.power.value`; rewriting that whole range with
        // the new identifier would corrupt the source. Guard with a literal-
        // text match so we only edit when the span IS the identifier.
        if let Some(span_text) = safe_slice(content, site.start, site.end) {
            if span_text == name {
                by_uri
                    .entry(site.file.clone())
                    .or_default()
                    .push((site.start, site.end));
            }
        }
    }

    drop(analysis);

    let mut result: Vec<(String, Vec<TextEdit>)> = Vec::new();
    for (uri, ranges) in by_uri {
        let Some(content) = content_by_uri.get(&uri) else {
            continue;
        };
        let edits: Vec<TextEdit> = ranges
            .into_iter()
            .map(|(start, end)| {
                let (line_start, col_start) = offset_to_line_col(start, content);
                let (line_end, col_end) = offset_to_line_col(end, content);
                TextEdit {
                    expected_old_text: None,
                    line_start,
                    col_start,
                    line_end,
                    col_end,
                    new_text: new_text.to_owned(),
                }
            })
            .collect();
        result.push((uri, edits));
    }
    result
}

fn safe_slice(source: &str, start: usize, end: usize) -> Option<&str> {
    if start <= end
        && end <= source.len()
        && source.is_char_boundary(start)
        && source.is_char_boundary(end)
    {
        Some(&source[start..end])
    } else {
        None
    }
}

/// Validate a SysML identifier for rename.
///
/// Mirrors `sysml_lsp_server::utils::validate_identifier` (the LSP layer used
/// to own this; the service now owns it so non-LSP transports validate too).
pub fn validate_identifier(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Name cannot be empty".to_owned());
    }

    let mut chars = name.chars();
    let first = chars.next().unwrap_or(' ');
    if !first.is_alphabetic() && first != '_' {
        return Err(format!(
            "Invalid identifier '{}': must start with a letter or underscore",
            name
        ));
    }
    for ch in chars {
        if !ch.is_alphanumeric() && ch != '_' {
            return Err(format!(
                "Invalid identifier '{}': contains invalid character '{}'",
                name, ch
            ));
        }
    }

    if SYSML_RESERVED_KEYWORDS.contains(&name) {
        return Err(format!("'{}' is a reserved SysML keyword", name));
    }

    Ok(())
}

/// SysML reserved keywords that cannot be used as identifiers.
const SYSML_RESERVED_KEYWORDS: &[&str] = &[
    "package",
    "part",
    "attribute",
    "action",
    "state",
    "port",
    "connection",
    "interface",
    "item",
    "requirement",
    "constraint",
    "allocation",
    "import",
    "alias",
    "ref",
    "in",
    "out",
    "inout",
    "private",
    "protected",
    "public",
    "abstract",
    "readonly",
    "derived",
    "end",
    "redefines",
    "subsets",
    "specializes",
    "entry",
    "exit",
    "do",
    "transition",
    "accept",
    "send",
    "if",
    "then",
    "else",
    "while",
    "for",
    "return",
    "def",
    "about",
    "all",
    "and",
    "as",
    "assert",
    "assign",
    "assume",
    "bind",
    "by",
    "case",
    "comment",
    "concern",
    "decide",
    "dependency",
    "doc",
    "enum",
    "exhibit",
    "expose",
    "filter",
    "first",
    "flow",
    "fork",
    "frame",
    "from",
    "hastype",
    "include",
    "individual",
    "istype",
    "join",
    "library",
    "merge",
    "message",
    "metadata",
    "nonunique",
    "not",
    "objective",
    "occurrence",
    "of",
    "or",
    "ordered",
    "parallel",
    "perform",
    "rendering",
    "rep",
    "require",
    "satisfy",
    "snapshot",
    "stakeholder",
    "subject",
    "succession",
    "that",
    "timeslice",
    "to",
    "use",
    "variant",
    "verification",
    "verify",
    "view",
    "viewpoint",
    "xor",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_identifier_accepts_simple_names() {
        assert!(validate_identifier("foo").is_ok());
        assert!(validate_identifier("Bar").is_ok());
        assert!(validate_identifier("_under").is_ok());
        assert!(validate_identifier("a_b_c1").is_ok());
    }

    #[test]
    fn validate_identifier_rejects_empty() {
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn validate_identifier_rejects_leading_digit() {
        assert!(validate_identifier("1foo").is_err());
    }

    #[test]
    fn validate_identifier_rejects_special_chars() {
        assert!(validate_identifier("foo-bar").is_err());
        assert!(validate_identifier("foo bar").is_err());
        assert!(validate_identifier("foo.bar").is_err());
    }

    #[test]
    fn validate_identifier_rejects_keywords() {
        assert!(validate_identifier("part").is_err());
        assert!(validate_identifier("package").is_err());
        assert!(validate_identifier("verify").is_err());
    }
}
