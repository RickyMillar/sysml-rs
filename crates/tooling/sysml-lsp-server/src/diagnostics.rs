//! Diagnostic generation and conversion.
//!
//! Handles the parse→resolve→validate pipeline and converts
//! sysml-span diagnostics into LSP protocol diagnostics.
//!
//! After the salsa migration, the LSP diagnostic pipeline runs through
//! `salsa_diagnostics()` in `lib.rs`. This module retains:
//! - Helper functions used by both salsa and standalone pipelines
//! - The standalone `diagnose_source()` (relocated to `diagnose_source_support.rs`) used by snapshot/UX tests
//! - The `get_library()` accessor

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

use std::sync::Arc;

#[cfg(test)]
use tower_lsp::lsp_types::*;
#[cfg(not(test))]
use tower_lsp::lsp_types::{DiagnosticSeverity, Range};

use sysml_core::ModelGraph;
use sysml_ide_db::Cancelled;
use sysml_id::ElementId;
#[cfg(test)]
use crate::lsp_types::{DiagnosticSeverity as SysmlSeverity, LspDiagnostic};
#[cfg(test)]
use sysml_span::Diagnostic as SysmlDiagnostic;

#[cfg(test)]
use crate::utils::to_lsp_range;
use crate::utils::{offset_to_position, parse_uri};
use crate::SysmlLanguageServer;

/// Convert a sysml-span Diagnostic to an LSP Diagnostic.
#[cfg(test)]
pub(crate) fn to_lsp_diagnostic(diag: &SysmlDiagnostic, source: &str) -> Diagnostic {
    let lsp_diag = LspDiagnostic::from_sysml(diag, source);
    let range = to_lsp_range(lsp_diag.range);
    let severity = lsp_diag.severity.map(|s| match s {
        SysmlSeverity::Error => DiagnosticSeverity::ERROR,
        SysmlSeverity::Warning => DiagnosticSeverity::WARNING,
        SysmlSeverity::Information => DiagnosticSeverity::INFORMATION,
        SysmlSeverity::Hint => DiagnosticSeverity::HINT,
    });
    let code = lsp_diag.code.map(NumberOrString::String);

    let related_information: Vec<DiagnosticRelatedInformation> = lsp_diag
        .related_information
        .into_iter()
        .filter_map(|info| {
            let uri = parse_uri(&info.location.uri)?;
            Some(DiagnosticRelatedInformation {
                location: Location {
                    uri,
                    range: to_lsp_range(info.location.range),
                },
                message: info.message,
            })
        })
        .collect();

    // Map sysml-span DiagnosticTag to LSP DiagnosticTag
    let tags: Vec<DiagnosticTag> = diag
        .tags
        .iter()
        .map(|t| match t {
            sysml_span::DiagnosticTag::Unnecessary => DiagnosticTag::UNNECESSARY,
            sysml_span::DiagnosticTag::Deprecated => DiagnosticTag::DEPRECATED,
        })
        .collect();

    Diagnostic {
        range,
        severity,
        code,
        source: lsp_diag.source,
        message: lsp_diag.message,
        related_information: if related_information.is_empty() {
            None
        } else {
            Some(related_information)
        },
        tags: if tags.is_empty() { None } else { Some(tags) },
        ..Default::default()
    }
}

// RSC-6.6: `is_likely_library_type`, `unresolved_name_from_message`, and
// `is_foreign_file_diagnostic` (plus the LIBRARY_* tables and the diagnostic
// caps below) used to be duplicated here. They now live in their one home,
// `sysml_service::diagnostics`, and the test harnesses import them from there.

#[cfg(test)]
fn suppress_spanless_diagnostics_for_publish(
    diags: &mut Vec<SysmlDiagnostic>,
    stage: &str,
    uri: &str,
) -> usize {
    let mut removed_count = 0usize;
    let mut preview = Vec::new();
    diags.retain(|diag| {
        if diag.span.is_some() {
            return true;
        }
        removed_count += 1;
        if preview.len() < 8 {
            preview.push(format!(
                "code={} severity={:?} message={}",
                diag.code.as_deref().unwrap_or("<none>"),
                diag.severity,
                diag.message
            ));
        }
        false
    });

    if removed_count > 0 {
        tracing::warn!(
            stage = %stage,
            uri = %uri,
            removed = removed_count,
            preview = %preview.join(" | "),
            "suppressing spanless diagnostics from LSP publish path"
        );
    }

    removed_count
}

