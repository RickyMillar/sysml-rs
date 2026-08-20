//! Find-references — resolve cursor → element id, then enumerate every span
//! across the workspace whose resolved target is that id.
//!
//! module did a name-based walk of every other host file via `parse_file`,
//! which over-matched (any element with the same simple name in a distant
//! file showed up) AND under-matched (cross-file uses that only exist after
//! workspace resolution were never seen). Both failure modes were observed
//! in `contract_resolution_features_baseline`.
//!
//! Now references = `position_map.find_references` for the in-file spans
//! (already id-keyed and correct) plus a lookup into `workspace_ref_index`
//! for every cross-file site whose resolved target matches the cursor's
//! element id. Library shadows (`Provenance::Library`) are filtered out —
//! find-references is a workspace operation; library defs aren't editable
//! and don't behave like cross-file usages of the user's element.
//!
//! Position columns follow the LSP convention: UTF-16 code units, 0-indexed.

use std::collections::HashMap;
use std::sync::Mutex;

use sysml_ide_db::{
    ref_index::Provenance, ref_index::RefKind, ref_index::RefSite, AnalysisHost, Cancelled,
};

use crate::position::{offset_to_line_col, position_to_offset};

/// One reference to an element, in line/col coordinates.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RefHit {
    pub uri: String,
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
    pub is_def: bool,
}

/// Compute every reference to the element at `(uri, line, col)`.
///
/// Returns an empty vec when the cursor is not over an identifiable element
/// (whitespace, comment, or unloaded URI). When the host has no workspace
/// project loaded for `uri`, cross-file refs are skipped — the caller
/// still gets in-file refs from the per-file position map.
pub fn compute_references(
    host: &Mutex<AnalysisHost>,
    uri: &str,
    line: u32,
    col: u32,
) -> Vec<RefHit> {
    // Resolve (SourceFile, project, snapshot) under a SMALL guard, then drop
    // the guard before running any salsa query — `ref_index_best` builds the
    // workspace-merged reverse index, and running it under the guard
    // serializes every other host user (precedent:
    // `compute_full_diagnostics`).
    let (analysis, sf, file_id, project_id) = {
        let guard = host.lock().unwrap();
        let Some(file_id) = guard.file_id(uri) else {
            return Vec::new();
        };
        let Some(sf) = guard.source_file(file_id) else {
            return Vec::new();
        };
        let project_id = guard.files().project_id(file_id);
        (guard.analysis(), sf, file_id, project_id)
    };

    let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
        let in_file_content = analysis.file_text(sf).to_owned();
        let offset = position_to_offset(line, col, &in_file_content);
        let position_map = analysis.position_map(sf);
        let id = position_map.element_id_at(offset)?;
        let in_file_refs = position_map.find_references(&id);
        Some((id, in_file_refs, in_file_content))
    }));

    let Ok(Some((element_id, in_file_refs, in_file_content))) = result else {
        return Vec::new();
    };

    let mut hits: Vec<RefHit> = Vec::new();

    // In-file refs: position_map.find_references is already id-keyed, so
    // these are correct without further filtering. Convert byte offsets to
    // line/col against the file's content.
    for (start, end, is_def) in &in_file_refs {
        let (line_start, col_start) = offset_to_line_col(*start, &in_file_content);
        let (line_end, col_end) = offset_to_line_col(*end, &in_file_content);
        hits.push(RefHit {
            uri: uri.to_owned(),
            line_start,
            col_start,
            line_end,
            col_end,
            is_def: *is_def,
        });
    }

    // Cross-file refs from the workspace-merged reverse index. None means
    // the host has no `ProjectFileSet` for this file — fall back to in-file
    // refs only (matches the pre-Phase-3 single-file behaviour).
    let Some(ref_idx) = analysis.ref_index_best(project_id) else {
        return hits;
    };

    let sites = ref_idx.get(&element_id);
    if sites.is_empty() {
        return hits;
    }

    // The site loop needs canonicalization-safe uri→file lookups on the
    // host (`site.file == uri` string compare can fail when one carries a
    // `file://` scheme and the other doesn't). Drop the snapshot FIRST —
    // re-locking the host with an `Analysis` alive on this thread is the
    // 2026-07-17 wedge (lock-order invariant on
    // `SysmlService::host_analysis`) — then resolve every site's
    // `SourceFile` under one small guard and fetch contents lock-free on a
    // fresh snapshot. Sites we can't resolve a SourceFile for are silently
    // dropped (defensive — shouldn't happen for sites that came from the
    // workspace ref-index). Library-provenance sites are skipped:
    // rename-across-project and find-refs both operate on the workspace;
    // library defs aren't editable. Comparing file_ids drops duplicates of
    // in-file refs already emitted from the position map.
    drop(analysis);
    let (analysis, resolved_sites) = {
        let guard = host.lock().unwrap();
        let resolved: Vec<(&RefSite, sysml_ide_db::SourceFile)> = sites
            .iter()
            .filter(|site| !matches!(site.provenance, Provenance::Library))
            .filter_map(|site| {
                let site_file_id = guard.file_id(&site.file)?;
                if site_file_id == file_id {
                    return None;
                }
                Some((site, guard.source_file(site_file_id)?))
            })
            .collect();
        (guard.analysis(), resolved)
    };

    // Per-file content cache so we convert byte offsets to line/col without
    // re-fetching `file_text` per hit.
    let mut content_cache: HashMap<String, String> = HashMap::new();

    for (site, other_sf) in resolved_sites {
        let content = match content_cache.get(&site.file) {
            Some(c) => c,
            None => {
                let Ok(text) = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
                    analysis.file_text(other_sf).to_owned()
                })) else {
                    continue;
                };
                content_cache.entry(site.file.clone()).or_insert(text)
            }
        };

        let (line_start, col_start) = offset_to_line_col(site.start, content);
        let (line_end, col_end) = offset_to_line_col(site.end, content);
        hits.push(RefHit {
            uri: site.file.clone(),
            line_start,
            col_start,
            line_end,
            col_end,
            // Definition sites belong to the keyed element itself in
            // another file (rare — multiple decl spans). RelationshipTarget
            // sites are use sites in source. NamedUse (Phase 2.5) would
            // also be a use site.
            is_def: matches!(site.kind, RefKind::Definition),
        });
    }

    hits
}
