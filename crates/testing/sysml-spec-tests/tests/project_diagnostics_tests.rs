//! Project-level diagnostic tests for example SysML projects.
//!
//! These tests run the full diagnostic pipeline (parse → resolve → elaborate →
//! validate → health checks) on multi-file projects with workspace-aware
//! resolution and standard library support. They detect regressions in the
//! complete tool chain.
//!
//! ## Running
//!
//! ```bash
//! SYSML_CORPUS_PATH=references/sysmlv2 \
//!   cargo test -p sysml-spec-tests --test project_diagnostics_tests -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use sysml_core::elaborate::elaborate;
use sysml_core::resolution::{resolve_references, resolve_references_excluding, FxHashSet};
use sysml_core::validate_semantic;
use sysml_core::{import_health_diagnostics, ModelGraph};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::library::{load_standard_library, LibraryConfig};
use sysml_runtime::actions::action_health_diagnostics;
use sysml_runtime::cases::verification_health_diagnostics;
use sysml_runtime::flows::flow_health_diagnostics;
use sysml_runtime::statemachine::state_machine_health_diagnostics;
use sysml_span::Diagnostic;

/// Discover all .sysml files in a directory.
fn discover_sysml_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("cannot read project directory")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|e| e == "sysml").unwrap_or(false))
        .collect();
    files.sort();
    files
}

/// A single diagnostic entry for comparison.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct DiagEntry {
    file: String,
    code: String,
    severity: String,
    message: String,
}

/// Run the full diagnostic pipeline on a project directory.
///
/// This mirrors the CLI `inspect --workspace` pipeline:
/// 1. Parse all files with tree-sitter
/// 2. Merge all model graphs
/// 3. Load and merge standard library
/// 4. Resolve references (with library exclusion)
/// 5. Elaborate ownership
/// 6. Structural + semantic validation
/// 7. Health checks (state machine, action, flow, verification, import)
/// 8. Filter to workspace files only
fn run_project_diagnostics(project_dir: &Path) -> BTreeMap<String, Vec<DiagEntry>> {
    let files = discover_sysml_files(project_dir);
    assert!(
        !files.is_empty(),
        "no .sysml files found in {}",
        project_dir.display()
    );

    // Step 1-2: Parse and merge
    let ts_parser = TreeSitterParser::new();
    let mut combined = ModelGraph::new();
    let mut all_diagnostics: Vec<Diagnostic> = Vec::new();
    let mut workspace_paths: Vec<String> = Vec::new();

    for file_path in &files {
        let source = std::fs::read_to_string(file_path)
            .unwrap_or_else(|e| panic!("cannot read {}: {}", file_path.display(), e));
        let path_str = file_path.to_string_lossy().to_string();
        workspace_paths.push(path_str.clone());

        let tree = ts_parser
            .parse_tree(&source)
            .unwrap_or_else(|| panic!("tree-sitter parse failed for {}", file_path.display()));

        let mut mgr = sysml_parser_incremental::build_model_graph(&tree, &source, &path_str);
        all_diagnostics.extend(mgr.diagnostics.drain(..));
        combined.merge(mgr.graph, false);
    }

    // Step 3-4: Load standard library and resolve
    if let Some(refs_dir) = sysml_spec_tests::try_find_references_dir() {
        let pest_parser = TreeSitterParser::new();
        let lib_config = LibraryConfig::from_corpus_path(&refs_dir);
        match load_standard_library(&pest_parser, &lib_config) {
            Ok(library) => {
                let library_element_ids: FxHashSet<_> = library.elements.keys().cloned().collect();
                combined.merge(library, true);
                let res_result = resolve_references_excluding(&mut combined, &library_element_ids);
                all_diagnostics.extend(res_result.diagnostics.into_vec());
            }
            Err(err) => {
                eprintln!("warning: library load failed, resolving without: {}", err);
                let res_result = resolve_references(&mut combined);
                all_diagnostics.extend(res_result.diagnostics.into_vec());
            }
        }
    } else {
        eprintln!("warning: no references dir found, resolving without library");
        let res_result = resolve_references(&mut combined);
        all_diagnostics.extend(res_result.diagnostics.into_vec());
    }

    // Step 5: Elaborate
    elaborate(&mut combined);

    // Step 6: Structural + semantic validation
    for error in combined.validate_structure() {
        all_diagnostics.push(error.to_diagnostic_with_graph(&combined));
    }
    for error in validate_semantic(&combined) {
        all_diagnostics.push(error.to_diagnostic_with_graph(&combined));
    }

    // Step 7: Health checks
    all_diagnostics.extend(state_machine_health_diagnostics(&combined));
    all_diagnostics.extend(action_health_diagnostics(&combined));
    all_diagnostics.extend(flow_health_diagnostics(&combined));
    all_diagnostics.extend(verification_health_diagnostics(&combined));
    all_diagnostics.extend(import_health_diagnostics(&combined));

    // Step 8: Filter to workspace files only
    all_diagnostics.retain(|diag| match &diag.span {
        Some(span) => workspace_paths.iter().any(|wp| span.file == *wp),
        None => false,
    });

    // Group by filename
    let mut by_file: BTreeMap<String, Vec<DiagEntry>> = BTreeMap::new();
    for wp in &workspace_paths {
        let short = Path::new(wp)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(wp)
            .to_string();
        by_file.entry(short).or_default();
    }

    for diag in &all_diagnostics {
        if let Some(span) = &diag.span {
            let short = Path::new(&span.file)
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(&span.file)
                .to_string();
            let entry = DiagEntry {
                file: short.clone(),
                code: diag.code.clone().unwrap_or_default(),
                severity: format!("{:?}", diag.severity),
                message: diag.message.clone(),
            };
            by_file.entry(short).or_default().push(entry);
        }
    }

    // Sort diagnostics within each file for stable comparison
    for entries in by_file.values_mut() {
        entries.sort();
        entries.dedup();
    }

    by_file
}

