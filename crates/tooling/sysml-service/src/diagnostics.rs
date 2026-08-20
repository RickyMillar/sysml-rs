//! Full diagnostic pipeline (parse → resolve → validate → health) routed
//! through the salsa AnalysisHost. Replaces the LSP-side
//! `salsa_diagnostics_with_status` computation; the LSP shell retains
//! cancellation, fingerprint dedupe, and `publishDiagnostics` throttling.
//!
//! The pipeline mirrors the LSP-side phases verbatim:
//! 1. Smart-quote detection
//! 2. Parse (`Analysis::parse_file`)
//! 3. Syntax-error cap (128) + phase gate (≥16 → skip phases 2–4.5)
//! 4. Resolve (4 variants for workspace × library combinations)
//! 5. Elaborate (4 variants)
//! 6. Validate (4 variants)
//! 7. Health diagnostics (state machine, action, flow, port, verification,
//!    constraint, requirement, physics)
//! 8. Constraint monitoring is added by S2.T8 commit 2.
//! 9. Post-processing: spanless suppression, scope-aware suppression,
//!    dedupe, priority sort, total cap (50).

use std::collections::HashMap;

use sysml_core::ModelGraph;
use sysml_ide_db::{Analysis, AnalysisHost, SourceFile};
use sysml_project::ProjectHandle;
use sysml_runtime::flows::flow_health_diagnostics;
use sysml_span::{Diagnostic as SysmlDiagnostic, Severity};

/// Syntax error cap before truncation.
pub const SYNTAX_ERROR_CAP: usize = 128;
/// Phase-gate threshold: skip resolution/validation when ≥ this many syntax errors.
pub const SYNTAX_GATE_THRESHOLD: usize = 16;
/// Maximum diagnostics returned.
pub const TOTAL_DIAGNOSTIC_CAP: usize = 50;

/// Known standard library namespace prefixes — suppress unresolved
/// diagnostics for these whether or not the library has loaded yet.
const LIBRARY_NAMESPACE_PREFIXES: &[&str] = &[
    "ScalarValues",
    "Quantities",
    "MeasurementReferences",
    "ISQ",
    "SI",
    "USCustomary",
    "Base",
    "Connections",
    "Parts",
    "Items",
    "Actions",
    "States",
    "Constraints",
    "Requirements",
    "Allocations",
    "SequenceOperations",
    "IntegerFunctions",
    "RealFunctions",
    "NumericalFunctions",
    "TrigFunctions",
    "CollectionFunctions",
    "ControlFunctions",
    "TransitionPerformances",
];

const LIBRARY_SCALAR_TYPES: &[&str] = &[
    "Real",
    "Integer",
    "String",
    "Boolean",
    "Natural",
    "Positive",
    "Complex",
    "Number",
    "Anything",
    "DataValue",
];

/// Smart/curly quote characters that cause parse errors when copied from
/// web pages, PDFs, or word processors.
const SMART_QUOTES: &[(char, char, &str)] = &[
    ('\u{2018}', '\'', "left single"),
    ('\u{2019}', '\'', "right single"),
    ('\u{201C}', '"', "left double"),
    ('\u{201D}', '"', "right double"),
];

pub fn is_likely_library_type(name: &str) -> bool {
    let normalized =
        name.trim_matches(|c: char| !c.is_ascii_alphanumeric() && c != '_' && c != ':');
    if normalized.is_empty() {
        return false;
    }
    if LIBRARY_SCALAR_TYPES
        .iter()
        .any(|ty| ty.eq_ignore_ascii_case(normalized))
    {
        return true;
    }
    let normalized_lower = normalized.to_ascii_lowercase();
    if LIBRARY_NAMESPACE_PREFIXES.iter().any(|prefix| {
        let prefix_lower = prefix.to_ascii_lowercase();
        normalized_lower == prefix_lower || normalized_lower.starts_with(&(prefix_lower + "::"))
    }) {
        return true;
    }
    normalized.contains("::")
}

