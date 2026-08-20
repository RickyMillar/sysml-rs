//! Grep-gate regression test: ensures production code does not read legacy
//! string props (`unresolved_value`) as expression sources.
//!
//! After Phase 6D, neither parser writes `unresolved_value`. Any production
//! code still reading it is wasted work or a latent bug. This test scans
//! `crates/*/src/` for prohibited patterns and fails if any are found.
//!
//! **Allowed**:
//! - `compile_simple_expression` — documented Synthesized Expression API
//! - `compile_expression` / `compile_expression_ast` — AST-first API
//! - Test files (`crates/*/tests/`, `#[cfg(test)]` blocks)
//! - The `ast_builder.rs` test assertions (negative assertions on `unresolved_value`)
//! - The `elaborate/` directory (writes constraint prop, reads for elaboration)
//! - This test file itself

use std::path::Path;
use std::process::Command;

/// Scan production source files for `get_prop("unresolved_value")` calls that
/// are NOT inside test modules or the documented exception list.
#[test]
fn no_production_unresolved_value_readers() {
    // CARGO_MANIFEST_DIR = .../crates/lang/sysml-runtime
    // Walk up to workspace root: sysml-runtime -> lang -> crates -> workspace root
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent() // lang
        .unwrap()
        .parent() // crates
        .unwrap()
        .parent() // workspace root (contains crates/)
        .unwrap();

    let crates_dir = workspace_root.join("crates");
    assert!(
        crates_dir.exists(),
        "crates/ directory not found at {:?}",
        crates_dir
    );

    // Use grep to find `get_prop("unresolved_value")` in production src/ files.
    // Exclude test files and known-OK locations.
    let output = Command::new("grep")
        .args(["-rn", "--include=*.rs", r#"get_prop("unresolved_value")"#])
        .arg(crates_dir.to_str().unwrap())
        .output()
        .expect("failed to run grep");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Filter to only production code (src/ directories), excluding:
    // - test files (*/tests/*)
    // - test modules (#[cfg(test)]) — we can't grep for this, but the file filter helps
    // - ast_builder.rs (negative assertions checking that unresolved_value is NOT set)
    // - elaborate/ (the elaboration bridge legitimately reads/writes during elaboration)
    // - dimension.rs (ISQ dimension extraction — reads value first, unresolved_value as fallback for library elements)
    // - this test file
    let violations: Vec<&str> = stdout
        .lines()
        .filter(|line| {
            // Must be in a src/ directory (production code)
            line.contains("/src/")
        })
        .filter(|line| {
            // Exclude known-OK locations
            let excluded_patterns = [
                "ast_builder.rs",         // negative assertions (legacy single-file form)
                "/ast_builder/", // negative assertions (current module form: ast_builder/tests.rs etc.)
                "/elaborate/",   // elaboration bridge
                "dimension.rs",  // ISQ dimension extraction (value-first fallback)
                "physics/constraints.rs", // RSC-1.6 sanctioned fallback: negative numeric
                // DEFAULTS land in `unresolved_value` strings
                // (parser pitfall; flow_feature_declared_default
                // tries typed `default`/`value` props FIRST)
                "no_legacy_string_readers", // this test
            ];
            !excluded_patterns.iter().any(|pat| line.contains(pat))
        })
        .filter(|line| {
            // Exclude lines that are clearly inside #[cfg(test)] modules.
            // Heuristic: if the line is in a mod tests block, skip it.
            // This is imperfect but catches the common case.
            !line.contains("#[cfg(test)]")
        })
        .collect();

    if !violations.is_empty() {
        let count = violations.len();
        let details = violations.join("\n");
        panic!(
            "Found {} production code site(s) reading `unresolved_value` \
             (writers removed in Phase 6D):\n\n{}\n\n\
             Fix: use `compile_expression(element, graph)` for model expressions, \
             or `compile_simple_expression(str)` for synthesized expression strings.",
            count, details
        );
    }
}

/// Verify that `compile_simple_expression` is still exported as the documented
/// Synthesized Expression API.
#[test]
fn synthesized_expression_api_exists() {
    // This test ensures the API exists and compiles.
    let ir = sysml_runtime::expressions::compile_simple_expression("x + 1");
    assert!(ir.is_ok(), "compile_simple_expression should parse 'x + 1'");
}

/// Verify that `compile_expression` is still exported as the primary
/// model expression API.
#[test]
fn model_expression_api_exists() {
    // compile_expression requires an Element and ModelGraph — just check the
    // function is importable (type-level check).
    let _fn_ref: fn(
        &sysml_core::Element,
        &sysml_core::ModelGraph,
    )
        -> Result<sysml_runtime::expressions::ExprIR, Vec<sysml_span::Diagnostic>> =
        sysml_runtime::expressions::compile_expression;
    let _ = _fn_ref; // suppress unused
}
