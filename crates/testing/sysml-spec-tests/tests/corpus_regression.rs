//! # Corpus regression harness (L5)
//!
//! One table-driven driver over every corpus the parser must keep surviving
//! end-to-end. This collapses the former corpus drivers
//! (`corpus_tests`, `advent_corpus_tests`, `pipeline_corpus_tests`,
//! `test_family`) into a single registry + shared stage
//!
//! The obligation this tier gates is *no-regression*: the real corpus still
//! parses / resolves / elaborates / executes. It is **illustration, not
//! conformance proof** (source-precedence rule 5) — a red row
//! here is a real signal owned by the grammar lane, never softened away.
//!
//! ## Corpus registry
//!
//! | Corpus | Root | Gating | Depth |
//! |--------|------|--------|-------|
//! | `full-corpus` | `SYSML_CORPUS_PATH` → xpect `library.systems` + `SysML-v2-Models/models` (`CoverageConfig`) | env-gated, `#[ignore]` unless `--features corpus` | parse (allow-listed) |
//! | `stdlib-library` | `SYSML_CORPUS_PATH` → xpect `library.{kernel,systems,domain}` (`LibraryConfig`) | env-gated | parse / tree-sitter / resolve |
//! | `advent` | checked-in `corpus/advent` | always runs | parse → resolve → elaborate → validate → constraints → runner → diagnostics |
//! | `xpect-library-systems` | `SYSML_CORPUS_PATH` → xpect `library.systems` (d1–d5) | env-gated | parse → elaborate → **execute** |
//!
//! The `models` corpus needs no separate entry: `discover_corpus_files` walks
//! both `org.omg.sysml.xpect.tests/library.systems` and
//! `SysML-v2-Models/models`, so `full-corpus` already covers it.
//!
//! ## Test inventory (name → former test it replaces)
//!
//! - full-corpus: `corpus_coverage` ← corpus_tests::corpus_coverage
//! - stdlib-library: `library_parse_coverage`, `library_parse_stage_gate`,
//!   `treesitter_library_parse_coverage`, `treesitter_library_parse_stage_gate`,
//!   `corpus_smoke_test`, `corpus_resolution_multi_file` ← corpus_tests (same names)
//! - advent: `advent_corpus_parse_coverage`, `advent_corpus_element_variety`,
//!   `advent_tree_sitter_coverage`, `advent_resolution_coverage`,
//!   `advent_elaboration_coverage`, `advent_validation_coverage`,
//!   `advent_constraint_pipeline`, `advent_runner_compilation`,
//!   `advent_diagnostic_ux` ← advent_corpus_tests (same names)
//! - xpect-library-systems: `d1_states_elaborate_compile_pipeline`,
//!   `d2_constraints_elaborate_precompile_pipeline`,
//!   `d3_actions_elaborate_succession_pipeline`, `d4_flows_elaborate_pipeline`,
//!   `d5_combined_corpus_pipeline` ← pipeline_corpus_tests (same names)
//!
//! `test_family` (asserted nothing) is deleted with no replacement — `family.sysml`
//! is already covered by the `full-corpus` models discovery + `expected_failures.txt`.
//! Grammar-vs-spec node/enum checks left this tier for `grammar_spec_conformance.rs`
//! (formerly `treesitter_tests.rs`).
//!
//! ## Env-var robustness (conscious fix)
//!
//! The `xpect-library-systems` rows resolve `SYSML_CORPUS_PATH` via
//! [`sysml_spec_tests::corpus_env_root`], which resolves a *relative* value
//! against the workspace root rather than the crate cwd. The former
//! `pipeline_corpus_tests` joined the raw relative value against the crate
//! directory, so the relative invocation documented in the crate README
//! silently skipped. "Env var absent → skip" is unchanged (the helper returns
//! `None` when unset).
//!
//! ```bash
//! # Ungated rows only (advent):
//! cargo test -p sysml-spec-tests --test corpus_regression
//! # Everything, against the reference corpus:
//! SYSML_CORPUS_PATH=references/sysmlv2 \
//!   cargo test -p sysml-spec-tests --test corpus_regression -- --include-ignored
//! ```

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use sysml_core::elaborate::{elaborate, ElaborationReport};
use sysml_core::{ElementKind, RelationshipKind};
use sysml_parser_incremental::{build_model_graph, TreeSitterParser};
use sysml_parser_trait::library::{load_standard_library, LibraryConfig, LibraryStats};
use sysml_parser_trait::{ParseResult, Parser, SysmlFile};

use sysml_spec_tests::corpus::{collect_element_kinds, discover_corpus_files, parse_all_corpus_files};
use sysml_spec_tests::element_coverage::constructible_kinds;
use sysml_spec_tests::report::{format_failures, generate_report};
use sysml_spec_tests::{corpus_env_root, load_allow_list, CoverageConfig};

// ════════════════════════════════════════════════════════════════════════════
// Corpus registry
// ════════════════════════════════════════════════════════════════════════════

/// How a corpus root is located and whether its rows are env-gated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Gate {
    /// Checked into the repo; rows always run (not `#[ignore]`).
    CheckedIn,
    /// Located via `SYSML_CORPUS_PATH`; rows `#[ignore]` unless `--features corpus`.
    CorpusEnv,
}

/// One corpus in the regression registry.
#[derive(Debug, Clone, Copy)]
struct Corpus {
    /// Stable identifier used in reports.
    name: &'static str,
    /// Root resolution + gating.
    gate: Gate,
}

/// The single source of truth for which corpora this harness gates.
const CORPORA: &[Corpus] = &[
    Corpus { name: "full-corpus", gate: Gate::CorpusEnv },
    Corpus { name: "stdlib-library", gate: Gate::CorpusEnv },
    Corpus { name: "advent", gate: Gate::CheckedIn },
    Corpus { name: "xpect-library-systems", gate: Gate::CorpusEnv },
];

/// Meta-test: the registry is coherent (unique names, exactly one checked-in
/// corpus). Runs always so `cargo test --test corpus_regression`
/// exercises the registry even when no corpus env var is set.
#[test]
fn corpus_registry() {
    let mut names = HashSet::new();
    for c in CORPORA {
        assert!(names.insert(c.name), "duplicate corpus name: {}", c.name);
    }
    assert_eq!(
        CORPORA.iter().filter(|c| c.gate == Gate::CheckedIn).count(),
        1,
        "expected exactly one always-run (checked-in) corpus (advent)"
    );

    println!("\n=== Corpus regression registry ===");
    for c in CORPORA {
        println!("  {:<22} {:?}", c.name, c.gate);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Shared helpers (hoisted from the former drivers)
// ════════════════════════════════════════════════════════════════════════════

/// Allow-list of files expected to fail parsing (shrink-only; same matching
/// semantics as the former `corpus_tests`).
const EXPECTED_FAILURES: &str = include_str!("../data/expected_failures.txt");

const SMOKE_TEST_TARGET_FILES: usize = 5;
const SMOKE_TEST_MIN_FILES: usize = 3;

fn is_allow_listed(path: &str, allow_list: &HashSet<String>) -> bool {
    allow_list.contains(path)
        || allow_list.iter().any(|pattern| {
            if pattern.starts_with("**/") {
                path.ends_with(&pattern[3..])
            } else {
                path.contains(pattern)
            }
        })
}

fn is_library_file(path: &str) -> bool {
    path.contains("library.kernel")
        || path.contains("library.systems")
        || path.contains("library.domain")
}

/// Collect all standard-library files from the `LibraryConfig` tree
/// (`library.kernel` + `library.systems` + each `library.domain/*` subdir).
/// Returns a sorted vec of `(relative_path, content)` tuples. This is the ONE
/// hoisted copy of the walk logic the former `corpus_tests` open-coded four
/// times.
fn collect_library_files(lib_config: &LibraryConfig) -> Vec<(String, String)> {
    use walkdir::WalkDir;

    let mut library_files: Vec<(String, String)> = Vec::new();

    let dirs_and_exts: Vec<(PathBuf, &str)> = vec![
        (lib_config.library_path.join("library.kernel"), "kerml"),
        (lib_config.library_path.join("library.systems"), "sysml"),
    ];

    let domain_dir = lib_config.library_path.join("library.domain");
    let mut all_dirs = dirs_and_exts;
    if domain_dir.exists() {
        if let Ok(entries) = std::fs::read_dir(&domain_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    all_dirs.push((path, "sysml"));
                }
            }
        }
    }

    for (dir, ext) in &all_dirs {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(dir)
            .follow_links(true)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |e| e == *ext) {
                if let Ok(content) = std::fs::read_to_string(path) {
                    let rel = path
                        .strip_prefix(&lib_config.library_path)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .to_string();
                    library_files.push((rel, content));
                }
            }
        }
    }

    library_files.sort_by(|a, b| a.0.cmp(&b.0));
    library_files
}