pub fn unresolved_name_from_message(message: &str) -> Option<String> {
    let quoted = |text: &str, quote: char| -> Option<String> {
        let start = text.find(quote)?;
        let rest = &text[start + quote.len_utf8()..];
        let end = rest.find(quote)?;
        Some(rest[..end].to_string())
    };

    quoted(message, '`')
        .or_else(|| quoted(message, '\''))
        .or_else(|| {
            let lower = message.to_ascii_lowercase();
            let start_key = "no definition ";
            let end_key = " found in scope";
            let start = lower.find(start_key)?;
            let from = start + start_key.len();
            let tail = &message[from..];
            let tail_lower = &lower[from..];
            let end = tail_lower.find(end_key)?;
            Some(tail[..end].trim().to_owned())
        })
}

pub fn is_foreign_file_diagnostic(diag: &SysmlDiagnostic, uri: &str) -> bool {
    let Some(span) = diag.span.as_ref() else {
        return true;
    };
    if span.file.is_empty() {
        return false;
    }
    if span.file == uri {
        return false;
    }
    match (parse_uri(&span.file), parse_uri(uri)) {
        (Some(a), Some(b)) => a != b,
        _ => true,
    }
}

fn parse_uri(s: &str) -> Option<String> {
    if let Some(stripped) = s.strip_prefix("file://") {
        Some(stripped.to_string())
    } else if s.starts_with('/') {
        Some(s.to_string())
    } else {
        None
    }
}

pub fn detect_smart_quotes(source: &str, uri: &str) -> Vec<SysmlDiagnostic> {
    let mut diagnostics = Vec::new();
    for (byte_offset, ch) in source.char_indices() {
        for &(smart, straight, desc) in SMART_QUOTES {
            if ch == smart {
                let end_byte = byte_offset + ch.len_utf8();
                let msg = format!(
                    "found {} quote \u{2018}{}\u{2019}; did you mean straight quote '{}'? \
                     (common when copying from web pages or documents)",
                    desc, smart, straight
                );
                let mut diag = SysmlDiagnostic::warning(msg);
                diag.span = Some(sysml_span::Span::new(uri, byte_offset, end_byte));
                diag.code = Some("smart-quote".into());
                diagnostics.push(diag);
            }
        }
    }
    diagnostics
}

/// Compute the full diagnostic pipeline for a loaded URI.
///
/// Returns sysml-span diagnostics already capped (128 syntax errors), gated
/// (≥16 syntax errors → skip phases 2–4.5), suppressed (foreign-file,
/// library-type, scope-overlap, spanless), deduplicated, sorted, and
/// truncated to `TOTAL_DIAGNOSTIC_CAP`.
///
/// The caller is responsible for any LSP-side wrapping (cancellation,
/// fingerprinting, conversion to `lsp_types::Diagnostic`, throttling,
/// `publishDiagnostics`).
pub fn compute_full_diagnostics(
    host: &std::sync::Mutex<AnalysisHost>,
    uri: &str,
) -> Vec<SysmlDiagnostic> {
    let (analysis, source_file, project_id, readiness) = {
        let guard = host.lock().unwrap();
        let Some(file_id) = guard.file_id(uri) else {
            return Vec::new();
        };
        let Some(sf) = guard.source_file(file_id) else {
            return Vec::new();
        };
        let pid = guard.files().project_id(file_id);
        // P-RA3: derive readiness while we already hold the host lock so
        // the final tier filter doesn't have to reacquire it.
        let readiness = crate::readiness::Readiness::from_host(&guard, uri);
        (guard.analysis(), sf, pid, readiness)
    };

    let mut diags = compute_pipeline(&analysis, source_file, project_id, uri);

    // P-RA3: gate every diagnostic by Readiness × DiagnosticTier as the
    // LAST step, after all the source-specific dedupe/sort/cap logic in
    // `post_process`. Anything that the file's current readiness state
    // can't honestly answer (e.g. NameResWorkspace before the workspace
    // index has populated the ProjectFileSet) is dropped here regardless
    // P-RA3 for the design.
    diags.retain(|d| readiness.answers(d.tier));
    diags
}

