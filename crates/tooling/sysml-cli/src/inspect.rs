//! CLI inspect command: dump semantic tokens, diagnostics, and CST for a SysML file.
//!
//! Thin wrapper over `SysmlService::inspect` (X6). The CLI owns file I/O,
//! workspace-dependency resolution, env-var-based stdlib opt-in, and the
//! text/JSON output formatting; the diagnostic + token pipeline lives in
//! `sysml-service` and is shared with the LSP/MCP/REST transports.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::{env, panic};

use sysml_parser_incremental::TreeSitterParser;
use sysml_service::inspect::{
    category_to_legacy_token_name, InspectFileResult, InspectResponse, InspectToken,
};
use sysml_service::SysmlService;
use sysml_span::{Diagnostic, LineIndex, Severity};

use crate::common::CliError;

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

/// What to show in the inspect output.
pub enum InspectMode {
    /// Show everything: tokens, diagnostics, and CST.
    All,
    /// Show only semantic tokens.
    Tokens,
    /// Show only diagnostics.
    Diagnostics,
    /// Show raw CST (tree-sitter parse tree).
    Cst,
}

/// Options for inspect command behavior.
pub struct InspectOptions {
    /// Whether to load and merge the standard library before resolution.
    pub use_stdlib: bool,
    /// Optional override path for standard library root directory.
    pub library_path: Option<PathBuf>,
    /// Suppress the progress subscriber (P-RA5).
    pub quiet: bool,
    /// Force the progress subscriber even when stderr is not a TTY (P-RA5).
    pub force_progress: bool,
}

/// Run the inspect command for a single file.
pub fn run(
    file: &Path,
    mode: InspectMode,
    json: bool,
    options: InspectOptions,
) -> Result<(), CliError> {
    let source = std::fs::read_to_string(file)
        .map_err(|e| CliError::internal(format!("cannot read '{}': {}", file.display(), e)))?;
    let file_path = file.to_string_lossy().to_string();
    let mut runtime_notes: Vec<String> = Vec::new();

    apply_library_path_env(&options);
    let service = SysmlService::empty();
    let _progress_handle =
        crate::progress::maybe_spawn(&service, options.quiet, options.force_progress);
    service
        .load_source(&file_path, &source)
        .map_err(CliError::from)?;
    enable_stdlib_if_requested(&service, &options, &mut runtime_notes);

    let mut response = service
        .inspect(Some(&file_path), false, None)
        .map_err(CliError::from)?;

    // The service returns one file entry for a single-file request.
    let file_result = response.files.pop().ok_or_else(|| {
        CliError::internal("service.inspect returned no files for single-file request")
    })?;

    let mut diagnostics: Vec<Diagnostic> = file_result.diagnostics;
    diagnostics.retain(|diag| diagnostic_targets_inspected_file(diag, file, &file_path));
    maybe_fail_on_spanless_diagnostics(&diagnostics, &file_path, "inspect::final");

    let line_index = LineIndex::new(&source);

    // --cst still parses tree-sitter locally — the service doesn't surface
    // the raw tree (it's not Send+Sync–friendly across the JSON boundary).
    let tree = if matches!(mode, InspectMode::Cst | InspectMode::All) {
        TreeSitterParser::new().parse_tree(&source)
    } else {
        None
    };

    if json {
        for note in &runtime_notes {
            eprintln!("info: {}", note);
        }
        output_json(
            &mode,
            &diagnostics,
            &file_result.tokens,
            tree.as_ref(),
            &line_index,
        )?;
    } else {
        output_text(
            &mode,
            &diagnostics,
            &file_result.tokens,
            tree.as_ref(),
            &source,
            &line_index,
            &runtime_notes,
        );
    }

    Ok(())
}