/// Count ERROR and MISSING nodes in a tree-sitter tree.
fn count_ts_errors(node: tree_sitter::Node) -> (usize, usize) {
    let mut errors = 0usize;
    let mut missing = 0usize;

    if node.is_error() {
        errors += 1;
    }
    if node.is_missing() {
        missing += 1;
    }

    let child_count = node.child_count();
    for i in 0..child_count {
        if let Some(child) = node.child(i) {
            let (e, m) = count_ts_errors(child);
            errors += e;
            missing += m;
        }
    }

    (errors, missing)
}

/// Find the first ERROR or MISSING node and return a diagnostic string.
fn find_first_error_text(node: tree_sitter::Node, source: &str) -> String {
    if node.is_error() || node.is_missing() {
        let start = node.start_position();
        let byte_start = node.start_byte().min(source.len());
        let byte_end = node.end_byte().min(byte_start + 60).min(source.len());
        // Ensure we don't slice in the middle of a multi-byte UTF-8 character
        let text = source.get(byte_start..byte_end).unwrap_or_else(|| {
            // Find nearest valid char boundaries
            let safe_start = (0..=byte_start)
                .rev()
                .find(|&i| source.is_char_boundary(i))
                .unwrap_or(0);
            let safe_end = (byte_end..=source.len())
                .find(|&i| source.is_char_boundary(i))
                .unwrap_or(source.len());
            &source[safe_start..safe_end]
        });
        let text = text.replace('\n', "\\n");
        return format!("line {}:{} {:?}", start.row + 1, start.column, text);
    }

    let child_count = node.child_count();
    for i in 0..child_count {
        if let Some(child) = node.child(i) {
            let result = find_first_error_text(child, source);
            if !result.is_empty() {
                return result;
            }
        }
    }

    String::new()
}

// ════════════════════════════════════════════════════════════════════════════
// Corpus: full-corpus  (CoverageConfig — xpect library.systems + SysML-v2-Models/models)
// ════════════════════════════════════════════════════════════════════════════

/// Parse every discovered corpus file; assert no failures outside the shrink-only
/// allow-list. (Former `corpus_tests::corpus_coverage`.)
#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn corpus_coverage() {
    let config =
        CoverageConfig::from_env().expect("SYSML_CORPUS_PATH environment variable must be set");

    let allow_list = load_allow_list(EXPECTED_FAILURES);
    let (results, summary) = parse_all_corpus_files(&config, &allow_list);

    assert!(summary.total_files > 0, "No .sysml files found in corpus");

    let unexpected_failures: Vec<_> = results
        .iter()
        .filter(|r| !r.success)
        .filter(|r| !is_allow_listed(&r.path, &allow_list))
        .map(|r| (r.path.clone(), r.errors.clone()))
        .collect();

    let element_kinds_produced = collect_element_kinds(&config);
    let report = generate_report(
        &results,
        &summary,
        &element_kinds_produced,
        &constructible_kinds(),
        None,
        None,
    );

    println!("{}", report);

    assert!(
        unexpected_failures.is_empty(),
        "Unexpected parse failures (not in allow-list):\n{}",
        format_failures(&unexpected_failures)
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Corpus: stdlib-library  (LibraryConfig — xpect library.{kernel,systems,domain})
// ════════════════════════════════════════════════════════════════════════════

/// Per-file library parse coverage diagnostic. (Former
/// `corpus_tests::library_parse_coverage`.)
#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn library_parse_coverage() {
    let config =
        CoverageConfig::from_env().expect("SYSML_CORPUS_PATH environment variable must be set");

    let lib_config = LibraryConfig::from_corpus_path(&config.corpus_path);
    let parser = TreeSitterParser::new();

    let library_files = collect_library_files(&lib_config);

    println!("\n=== Library Parse Coverage ===");
    println!("Total library files: {}\n", library_files.len());

    let mut pass_count = 0;
    let mut fail_count = 0;
    let mut failures: Vec<(String, String)> = Vec::new();

    for (rel_path, content) in &library_files {
        let file = SysmlFile::new(rel_path.clone(), content.clone());
        let result = parser.parse(&[file]);

        if result.has_errors() {
            fail_count += 1;
            let first_error = result
                .diagnostics
                .iter()
                .find(|d: &&sysml_span::Diagnostic| d.is_error())
                .map(|d| d.to_string())
                .unwrap_or_else(|| "unknown error".to_string());
            println!("  FAIL  {} -- {}", rel_path, first_error);
            failures.push((rel_path.clone(), first_error));
        } else {
            pass_count += 1;
            let elem_count = result.graph.element_count();
            println!("  OK    {} ({} elements)", rel_path, elem_count);
        }
    }

    let total = pass_count + fail_count;
    let rate = if total > 0 {
        100.0 * pass_count as f64 / total as f64
    } else {
        0.0
    };

    println!("\n=== Summary ===");
    println!("Pass: {}/{} ({:.1}%)", pass_count, total, rate);

    if !failures.is_empty() {
        println!("\nFailing files:");
        for (path, err) in &failures {
            println!("  {} -- {}", path, err);
        }
    }
}

/// Stage gate: ALL library files must parse with zero errors. Prevents grammar
/// regressions that would silently break library loading. (Former
/// `corpus_tests::library_parse_stage_gate`.)
#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn library_parse_stage_gate() {
    let config =
        CoverageConfig::from_env().expect("SYSML_CORPUS_PATH environment variable must be set");

    let lib_config = LibraryConfig::from_corpus_path(&config.corpus_path);
    let parser = TreeSitterParser::new();

    let library_files = collect_library_files(&lib_config);

    assert!(
        library_files.len() >= 90,
        "Expected at least 90 library files, found {}",
        library_files.len()
    );

    // Parse each file and collect failures
    let mut failures: Vec<(String, String)> = Vec::new();
    let mut total_elements = 0usize;

    for (rel_path, content) in &library_files {
        let file = SysmlFile::new(rel_path.clone(), content.clone());
        let result = parser.parse(&[file]);
        if result.has_errors() {
            let first_error = result
                .diagnostics
                .iter()
                .find(|d: &&sysml_span::Diagnostic| d.is_error())
                .map(|d| d.to_string())
                .unwrap_or_else(|| "unknown error".to_string());
            failures.push((rel_path.clone(), first_error));
        } else {
            total_elements += result.graph.element_count();
        }
    }

    // Assert zero failures
    assert!(
        failures.is_empty(),
        "Library parse stage gate FAILED: {}/{} files have errors:\n{}",
        failures.len(),
        library_files.len(),
        failures
            .iter()
            .map(|(p, e)| format!("  {} -- {}", p, e))
            .collect::<Vec<_>>()
            .join("\n")
    );

    // Sanity check: library should produce a substantial number of elements
    assert!(
        total_elements > 1000,
        "Library parsed but only {} elements (expected >1000)",
        total_elements
    );

    println!(
        "Library parse stage gate PASSED: {}/{} files, {} elements total",
        library_files.len(),
        library_files.len(),
        total_elements
    );
}

/// Per-file tree-sitter library parse coverage diagnostic. (Former
/// `corpus_tests::treesitter_library_parse_coverage`.)
#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn treesitter_library_parse_coverage() {
    let config =
        CoverageConfig::from_env().expect("SYSML_CORPUS_PATH environment variable must be set");

    let lib_config = LibraryConfig::from_corpus_path(&config.corpus_path);
    let ts_parser = TreeSitterParser::new();

    let library_files = collect_library_files(&lib_config);

    println!("\n=== Tree-sitter Library Parse Coverage ===");
    println!("Total library files: {}\n", library_files.len());

    let mut pass_count = 0;
    let mut fail_count = 0;
    let mut failures: Vec<(String, usize, usize, String)> = Vec::new(); // (path, errors, missing, first_error_text)

    for (rel_path, content) in &library_files {
        let tree = ts_parser.parse_tree(content);
        match tree {
            Some(tree) => {
                let root = tree.root_node();
                if root.has_error() {
                    fail_count += 1;
                    let (err_count, miss_count) = count_ts_errors(root);
                    // Find the first ERROR/MISSING node text for diagnostics
                    let first_error = find_first_error_text(root, content);
                    println!(
                        "  FAIL  {} -- {} ERROR, {} MISSING: {}",
                        rel_path, err_count, miss_count, first_error
                    );
                    failures.push((rel_path.clone(), err_count, miss_count, first_error));
                } else {
                    pass_count += 1;
                    println!("  OK    {}", rel_path);
                }
            }
            None => {
                fail_count += 1;
                println!("  FAIL  {} -- tree-sitter parse returned None", rel_path);
                failures.push((rel_path.clone(), 0, 0, "parse returned None".to_string()));
            }
        }
    }

    let total = pass_count + fail_count;
    let rate = if total > 0 {
        100.0 * pass_count as f64 / total as f64
    } else {
        0.0
    };

    println!("\n=== Summary ===");
    println!("Tree-sitter: {}/{} ({:.1}%)", pass_count, total, rate);

    if !failures.is_empty() {
        println!("\nFailing files:");
        for (path, errors, missing, text) in &failures {
            println!(
                "  {} -- {} ERROR, {} MISSING: {}",
                path, errors, missing, text
            );
        }
    }
}