// ── LSP integration helpers ───

impl SysmlLanguageServer {
    /// Get the loaded library, or None if not loaded.
    ///
    /// P-RA4: reads directly from `AnalysisHost::library_graph()` (the
    /// canonical owner) and derives the element-id set on demand.
    /// Replaces the retired `LibraryState::Loaded` pattern.
    pub(crate) async fn get_library(
        &self,
    ) -> Option<(
        Arc<ModelGraph>,
        Arc<sysml_core::resolution::FxHashSet<ElementId>>,
    )> {
        let host = self.analysis_host.lock().unwrap();
        let lib = host.library_graph()?;
        let data = lib.data(host.db());
        let graph = Arc::new(data.graph().clone());
        let element_ids = Arc::new(data.element_ids().clone());
        Some((graph, element_ids))
    }

    /// Compute diagnostics using the salsa incremental query chain.
    ///
    /// LSP-residual responsibilities (LSP-39 / LSP-44 / PROTOCOL-GLUE):
    /// - cancellation correlation via `Cancelled::catch`
    /// - sysml-span → `tower_lsp::lsp_types::Diagnostic` shape conversion
    ///
    /// All pipeline computation (parse → resolve → validate → health → constraints,
    /// capping, gating, library-type suppression, scope-overlap suppression,
    /// dedupe, sort, total cap) lives on `sysml_service::diagnostics::compute_full_diagnostics`,
    /// reached via the typed `service.diagnostics(uri)` entry.
    pub(crate) async fn salsa_diagnostics_with_status(
        &self,
        uri: &str,
    ) -> (Vec<tower_lsp::lsp_types::Diagnostic>, bool) {
        let Some((source_file, analysis)) = self.salsa_file_context(uri).await else {
            return (vec![], false);
        };
        let uri_owned = uri.to_owned();
        let service = self.service.clone();

        // Extract the file content under cancellation protection, then DROP
        // the `Analysis` snapshot before calling `service.diagnostics`, which
        // re-locks the host. Holding a salsa snapshot while blocking on the
        // host mutex deadlocks against a concurrent host mutation: salsa makes
        // the mutation wait for every outstanding snapshot to drop, but this
        // one cannot drop while we are parked on the mutex the mutation holds.
        // `file_text` is a cheap salsa query (not a lock), so cancellation can
        // still unwind it if an edit arrives mid-extraction.
        let content = match Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            analysis.file_text(source_file).to_owned()
        })) {
            Ok(content) => content,
            Err(_cancelled) => {
                tracing::debug!(uri, "salsa diagnostics cancelled (new edit arrived)");
                return (vec![], true);
            }
        };
        drop(analysis);

        let result = Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            let diag_cycle_start = std::time::Instant::now();
            let span_diagnostics = service.diagnostics(&uri_owned).unwrap_or_default();
            let lsp_diagnostics: Vec<tower_lsp::lsp_types::Diagnostic> = span_diagnostics
                .iter()
                .filter_map(|d| Self::span_diagnostic_to_lsp_static(d, &uri_owned, &content))
                .collect();
            let total_ms = diag_cycle_start.elapsed().as_millis();
            if total_ms > 50 {
                tracing::info!(
                    uri = %uri_owned,
                    diag_count = lsp_diagnostics.len(),
                    total_ms,
                    "diagnostic cycle timing"
                );
            }
            lsp_diagnostics
        }));

        match result {
            Ok(diags) => (diags, false),
            Err(_cancelled) => {
                tracing::debug!(uri, "salsa diagnostics cancelled (new edit arrived)");
                (vec![], true)
            }
        }
    }


    #[cfg(test)]
    pub(crate) async fn salsa_diagnostics(
        &self,
        uri: &str,
    ) -> Vec<tower_lsp::lsp_types::Diagnostic> {
        let (diagnostics, _cancelled) = self.salsa_diagnostics_with_status(uri).await;
        diagnostics
    }

    /// Convert a sysml_span::Diagnostic to an LSP Diagnostic (static version).
    ///
    /// This version doesn't take `&self` so it can be used inside `Cancelled::catch`.
    /// Accepts the file content directly from the salsa query result.
    fn span_diagnostic_to_lsp_static(
        diag: &sysml_span::Diagnostic,
        uri: &str,
        content: &str,
    ) -> Option<tower_lsp::lsp_types::Diagnostic> {
        let span = diag.span.as_ref()?;
        let same_file = if span.file.is_empty() || span.file == uri {
            true
        } else {
            match (parse_uri(&span.file), parse_uri(uri)) {
                (Some(span_uri), Some(doc_uri)) => span_uri == doc_uri,
                _ => false,
            }
        };
        if !same_file {
            return None;
        }

        let start = offset_to_position(span.start, content);
        let end = offset_to_position(span.end, content);

        let severity = match diag.severity {
            sysml_span::Severity::Error => DiagnosticSeverity::ERROR,
            sysml_span::Severity::Warning => DiagnosticSeverity::WARNING,
            sysml_span::Severity::Info => DiagnosticSeverity::INFORMATION,
        };

        let tags: Vec<tower_lsp::lsp_types::DiagnosticTag> = diag
            .tags
            .iter()
            .map(|tag| match tag {
                sysml_span::DiagnosticTag::Unnecessary => {
                    tower_lsp::lsp_types::DiagnosticTag::UNNECESSARY
                }
                sysml_span::DiagnosticTag::Deprecated => {
                    tower_lsp::lsp_types::DiagnosticTag::DEPRECATED
                }
            })
            .collect();

        let parsed_uri = parse_uri(uri);
        let related_spans = parsed_uri
            .clone()
            .map(|doc_uri| {
                diag.related
                    .iter()
                    .filter_map(|related| {
                        let same_file = if related.span.file.is_empty() || related.span.file == uri
                        {
                            true
                        } else {
                            match (parse_uri(&related.span.file), parse_uri(uri)) {
                                (Some(span_uri), Some(doc_uri_for_cmp)) => {
                                    span_uri == doc_uri_for_cmp
                                }
                                _ => false,
                            }
                        };
                        if !same_file {
                            return None;
                        }

                        let related_start = offset_to_position(related.span.start, content);
                        let related_end = offset_to_position(related.span.end, content);
                        Some(tower_lsp::lsp_types::DiagnosticRelatedInformation {
                            location: tower_lsp::lsp_types::Location {
                                uri: doc_uri.clone(),
                                range: Range {
                                    start: related_start,
                                    end: related_end,
                                },
                            },
                            message: related.message.clone(),
                        })
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        // Avoid stacking all notes at one location when richer related spans
        // are available.
        let notes_as_related = parsed_uri
            
            .map(|doc_uri| {
                if related_spans.is_empty() {
                    diag.notes
                        .iter()
                        .map(|note| tower_lsp::lsp_types::DiagnosticRelatedInformation {
                            location: tower_lsp::lsp_types::Location {
                                uri: doc_uri.clone(),
                                range: Range { start, end },
                            },
                            message: note.clone(),
                        })
                        .collect::<Vec<_>>()
                } else {
                    Vec::new()
                }
            })
            .unwrap_or_default();
        let mut all_related = related_spans;
        all_related.extend(notes_as_related);

        Some(tower_lsp::lsp_types::Diagnostic {
            range: Range { start, end },
            severity: Some(severity),
            code: diag
                .code
                .as_ref()
                .map(|code| tower_lsp::lsp_types::NumberOrString::String(code.clone())),
            source: Some("sysml".to_owned()),
            message: diag.message.clone(),
            related_information: if all_related.is_empty() {
                None
            } else {
                Some(all_related)
            },
            tags: if tags.is_empty() { None } else { Some(tags) },
            ..Default::default()
        })
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn foreign_file_detection_accepts_equivalent_uri_forms() {
        let mut diag = SysmlDiagnostic::warning("test");
        diag.span = Some(sysml_span::Span::new("/tmp/sysml_eq.sysml", 1, 5));

        assert!(
            !sysml_service::diagnostics::is_foreign_file_diagnostic(
                &diag,
                "file:///tmp/sysml_eq.sysml"
            ),
            "equivalent path and file URI should not be treated as foreign"
        );
    }

    #[test]
    fn foreign_file_detection_rejects_other_workspace_file() {
        let mut diag = SysmlDiagnostic::warning("test");
        diag.span = Some(sysml_span::Span::new("file:///workspace/a.sysml", 1, 5));

        assert!(
            sysml_service::diagnostics::is_foreign_file_diagnostic(
                &diag,
                "file:///workspace/b.sysml"
            ),
            "different file URI should be treated as foreign"
        );
    }

    #[test]
    fn suppress_spanless_publish_diagnostics_removes_unanchored_entries() {
        let mut spanned = SysmlDiagnostic::warning("spanned");
        spanned.span = Some(sysml_span::Span::new("file:///test.sysml", 0, 3));

        let mut spanless = SysmlDiagnostic::warning("spanless");
        spanless.code = Some("TEST".to_string());

        let mut diags = vec![spanned, spanless];
        let removed = suppress_spanless_diagnostics_for_publish(
            &mut diags,
            "test::stage",
            "file:///test.sysml",
        );

        assert_eq!(removed, 1);
        assert_eq!(diags.len(), 1);
        assert!(diags[0].span.is_some());
    }

    #[test]
    fn span_diagnostic_to_lsp_static_preserves_code_and_info_severity() {
        let mut diag = SysmlDiagnostic::info("info message");
        diag.code = Some("E200".to_string());
        diag.span = Some(sysml_span::Span::new("file:///test.sysml", 0, 4));
        diag.notes
            .push("resolution performed without standard library".to_string());
        diag.tags.push(sysml_span::DiagnosticTag::Unnecessary);

        let converted =
            SysmlLanguageServer::span_diagnostic_to_lsp_static(&diag, "file:///test.sysml", "part")
                .expect("diagnostic should convert");

        assert_eq!(
            converted.code,
            Some(tower_lsp::lsp_types::NumberOrString::String(
                "E200".to_string()
            ))
        );
        assert_eq!(
            converted.severity,
            Some(tower_lsp::lsp_types::DiagnosticSeverity::INFORMATION)
        );
        assert!(converted.related_information.is_some());
        assert_eq!(converted.tags.unwrap().len(), 1);
    }

    #[test]
    fn span_diagnostic_to_lsp_static_accepts_equivalent_path_and_uri() {
        let mut diag = SysmlDiagnostic::warning("warn");
        diag.code = Some("W001".to_string());
        diag.span = Some(sysml_span::Span::new("/tmp/sysml_eq.sysml", 0, 1));

        let converted = SysmlLanguageServer::span_diagnostic_to_lsp_static(
            &diag,
            "file:///tmp/sysml_eq.sysml",
            "x",
        );
        assert!(converted.is_some(), "equivalent path/URI should convert");
    }

    #[test]
    fn span_diagnostic_to_lsp_static_prefers_related_spans_over_notes() {
        let mut diag = SysmlDiagnostic::warning("warn");
        diag.code = Some("W002".to_string());
        diag.span = Some(sysml_span::Span::new("file:///test.sysml", 0, 4));
        diag.notes.push("fallback note".to_string());
        diag.related.push(sysml_span::RelatedLocation::new(
            sysml_span::Span::new("file:///test.sysml", 5, 9),
            "related span".to_string(),
        ));

        let converted = SysmlLanguageServer::span_diagnostic_to_lsp_static(
            &diag,
            "file:///test.sysml",
            "part data",
        )
        .expect("diagnostic should convert");

        let related = converted
            .related_information
            .expect("related information should be present");
        assert_eq!(
            related.len(),
            1,
            "notes should not be duplicated when related spans exist"
        );
        assert_eq!(related[0].message, "related span");
        assert_eq!(related[0].location.range.start.character, 5);
        assert_eq!(related[0].location.range.end.character, 9);
    }
}