/// Run the inspect command in workspace mode — parse all project files together
/// and resolve references across file boundaries.
pub fn run_workspace(
    root: &Path,
    focus_file: Option<&str>,
    _mode: InspectMode,
    json: bool,
    options: InspectOptions,
    include_workspace_deps: bool,
) -> Result<(), CliError> {
    let mut runtime_notes: Vec<String> = Vec::new();
    let files = collect_workspace_files(root, include_workspace_deps, &mut runtime_notes)?;
    if files.is_empty() {
        return Err(CliError::user(format!(
            "no .sysml files found in '{}'",
            root.display()
        )));
    }

    runtime_notes.push(format!(
        "workspace: {} ({} files)",
        root.display(),
        files.len()
    ));

    apply_library_path_env(&options);
    let service = SysmlService::empty();
    let _progress_handle =
        crate::progress::maybe_spawn(&service, options.quiet, options.force_progress);
    let mut file_sources: Vec<(PathBuf, String)> = Vec::with_capacity(files.len());
    for file_path in &files {
        let source = std::fs::read_to_string(file_path).map_err(|e| {
            CliError::internal(format!("cannot read '{}': {}", file_path.display(), e))
        })?;
        let uri = file_path.to_string_lossy().to_string();
        service
            .load_workspace_source(&uri, &source)
            .map_err(CliError::from)?;
        file_sources.push((file_path.clone(), source));
    }
    enable_stdlib_if_requested(&service, &options, &mut runtime_notes);

    let response = service
        .inspect(None, true, focus_file)
        .map_err(CliError::from)?;
    let all_diagnostics = flatten_workspace_diagnostics(&response);

    if json {
        for note in &runtime_notes {
            eprintln!("info: {}", note);
        }
        output_workspace_json(&all_diagnostics, &file_sources, focus_file)?;
    } else {
        output_workspace_text(&all_diagnostics, &file_sources, &runtime_notes, focus_file);
    }

    Ok(())
}

fn flatten_workspace_diagnostics(response: &InspectResponse) -> Vec<Diagnostic> {
    response
        .files
        .iter()
        .flat_map(|f: &InspectFileResult| f.diagnostics.iter().cloned())
        .collect()
}

fn apply_library_path_env(options: &InspectOptions) {
    if let Some(path) = &options.library_path {
        // The service-side stdlib loader reads `SYSML_LIBRARY_PATH` (via
        // `LibraryConfig::default_library_path`). Surface the CLI flag
        // through the same channel rather than threading a new parameter
        // through the host.
        env::set_var("SYSML_LIBRARY_PATH", path);
    }
}

fn enable_stdlib_if_requested(
    service: &SysmlService,
    options: &InspectOptions,
    runtime_notes: &mut Vec<String>,
) {
    if !options.use_stdlib {
        runtime_notes.push("standard library loading disabled (--no-stdlib)".to_owned());
        return;
    }
    let loaded = service
        .host_arc()
        .lock()
        .unwrap()
        .enable_stdlib()
        .unwrap_or(false);
    if loaded {
        runtime_notes.push("standard library enabled".to_owned());
    } else {
        runtime_notes.push(
            "standard library not found (set SYSML_LIBRARY_PATH or pass --library-path)"
                .to_owned(),
        );
    }
}