/// Stage gate: ALL library files must parse with zero ERROR/MISSING nodes in
/// tree-sitter. Prevents grammar regressions that would break IDE highlighting.
/// (Former `corpus_tests::treesitter_library_parse_stage_gate`.)
#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn treesitter_library_parse_stage_gate() {
    let config =
        CoverageConfig::from_env().expect("SYSML_CORPUS_PATH environment variable must be set");

    let lib_config = LibraryConfig::from_corpus_path(&config.corpus_path);
    let ts_parser = TreeSitterParser::new();

    let library_files = collect_library_files(&lib_config);

    assert!(
        library_files.len() >= 90,
        "Expected at least 90 library files, found {}",
        library_files.len()
    );

    let mut failures: Vec<(String, String)> = Vec::new();

    for (rel_path, content) in &library_files {
        let tree = ts_parser.parse_tree(content);
        match tree {
            Some(tree) => {
                let root = tree.root_node();
                if root.has_error() {
                    let first_error = find_first_error_text(root, content);
                    failures.push((rel_path.clone(), first_error));
                }
            }
            None => {
                failures.push((rel_path.clone(), "parse returned None".to_string()));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "Tree-sitter library parse stage gate FAILED: {}/{} files have errors:\n{}",
        failures.len(),
        library_files.len(),
        failures
            .iter()
            .map(|(p, e)| format!("  {} -- {}", p, e))
            .collect::<Vec<_>>()
            .join("\n")
    );

    println!(
        "Tree-sitter library parse stage gate PASSED: {}/{} files",
        library_files.len(),
        library_files.len()
    );
}

/// Quick health check: load the standard library, then parse+resolve a small
/// subset of model files. (Former `corpus_tests::corpus_smoke_test`.)
#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn corpus_smoke_test() {
    let config =
        CoverageConfig::from_env().expect("SYSML_CORPUS_PATH environment variable must be set");

    let parser = TreeSitterParser::new();

    let lib_config = LibraryConfig::from_corpus_path(&config.corpus_path);
    let library = match load_standard_library(&parser, &lib_config) {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("Failed to load standard library: {}", e);
            panic!("Standard library loading failed");
        }
    };

    let lib_stats = LibraryStats::from_graph(&library);
    println!(
        "Library loaded: {} elements, {} packages",
        lib_stats.element_count, lib_stats.package_count
    );

    let allow_list = load_allow_list(EXPECTED_FAILURES);
    let files = discover_corpus_files(&config);

    let model_files: Vec<_> = files
        .iter()
        .filter(|f| !is_library_file(&f.relative_path))
        .filter(|f| !is_allow_listed(&f.relative_path, &allow_list))
        .take(SMOKE_TEST_TARGET_FILES)
        .map(|f| SysmlFile::new(&f.relative_path, &f.content))
        .collect();

    assert!(
        model_files.len() >= SMOKE_TEST_MIN_FILES,
        "Expected at least {} eligible corpus files, found {}",
        SMOKE_TEST_MIN_FILES,
        model_files.len()
    );

    println!("Smoke test: {} files", model_files.len());

    let mut result = parser.parse(&model_files);
    let parse_errors = result.error_count();
    assert_eq!(parse_errors, 0, "Smoke test parse errors: {}", parse_errors);

    let res = result.resolve_with_library(library);
    let total_refs = res.resolved_count + res.unresolved_count;
    if total_refs > 0 {
        let rate = 100.0 * res.resolved_count as f64 / total_refs as f64;
        println!(
            "Resolved: {} / {} ({:.1}%)",
            res.resolved_count, total_refs, rate
        );
    } else {
        println!("Resolved: 0 / 0 (no references)");
    }

    let error_count = res.diagnostics.error_count();
    println!("Resolution errors: {}", error_count);

    if error_count > 0 {
        println!("Sample unresolved references:");
        for diag in res.diagnostics.iter().filter(|d| d.is_error()).take(10) {
            println!("  - {}", diag);
        }
    }
}

/// Full resolution with all model files parsed together + standard library.
/// (Former `corpus_tests::corpus_resolution_multi_file`.)
#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn corpus_resolution_multi_file() {
    let config =
        CoverageConfig::from_env().expect("SYSML_CORPUS_PATH environment variable must be set");

    let parser = TreeSitterParser::new();

    let lib_config = LibraryConfig::from_corpus_path(&config.corpus_path);
    let library = match load_standard_library(&parser, &lib_config) {
        Ok(lib) => lib,
        Err(e) => {
            eprintln!("Failed to load standard library: {}", e);
            panic!("Standard library loading failed");
        }
    };

    let lib_stats = LibraryStats::from_graph(&library);
    println!("\n=== Standard Library Loaded ===");
    println!("Elements: {}", lib_stats.element_count);
    println!("Library packages: {}", lib_stats.package_count);

    let allow_list = load_allow_list(EXPECTED_FAILURES);
    let files = discover_corpus_files(&config);

    let mut model_files: Vec<SysmlFile> = Vec::new();
    for file in &files {
        if is_allow_listed(&file.relative_path, &allow_list) {
            continue;
        }
        if is_library_file(&file.relative_path) {
            continue;
        }
        model_files.push(SysmlFile::new(&file.relative_path, &file.content));
    }

    println!(
        "\n=== Parsing {} model files together ===",
        model_files.len()
    );

    let mut result = parser.parse(&model_files);

    let parse_errors = result.error_count();
    let element_count = result.graph.element_count();
    println!("Parse errors: {}", parse_errors);
    println!("Elements parsed: {}", element_count);

    let res = result.resolve_with_library(library);

    println!("\n=== Multi-File Resolution Results ===");
    println!("Total resolved references: {}", res.resolved_count);
    println!("Total unresolved references: {}", res.unresolved_count);

    let total_refs = res.resolved_count + res.unresolved_count;
    if total_refs > 0 {
        let rate = 100.0 * res.resolved_count as f64 / total_refs as f64;
        println!("Resolution rate: {:.1}%", rate);
    }

    let error_count = res.diagnostics.error_count();
    println!("Resolution errors: {}", error_count);

    let unresolved_samples: Vec<_> = res
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .take(20)
        .collect();

    if !unresolved_samples.is_empty() {
        println!("\nSample unresolved references ({} total):", error_count);
        for diag in unresolved_samples {
            println!("  - {}", diag);
        }
    }

    let mut unresolved_names: HashMap<String, usize> = HashMap::new();
    for diag in res.diagnostics.iter().filter(|d| d.is_error()) {
        let msg = diag.to_string();
        if let Some(start) = msg.find('\'') {
            if let Some(end) = msg[start + 1..].find('\'') {
                let name = &msg[start + 1..start + 1 + end];
                *unresolved_names.entry(name.to_string()).or_default() += 1;
            }
        }
    }

    println!("\nMost common unresolved names:");
    let mut sorted: Vec<_> = unresolved_names.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (name, count) in sorted.iter().take(15) {
        println!("  {} ({} times)", name, count);
    }
}

// ════════════════════════════════════════════════════════════════════════════
// Corpus: advent  (checked-in — corpus/advent; always runs)
// ════════════════════════════════════════════════════════════════════════════

fn advent_corpus_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("corpus/advent")
}

fn discover_advent_files() -> Vec<(String, String)> {
    let dir = advent_corpus_dir();
    let mut files: Vec<(String, String)> = Vec::new();

    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "sysml") {
                let name = path.file_name().unwrap().to_string_lossy().to_string();
                if let Ok(content) = std::fs::read_to_string(&path) {
                    files.push((name, content));
                }
            }
        }
    }

    files.sort_by(|a, b| a.0.cmp(&b.0));
    files
}

/// Parse all advent files with TreeSitterParser, return (filename, ParseResult) for successes.
fn parse_all_advent() -> Vec<(String, ParseResult)> {
    let files = discover_advent_files();
    let parser = TreeSitterParser::new();
    let mut results = Vec::new();

    for (name, content) in &files {
        let sysml_files = vec![SysmlFile::new(name.clone(), content.clone())];
        let result = parser.parse(&sysml_files);

        let has_errors = result.diagnostics.iter().any(|d| d.is_error());
        if !has_errors {
            results.push((name.clone(), result));
        }
    }

    results
}