/// Helper: collect just the diagnostic codes for a file
fn codes_for_file(results: &BTreeMap<String, Vec<DiagEntry>>, file: &str) -> Vec<String> {
    results
        .get(file)
        .map(|entries| entries.iter().map(|e| e.code.clone()).collect())
        .unwrap_or_default()
}

/// Helper: count diagnostics by severity for a file
fn count_by_severity(
    results: &BTreeMap<String, Vec<DiagEntry>>,
    file: &str,
) -> (usize, usize, usize) {
    let entries = results.get(file).map(|e| e.as_slice()).unwrap_or(&[]);
    let errors = entries.iter().filter(|e| e.severity == "Error").count();
    let warnings = entries.iter().filter(|e| e.severity == "Warning").count();
    let infos = entries.iter().filter(|e| e.severity == "Info").count();
    (errors, warnings, infos)
}

// ---------------------------------------------------------------------------
// Coffee Machine Project Tests
// ---------------------------------------------------------------------------

fn coffee_machine_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../tests/fixtures/book-examples/coffee-machine")
}

#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn coffee_machine_full_diagnostics() {
    let dir = coffee_machine_dir();
    if !dir.exists() {
        eprintln!(
            "skipping: coffee-machine examples not found at {}",
            dir.display()
        );
        return;
    }

    let results = run_project_diagnostics(&dir);

    // Print full diagnostic report
    println!("\n=== Coffee Machine Project Diagnostics ===\n");
    let mut total_errors = 0;
    let mut total_warnings = 0;
    let mut total_infos = 0;

    for (file, entries) in &results {
        let (e, w, i) = count_by_severity(&results, file);
        total_errors += e;
        total_warnings += w;
        total_infos += i;
        println!("  {} — {} errors, {} warnings, {} info", file, e, w, i);
        for entry in entries {
            println!(
                "    [{:?}] {} — {}",
                entry.severity, entry.code, entry.message
            );
        }
    }

    println!(
        "\n  TOTAL: {} errors, {} warnings, {} info\n",
        total_errors, total_warnings, total_infos
    );

    // -----------------------------------------------------------------------
    // Assertions: expected diagnostics per file.
    //
    // These are the KNOWN diagnostics as of 2026-03-03. When a diagnostic is
    // fixed, update the expected values here. When a NEW diagnostic appears,
    // the test fails — investigate before updating.
    // -----------------------------------------------------------------------

    // Clean files — should have zero diagnostics (errors + warnings + info)
    let clean_files = [
        "actions.sysml",
        "calculations.sysml",
        "connections.sysml",
        "definitions.sysml",
        "metadata.sysml",
        "package-structure.sysml",
        "ports-and-interfaces.sysml",
        "states.sysml",
        "typing-and-specialization.sysml",
        "views.sysml",
    ];

    for file in &clean_files {
        let (e, w, i) = count_by_severity(&results, file);
        assert!(
            e == 0 && w == 0 && i == 0,
            "{} should be clean but has {} errors, {} warnings, {} info.\n  Diagnostics: {:?}",
            file,
            e,
            w,
            i,
            results.get(*file),
        );
    }

    // flows.sysml — informational diagnostics only (no errors or warnings)
    // FL001/FL002 are info-level for typed flows and succession flows
    // FL006 = succession flow ordering info, FL008 = payload type info
    {
        let (e, w, _i) = count_by_severity(&results, "flows.sysml");
        assert!(
            e == 0 && w == 0,
            "flows.sysml should have no errors/warnings but has {} errors, {} warnings.\n  Diagnostics: {:?}",
            e, w,
            results.get("flows.sysml"),
        );
        let codes = codes_for_file(&results, "flows.sysml");
        for code in &codes {
            assert!(
                ["FL001", "FL002", "FL006", "FL008"].contains(&code.as_str()),
                "flows.sysml has unexpected diagnostic code: {} (full: {:?})",
                code,
                results.get("flows.sysml"),
            );
        }
    }

    // requirements.sysml — overlapping import (`import Calculations::Calculations`
    // + `import Calculations::*`) produces IM003 info
    {
        let (e, w, _i) = count_by_severity(&results, "requirements.sysml");
        assert!(
            e == 0 && w == 0,
            "requirements.sysml should have no errors/warnings but has {} errors, {} warnings.\n  Diagnostics: {:?}",
            e, w,
            results.get("requirements.sysml"),
        );
        let codes = codes_for_file(&results, "requirements.sysml");
        for code in &codes {
            assert!(
                ["IM003"].contains(&code.as_str()),
                "requirements.sysml has unexpected diagnostic code: {} (full: {:?})",
                code,
                results.get("requirements.sysml"),
            );
        }
    }
}

