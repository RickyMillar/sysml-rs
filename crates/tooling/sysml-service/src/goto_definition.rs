//! Goto-definition — resolve cursor → element → relationship-following
//! ladder → typed-usage type def → target span.
//!
//! Replaces the primary path of the LSP-side `navigation::goto_definition`.
//! The LSP shell keeps `project://` URI rewriting, the word-fallback
//! workspace lookup (used when the cursor isn't over an identifiable
//! element), and the conversion of the returned `GotoTarget` to an LSP
//! `Location`.

use std::sync::Mutex;

use sysml_core::{Element, ModelGraph};
use sysml_ide_db::{Analysis, AnalysisHost, Cancelled};

use crate::position::{offset_to_line_col, position_to_offset};

/// One goto-definition target, in line/col coordinates.
///
/// Columns follow the LSP convention (UTF-16 code units, 0-indexed).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GotoTarget {
    pub uri: String,
    pub line_start: u32,
    pub col_start: u32,
    pub line_end: u32,
    pub col_end: u32,
}

/// Follow relationship elements to their meaningful targets.
///
/// For example, if a user clicks on a FeatureTyping element, this returns
/// the referenced type definition rather than the typing relationship
/// itself. Non-relationship elements are returned unchanged.
pub fn resolve_goto_target<'a>(element: &'a Element, graph: &'a ModelGraph) -> &'a Element {
    use sysml_core::ElementKind;

    if !element.kind.is_relationship() {
        return element;
    }

    let (resolved_prop, unresolved_prop) = match element.kind {
        ElementKind::FeatureTyping => ("type", "unresolved_type"),
        ElementKind::Specialization => ("general", "unresolved_general"),
        ElementKind::Subsetting => ("subsettedFeature", "unresolved_subsettedFeature"),
        ElementKind::Redefinition => ("redefinedFeature", "unresolved_redefinedFeature"),
        ElementKind::ReferenceSubsetting => ("referencedFeature", "unresolved_referencedFeature"),
        _ => return element,
    };

    if let Some(target_id) = element.props.get(resolved_prop).and_then(|v| v.as_ref()) {
        if let Some(target) = graph.get_element(target_id) {
            return target;
        }
    }

    if let Some(name) = element.props.get(unresolved_prop).and_then(|v| v.as_str()) {
        if let Some(target) = graph.resolve_qname(name) {
            return target;
        }
        if let Some(target) = graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some(name) && e.kind.is_definition())
        {
            return target;
        }
    }

    if let Some(owner_id) = &element.owner {
        if let Some(owner) = graph.get_element(owner_id) {
            return owner;
        }
    }

    element
}

/// Find the type name for a usage element by following FeatureTyping
/// relationships. Mirrors the LSP-side `hover::find_element_type` helper.
pub fn find_element_type(element: &Element, graph: &ModelGraph) -> Option<String> {
    if let Some(typing) = element.props.get("typing").and_then(|v| v.as_str()) {
        return Some(typing.to_owned());
    }

    for child in graph.owned_members(&element.id) {
        if child.kind == sysml_core::ElementKind::FeatureTyping {
            if let Some(target_id) = child.props.get("type").and_then(|v| v.as_ref()) {
                if let Some(target) = graph.get_element(target_id) {
                    if let Some(name) = &target.name {
                        return Some(name.clone());
                    }
                }
            }
            if let Some(name) = child.props.get("unresolved_type").and_then(|v| v.as_str()) {
                return Some(name.to_owned());
            }
        }
    }

    None
}