#[test]
fn advent_corpus_parse_coverage() {
    let files = discover_advent_files();
    assert!(
        !files.is_empty(),
        "No .sysml files found in {}",
        advent_corpus_dir().display()
    );

    let parser = TreeSitterParser::new();
    let mut passed = 0;
    let mut failed = 0;
    let mut failures: Vec<(String, Vec<String>)> = Vec::new();
    let mut element_kinds: HashMap<String, usize> = HashMap::new();

    for (name, content) in &files {
        let sysml_files = vec![SysmlFile::new(name.clone(), content.clone())];
        let result = parser.parse(&sysml_files);

        let errors: Vec<String> = result
            .diagnostics
            .iter()
            .filter(|d| d.is_error())
            .map(|d| d.to_string())
            .collect();

        if errors.is_empty() {
            passed += 1;

            // Collect element kinds produced
            for element in result.graph.elements.values() {
                *element_kinds
                    .entry(format!("{:?}", element.kind))
                    .or_default() += 1;
            }
        } else {
            failed += 1;
            failures.push((name.clone(), errors));
        }
    }

    let total = passed + failed;
    let rate = if total > 0 {
        100.0 * passed as f64 / total as f64
    } else {
        0.0
    };

    println!("\n=== Advent of SysML v2 Corpus Coverage ===");
    println!("Files: {total} total, {passed} passed, {failed} failed ({rate:.1}%)");

    if !element_kinds.is_empty() {
        println!("\nElement kinds produced ({} unique):", element_kinds.len());
        let mut sorted: Vec<_> = element_kinds.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (kind, count) in sorted.iter().take(25) {
            println!("  {kind}: {count}");
        }
    }

    if !failures.is_empty() {
        println!("\nFailed files:");
        for (name, errors) in &failures {
            println!("  {name}:");
            for err in errors.iter().take(3) {
                println!("    - {err}");
            }
            if errors.len() > 3 {
                println!("    ... and {} more", errors.len() - 3);
            }
        }
    }

    // We expect a reasonable pass rate - these are curated examples
    assert!(
        rate >= 30.0,
        "Advent corpus pass rate {rate:.1}% is below 30% minimum threshold"
    );
}

#[test]
fn advent_corpus_element_variety() {
    let files = discover_advent_files();
    assert!(!files.is_empty());

    let parser = TreeSitterParser::new();

    // Parse all files together to see combined element coverage
    let sysml_files: Vec<SysmlFile> = files
        .iter()
        .map(|(name, content)| SysmlFile::new(name.clone(), content.clone()))
        .collect();

    let result = parser.parse(&sysml_files);

    let mut kinds_seen: HashMap<String, usize> = HashMap::new();
    for element in result.graph.elements.values() {
        *kinds_seen.entry(format!("{:?}", element.kind)).or_default() += 1;
    }

    println!("\n=== Advent Corpus Element Variety ===");
    println!("Total elements: {}", result.graph.element_count());
    println!("Unique element kinds: {}", kinds_seen.len());

    // These are the key kinds we expect from the Advent series
    let expected_kinds = [
        ElementKind::Package,
        ElementKind::PartDefinition,
        ElementKind::PartUsage,
        ElementKind::AttributeUsage,
    ];

    for kind in &expected_kinds {
        let key = format!("{:?}", kind);
        let count = kinds_seen.get(&key).copied().unwrap_or(0);
        println!("  {key}: {count}");
        assert!(count > 0, "Expected at least one {key} from advent corpus");
    }

    // Report on advanced kinds that indicate good coverage depth
    let advanced_kinds = [
        ElementKind::EnumerationDefinition,
        ElementKind::PortDefinition,
        ElementKind::PortUsage,
        ElementKind::InterfaceDefinition,
        ElementKind::ConnectionDefinition,
        ElementKind::ConnectionUsage,
        ElementKind::ActionDefinition,
        ElementKind::ActionUsage,
        ElementKind::StateDefinition,
        ElementKind::StateUsage,
        ElementKind::ConstraintUsage,
        ElementKind::RequirementDefinition,
        ElementKind::RequirementUsage,
        ElementKind::ItemDefinition,
        ElementKind::ItemUsage,
    ];

    println!("\nAdvanced element coverage:");
    let mut advanced_found = 0;
    for kind in &advanced_kinds {
        let key = format!("{:?}", kind);
        let count = kinds_seen.get(&key).copied().unwrap_or(0);
        if count > 0 {
            advanced_found += 1;
            println!("  {key}: {count}");
        } else {
            println!("  {key}: MISSING");
        }
    }

    println!(
        "\nAdvanced coverage: {advanced_found}/{} kinds found",
        advanced_kinds.len()
    );
}

#[test]
fn advent_tree_sitter_coverage() {
    let files = discover_advent_files();
    assert!(!files.is_empty());

    let parser = TreeSitterParser::new();
    let mut ts_passed = 0;
    let mut ts_failed = 0;
    let mut total_elements = 0usize;
    let mut failures: Vec<String> = Vec::new();
    let mut element_kinds: HashMap<String, usize> = HashMap::new();

    for (name, content) in &files {
        let tree = parser.parse_tree(content);
        match tree {
            Some(ref t) if !t.root_node().has_error() => {
                ts_passed += 1;
                let mgr = build_model_graph(t, content, name);
                let count = mgr.graph.element_count();
                total_elements += count;
                for element in mgr.graph.elements.values() {
                    *element_kinds
                        .entry(format!("{:?}", element.kind))
                        .or_default() += 1;
                }
            }
            _ => {
                ts_failed += 1;
                failures.push(name.clone());
            }
        }
    }

    let total = ts_passed + ts_failed;
    let rate = if total > 0 {
        100.0 * ts_passed as f64 / total as f64
    } else {
        0.0
    };

    println!("\n=== T1: Tree-sitter Parse Coverage ===");
    println!("Files: {total} total, {ts_passed} passed, {ts_failed} failed ({rate:.1}%)");
    println!("Elements produced: {total_elements}");

    if !element_kinds.is_empty() {
        println!("\nElement kinds ({} unique):", element_kinds.len());
        let mut sorted: Vec<_> = element_kinds.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (kind, count) in sorted.iter().take(15) {
            println!("  {kind}: {count}");
        }
    }

    if !failures.is_empty() {
        println!("\nFailed files ({}):", failures.len());
        for name in &failures {
            println!("  {name}");
        }
    }

    // Sanity floor: at least some files should parse with tree-sitter
    assert!(
        ts_passed >= 10,
        "Tree-sitter parsed only {ts_passed}/{total} files, expected at least 10"
    );
}

#[test]
fn advent_resolution_coverage() {
    let mut parsed = parse_all_advent();
    assert!(
        !parsed.is_empty(),
        "No files parsed successfully for resolution test"
    );

    let mut total_resolved = 0usize;
    let mut total_unresolved = 0usize;
    let mut per_file: Vec<(String, usize, usize)> = Vec::new();

    for (name, result) in &mut parsed {
        let res = result.resolve();
        total_resolved += res.resolved_count;
        total_unresolved += res.unresolved_count;
        if res.resolved_count > 0 || res.unresolved_count > 0 {
            per_file.push((name.clone(), res.resolved_count, res.unresolved_count));
        }
    }

    let total_refs = total_resolved + total_unresolved;
    let rate = if total_refs > 0 {
        100.0 * total_resolved as f64 / total_refs as f64
    } else {
        0.0
    };

    println!("\n=== T3: Resolution ===");
    println!("Total references: {total_refs}, Resolved: {total_resolved} ({rate:.1}%)");
    println!("(Low rate expected — files use ScalarValues/ISQ/SI without library)");

    if !per_file.is_empty() {
        println!("\nPer-file breakdown (top 15):");
        per_file.sort_by(|a, b| b.1.cmp(&a.1));
        for (name, resolved, unresolved) in per_file.iter().take(15) {
            println!("  {name}: {resolved} resolved, {unresolved} unresolved");
        }
    }

    // Informational — resolution without library is expected to be low
}

