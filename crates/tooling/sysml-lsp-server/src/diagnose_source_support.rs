//! Standalone (no-library) diagnostic pipeline — TEST SUPPORT ONLY (RSC-6.6).
//!
//! This is the proven `diagnose_source` pipeline, relocated out of the
//! production `diagnostics.rs` module (where it was `#[allow(dead_code)]`
//! and only ever exercised by tests) into a `#[cfg(test)]` home. It is the
//! single shared test helper for `snapshot_tests`, `diagnostic_ux_tests`,
//! and `ux_workflow_tests`.
//!
//! WHY NOT the production `sysml_service::compute_full_diagnostics`: that
//! path applies readiness gating (drops Semantic/Constraint/ImportHealth
//! tiers when no project is indexed) and resolves against whatever library
//! is loaded. These tests deliberately exercise the parser/resolver/health
//! surface WITHOUT the standard library and WITHOUT readiness gating, so a
//! distinct no-library pipeline is required — it is a separate test
//! configuration, not a redundant copy of the production path.
//!
//! Behaviour is byte-identical to the pre-RSC-6.6 `diagnose_source`.

use std::collections::HashMap;
use std::env;

use sysml_core::physics::health::physics_health_diagnostics;
use sysml_core::{elaborate::elaborate, import_health_diagnostics, ModelGraph};
use sysml_parser_incremental::{build_model_graph, TreeSitterParser};
use sysml_runtime::flows::flow_health_diagnostics;
use sysml_span::{Diagnostic as SysmlDiagnostic, Severity};

use crate::background::ResolutionTier;
use sysml_service::diagnostics::{
    detect_smart_quotes, is_likely_library_type, unresolved_name_from_message, SYNTAX_ERROR_CAP,
    SYNTAX_GATE_THRESHOLD, TOTAL_DIAGNOSTIC_CAP,
};

/// Options controlling which diagnostic phases run.
pub(crate) struct DiagnoseOptions {
    pub resolution: bool,
    pub validation: bool,
}

/// Result of the standalone diagnostic pipeline.
pub(crate) struct DiagnoseResult {
    pub diagnostics: Vec<SysmlDiagnostic>,
    pub graph: ModelGraph,
}

fn diagnostic_overlaps_ranges(diag: &SysmlDiagnostic, ranges: &[(usize, usize)]) -> bool {
    if let Some(span) = diag.span.as_ref() {
        return ranges
            .iter()
            .any(|&(start, end)| span.start < end && span.end > start);
    }
    false
}