/// The comprehensive diagnostic pipeline (phases 0–5 + `post_process`),
/// *without* the readiness-tier gate that `compute_full_diagnostics` applies
/// as its final step.
///
/// This is the single home of the diagnostic computation that every transport
/// shares. It is `pub` so cross-crate test harnesses (notably the LSP server's
/// `salsa_ux_tests`) can verify the *real* pipeline rather than maintaining a
/// drifting reimplementation — see RSC-6.6. Production callers should prefer
/// `compute_full_diagnostics`, which adds the `Readiness × DiagnosticTier`
/// filter on top.
pub fn compute_pipeline(
    analysis: &Analysis,
    source_file: SourceFile,
    project_id: Option<ProjectHandle>,
    uri: &str,
) -> Vec<SysmlDiagnostic> {
    let content = analysis.file_text(source_file).to_owned();
    let mut all_diagnostics: Vec<SysmlDiagnostic> = Vec::new();

    // Phase 0: smart quotes
    all_diagnostics.extend(detect_smart_quotes(&content, uri));

    // Phase 1: parse
    let parse_result = analysis.parse_file(source_file);
    for diag in parse_result.diagnostics() {
        if !is_foreign_file_diagnostic(diag, uri) {
            all_diagnostics.push(diag.clone());
        }
    }

    // Phase 1 cap: cap syntax errors at SYNTAX_ERROR_CAP
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
        cap_diag.span = all_diagnostics.first().and_then(|d| d.span.clone());
        all_diagnostics.push(cap_diag);
    }

    let syntax_error_count_for_gate = all_diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let skip_later_phases = syntax_error_count_for_gate >= SYNTAX_GATE_THRESHOLD;

    if !skip_later_phases {
        // Locals reused by workspace-merged import-health, physics-health,
        // and the strict-mode predicate below. The resolve/validate/elaborate
        // trio uses the `*_best` accessor and doesn't need them.
        let library = analysis.library_graph();
        let workspace_files = project_id.and_then(|pid| analysis.project_file_set(pid));

        // Phase 2: resolution (workspace + library context picked automatically)
        let resolved = analysis.resolve_file_best(source_file, project_id);
        for diag in resolved.diagnostics() {
            if is_foreign_file_diagnostic(diag, uri) {
                continue;
            }
            let is_library_type = unresolved_name_from_message(&diag.message)
                .map(|name| is_likely_library_type(&name))
                .unwrap_or(false);
            if is_library_type {
                continue;
            }
            all_diagnostics.push(diag.clone());
        }

        // Phase 2.5: elaboration
        let elaborated = analysis.elaborate_file_best(source_file, project_id);

        // Phase 3: validation
        let validated = analysis.validate_file_best(source_file, project_id);
        for diag in validated.diagnostics() {
            if !is_foreign_file_diagnostic(diag, uri) {
                all_diagnostics.push(diag.clone());
            }
        }

        // Workspace-merged graph (salsa-memoized): used by flow health and
        // import health below — both need cross-file context to avoid
        // false positives on elements defined in sibling files.
        let workspace_merged = workspace_files.map(|pfs| {
            sysml_ide_db::elaborate_workspace_best(analysis.db(), pfs, library.clone())
        });
        let workspace_merged_graph: Option<&ModelGraph> = workspace_merged
            .as_ref()
            .map(|elab| elab.graph().as_ref());

        // Phase 4: health diagnostics
        let graph = elaborated.graph();
        let health_fns = sysml_runtime::health::GRAPH_HEALTH_FNS;
        for health_fn in health_fns {
            for diag in health_fn(graph) {
                if !is_foreign_file_diagnostic(&diag, uri) {
                    all_diagnostics.push(diag);
                }
            }
        }

        // Flow health runs against the workspace-merged graph when one is
        // available: flow endpoints like `waterTank.waterOut` resolve
        // through the part's typing definition, which routinely lives in a
        // sibling file (FL007 false-positived on the per-file graph).
        // Foreign-file diagnostics are filtered the same way import health
        // filters them.
        for diag in flow_health_diagnostics(workspace_merged_graph.unwrap_or(graph)) {
            if !is_foreign_file_diagnostic(&diag, uri) {
                all_diagnostics.push(diag);
            }
        }

        // Import health: must see the workspace-merged graph when one is
        // available so IM001 ("namespace unresolved in current workspace
        // context") doesn't false-positive on cross-file imports. Falls
        // back to file-only checking when neither workspace nor library
        // context is loaded.
        let lib_data = library.map(|lib| lib.data(analysis.db()));
        let library_graph: Option<&ModelGraph> = lib_data.as_ref().map(|d| d.graph());
        for diag in sysml_core::import_health_diagnostics_with_context(
            graph,
            library_graph,
            workspace_merged_graph,
        ) {
            if !is_foreign_file_diagnostic(&diag, uri) {
                all_diagnostics.push(diag);
            }
        }

        // Physics health diagnostics go through the salsa-cached query
        // (ADR-011 §3 / S3.T11). The body still calls
        // `sysml_core::physics::health::physics_health_diagnostics`, but
        // every subsequent call against an unchanged graph revision is a
        // pure cache hit.
        let physics_diags = match workspace_files {
            Some(pfs) => sysml_ide_db::workspace_physics_health_best(analysis.db(), pfs, library),
            None => sysml_ide_db::file_physics_health(analysis.db(), source_file),
        };
        for diag in physics_diags.diagnostics() {
            if !is_foreign_file_diagnostic(diag, uri) {
                all_diagnostics.push(diag.clone());
            }
        }

        // Phase 4.5: live constraint monitoring (C001 / C002)
        for cd in crate::constraint_monitor::check_constraints(graph, uri) {
            let mut diag = if cd.is_assert {
                SysmlDiagnostic::warning(cd.message)
            } else {
                SysmlDiagnostic::info(cd.message)
            };
            diag.span = cd.span;
            diag.code = Some(if cd.is_assert { "C001" } else { "C002" }.into());
            all_diagnostics.push(diag);
        }

        // Phase 5: strict-mode UX enrichment (P5.3).
        //
        // When the file is opened in strict single-file mode (`ProjectKind::Strict`
        // on the `ProjectFileSet`), enrich any IM010 diagnostic with a neighbour
        // file hint and prepend a single IM012 banner explaining why cross-file
        // imports can't resolve. Both signals are no-ops in Discovered /
        // DiscoveredViaManifest projects.
        if workspace_files
            .map(|pfs| pfs.is_strict(analysis.db()))
            .unwrap_or(false)
        {
            attach_strict_mode_diagnostics(&mut all_diagnostics, &content, uri);
        }
    }

    post_process(&mut all_diagnostics);
    all_diagnostics
}