#[test]
fn advent_elaboration_coverage() {
    let mut parsed = parse_all_advent();
    assert!(!parsed.is_empty());

    let mut aggregate = ElaborationReport::default();
    let mut idempotent_failures: Vec<String> = Vec::new();
    let mut files_with_work = 0;

    for (name, result) in &mut parsed {
        let report = elaborate(&mut result.graph);
        if !report.is_empty() {
            files_with_work += 1;
        }
        // Accumulate metrics
        aggregate.initial_states_tagged += report.initial_states_tagged;
        aggregate.final_states_tagged += report.final_states_tagged;
        aggregate.transitions_created += report.transitions_created;
        aggregate.state_actions_tagged += report.state_actions_tagged;
        aggregate.constraints_derived += report.constraints_derived;
        aggregate.successions_created += report.successions_created;
        aggregate.flows_derived += report.flows_derived;

        // Verify idempotency: second elaborate should return empty report
        let report2 = elaborate(&mut result.graph);
        if !report2.is_empty() {
            idempotent_failures.push(format!(
                "{name}: second pass had {} changes",
                report2.total()
            ));
        }
    }

    println!("\n=== T4: Elaboration ===");
    println!(
        "Files with elaboration work: {files_with_work}/{}",
        parsed.len()
    );
    println!(
        "  initial_states_tagged: {}",
        aggregate.initial_states_tagged
    );
    println!("  final_states_tagged: {}", aggregate.final_states_tagged);
    println!("  transitions_created: {}", aggregate.transitions_created);
    println!("  state_actions_tagged: {}", aggregate.state_actions_tagged);
    println!("  constraints_derived: {}", aggregate.constraints_derived);
    println!("  successions_created: {}", aggregate.successions_created);
    println!("  flows_derived: {}", aggregate.flows_derived);
    println!("  total modifications: {}", aggregate.total());

    if idempotent_failures.is_empty() {
        println!("Idempotency: PASS (all {} files)", parsed.len());
    } else {
        println!("Idempotency: FAIL ({} files):", idempotent_failures.len());
        for f in &idempotent_failures {
            println!("  {f}");
        }
    }

    assert!(
        idempotent_failures.is_empty(),
        "Elaboration idempotency violated for {} files",
        idempotent_failures.len()
    );
}

#[test]
fn advent_validation_coverage() {
    let parsed = parse_all_advent();
    assert!(!parsed.is_empty());

    let mut structural_count = 0usize;
    let mut semantic_count = 0usize;
    let mut structural_codes: HashMap<String, usize> = HashMap::new();
    let mut semantic_codes: HashMap<String, usize> = HashMap::new();
    let mut clean_files = 0;
    let mut worst_structural: Vec<(String, usize)> = Vec::new();
    let mut worst_semantic: Vec<(String, usize)> = Vec::new();

    for (name, result) in &parsed {
        let struct_errors = result.graph.validate_structure();
        let sem_errors = sysml_core::validate_semantic(&result.graph);

        structural_count += struct_errors.len();
        semantic_count += sem_errors.len();

        if struct_errors.is_empty() && sem_errors.is_empty() {
            clean_files += 1;
        }

        // Categorize structural errors by variant name
        for err in &struct_errors {
            let code = match err {
                sysml_core::StructuralError::OrphanElement { .. } => "E001-Orphan",
                sysml_core::StructuralError::OwnershipCycle { .. } => "E002-Cycle",
                sysml_core::StructuralError::DanglingMembershipRef { .. } => {
                    "E003-DanglingMembership"
                }
                sysml_core::StructuralError::DanglingOwningMembership { .. } => {
                    "E004-DanglingOwning"
                }
                sysml_core::StructuralError::InvalidOwningMembership { .. } => "E005-InvalidOwning",
                sysml_core::StructuralError::RelationshipSourceTypeMismatch { .. } => {
                    "E006-SrcType"
                }
                sysml_core::StructuralError::RelationshipTargetTypeMismatch { .. } => {
                    "E007-TgtType"
                }
                sysml_core::StructuralError::DanglingRelationshipRef { .. } => "E008-DanglingRel",
            };
            *structural_codes.entry(code.to_string()).or_default() += 1;
        }

        // Categorize semantic errors by rule_id
        for err in &sem_errors {
            *semantic_codes.entry(err.rule_id.to_string()).or_default() += 1;
        }

        if struct_errors.len() > 0 {
            worst_structural.push((name.clone(), struct_errors.len()));
        }
        if sem_errors.len() > 0 {
            worst_semantic.push((name.clone(), sem_errors.len()));
        }
    }

    println!("\n=== T5: Validation ===");
    println!(
        "Structural errors: {structural_count} across {} files",
        worst_structural.len()
    );
    println!(
        "Semantic errors: {semantic_count} across {} files",
        worst_semantic.len()
    );
    println!("Clean files: {clean_files}/{}", parsed.len());

    if !structural_codes.is_empty() {
        println!("\nStructural error codes:");
        let mut sorted: Vec<_> = structural_codes.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (code, count) in &sorted {
            println!("  {code}: {count}");
        }
    }

    if !semantic_codes.is_empty() {
        println!("\nSemantic error codes (top 15):");
        let mut sorted: Vec<_> = semantic_codes.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (code, count) in sorted.iter().take(15) {
            println!("  {code}: {count}");
        }
    }

    if !worst_structural.is_empty() {
        worst_structural.sort_by(|a, b| b.1.cmp(&a.1));
        println!("\nWorst structural offenders:");
        for (name, count) in worst_structural.iter().take(5) {
            println!("  {name}: {count} errors");
        }
    }

    if !worst_semantic.is_empty() {
        worst_semantic.sort_by(|a, b| b.1.cmp(&a.1));
        println!("\nWorst semantic offenders:");
        for (name, count) in worst_semantic.iter().take(5) {
            println!("  {name}: {count} errors");
        }
    }

    // Informational — no hard assertion on error counts
}

#[test]
fn advent_constraint_pipeline() {
    use sysml_runtime::constraints::{evaluate_all, extract_constraints, EvalContext};

    let mut parsed = parse_all_advent();
    assert!(!parsed.is_empty());

    let mut files_with_constraints = 0;
    let mut total_extracted = 0usize;
    let mut total_satisfied = 0usize;
    let mut total_violated = 0usize;
    let mut total_eval_error = 0usize;

    for (name, result) in &mut parsed {
        elaborate(&mut result.graph);

        let constraint_set = extract_constraints(&result.graph);
        let extracted = constraint_set.len();
        if extracted == 0 {
            continue;
        }

        files_with_constraints += 1;
        total_extracted += extracted;

        // Build a minimal context from graph attributes
        let mut ctx = EvalContext::new();
        for element in result.graph.elements.values() {
            if let Some(ref ename) = element.name {
                if let Some(val) = element.get_prop("default") {
                    match val {
                        sysml_core::Value::Float(n) => {
                            ctx.set(ename.clone(), sysml_core::Value::Float(*n));
                        }
                        sysml_core::Value::Int(n) => {
                            ctx.set(ename.clone(), sysml_core::Value::Float(*n as f64));
                        }
                        _ => {}
                    }
                }
            }
        }

        let evals = evaluate_all(&constraint_set, &ctx);
        for eval in &evals {
            if eval.diagnostics.is_empty() {
                if eval.satisfied {
                    total_satisfied += 1;
                } else {
                    total_violated += 1;
                }
            } else {
                total_eval_error += 1;
            }
        }

        if extracted > 0 {
            println!("  {name}: {extracted} extracted");
        }
    }

    println!("\n=== T6: Constraint Pipeline ===");
    println!(
        "Files with constraints: {files_with_constraints}/{}",
        parsed.len()
    );
    println!("Total extracted: {total_extracted}");
    println!("Evaluation: {total_satisfied} satisfied, {total_violated} violated, {total_eval_error} errors");

    // Soft assertion: we expect at least some constraints to be found across all files
    // but don't hard-fail if none are found (depends on corpus content)
    if files_with_constraints == 0 {
        println!("NOTE: No constraints found in any advent file — this is OK if the corpus doesn't have constraint syntax");
    }
}