/// Compute the goto-definition target for `(uri, line, col)`.
///
/// Mirrors the primary LSP-side path:
///   1. position → element_id via `position_map`
///   2. follow `resolve_goto_target` ladder
///   3. for typed usages, look up the type definition (in-file or
///      workspace-wide via host walk)
///   4. return the target's first span as line/col
///
/// Returns `None` when the cursor doesn't resolve to an element (the LSP
/// shell can then try its word-fallback workspace lookup).
pub fn compute_goto_definition(
    host: &Mutex<AnalysisHost>,
    uri: &str,
    line: u32,
    col: u32,
) -> Option<GotoTarget> {
    // Phase 1 — resolve every host-keyed handle under a SMALL guard, then
    // run the queries lock-free — `resolve_file_best` plus the cross-file
    // definition walk under the guard serializes every other host user
    // (precedent: `compute_full_diagnostics`). The walk gets a pre-resolved
    // `(uri, SourceFile)` list instead of the guard.
    let (target_uri, target_start, target_end, content_for_target) = {
        let (analysis, sf, project_id, other_files) = {
            let guard = host.lock().unwrap();
            let file_id = guard.file_id(uri)?;
            let sf = guard.source_file(file_id)?;
            let project_id = guard.files().project_id(file_id);
            let other_files: Vec<(String, sysml_ide_db::SourceFile)> = guard
                .files()
                .file_ids()
                .filter_map(|fid| {
                    let u = guard.files().uri(fid)?;
                    if u == uri {
                        return None;
                    }
                    Some((u.to_string(), guard.source_file(fid)?))
                })
                .collect();
            (guard.analysis(), sf, project_id, other_files)
        };

        let (raw_uri, span_start, span_end, content) =
            Cancelled::catch(std::panic::AssertUnwindSafe(|| {
                let content = analysis.file_text(sf).to_owned();
                let offset = position_to_offset(line, col, &content);
                let position_map = analysis.position_map(sf);
                let element_id = position_map.element_id_at(offset)?;
                // Use the workspace-resolved graph so cross-file refs
                // (e.g. `WaterPort` declared in ports-and-interfaces.sysml
                // and used bare in connections.sysml) have their resolved
                // target attached. `resolve_file_best` picks the strongest
                // available context (workspace + library / workspace /
                // library / single-file).
                let resolved = analysis.resolve_file_best(sf, project_id);
                let graph = resolved.graph();
                let element = graph.get_element(&element_id)?;
                let resolved_elem = resolve_goto_target(element, graph);

                // Cross-file type-definition lookup: only follow when the
                // cursor was on a FeatureTyping (a `: Type` reference). For
                // Subsetting / Redefinition / decl-name cursors the right
                // target is the in-file feature itself — host-walking by
                // type name would jump to the wrong file (e.g. `:>>
                // CoffeeMachine::waterTank` would land at the FeatureTyping
                // target `WaterTankWithPorts` instead of the redefined
                // feature). For FeatureTyping cursors, try in-file first,
                // then walk every other host file's parse graph for a
                // definition with the same name — mirrors hover's
                // `find_definition_across_host` pattern (Phase 1.1 of
                let is_typing_cursor = matches!(
                    element.kind,
                    sysml_core::ElementKind::FeatureTyping
                        | sysml_core::ElementKind::ConjugatedPortTyping
                );
                if is_typing_cursor {
                    if let Some(type_name) = find_element_type(resolved_elem, graph) {
                        if let Some(local_def) = graph.elements.values().find(|e| {
                            e.kind.is_definition()
                                && e.name.as_deref() == Some(type_name.as_str())
                        }) {
                            let span = local_def.spans.first()?;
                            return Some((span.file.clone(), span.start, span.end, content));
                        }
                        if let Some(t) = find_def_in_files(&analysis, &other_files, &type_name) {
                            return Some(t);
                        }
                    }
                }

                let span = resolved_elem.spans.first()?;
                Some((span.file.clone(), span.start, span.end, content))
            }))
            .ok()
            .flatten()?;

        // If the target is in the current file we already have its content.
        let content = if raw_uri == uri {
            Some(content)
        } else {
            // Cross-file: the target uri needs a canonicalization-safe
            // host lookup. Drop the snapshot FIRST — re-locking the host
            // with an `Analysis` alive on this thread is the 2026-07-17
            // wedge (lock-order invariant on
            // `SysmlService::host_analysis`) — then fetch content
            // lock-free on a fresh snapshot.
            drop(analysis);
            let (analysis, target_sf) = {
                let guard = host.lock().unwrap();
                let target_sf = guard
                    .file_id(&raw_uri)
                    .and_then(|fid| guard.source_file(fid));
                (guard.analysis(), target_sf)
            };
            target_sf.and_then(|target_sf| {
                Cancelled::catch(std::panic::AssertUnwindSafe(|| {
                    analysis.file_text(target_sf).to_owned()
                }))
                .ok()
            })
        };

        (raw_uri, span_start, span_end, content?)
    };

    let (line_start, col_start) = offset_to_line_col(target_start, &content_for_target);
    let (line_end, col_end) = offset_to_line_col(target_end, &content_for_target);

    Some(GotoTarget {
        uri: target_uri,
        line_start,
        col_start,
        line_end,
        col_end,
    })
}

/// Walk the pre-resolved `(uri, SourceFile)` list (already excludes the
/// cursor's own file) for a definition-kind element with the given name.
/// Returns the target's `(file_uri, span_start, span_end, content)` ready
/// for `compute_goto_definition` to convert into a `GotoTarget`.
///
/// Takes the snapshot + file list rather than a host guard so the per-file
/// parse walk runs with NO guard held — the caller enumerates the list
/// under its own small guard and drops it first (precedent:
/// `compute_full_diagnostics`). Same shape as the helper hover uses
/// internally; once Phase 2's id-reverse index lands both can share it.
fn find_def_in_files(
    analysis: &Analysis,
    files: &[(String, sysml_ide_db::SourceFile)],
    name: &str,
) -> Option<(String, usize, usize, String)> {
    for (_uri, sf) in files {
        let sf = *sf;
        let parsed = analysis.parse_file(sf);
        let graph = parsed.graph();
        let Some(def) = graph
            .elements
            .values()
            .find(|e| e.kind.is_definition() && e.name.as_deref() == Some(name))
        else {
            continue;
        };
        let span = def.spans.first()?;
        let content = analysis.file_text(sf).to_owned();
        return Some((span.file.clone(), span.start, span.end, content));
    }
    None
}