fn collect_workspace_files(
    root: &Path,
    include_workspace_deps: bool,
    runtime_notes: &mut Vec<String>,
) -> Result<Vec<PathBuf>, CliError> {
    let search_root = match sysml_manifest::find_manifest(root) {
        Ok(Some((manifest_path, _))) => manifest_path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| root.to_path_buf()),
        _ => root.to_path_buf(),
    };
    // P4-closeout follow-up: pure enumeration via the same walker that
    // `service.open_context(OpenTarget::Folder)` uses internally. The
    // 100_000 cap matches what `discover_lsp_workspace` passes — large
    // enough that no realistic single-root workspace hits it, small
    // enough to surface runaway scans.
    let mut files = sysml_project::discovery::discover(&search_root, 100_000)
        .map(|d| d.files)
        .map_err(|e| CliError::internal(format!("workspace discovery failed: {e}")))?;

    if !include_workspace_deps {
        runtime_notes.push("workspace dependencies: disabled (--no-workspace-deps)".to_owned());
        files.sort();
        files.dedup();
        return Ok(files);
    }

    match sysml_manifest::find_manifest(root)
        .map_err(|e| CliError::internal(format!("failed to locate workspace manifest: {e}")))?
    {
        Some((manifest_path, manifest)) => {
            let manifest_dir = manifest_path
                .parent()
                .ok_or_else(|| CliError::internal("manifest path has no parent directory"))?;

            match sysml_resolve::resolve(&manifest, manifest_dir) {
                Ok(graph) => {
                    let dep_files = sysml_resolve::source_paths(&graph);
                    runtime_notes.push(format!(
                        "workspace dependencies: resolved {} package(s), loaded {} source file(s)",
                        graph.packages.len(),
                        dep_files.len()
                    ));
                    files.extend(dep_files);
                }
                Err(err) => {
                    runtime_notes.push(format!(
                        "workspace dependencies: resolution failed ({err}); continuing with workspace files only"
                    ));
                }
            }
        }
        None => {
            runtime_notes.push(
                "workspace dependencies: no sysml.toml found; using workspace files only"
                    .to_owned(),
            );
        }
    }

    let mut unique = BTreeSet::new();
    for file in files {
        unique.insert(file);
    }

    Ok(unique.into_iter().collect())
}

/// Output workspace diagnostics grouped by file as human-readable text.
fn output_workspace_text(
    diagnostics: &[Diagnostic],
    file_sources: &[(PathBuf, String)],
    runtime_notes: &[String],
    focus_file: Option<&str>,
) {
    if !runtime_notes.is_empty() {
        for note in runtime_notes {
            println!("info: {}", note);
        }
        println!();
    }

    let files_to_show: Vec<_> = if let Some(focus) = focus_file {
        file_sources
            .iter()
            .filter(|(path, _)| {
                path.to_string_lossy().ends_with(focus)
                    || path.file_name().and_then(|n| n.to_str()) == Some(focus)
            })
            .collect()
    } else {
        file_sources.iter().collect()
    };

    for (path, _source) in &files_to_show {
        let path_str = path.to_string_lossy();
        let file_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.span
                    .as_ref()
                    .map(|s| diag_targets_path(&s.file, &path_str))
                    .unwrap_or(false)
            })
            .collect();

        let short_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&path_str);
        println!("=== {} ({} diagnostics) ===", short_name, file_diags.len());
        if file_diags.is_empty() {
            println!("  (no diagnostics)");
        } else {
            for diag in &file_diags {
                println!("  {}", diag);
            }
        }
        println!();
    }

    let spanless: Vec<_> = diagnostics.iter().filter(|d| d.span.is_none()).collect();
    if !spanless.is_empty() {
        println!("=== General ({} diagnostics) ===", spanless.len());
        for diag in &spanless {
            println!("  {}", diag);
        }
        println!();
    }

    let total = diagnostics.len();
    let errors = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .count();
    println!(
        "summary: {} diagnostics ({} errors, {} warnings, {} info)",
        total,
        errors,
        warnings,
        total - errors - warnings
    );
}

/// Does a diagnostic's `span.file` refer to this workspace file?
///
/// The salsa host attributes diagnostics using its canonical `file://` URI
/// form (see `sysml-service` `open_context`), while the CLI tracks workspace
/// files as plain filesystem paths. A raw `span.file == path` comparison
/// therefore never matches, so import-health (IM001) and resolution (E200)
/// diagnostics were silently dropped from the per-file / `--focus` projection —
/// the workspace-inspect fail-soft: unresolved imports produced diagnostics
/// that never reached the focused file's output. Normalise the `file://`
/// scheme off both sides before comparing.
fn diag_targets_path(span_file: &str, path_str: &str) -> bool {
    let strip = |s: &str| s.strip_prefix("file://").unwrap_or(s).to_owned();
    strip(span_file) == strip(path_str)
}

