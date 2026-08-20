// Test crate: these patterns are legitimate in test/coverage tooling
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::str_to_string,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::panic
)]

//! # sysml-spec-tests
//!
//! Spec-conformance and regression primitives for the SysML v2 implementation.
//!
//! This crate tracks how much of the SysML v2 language specification is covered
//! by the tree-sitter parser ([`sysml_parser_incremental::TreeSitterParser`], the
//! sole parser, which implements [`sysml_parser_trait::Parser`]). The heavy
//! integration tests (corpus, cross-transport identity, service-command
//! baselines, pipeline, pilot-dump equivalence) live in `tests/`.
//!
//! ## Coverage Dimensions
//!
//! - **Corpus Files**: Parse real-world .sysml files from reference materials
//! - **ElementKinds**: Verify all parseable types are produced
//! - **Operators**: Validate operators against the xtext grammar (via codegen)
//! - **Tree-sitter Validation**: Validate the tree-sitter grammar against spec
//!
//! ## Usage
//!
//! Tests are `#[ignore]` by default and enabled via environment variable:
//!
//! ```bash
//! # Enable corpus tests
//! SYSML_CORPUS_PATH=/path/to/references/sysmlv2 cargo test -p sysml-spec-tests -- --ignored
//! ```

pub mod candidate_report;
pub mod corpus;
pub mod element_coverage;
pub mod eval_pipeline;
pub mod path_canon;
pub mod report;
pub mod treesitter_validation;

use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn resolve_env_path(env_path: &str) -> Option<PathBuf> {
    let candidate = PathBuf::from(env_path);
    if candidate.exists() {
        return Some(candidate);
    }

    if candidate.is_relative() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        if let Some(repo_root) = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
        {
            let joined = repo_root.join(&candidate);
            if joined.exists() {
                return Some(joined);
            }
        }
    }

    None
}

/// Resolve `SYSML_CORPUS_PATH` into a corpus root, resolving a **relative**
/// value against the workspace root (`CARGO_MANIFEST_DIR/../../..`) rather than
/// the crate's cwd.
///
/// Returns `None` when the env var is unset, preserving the "absent → skip"
/// gating used by the env-gated corpus rows. This is the single home for the
/// robustness fix that the old `pipeline_corpus_tests` lacked: it joined the
/// raw relative env value against the crate directory (where `cargo test` runs),
/// so the relative invocation documented in the crate README silently resolved
/// to a non-existent path and the pipeline rows skipped instead of running.
pub fn corpus_env_root() -> Option<PathBuf> {
    let raw = std::env::var("SYSML_CORPUS_PATH").ok()?;
    Some(resolve_env_path(&raw).unwrap_or_else(|| PathBuf::from(raw)))
}

/// Locate the sysmlv2 references directory.
///
/// Searches in order:
/// 1. SYSML_CORPUS_PATH environment variable
/// 2. SYSML_REFS_DIR environment variable
/// 3. Common relative paths (references/sysmlv2)
///
/// # Panics
///
/// Panics if the references directory cannot be found. This ensures we fail fast
/// with a clear error message rather than silently using stale fallback data.
pub fn find_references_dir() -> PathBuf {
    // Check environment variable first
    if let Ok(env_path) = std::env::var("SYSML_CORPUS_PATH") {
        if let Some(path) = resolve_env_path(&env_path) {
            return path;
        }
    }

    if let Ok(env_path) = std::env::var("SYSML_REFS_DIR") {
        if let Some(path) = resolve_env_path(&env_path) {
            return path;
        }
    }

    // Try common relative paths
    let candidates = [
        "../../../references/sysmlv2",
        "../../references/sysmlv2",
        "../references/sysmlv2",
        "references/sysmlv2",
    ];

    for candidate in candidates {
        let path = Path::new(candidate);
        if path.exists() && path.join("SysML-vocab.ttl").exists() {
            return path.to_path_buf();
        }
    }

    panic!(
        "Could not find sysmlv2 references directory.\n\
         Set SYSML_CORPUS_PATH or SYSML_REFS_DIR, or ensure references/sysmlv2 is available.\n\
         Searched: {:?}",
        candidates
    );
}