fn fail_on_spanless_diagnostics_enabled() -> bool {
    env::var("SYSML_FAIL_ON_SPANLESS_DIAGNOSTICS")
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn maybe_fail_on_spanless_diagnostics(diags: &[SysmlDiagnostic], stage: &str, uri: &str) {
    if !fail_on_spanless_diagnostics_enabled() {
        return;
    }
    let spanless: Vec<_> = diags.iter().filter(|d| d.span.is_none()).collect();
    if spanless.is_empty() {
        return;
    }

    let preview = spanless
        .iter()
        .take(8)
        .map(|d| {
            format!(
                "code={} severity={:?} message={}",
                d.code.as_deref().unwrap_or("<none>"),
                d.severity,
                d.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    panic!(
        "spanless diagnostics detected (stage={}, uri={}, count={})\n{}",
        stage,
        uri,
        spanless.len(),
        preview
    );
}

/// Run the full diagnostic pipeline on SysML source text.
///
/// This is the pure, synchronous core of the diagnostic pipeline — no LSP
/// server, no async, no client. It runs:
///
/// 1. Tree-sitter parse → `build_model_graph()`
/// 2. Pest hybrid enrichment for the first syntax error
/// 3. Syntax error cap (128), phase gating (≥16)
/// 4. Resolution (local, no library) if enabled
/// 5. Validation (structural, S001, semantic) if enabled
/// 6. Scope-aware suppression, dedup, priority sorting, total cap (50)
pub(crate) fn diagnose_source(source: &str, uri: &str, opts: &DiagnoseOptions) -> DiagnoseResult {
    let ts_parser = TreeSitterParser::new();

    let mut all_diagnostics: Vec<SysmlDiagnostic> = Vec::new();

    // Phase 0: Detect smart/curly quotes (common copy-paste issue)
    all_diagnostics.extend(detect_smart_quotes(source, uri));

    // Phase 1: Parse with tree-sitter (canonical parser per ADR-014).
    //
    // The TS ast_builder handles both ERROR nodes (`Syntax error near
    // `X`` / `Unexpected keyword `X``) and MISSING nodes (`expected `;`
    // after `X``), so the diagnostic stream from `build_model_graph` is
    // the strict-syntax oracle. The Pest enricher was retired in TS-3.3
    // once `diagnostic_ux_tests.rs` confirmed parity.
    let mut ts_result = match ts_parser.parse_tree(source) {
        Some(tree) => build_model_graph(&tree, source, uri),
        None => {
            let mut diag = SysmlDiagnostic::error("Tree-sitter parsing failed");
            diag.span = Some(sysml_span::Span::new(uri, 0, 0));
            all_diagnostics.push(diag);
            sysml_parser_incremental::ModelGraphResult::new()
        }
    };

    all_diagnostics.append(&mut ts_result.diagnostics);

    // Cap syntax errors at 128
    let syntax_error_count = all_diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    if syntax_error_count > SYNTAX_ERROR_CAP {
        let mut error_count = 0usize;
        all_diagnostics.retain(|d| {
            if d.severity == Severity::Error {
                error_count += 1;
                error_count <= SYNTAX_ERROR_CAP
            } else {
                true
            }
        });
        let mut cap_diag = SysmlDiagnostic::info(format!(
            "Showing first {} of {} syntax errors",
            SYNTAX_ERROR_CAP, syntax_error_count
        ));
        cap_diag.span = all_diagnostics
            .iter()
            .find(|d| d.severity == Severity::Error)
            .and_then(|d| d.span.clone());
        all_diagnostics.push(cap_diag);
    }

    let mut graph = ts_result.graph;

    // Phase gating
    let syntax_error_count_for_gate = all_diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let skip_later_phases = syntax_error_count_for_gate >= SYNTAX_GATE_THRESHOLD;

    // Syntax error spans for scope-aware suppression
    let syntax_error_spans: Vec<(usize, usize)> = all_diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .filter_map(|d| d.span.as_ref().map(|s| (s.start, s.end)))
        .collect();

    // Phase 2: Resolution (local, no library)
    let mut resolution_tier = ResolutionTier::T1Syntax;

    if opts.resolution && !skip_later_phases {
        let res = sysml_core::resolution::resolve_references(&mut graph);
        for diag in res.diagnostics {
            let is_library_type = unresolved_name_from_message(&diag.message)
                .map(|name| is_likely_library_type(&name))
                .unwrap_or(false);
            // Suppress resolution errors (E2xx) whose span overlaps a
            // syntax-error region: name resolution over un-parseable text is
            // unreliable, so an E200 there is cascade noise stacked on the real
            // syntax error (mirrors `sysml_service::diagnostics::post_process`).
            if !is_library_type && !diagnostic_overlaps_ranges(&diag, &syntax_error_spans) {
                let mut tagged_diag = diag;
                tagged_diag
                    .notes
                    .push("resolution performed without standard library".to_owned());
                all_diagnostics.push(tagged_diag);
            }
        }
        resolution_tier = ResolutionTier::T2Local;
    }

    // Phase 2.5: Elaborate parsed ownership structure into execution-ready links.
    if !skip_later_phases && opts.validation {
        elaborate(&mut graph);
    }

    // E200 spans for semantic suppression
    let unresolved_spans: Vec<(usize, usize)> = all_diagnostics
        .iter()
        .filter(|d| d.code.as_deref() == Some("E200"))
        .filter_map(|d| d.span.as_ref().map(|s| (s.start, s.end)))
        .collect();

    // Phase 3: Validation
    if !skip_later_phases
        && opts.validation
        && resolution_tier == ResolutionTier::T2Local
    {
        let overlaps_syntax_error = |diag: &SysmlDiagnostic| -> bool {
            diagnostic_overlaps_ranges(diag, &syntax_error_spans)
        };

        let overlaps_unresolved = |diag: &SysmlDiagnostic| -> bool {
            let is_semantic = diag
                .code
                .as_ref()
                .map(|c| c.starts_with('S'))
                .unwrap_or(false);
            if !is_semantic {
                return false;
            }
            diagnostic_overlaps_ranges(diag, &unresolved_spans)
        };

        // Structural validation
        let structure_errors = graph.validate_structure();
        for error in structure_errors {
            let diag = error.to_diagnostic_with_graph(&graph);
            if !overlaps_syntax_error(&diag) {
                all_diagnostics.push(diag);
            }
        }

        let relationship_errors = graph.validate_relationship_types();
        for error in relationship_errors {
            let diag = error.to_diagnostic_with_graph(&graph);
            if !overlaps_syntax_error(&diag) {
                all_diagnostics.push(diag);
            }
        }

        // Property validation (V001-V005)
        // validate_graph_properties() filters out V001 for resolution-populated
        // and derived properties via is_post_parse_validatable().
        let prop_result = sysml_core::validate_graph_properties(&graph);
        for error in prop_result.errors {
            let diag: SysmlDiagnostic = error.into();
            if diag.span.is_none() {
                continue; // Skip spanless property warnings — they appear at (0,0)
            }
            if overlaps_syntax_error(&diag) {
                continue;
            }
            let mut warning_diag = diag;
            warning_diag.severity = Severity::Warning;
            all_diagnostics.push(warning_diag);
        }

        // Semantic validation — S001 duplicate names + S010-S140
        // (run unconditionally in standalone mode — no T3-only gating
        // since we don't have library tiers here)
        let semantic_errors = sysml_core::validate_semantic(&graph);
        for error in semantic_errors {
            let diag = error.to_diagnostic_with_graph(&graph);
            if !overlaps_syntax_error(&diag) && !overlaps_unresolved(&diag) {
                all_diagnostics.push(diag);
            }
        }

        // Simple graph-level health diagnostics — the canonical shared set
        // (state machine, action, port, verification, constraint, requirement,
        // quantity mismatch, quantity expression). Iterates the single source of
        // truth in `sysml_runtime::health` so this test pipeline cannot drift
        // from the production pipeline. Flow / import / physics are NOT in this
        // set (they need different graph routing) and stay wired separately below.
        for health_fn in sysml_runtime::health::GRAPH_HEALTH_FNS {
            for diag in health_fn(&graph) {
                if !overlaps_syntax_error(&diag) {
                    all_diagnostics.push(diag);
                }
            }
        }

        // Flow health diagnostics (missing endpoints / self-loops / multicast).
        for diag in flow_health_diagnostics(&graph) {
            if !overlaps_syntax_error(&diag) {
                all_diagnostics.push(diag);
            }
        }

        // Import health diagnostics (unknown namespaces / circular chains / duplicates).
        for diag in import_health_diagnostics(&graph) {
            if !overlaps_syntax_error(&diag) {
                all_diagnostics.push(diag);
            }
        }

        // Physics health diagnostics (domain mismatches, direction conflicts, conservation).
        for diag in physics_health_diagnostics(&graph) {
            if !overlaps_syntax_error(&diag) {
                all_diagnostics.push(diag);
            }
        }
    }

    // Phase 4: Live constraint monitoring
    if !skip_later_phases
        && resolution_tier == ResolutionTier::T2Local
    {
        for violation in sysml_service::constraint_monitor::check_constraints(&graph, uri) {
            let mut diag = if violation.is_assert {
                SysmlDiagnostic::warning(&violation.message)
            } else {
                SysmlDiagnostic::info(&violation.message)
            };
            diag.span = violation.span;
            diag.code = Some(if violation.is_assert { "C001" } else { "C002" }.to_owned());
            all_diagnostics.push(diag);
        }
    }

    // Deduplication
    {
        let mut seen: HashMap<(usize, usize, String), usize> = HashMap::new();
        let mut to_remove = Vec::new();
        for (idx, diag) in all_diagnostics.iter().enumerate() {
            if let (Some(span), Some(code)) = (diag.span.as_ref(), diag.code.as_ref()) {
                let key = (span.start, span.end, code.clone());
                if let Some(&prev_idx) = seen.get(&key) {
                    if diag.message.len() > all_diagnostics[prev_idx].message.len() {
                        to_remove.push(prev_idx);
                        seen.insert(key, idx);
                    } else {
                        to_remove.push(idx);
                    }
                } else {
                    seen.insert(key, idx);
                }
            }
        }
        to_remove.sort_unstable();
        to_remove.dedup();
        for idx in to_remove.into_iter().rev() {
            all_diagnostics.remove(idx);
        }
    }

    // Priority sorting
    all_diagnostics.sort_by(|a, b| {
        let sev = b.severity.cmp(&a.severity);
        if sev != std::cmp::Ordering::Equal {
            return sev;
        }
        let phase_rank = |d: &SysmlDiagnostic| -> u8 {
            match d.code.as_deref() {
                None => 0,
                Some(c) if c.starts_with("E2") => 1,
                Some(c) if c.starts_with("E0") => 2,
                Some(c) if c.starts_with("SM") => 3,
                Some(c) if c.starts_with("AX") => 3,
                Some(c) if c.starts_with("FL") => 3,
                Some(c) if c.starts_with("VC") => 3,
                Some(c) if c.starts_with("IM") => 3,
                Some(c) if c.starts_with('S') => 4,
                Some(c) if c.starts_with('V') => 5,
                _ => 6,
            }
        };
        let phase = phase_rank(a).cmp(&phase_rank(b));
        if phase != std::cmp::Ordering::Equal {
            return phase;
        }
        let pos_a = a.span.as_ref().map(|s| s.start).unwrap_or(usize::MAX);
        let pos_b = b.span.as_ref().map(|s| s.start).unwrap_or(usize::MAX);
        pos_a.cmp(&pos_b)
    });

    // Total cap
    if all_diagnostics.len() > TOTAL_DIAGNOSTIC_CAP {
        all_diagnostics.truncate(TOTAL_DIAGNOSTIC_CAP);
    }
    maybe_fail_on_spanless_diagnostics(&all_diagnostics, "diagnose_source::final", uri);

    DiagnoseResult {
        diagnostics: all_diagnostics,
        graph,
    }
}