/// Strict-mode enrichment (P5.3): inject IM012 + neighbour notes on IM010.
///
/// Called only when the file's `ProjectFileSet` is `kind == Strict`. Reads
/// sibling `.sysml` / `.kerml` files from disk via `peek_neighbours` to
/// surface "here's where this name is actually defined" hints — the user
/// can then open the folder (or create `sysml.toml`) to get imports
/// resolving for real.
fn attach_strict_mode_diagnostics(
    all_diagnostics: &mut Vec<SysmlDiagnostic>,
    content: &str,
    uri: &str,
) {
    // Early-exit when there are no resolution failures to enrich. Both
    // IM012 emission and neighbour peeking pay no rent on a clean file.
    let is_resolution_failure = |d: &SysmlDiagnostic| -> bool {
        matches!(d.code.as_deref(), Some("IM010"))
            || (d.severity == Severity::Error
                && matches!(d.code.as_deref(), Some(code) if code.starts_with("E2")))
    };
    if !all_diagnostics.iter().any(is_resolution_failure) {
        return;
    }

    // Try to resolve to a real disk path. Synthetic buffers (`inmemory://`,
    // `untitled:`) have no siblings to scan, but they still benefit from
    // the IM012 banner — fall through with an empty NeighbourIndex.
    let index = match uri_to_path(uri) {
        Some(p) => sysml_project::discovery::peek_neighbours(&p),
        None => sysml_project::discovery::NeighbourIndex::default(),
    };

    for diag in all_diagnostics.iter_mut() {
        let is_im010 = matches!(diag.code.as_deref(), Some("IM010"));
        let is_e2 = diag.severity == Severity::Error
            && matches!(diag.code.as_deref(), Some(code) if code.starts_with("E2"));
        if !is_im010 && !is_e2 {
            continue;
        }
        let Some(name) = unresolved_name_from_message(&diag.message) else {
            continue;
        };
        let neighbours = index.lookup(&name);
        if neighbours.is_empty() {
            continue;
        }
        let file_list = neighbours
            .iter()
            .take(3)
            .map(|p| {
                p.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("<unknown>")
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join(", ");
        // Upgrade bare E2xx to IM010 — finding the name in a neighbour
        // makes this an actionable strict-mode failure, not a generic
        // unresolved reference. Code actions branch on IM010 to surface
        // strict-flavour quick-fixes.
        if !is_im010 {
            diag.code = Some("IM010".to_string());
        }
        // P-RA2 Slice 4: IM010 (whether freshly upgraded or already present)
        // is import-health tier — only meaningful once the workspace can see
        // its neighbours.
        diag.tier = sysml_span::DiagnosticTier::ImportHealth;
        diag.notes.push(format!(
            "'{}' is also declared in: {} (open the parent folder so cross-file imports resolve)",
            name, file_list
        ));
    }

    // Prepend a single IM012 banner. Span covers the first line of the
    // file so the LSP shows it inline; post_process filters spanless diags.
    let first_line_end = content
        .char_indices()
        .find(|(_, c)| *c == '\n')
        .map(|(i, _)| i)
        .unwrap_or(content.len());
    let mut banner = SysmlDiagnostic::info(
        "file opened in strict single-file mode — cross-file imports cannot resolve; \
         open the parent folder or add a sysml.toml manifest to enable workspace resolution"
            .to_string(),
    );
    banner.code = Some("IM012".into());
    // P-RA2 Slice 4: IM012 strict-mode banner is the canonical ImportHealth
    // signal — only surfaces once we know the file's PFS is in Strict mode.
    banner.tier = sysml_span::DiagnosticTier::ImportHealth;
    banner.span = Some(sysml_span::Span {
        file: uri.to_string(),
        start: 0,
        end: first_line_end,
        line: Some(1),
        col: Some(1),
    });
    all_diagnostics.push(banner);
}

/// Convert a `file://` URI or a raw filesystem path to a `PathBuf`.
/// Returns `None` for non-file schemes (`inmemory://`, `untitled:`,
/// `remote://`) — strict-mode enrichment is best-effort and only
/// applies when there are real sibling files on disk to peek at.
fn uri_to_path(uri: &str) -> Option<std::path::PathBuf> {
    if let Some(stripped) = uri.strip_prefix("file://") {
        return Some(std::path::PathBuf::from(stripped));
    }
    // The service represents loaded files as plain absolute paths.
    // Reject anything that looks like a non-file URI scheme.
    if uri.contains("://") {
        return None;
    }
    if uri.starts_with("inmemory:") || uri.starts_with("untitled:") {
        return None;
    }
    Some(std::path::PathBuf::from(uri))
}

fn post_process(all_diagnostics: &mut Vec<SysmlDiagnostic>) {
    // Spanless suppression
    all_diagnostics.retain(|d| d.span.is_some());

    // Scope-aware suppression: drop derived diagnostics that overlap a
    // code-less syntax-error span. When the parser couldn't make sense of a
    // region, every downstream verdict over it is unreliable — resolution
    // (E2xx), structural (Sxxx), validation (Vxxx), and lexical (E0xx) — so
    // surfacing them on top of the real syntax error is pure cascade noise
    // (e.g. `@@ this` → an E200 "no definition 'this' found" stacked on the
    // syntax error). We keep the one real syntax error and drop the cascade.
    // Only diagnostics whose span actually OVERLAPS a syntax-error span are
    // dropped, so a genuine unresolved-name error elsewhere in the file still
    // shows.
    let syntax_error_spans: Vec<(usize, usize)> = all_diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error && d.code.is_none())
        .filter_map(|d| d.span.as_ref().map(|s| (s.start, s.end)))
        .collect();
    if !syntax_error_spans.is_empty() {
        all_diagnostics.retain(|d| {
            if d.code.is_none() {
                return true;
            }
            let code = d.code.as_deref().unwrap_or("");
            let is_suppressible = code.starts_with("E0")
                || code.starts_with("E2")
                || code.starts_with('S')
                || code.starts_with('V');
            if !is_suppressible {
                return true;
            }
            if let Some(span) = d.span.as_ref() {
                !syntax_error_spans
                    .iter()
                    .any(|&(s, e)| span.start < e && span.end > s)
            } else {
                true
            }
        });
    }

    // Dedupe by (start, end, code) — keep the longer message
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

    // Priority sort: severity > phase category > position
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

    if all_diagnostics.len() > TOTAL_DIAGNOSTIC_CAP {
        all_diagnostics.truncate(TOTAL_DIAGNOSTIC_CAP);
    }
}