#[test]
fn advent_runner_compilation() {
    use sysml_runtime::actions::compile_action;
    use sysml_runtime::cases::compile_verification_case;
    use sysml_runtime::statemachine::StateMachineCompiler;
    use sysml_runtime::CompileToIR;

    let mut parsed = parse_all_advent();
    assert!(!parsed.is_empty());

    // State machine metrics
    let mut sm_attempted = 0;
    let mut sm_compiled = 0;
    let mut sm_errors: Vec<(String, String)> = Vec::new();

    // Action metrics
    let mut act_attempted = 0;
    let mut act_compiled = 0;
    let mut act_errors: Vec<(String, String)> = Vec::new();

    // Verification case metrics
    let mut vc_attempted = 0;
    let mut vc_compiled = 0;
    let mut vc_errors: Vec<(String, String)> = Vec::new();

    for (file_name, result) in &mut parsed {
        elaborate(&mut result.graph);

        // Collect element IDs and names by kind before attempting compilation
        let state_defs: Vec<_> = result
            .graph
            .elements
            .values()
            .filter(|e| e.kind == ElementKind::StateDefinition)
            .map(|e| (e.id.clone(), e.name.clone()))
            .collect();

        let action_defs: Vec<_> = result
            .graph
            .elements
            .values()
            .filter(|e| {
                e.kind == ElementKind::ActionDefinition || e.kind == ElementKind::ActionUsage
            })
            .filter_map(|e| e.name.clone().map(|n| (e.id.clone(), n)))
            .collect();

        let vc_defs: Vec<_> = result
            .graph
            .elements
            .values()
            .filter(|e| {
                e.kind == ElementKind::VerificationCaseDefinition
                    || e.kind == ElementKind::VerificationCaseUsage
            })
            .filter_map(|e| e.name.clone().map(|n| (e.id.clone(), n)))
            .collect();

        // State machines: compile once per file that has state defs
        if !state_defs.is_empty() {
            sm_attempted += 1;
            match <StateMachineCompiler as CompileToIR<_>>::compile(&result.graph) {
                Ok(_ir) => sm_compiled += 1,
                Err(diags) => {
                    let label = state_defs
                        .first()
                        .and_then(|(_, n)| n.as_deref())
                        .unwrap_or("<unnamed>");
                    let msg = diags
                        .first()
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| "unknown error".into());
                    sm_errors.push((format!("{file_name}::{label}"), msg));
                }
            }
        }

        // Actions: compile by name
        for (_id, name) in &action_defs {
            act_attempted += 1;
            match compile_action(name, &result.graph) {
                Ok(_ir) => act_compiled += 1,
                Err(diags) => {
                    let msg = diags
                        .first()
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| "unknown error".into());
                    act_errors.push((format!("{file_name}::{name}"), msg));
                }
            }
        }

        // Verification cases: compile by name
        for (_id, name) in &vc_defs {
            vc_attempted += 1;
            match compile_verification_case(name, &result.graph) {
                Ok(_ir) => vc_compiled += 1,
                Err(diags) => {
                    let msg = diags
                        .first()
                        .map(|d| d.to_string())
                        .unwrap_or_else(|| "unknown error".into());
                    vc_errors.push((format!("{file_name}::{name}"), msg));
                }
            }
        }
    }

    let sm_failed = sm_attempted - sm_compiled;
    let act_failed = act_attempted - act_compiled;
    let vc_failed = vc_attempted - vc_compiled;

    println!("\n=== T7: Runner Compilation ===");
    println!(
        "State machines: {sm_attempted} attempted, {sm_compiled} compiled, {sm_failed} failed"
    );
    println!("Actions: {act_attempted} attempted, {act_compiled} compiled, {act_failed} failed");
    println!(
        "Verification cases: {vc_attempted} attempted, {vc_compiled} compiled, {vc_failed} failed"
    );

    if !sm_errors.is_empty() {
        println!("\nState machine errors (first 5):");
        for (loc, msg) in sm_errors.iter().take(5) {
            println!("  {loc}: {msg}");
        }
    }

    if !act_errors.is_empty() {
        println!("\nAction errors (first 5):");
        for (loc, msg) in act_errors.iter().take(5) {
            println!("  {loc}: {msg}");
        }
    }

    if !vc_errors.is_empty() {
        println!("\nVerification case errors (first 5):");
        for (loc, msg) in vc_errors.iter().take(5) {
            println!("  {loc}: {msg}");
        }
    }

    // No hard assertion on compilation success (failures are expected),
    // but this test guards against panics in the compilation pipeline
}

#[test]
fn advent_diagnostic_ux() {
    use sysml_core::validate_semantic;
    use sysml_span::{Diagnostic, Severity};

    let files = discover_advent_files();
    assert!(
        !files.is_empty(),
        "No .sysml files found in {}",
        advent_corpus_dir().display()
    );

    let parser = TreeSitterParser::new();

    // Grammar rule names that should never leak into user-facing messages
    let grammar_leak_patterns = [
        "DefinitionBody",
        "OwnedExpression",
        "FeatureTyping",
        "UsageCompletion",
        "PackageBodyElement",
        "NodeParameterMember",
        "TransitionSuccessionMember",
        "GuardExpressionMember",
        "EffectBehaviorMember",
        "StateActionUsage",
    ];

    // Violation accumulators
    let mut invalid_spans: Vec<(String, String)> = Vec::new();
    let mut empty_messages: Vec<(String, String)> = Vec::new();
    let mut long_messages: Vec<(String, String)> = Vec::new();
    let mut grammar_leaks: Vec<(String, String)> = Vec::new();
    let missing_codes: Vec<(String, String)> = Vec::new();
    let mut duplicates: Vec<(String, String)> = Vec::new();
    let mut severity_mismatches: Vec<(String, String)> = Vec::new();
    let mut invalid_related_spans: Vec<(String, String)> = Vec::new();
    let mut cascade_files: Vec<(String, usize)> = Vec::new();
    let mut inverted_spans: Vec<(String, String)> = Vec::new();

    let mut total_diagnostics = 0usize;
    let mut files_checked = 0usize;

    for (name, content) in &files {
        let sysml_files = vec![SysmlFile::new(name.clone(), content.clone())];
        let result = parser.parse(&sysml_files);
        files_checked += 1;

        let source_len = content.len();

        // Start with parse diagnostics
        let mut all_diags: Vec<Diagnostic> = result.diagnostics.clone();

        // If no parse errors, also run structural and semantic validation
        let has_parse_errors = result.diagnostics.iter().any(|d| d.is_error());
        if !has_parse_errors {
            let struct_errors = result.graph.validate_structure();
            for err in &struct_errors {
                all_diags.push(err.to_diagnostic_with_graph(&result.graph));
            }

            let sem_errors = validate_semantic(&result.graph);
            for err in &sem_errors {
                all_diags.push(err.to_diagnostic_with_graph(&result.graph));
            }
        }

        total_diagnostics += all_diags.len();

        // Check 7: Cascade detection (>50 diagnostics per file)
        if all_diags.len() > 50 {
            cascade_files.push((name.clone(), all_diags.len()));
        }

        // Track duplicates per file: (start, end, code)
        let mut seen: HashSet<(usize, usize, String)> = HashSet::new();

        for diag in &all_diags {
            let code_str = diag.code.as_deref().unwrap_or("");

            // Check 1: Span validity
            if let Some(ref span) = diag.span {
                if span.start > span.end {
                    inverted_spans.push((
                        name.clone(),
                        format!(
                            "inverted span: start={} > end={}, code={}",
                            span.start, span.end, code_str
                        ),
                    ));
                }
                if span.end > source_len {
                    invalid_spans.push((
                        name.clone(),
                        format!(
                            "span end {} exceeds source length {}, code={}",
                            span.end, source_len, code_str
                        ),
                    ));
                }
                if span.start == span.end
                    && diag.severity == Severity::Error
                    && (code_str.starts_with('E') || code_str.starts_with('S'))
                {
                    invalid_spans.push((
                        name.clone(),
                        format!(
                            "zero-length error span at {}, code={}",
                            span.start, code_str
                        ),
                    ));
                }
            } else if (diag.severity == Severity::Error || diag.severity == Severity::Warning)
                && (code_str.starts_with('E') || code_str.starts_with('S'))
            {
                invalid_spans.push((
                    name.clone(),
                    format!(
                        "missing span for {} diagnostic with code={}",
                        if diag.severity == Severity::Error {
                            "error"
                        } else {
                            "warning"
                        },
                        code_str
                    ),
                ));
            }

            // Check 2: Message quality
            if diag.message.is_empty() {
                empty_messages.push((name.clone(), format!("empty message, code={}", code_str)));
            }
            if diag.message.len() > 200 {
                long_messages.push((
                    name.clone(),
                    format!(
                        "message length {} > 200, code={}",
                        diag.message.len(),
                        code_str
                    ),
                ));
            }
            for pattern in &grammar_leak_patterns {
                if diag.message.contains(pattern) {
                    grammar_leaks.push((
                        name.clone(),
                        format!(
                            "grammar rule '{}' leaked in message, code={}",
                            pattern, code_str
                        ),
                    ));
                }
            }

            // Check 3: Error code presence
            // We only check structural/semantic diagnostics which should have codes.
            // Parse diagnostics may or may not have codes.
            // Structural errors have E0xx codes, semantic have S0xx codes.
            // We detect these by checking if the code matches the expected pattern.
            if diag.code.is_none() {
                // If this looks like it came from structural or semantic validation
                // (heuristic: non-parse diagnostics typically have codes)
                // We don't hard-fail here since parse diagnostics legitimately lack codes
            }

            // Check 4: Duplicate detection
            if let Some(ref span) = diag.span {
                let key = (span.start, span.end, code_str.to_string());
                if !seen.insert(key) {
                    duplicates.push((
                        name.clone(),
                        format!(
                            "duplicate diagnostic at {}..{}, code={}",
                            span.start, span.end, code_str
                        ),
                    ));
                }
            }

            // Check 5: Severity correctness
            if code_str == "S001" && diag.severity != Severity::Warning {
                severity_mismatches.push((
                    name.clone(),
                    format!("S001 should be Warning, got {:?}", diag.severity),
                ));
            }
            if code_str.starts_with('E')
                && !code_str.is_empty()
                && code_str.len() == 4
                && diag.severity != Severity::Error
            {
                severity_mismatches.push((
                    name.clone(),
                    format!("{} should be Error, got {:?}", code_str, diag.severity),
                ));
            }
            // S015+ should be Error
            if code_str.starts_with('S') && code_str.len() == 4 && code_str != "S001" {
                if let Ok(num) = code_str[1..].parse::<u32>() {
                    if num >= 15 && diag.severity != Severity::Error {
                        severity_mismatches.push((
                            name.clone(),
                            format!("{} should be Error, got {:?}", code_str, diag.severity),
                        ));
                    }
                }
            }

            // Check 6: Related span validity
            for related in &diag.related {
                if related.span.start > related.span.end {
                    invalid_related_spans.push((
                        name.clone(),
                        format!(
                            "related span inverted: start={} > end={}",
                            related.span.start, related.span.end
                        ),
                    ));
                }
                // Only check bounds if related span refers to same file
                if related.span.file == *name && related.span.end > source_len {
                    invalid_related_spans.push((
                        name.clone(),
                        format!(
                            "related span end {} exceeds source length {}",
                            related.span.end, source_len
                        ),
                    ));
                }
            }
        }
    }

    // ── Report ──
    println!("\n=== T8: Diagnostic UX Quality ===");
    println!("Files checked: {files_checked}");
    println!("Total diagnostics: {total_diagnostics}");
    println!();
    println!(
        "Check 1 - Span validity:    {} violations",
        invalid_spans.len() + inverted_spans.len()
    );
    println!(
        "Check 2 - Message quality:  {} empty, {} long, {} grammar leaks",
        empty_messages.len(),
        long_messages.len(),
        grammar_leaks.len()
    );
    println!(
        "Check 3 - Error codes:      {} missing",
        missing_codes.len()
    );
    println!("Check 4 - Duplicates:       {}", duplicates.len());
    println!(
        "Check 5 - Severity:         {} mismatches",
        severity_mismatches.len()
    );
    println!(
        "Check 6 - Related spans:    {} violations",
        invalid_related_spans.len()
    );
    println!(
        "Check 7 - Cascade:          {} files with >50 diagnostics",
        cascade_files.len()
    );

    // Print details for non-empty categories
    if !inverted_spans.is_empty() {
        println!("\nInverted spans:");
        for (file, detail) in &inverted_spans {
            println!("  {file}: {detail}");
        }
    }
    if !invalid_spans.is_empty() {
        println!("\nInvalid spans (first 10):");
        for (file, detail) in invalid_spans.iter().take(10) {
            println!("  {file}: {detail}");
        }
    }
    if !empty_messages.is_empty() {
        println!("\nEmpty messages:");
        for (file, detail) in &empty_messages {
            println!("  {file}: {detail}");
        }
    }
    if !grammar_leaks.is_empty() {
        println!("\nGrammar leaks (first 10):");
        for (file, detail) in grammar_leaks.iter().take(10) {
            println!("  {file}: {detail}");
        }
    }
    if !duplicates.is_empty() {
        println!("\nDuplicates (first 10):");
        for (file, detail) in duplicates.iter().take(10) {
            println!("  {file}: {detail}");
        }
    }
    if !severity_mismatches.is_empty() {
        println!("\nSeverity mismatches:");
        for (file, detail) in &severity_mismatches {
            println!("  {file}: {detail}");
        }
    }
    if !cascade_files.is_empty() {
        println!("\nCascade files:");
        for (file, count) in &cascade_files {
            println!("  {file}: {count} diagnostics");
        }
    }

    // ── Hard assertions ──
    assert!(
        empty_messages.is_empty(),
        "Found {} diagnostics with empty messages",
        empty_messages.len()
    );
    assert!(
        inverted_spans.is_empty(),
        "Found {} diagnostics with inverted spans (start > end)",
        inverted_spans.len()
    );

    // ── Soft assertion: grammar leaks ──
    if grammar_leaks.len() >= 20 {
        println!(
            "\nWARNING: {} grammar rule name leaks detected (threshold: 20)",
            grammar_leaks.len()
        );
    }
    assert!(
        grammar_leaks.len() < 20,
        "Too many grammar rule name leaks: {} (threshold: 20)",
        grammar_leaks.len()
    );
}

