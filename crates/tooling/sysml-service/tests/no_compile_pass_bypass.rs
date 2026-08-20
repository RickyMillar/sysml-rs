//! RSC-6.5 consistency-sweep guard.
//!
//! Service commands must acquire graphs through the salsa-memoized accessors —
//! `require_graph` (parse-only), `elaborated_graph` (resolve+elaborate a single
//! file via `elaborate_file_best`), or `workspace_aware_graph` (the workspace
//! query). They must NOT re-run the compile passes (`elaborate`,
//! `resolve_references`) by hand: those are salsa-tracked queries, and a manual
//! call both bypasses memoization (perf DNA) and risks operating on an
//! UNRESOLVED graph — the exact L13 defect that `export_smodel` / `views_render`
//! once carried (ledger L13; both now route through the accessors).
//!
//! This test scans `sysml-service/src/` and fails if any production line calls
//! a raw compile pass. Test code (the bottom `#[cfg(test)] mod tests` of each
//! file) legitimately builds graphs by hand for fixtures, so a hit is a
//! violation only when it precedes the file's first `#[cfg(test)]` marker —
//! this crate follows the colocated-tests-at-the-bottom convention throughout.

use std::path::{Path, PathBuf};

/// Patterns that name a salsa-owned compile pass being run directly. Each has a
/// tracked-query home (`elaborate_file_best` / `elaborate_workspace_best` /
/// `resolve_file_best`) that service code must route through instead.
const PROHIBITED: &[&str] = &[
    "elaborate::elaborate(",
    "elaborate_with_library(",
    "resolve_references(",
    // Constraint precompilation is a salsa-memoized walk: production code must
    // route through `workspace_precompiled_constraints` (which wraps the tracked
    // `file_precompiled_constraints` / `workspace_precompiled_constraints_best`
    // queries), never the raw runtime primitive.
    "extract_and_precompile(",
];

/// Recursively collect `*.rs` files under `dir`.
fn rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn no_production_compile_pass_bypass() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(src_dir.exists(), "src/ not found at {:?}", src_dir);

    let mut files = Vec::new();
    rs_files(&src_dir, &mut files);
    assert!(!files.is_empty(), "no source files scanned");

    let mut violations: Vec<String> = Vec::new();

    for file in &files {
        let text = std::fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("read {}: {e}", file.display()));

        // Everything from the first `#[cfg(test)]` onward is the test module
        // (colocated-at-bottom convention). Production code is strictly above it.
        let first_test_line = text
            .lines()
            .position(|l| l.trim_start().starts_with("#[cfg(test)]"));

        for (idx, line) in text.lines().enumerate() {
            if let Some(test_start) = first_test_line {
                if idx >= test_start {
                    break; // reached the test module — stop scanning this file
                }
            }
            // Ignore comment lines (doc/// references to the pattern, like this).
            let trimmed = line.trim_start();
            if trimmed.starts_with("//") {
                continue;
            }
            if let Some(pat) = PROHIBITED.iter().find(|p| line.contains(**p)) {
                violations.push(format!(
                    "{}:{}: {}  [matched `{pat}`]",
                    file.file_name().unwrap().to_string_lossy(),
                    idx + 1,
                    line.trim()
                ));
            }
        }
    }

    assert!(
        violations.is_empty(),
        "RSC-6.5: {} service production site(s) run a raw compile pass instead of \
         routing through a salsa accessor (require_graph / elaborated_graph / \
         workspace_aware_graph):\n\n{}\n\nFix: acquire the graph via the accessor \
         that matches the need; never call elaborate()/resolve_references() in a \
         service command.",
        violations.len(),
        violations.join("\n"),
    );
}