/// Output workspace diagnostics as JSON.
fn output_workspace_json(
    diagnostics: &[Diagnostic],
    file_sources: &[(PathBuf, String)],
    focus_file: Option<&str>,
) -> Result<(), CliError> {
    let mut file_entries = Vec::new();

    let files_to_show: Vec<_> = if let Some(focus) = focus_file {
        file_sources
            .iter()
            .filter(|(path, _)| {
                path.to_string_lossy().ends_with(focus)
                    || path.file_name().and_then(|n| n.to_str()) == Some(focus)
            })
            .collect()
    } else {
        file_sources.iter().collect()
    };

    for (path, source) in &files_to_show {
        let path_str = path.to_string_lossy();
        let line_index = LineIndex::new(source);
        let file_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| {
                d.span
                    .as_ref()
                    .map(|s| diag_targets_path(&s.file, &path_str))
                    .unwrap_or(false)
            })
            .collect();

        let diag_json: Vec<serde_json::Value> = file_diags
            .iter()
            .map(|d| {
                let mut obj = serde_json::json!({
                    "severity": format!("{}", d.severity),
                    "message": d.message,
                });
                if let Some(code) = &d.code {
                    obj["code"] = serde_json::json!(code);
                }
                if let Some(span) = &d.span {
                    let (start_line, start_col) = line_index.line_col(span.start);
                    let (end_line, end_col) = line_index.line_col(span.end);
                    obj["range"] = serde_json::json!({
                        "start": {"line": start_line, "column": start_col},
                        "end": {"line": end_line, "column": end_col},
                    });
                }
                if !d.notes.is_empty() {
                    obj["notes"] = serde_json::json!(d.notes);
                }
                obj
            })
            .collect();

        file_entries.push(serde_json::json!({
            "file": path_str,
            "diagnostics": diag_json,
        }));
    }

    let output = serde_json::json!({
        "files": file_entries,
        "total_diagnostics": diagnostics.len(),
    });

    let formatted = serde_json::to_string_pretty(&output)
        .map_err(|e| CliError::internal(format!("JSON serialization failed: {}", e)))?;
    println!("{}", formatted);
    Ok(())
}

/// Output as human-readable text.
fn output_text(
    mode: &InspectMode,
    diagnostics: &[Diagnostic],
    tokens: &[InspectToken],
    tree: Option<&tree_sitter::Tree>,
    source: &str,
    line_index: &LineIndex,
    runtime_notes: &[String],
) {
    if !runtime_notes.is_empty() {
        for note in runtime_notes {
            println!("info: {}", note);
        }
        println!();
    }

    match mode {
        InspectMode::Diagnostics => {
            print_diagnostics(diagnostics);
        }
        InspectMode::Tokens => {
            print_tokens(tokens, line_index);
        }
        InspectMode::Cst => {
            if let Some(tree) = tree {
                print_cst(tree, source);
            } else {
                println!("(CST unavailable: tree-sitter parse failed)");
            }
        }
        InspectMode::All => {
            println!("=== Diagnostics ({}) ===", diagnostics.len());
            print_diagnostics(diagnostics);
            println!();
            println!("=== Semantic Tokens ({}) ===", tokens.len());
            print_tokens(tokens, line_index);
            println!();
            println!("=== CST ===");
            if let Some(tree) = tree {
                print_cst(tree, source);
            } else {
                println!("(CST unavailable: tree-sitter parse failed)");
            }
        }
    }
}