/// Check if the references directory is available (without panicking).
///
/// Use this in tests that want to skip gracefully when spec files aren't available.
pub fn try_find_references_dir() -> Option<PathBuf> {
    // Check environment variable first
    if let Ok(env_path) = std::env::var("SYSML_CORPUS_PATH") {
        if let Some(path) = resolve_env_path(&env_path) {
            return Some(path);
        }
    }

    if let Ok(env_path) = std::env::var("SYSML_REFS_DIR") {
        if let Some(path) = resolve_env_path(&env_path) {
            return Some(path);
        }
    }

    // Try common relative paths
    let candidates = [
        "../../../references/sysmlv2",
        "../../references/sysmlv2",
        "../references/sysmlv2",
        "references/sysmlv2",
    ];

    for candidate in candidates {
        let path = Path::new(candidate);
        if path.exists() && path.join("SysML-vocab.ttl").exists() {
            return Some(path.to_path_buf());
        }
    }

    None
}

/// Configuration for coverage tests.
#[derive(Debug, Clone)]
pub struct CoverageConfig {
    /// Path to the SysML v2 references directory.
    pub corpus_path: PathBuf,
    /// Paths within corpus to search for .sysml files.
    pub corpus_subdirs: Vec<&'static str>,
}

impl CoverageConfig {
    /// Create a new configuration from environment variable.
    ///
    /// Returns `None` if `SYSML_CORPUS_PATH` is not set.
    pub fn from_env() -> Option<Self> {
        let corpus_path = std::env::var("SYSML_CORPUS_PATH").ok()?;
        let corpus_path =
            resolve_env_path(&corpus_path).unwrap_or_else(|| PathBuf::from(corpus_path));
        Some(CoverageConfig {
            corpus_path: PathBuf::from(corpus_path),
            corpus_subdirs: vec![
                // Standard library (21 files)
                "SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.systems",
                // Example models
                "SysML-v2-Models/models",
            ],
        })
    }

    /// Create configuration for local development (relative path).
    pub fn local_dev() -> Self {
        CoverageConfig {
            corpus_path: PathBuf::from("../references/sysmlv2"),
            corpus_subdirs: vec![
                "SysML-v2-Pilot-Implementation/org.omg.sysml.xpect.tests/library.systems",
                "SysML-v2-Models/models",
            ],
        }
    }
}

/// Result of parsing a single corpus file.
#[derive(Debug, Clone)]
pub struct FileParseResult {
    /// Relative path to the file.
    pub path: String,
    /// Whether parsing succeeded.
    pub success: bool,
    /// Error messages if parsing failed.
    pub errors: Vec<String>,
    /// Number of elements produced.
    pub element_count: usize,
}

/// Summary of corpus coverage.
#[derive(Debug, Clone, Default)]
pub struct CoverageSummary {
    /// Total files attempted.
    pub total_files: usize,
    /// Successfully parsed files.
    pub passed_files: usize,
    /// Failed files.
    pub failed_files: usize,
    /// Files in allow-list that failed (expected).
    pub expected_failures: usize,
    /// Files NOT in allow-list that failed (unexpected).
    pub unexpected_failures: usize,
    /// Grammar rules exercised.
    pub rules_exercised: HashSet<String>,
    /// Element kinds produced.
    pub element_kinds_produced: HashSet<String>,
}

impl CoverageSummary {
    /// Get the pass percentage.
    pub fn pass_percentage(&self) -> f64 {
        if self.total_files == 0 {
            0.0
        } else {
            (self.passed_files as f64 / self.total_files as f64) * 100.0
        }
    }
}

/// Load the allow-list of expected failures from a file.
pub fn load_allow_list(content: &str) -> HashSet<String> {
    content
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .map(|line| line.trim().to_string())
        .collect()
}

/// Load the list of constructible kinds from a file.
pub fn load_constructible_kinds(content: &str) -> HashSet<String> {
    content
        .lines()
        .filter(|line| !line.trim().is_empty() && !line.trim().starts_with('#'))
        .map(|line| line.trim().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_allow_list_basic() {
        let content = r#"
# This is a comment
file1.sysml
file2.sysml

# Another comment
file3.sysml
"#;
        let list = load_allow_list(content);
        assert_eq!(list.len(), 3);
        assert!(list.contains("file1.sysml"));
        assert!(list.contains("file2.sysml"));
        assert!(list.contains("file3.sysml"));
    }

}