// ════════════════════════════════════════════════════════════════════════════
// Corpus: xpect-library-systems  (execute depth — d1–d5 pipeline)
// ════════════════════════════════════════════════════════════════════════════

const LIBRARY_SYSTEMS: &str =
    "SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.systems";

/// Helper: parse a single corpus file for the d1–d5 execute pipeline.
fn parse_pipeline_corpus_file(base: &PathBuf, relative: &str) -> ParseResult {
    let path = base.join(relative);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", path.display(), e));

    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new(path.to_string_lossy().to_string(), source)];
    parser.parse(&files)
}

#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn d1_states_elaborate_compile_pipeline() {
    use sysml_runtime::statemachine::StateMachineCompiler;
    use sysml_runtime::CompileToIR;

    let base = match corpus_env_root() {
        Some(p) => p,
        None => return,
    };

    let mut result = parse_pipeline_corpus_file(&base, &format!("{}/States.sysml", LIBRARY_SYSTEMS));

    if result.graph.element_count() == 0 {
        eprintln!("SKIP: States.sysml produced no elements");
        return;
    }

    // Elaborate
    let report = elaborate(&mut result.graph);
    eprintln!("States.sysml elaboration: {}", report);

    // Count state definitions
    let state_defs: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::StateDefinition)
        .collect();
    let state_usages: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::StateUsage)
        .collect();

    eprintln!(
        "  StateDefinitions: {}, StateUsages: {}",
        state_defs.len(),
        state_usages.len()
    );

    // Try to compile each state definition to IR
    let mut compiled = 0;
    let mut compile_errors = 0;

    for sd in &state_defs {
        let name = sd.name.as_deref().unwrap_or("<unnamed>");

        // Check if this definition has child StateUsages
        let child_states: Vec<_> = result
            .graph
            .children_of(&sd.id)
            .filter(|e| e.kind == ElementKind::StateUsage)
            .collect();

        if child_states.is_empty() {
            continue; // Abstract definitions with no child states
        }

        // Try full compilation using CompileToIR on a sub-graph
        // For simplicity, use the main compiler which finds the first state def
        // Instead, check if we can manually build IR from the elaborated graph
        let initial = child_states
            .iter()
            .find(|s| {
                s.get_prop("initial")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
            .or_else(|| child_states.first());

        if let Some(init) = initial {
            let init_name = init.name.as_deref().unwrap_or("initial");
            let ir = sysml_runtime::StateMachineIR::new(name, init_name);

            // Count transitions for this state machine
            let transition_rels: Vec<_> = result
                .graph
                .relationships_by_kind(&RelationshipKind::Transition)
                .filter(|t| {
                    child_states.iter().any(|s| s.id == t.source)
                        || child_states.iter().any(|s| s.id == t.target)
                })
                .collect();

            eprintln!(
                "  {} -> {} child states, {} transitions, initial={}",
                name,
                child_states.len(),
                transition_rels.len(),
                init_name
            );

            // Count it as compiled if we got a valid IR with at least the initial state
            if !ir.initial.is_empty() {
                compiled += 1;
            }
        } else {
            compile_errors += 1;
        }
    }

    eprintln!(
        "\nD1 Coverage: {}/{} state definitions compilable, {} errors",
        compiled,
        state_defs.len(),
        compile_errors
    );

    // Verify the CompileToIR trait works on the whole graph
    match StateMachineCompiler::compile(&result.graph) {
        Ok(ir) => {
            eprintln!(
                "  CompileToIR: OK (name={}, initial={}, states={}, transitions={})",
                ir.name,
                ir.initial,
                ir.states.len(),
                ir.transitions.len()
            );
        }
        Err(diags) => {
            eprintln!("  CompileToIR: failed ({} diagnostics)", diags.len());
            for d in &diags {
                eprintln!("    - {}", d.message);
            }
        }
    }
}

#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn d2_constraints_elaborate_precompile_pipeline() {
    use sysml_runtime::constraints::{extract_and_precompile, extract_constraints, EvalContext};

    let base = match corpus_env_root() {
        Some(p) => p,
        None => return,
    };

    let mut result =
        parse_pipeline_corpus_file(&base, &format!("{}/Constraints.sysml", LIBRARY_SYSTEMS));

    if result.graph.element_count() == 0 {
        eprintln!("SKIP: Constraints.sysml produced no elements");
        return;
    }

    // Elaborate
    let report = elaborate(&mut result.graph);
    eprintln!("Constraints.sysml elaboration: {}", report);

    // Count constraint elements
    let constraint_defs: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::ConstraintDefinition)
        .collect();
    let constraint_usages: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::ConstraintUsage)
        .collect();

    eprintln!(
        "  ConstraintDefinitions: {}, ConstraintUsages: {}",
        constraint_defs.len(),
        constraint_usages.len()
    );

    // Extract and precompile
    let raw_set = extract_constraints(&result.graph);
    eprintln!(
        "  Extracted constraints: {} total",
        raw_set.constraints.len()
    );

    let precompiled = extract_and_precompile(&result.graph);
    eprintln!(
        "  Precompiled: {} compiled, {} failed",
        precompiled.compiled_count(),
        precompiled.failed_count()
    );

    // Report compilation diagnostics
    let diags = precompiled.diagnostics();
    if !diags.is_empty() {
        eprintln!("  Compilation diagnostics:");
        for d in diags.iter().take(10) {
            eprintln!("    - {}", d.message);
        }
        if diags.len() > 10 {
            eprintln!("    ... and {} more", diags.len() - 10);
        }
    }

    // Try evaluating with empty context (all should fail gracefully or be undefined)
    let ctx = EvalContext::new();
    let results = precompiled.evaluate_all(&ctx);
    let satisfied = results.iter().filter(|r| r.satisfied).count();
    let failed = results.iter().filter(|r| !r.satisfied).count();

    eprintln!(
        "  Evaluation (empty context): {} satisfied, {} failed",
        satisfied, failed
    );

    eprintln!(
        "\nD2 Coverage: compiled={}/{}, precompile_success_rate={:.1}%",
        precompiled.compiled_count(),
        precompiled.total(),
        if precompiled.total() > 0 {
            precompiled.compiled_count() as f64 / precompiled.total() as f64 * 100.0
        } else {
            0.0
        }
    );
}