/// Regression test: ensure the clean files STAY clean.
/// This runs faster by only checking files expected to be diagnostic-free.
#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn coffee_machine_clean_files_regression() {
    let dir = coffee_machine_dir();
    if !dir.exists() {
        eprintln!(
            "skipping: coffee-machine examples not found at {}",
            dir.display()
        );
        return;
    }

    let results = run_project_diagnostics(&dir);

    let clean_files = [
        "actions.sysml",
        "calculations.sysml",
        "connections.sysml",
        "definitions.sysml",
        "metadata.sysml",
        "package-structure.sysml",
        "ports-and-interfaces.sysml",
        // requirements.sysml has IM003 (overlapping Calculations import)
        "states.sysml",
        "typing-and-specialization.sysml",
        "views.sysml",
    ];

    let mut failures = Vec::new();
    for file in &clean_files {
        let (e, w, i) = count_by_severity(&results, file);
        if e > 0 || w > 0 || i > 0 {
            failures.push(format!(
                "  {} — {} errors, {} warnings, {} info: {:?}",
                file,
                e,
                w,
                i,
                results.get(*file),
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "Regression: clean files now have diagnostics!\n{}",
        failures.join("\n"),
    );
}

/// Track total diagnostic count to detect both regressions AND improvements.
/// When a fix reduces diagnostics, update the expected count.
#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn coffee_machine_diagnostic_count_snapshot() {
    let dir = coffee_machine_dir();
    if !dir.exists() {
        eprintln!(
            "skipping: coffee-machine examples not found at {}",
            dir.display()
        );
        return;
    }

    let results = run_project_diagnostics(&dir);

    let total: usize = results.values().map(|v| v.len()).sum();

    // Expected total as of 2026-03-03.
    // If this number DECREASES, congratulations — update it!
    // If this number INCREASES, investigate the regression.
    let expected_total = 7; // flows(6 info) + requirements(1 IM003 info)

    if total < expected_total {
        panic!(
            "Diagnostic count IMPROVED: {} → {} (expected {}). \
             Update expected_total in this test! Details:\n{:#?}",
            expected_total, total, expected_total, results,
        );
    }

    if total > expected_total {
        panic!(
            "Diagnostic count REGRESSED: {} → {} (expected {}). \
             New diagnostics appeared — investigate before updating. Details:\n{:#?}",
            expected_total, total, expected_total, results,
        );
    }

    println!("Diagnostic count stable at {}", total);
}