fn diagnostic_targets_inspected_file(diag: &Diagnostic, file: &Path, file_path: &str) -> bool {
    match &diag.span {
        Some(span) => {
            if span.file == file_path {
                return true;
            }
            let inspected_name = file.file_name();
            let span_name = Path::new(&span.file).file_name();
            inspected_name.is_some() && inspected_name == span_name
        }
        None => {
            diag.message.contains("standard library")
                || diag
                    .notes
                    .iter()
                    .any(|note| note.to_lowercase().contains("standard library"))
        }
    }
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

#[allow(clippy::panic)]
fn maybe_fail_on_spanless_diagnostics(diagnostics: &[Diagnostic], file_path: &str, stage: &str) {
    if !fail_on_spanless_diagnostics_enabled() {
        return;
    }
    let spanless: Vec<_> = diagnostics.iter().filter(|d| d.span.is_none()).collect();
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
    panic::panic_any(format!(
        "spanless diagnostics detected (stage={}, file={}, count={})\n{}",
        stage,
        file_path,
        spanless.len(),
        preview
    ));
}

fn print_diagnostics(diagnostics: &[Diagnostic]) {
    if diagnostics.is_empty() {
        println!("(no diagnostics)");
        return;
    }
    for diag in diagnostics {
        println!("{}", diag);
    }
}

fn print_tokens(tokens: &[InspectToken], line_index: &LineIndex) {
    if tokens.is_empty() {
        println!("(no tokens)");
        return;
    }
    for token in tokens {
        let (start_line, start_col) = line_index.line_col(token.start);
        let (end_line, end_col) = line_index.line_col(token.end);
        let mods = if token.modifiers.is_empty() {
            String::new()
        } else {
            format!("  [{}]", token.modifiers.join(", "))
        };
        let label = category_to_legacy_token_name(&token.token_type);
        println!(
            "{}:{}..{}:{}  {}{}",
            start_line, start_col, end_line, end_col, label, mods
        );
    }
}

fn print_cst(tree: &tree_sitter::Tree, source: &str) {
    print_cst_node(tree.root_node(), source, 0);
}

fn print_cst_node(node: tree_sitter::Node, source: &str, indent: usize) {
    let prefix = "  ".repeat(indent);
    let start = node.start_position();
    let end = node.end_position();

    if node.child_count() == 0 {
        let start_byte = node.start_byte();
        let end_byte = node.end_byte().min(source.len());
        let text = safe_slice(source, start_byte, end_byte).unwrap_or("");
        let truncated: String = text.chars().take(40).collect();
        println!(
            "{}{} [{},{}]-[{},{}] {:?}",
            prefix,
            node.kind(),
            start.row,
            start.column,
            end.row,
            end.column,
            truncated
        );
    } else {
        println!(
            "{}{} [{},{}]-[{},{}]",
            prefix,
            node.kind(),
            start.row,
            start.column,
            end.row,
            end.column
        );
        let child_count = node.child_count();
        for i in 0..child_count {
            if let Some(child) = node.child(i) {
                print_cst_node(child, source, indent + 1);
            }
        }
    }
}

/// Output as JSON.
fn output_json(
    mode: &InspectMode,
    diagnostics: &[Diagnostic],
    tokens: &[InspectToken],
    tree: Option<&tree_sitter::Tree>,
    line_index: &LineIndex,
) -> Result<(), CliError> {
    let value = match mode {
        InspectMode::Diagnostics => diagnostics_to_json(diagnostics, line_index),
        InspectMode::Tokens => tokens_to_json(tokens, line_index),
        InspectMode::Cst => cst_to_json(tree),
        InspectMode::All => {
            serde_json::json!({
                "diagnostics": diagnostics_to_json(diagnostics, line_index),
                "tokens": tokens_to_json(tokens, line_index),
                "cst": tree.map(|t| t.root_node().to_sexp()).unwrap_or_default(),
            })
        }
    };

    let output = serde_json::to_string_pretty(&value)
        .map_err(|e| CliError::internal(format!("JSON serialization failed: {}", e)))?;
    println!("{}", output);
    Ok(())
}

fn diagnostics_to_json(diagnostics: &[Diagnostic], line_index: &LineIndex) -> serde_json::Value {
    let items: Vec<serde_json::Value> = diagnostics
        .iter()
        .map(|d| {
            let mut obj = serde_json::json!({
                "severity": format!("{}", d.severity),
                "message": d.message,
            });
            if let Some(code) = &d.code {
                obj["code"] = serde_json::json!(code);
            }
            if let Some(span) = &d.span {
                let (start_line, start_col) = line_index.line_col(span.start);
                let (end_line, end_col) = line_index.line_col(span.end);
                obj["range"] = serde_json::json!({
                    "start": {"line": start_line, "column": start_col},
                    "end": {"line": end_line, "column": end_col},
                });
            }
            if !d.notes.is_empty() {
                obj["notes"] = serde_json::json!(d.notes);
            }
            obj
        })
        .collect();
    serde_json::json!(items)
}

fn tokens_to_json(tokens: &[InspectToken], line_index: &LineIndex) -> serde_json::Value {
    let items: Vec<serde_json::Value> = tokens
        .iter()
        .map(|t| {
            let (start_line, start_col) = line_index.line_col(t.start);
            let (end_line, end_col) = line_index.line_col(t.end);
            let mut obj = serde_json::json!({
                "type": category_to_legacy_token_name(&t.token_type),
                "range": {
                    "start": {"line": start_line, "column": start_col},
                    "end": {"line": end_line, "column": end_col},
                }
            });
            if !t.modifiers.is_empty() {
                obj["modifiers"] = serde_json::json!(t.modifiers);
            }
            obj
        })
        .collect();
    serde_json::json!(items)
}

fn cst_to_json(tree: Option<&tree_sitter::Tree>) -> serde_json::Value {
    serde_json::json!(tree.map(|t| t.root_node().to_sexp()).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::{diag_targets_path, run, safe_slice, InspectMode, InspectOptions};
    use std::path::PathBuf;

    #[test]
    fn diag_targets_path_normalises_file_scheme() {
        // The salsa host attributes diagnostics with the canonical `file://`
        // URI; the CLI holds plain filesystem paths. They must still match, or
        // import-health/resolution diagnostics vanish from the per-file output.
        let path = "/tmp/ws/model/main.sysml";
        assert!(diag_targets_path("file:///tmp/ws/model/main.sysml", path));
        assert!(diag_targets_path("/tmp/ws/model/main.sysml", path));
        // Both sides carrying the scheme also matches.
        assert!(diag_targets_path(
            "file:///tmp/ws/model/main.sysml",
            "file:///tmp/ws/model/main.sysml"
        ));
        // A genuinely different file must not match.
        assert!(!diag_targets_path("file:///tmp/ws/model/other.sysml", path));
    }

    #[test]
    fn safe_slice_rejects_non_char_boundary() {
        let source = "aa\u{2026}bb";
        let ellipsis_idx = source.find('\u{2026}').expect("ellipsis should exist");
        assert!(safe_slice(source, ellipsis_idx + 1, source.len()).is_none());
    }

    #[test]
    fn inspect_handles_unicode_fixture_without_panicking() {
        let fixture = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../../tests/fixtures/shared/sensemetry.sysml");
        assert!(
            fixture.exists(),
            "fixture should exist: {}",
            fixture.display()
        );

        let result = std::panic::catch_unwind(|| {
            run(
                &fixture,
                InspectMode::Diagnostics,
                true,
                InspectOptions {
                    use_stdlib: false,
                    library_path: None,
                    quiet: true,
                    force_progress: false,
                },
            )
        });
        assert!(
            result.is_ok(),
            "inspect should not panic on unicode fixture"
        );
        assert!(
            result.expect("catch_unwind should contain result").is_ok(),
            "inspect should return Ok for unicode fixture"
        );
    }
}