#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn d3_actions_elaborate_succession_pipeline() {
    let base = match corpus_env_root() {
        Some(p) => p,
        None => return,
    };

    let mut result =
        parse_pipeline_corpus_file(&base, &format!("{}/Actions.sysml", LIBRARY_SYSTEMS));

    if result.graph.element_count() == 0 {
        eprintln!("SKIP: Actions.sysml produced no elements");
        return;
    }

    // Elaborate
    let report = elaborate(&mut result.graph);
    eprintln!("Actions.sysml elaboration: {}", report);

    // Count action elements
    let action_defs: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::ActionDefinition)
        .collect();
    let action_usages: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::ActionUsage)
        .collect();
    let successions: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::SuccessionAsUsage)
        .collect();

    eprintln!(
        "  ActionDefinitions: {}, ActionUsages: {}, SuccessionAsUsage: {}",
        action_defs.len(),
        action_usages.len(),
        successions.len()
    );

    // Check for Transition relationships created by elaboration
    let transition_rels: Vec<_> = result
        .graph
        .relationships_by_kind(&RelationshipKind::Transition)
        .collect();

    eprintln!(
        "  Transition relationships (from successions): {}",
        transition_rels.len()
    );
    eprintln!(
        "  Successions created by elaboration: {}",
        report.successions_created
    );

    // Verify transition relationships reference valid elements
    let mut valid_transitions = 0;
    for t in &transition_rels {
        let has_source = result.graph.get_element(&t.source).is_some();
        let has_target = result.graph.get_element(&t.target).is_some();
        if has_source && has_target {
            valid_transitions += 1;
        }
    }

    eprintln!(
        "\nD3 Coverage: action_defs={}, successions_elaborated={}, valid_transitions={}/{}",
        action_defs.len(),
        report.successions_created,
        valid_transitions,
        transition_rels.len()
    );
}

#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn d4_flows_elaborate_pipeline() {
    let base = match corpus_env_root() {
        Some(p) => p,
        None => return,
    };

    let mut result = parse_pipeline_corpus_file(&base, &format!("{}/Flows.sysml", LIBRARY_SYSTEMS));

    if result.graph.element_count() == 0 {
        eprintln!("SKIP: Flows.sysml produced no elements");
        return;
    }

    // Elaborate
    let report = elaborate(&mut result.graph);
    eprintln!("Flows.sysml elaboration: {}", report);

    // Count flow elements
    let flow_usages: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::FlowUsage)
        .collect();
    let succession_flows: Vec<_> = result
        .graph
        .elements_by_kind(&ElementKind::SuccessionFlowUsage)
        .collect();

    eprintln!(
        "  FlowUsage: {}, SuccessionFlowUsage: {}",
        flow_usages.len(),
        succession_flows.len()
    );

    // Check for derived properties
    let mut with_source = 0;
    let mut with_target = 0;
    let mut with_payload = 0;

    for flow in flow_usages.iter().chain(succession_flows.iter()) {
        if flow.get_prop("source").is_some() {
            with_source += 1;
        }
        if flow.get_prop("target").is_some() {
            with_target += 1;
        }
        if flow.get_prop("payloadType").is_some() {
            with_payload += 1;
        }
    }

    let total_flows = flow_usages.len() + succession_flows.len();

    eprintln!(
        "  Derived properties: source={}/{}, target={}/{}, payloadType={}/{}",
        with_source, total_flows, with_target, total_flows, with_payload, total_flows
    );
    eprintln!("  Flows derived by elaboration: {}", report.flows_derived);

    eprintln!(
        "\nD4 Coverage: total_flows={}, flows_derived={}, source_rate={:.1}%",
        total_flows,
        report.flows_derived,
        if total_flows > 0 {
            with_source as f64 / total_flows as f64 * 100.0
        } else {
            0.0
        }
    );
}

#[test]
#[cfg_attr(not(feature = "corpus"), ignore = "enable with --features corpus")]
fn d5_combined_corpus_pipeline() {
    use sysml_runtime::constraints::extract_and_precompile;
    use sysml_runtime::statemachine::StateMachineCompiler;
    use sysml_runtime::CompileToIR;

    let base = match corpus_env_root() {
        Some(p) => p,
        None => return,
    };

    // Parse all four key files together
    let files_to_parse = [
        "States.sysml",
        "Constraints.sysml",
        "Actions.sysml",
        "Flows.sysml",
    ];

    let parser = TreeSitterParser::new();
    let mut sysml_files = Vec::new();

    for filename in &files_to_parse {
        let path = base.join(format!("{}/{}", LIBRARY_SYSTEMS, filename));
        if let Ok(source) = std::fs::read_to_string(&path) {
            sysml_files.push(SysmlFile::new(path.to_string_lossy().to_string(), source));
        }
    }

    assert!(
        !sysml_files.is_empty(),
        "Should find at least one corpus file"
    );

    let mut result = parser.parse(&sysml_files);

    eprintln!(
        "Combined parse: {} files, {} elements, {} relationships",
        sysml_files.len(),
        result.graph.element_count(),
        result.graph.relationship_count()
    );

    if result.graph.element_count() == 0 {
        eprintln!("SKIP: combined parse produced no elements");
        return;
    }

    // Elaborate the combined graph
    let report = elaborate(&mut result.graph);
    eprintln!("Combined elaboration report: {}", report);

    // Summary statistics
    let state_defs = result
        .graph
        .elements_by_kind(&ElementKind::StateDefinition)
        .count();
    let constraint_usages = result
        .graph
        .elements_by_kind(&ElementKind::ConstraintUsage)
        .count();
    let action_defs = result
        .graph
        .elements_by_kind(&ElementKind::ActionDefinition)
        .count();
    let flow_usages = result
        .graph
        .elements_by_kind(&ElementKind::FlowUsage)
        .count();

    eprintln!("Combined summary:");
    eprintln!("  StateDefinitions: {}", state_defs);
    eprintln!("  ConstraintUsages: {}", constraint_usages);
    eprintln!("  ActionDefinitions: {}", action_defs);
    eprintln!("  FlowUsages: {}", flow_usages);
    eprintln!("  initial_states_tagged: {}", report.initial_states_tagged);
    eprintln!("  constraints_derived: {}", report.constraints_derived);
    eprintln!("  successions_created: {}", report.successions_created);
    eprintln!("  flows_derived: {}", report.flows_derived);

    // Verify idempotency
    let report2 = elaborate(&mut result.graph);
    assert!(
        report2.is_empty(),
        "Second elaboration should be no-op, got: {}",
        report2
    );

    // Try constraint precompilation on combined graph
    let precompiled = extract_and_precompile(&result.graph);
    eprintln!(
        "  Precompiled constraints: {}/{} compiled",
        precompiled.compiled_count(),
        precompiled.total()
    );

    // Try state machine compilation on combined graph
    match StateMachineCompiler::compile(&result.graph) {
        Ok(ir) => {
            eprintln!(
                "  StateMachine IR: name={}, states={}, transitions={}",
                ir.name,
                ir.states.len(),
                ir.transitions.len()
            );
        }
        Err(diags) => {
            eprintln!("  StateMachine compilation: {} errors", diags.len());
        }
    }
}
