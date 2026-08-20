//! Canonical example-loading harness for `sysml-runtime` integration tests
//! and diagnostics.
//!
//! WHY THIS EXISTS (Task #10 write-leak arc, Jul 2026): a hand-rolled
//! diagnostic once reimplemented parse → elaborate → compile → orchestrator
//! setup for an example workspace from scratch and produced a fully spurious
//! verdict — a complete sign/phase flip at tick 1 — against a blessed golden
//! trajectory for the SAME model, and nearly got an innocent fix reverted.
//! Root cause was two silent divergences from the blessed setup:
//!   1. `dt_ms`/`max_time_ms` were passed as `None, None` instead of the
//!      fixture's intended window, so the orchestrator silently built against
//!      the 1ms/60s *defaults* instead.
//!   2. Its own file-collection helper (copied from the blessed one) walked
//!      `read_dir` in the OS's return order.
//! A follow-up survey found divergence #2 was not unique to that diagnostic:
//! every hand-rolled integration test that loaded an `examples/<name>`
//! directory collected `.sysml` files WITHOUT sorting them. `read_dir` order
//! is filesystem/OS-dependent, not alphabetical, and ElementIds are minted in
//! file-then-declaration order — so any such test could assign IDs in a
//! different order than a baseline recorded against the sorted, blessed path,
//! and diverge for a reason that has nothing to do with the code under test.
//!
//! RULE: a harness divergence produces a verdict that is inadmissible as
//! evidence of a real regression — "fail hard, not fail differently." Any
//! test or diagnostic that loads a workspace example MUST go through this
//! module's `load_dir` / `load_example_graph` / `load_example_orchestrator`
//! — never a hand-rolled copy of `read_dir` + `TreeSitterParser` +
//! `elaborate`. If a fixture needs a shape this module doesn't support, add
//! a parameter here; don't fork the loader.

// Each test binary that does `mod common;` only calls the subset of these
// helpers it needs — cargo builds `common` fresh per binary, so unused
// functions warn per-binary. Standard shared-test-module idiom.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sysml_core::{elaborate, ModelGraph};
use sysml_ide_db::eval_context_seed;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::orchestrator::Orchestrator;

/// Resolve an `examples/<name>` directory relative to this crate's manifest.
pub fn workspace_example_dir(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples")
        .join(name)
}

/// Recursively collect every `.sysml` file under `dir`, SORTED for
/// deterministic parse/elaborate order. This is the one line every
/// non-blessed test was missing — see the module doc comment.
fn collect_sysml_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                collect_sysml_files(&path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("sysml") {
                out.push(path);
            }
        }
    }
}

/// Parse + elaborate every `.sysml` file under `dir` into one `ModelGraph`,
/// with a stable file-set, order, parse, and elaborate call sequence.
pub fn load_dir(dir: &Path) -> ModelGraph {
    assert!(dir.exists(), "example dir not found: {}", dir.display());
    let mut files = Vec::new();
    collect_sysml_files(dir, &mut files);
    assert!(!files.is_empty(), "no .sysml files in {}", dir.display());
    let parser = TreeSitterParser::new();
    let inputs: Vec<SysmlFile> = files
        .iter()
        .map(|p| {
            let src = std::fs::read_to_string(p).unwrap();
            SysmlFile::new(p.file_name().unwrap().to_str().unwrap().to_owned(), src)
        })
        .collect();
    let mut graph = parser.parse(&inputs).graph;
    elaborate::elaborate(&mut graph);
    graph
}

/// `load_dir` for a named `examples/<name>` subdirectory.
pub fn load_example_graph(name: &str) -> ModelGraph {
    load_dir(&workspace_example_dir(name))
}

/// Build a workspace `Orchestrator` for `examples/<name>` through the
/// canonical `ModelCompiler` pipeline: elaborate (via `load_dir`) →
/// `ModelCompiler::new().with_source_dir()`
/// → `context_from_graph` seed → `extract_and_precompile` constraints →
/// `build_workspace_orchestrator`.
///
/// `overrides` are `(qualified_name, literal)` pairs, applied the same way
/// `build_workspace_orchestrator` takes them natively. `dt_ms` / `max_time_ms`
/// are REQUIRED, not optional-with-a-silent-default: the write-leak incident
/// happened in part because a caller passed `None, None` and silently got the
/// 1ms/60s fallback instead of the fixture's intended window. Forcing every
/// caller to state them here makes that divergence impossible to repeat.
///
/// `.with_source_dir(&dir)` is always applied (needed for `@DataSource`
/// resolution on fixtures that sample external data; a no-op for fixtures that
/// don't use it) — one call shape for every example, no divergence knob.
pub fn load_example_orchestrator(
    example_dir: &str,
    overrides: &[(String, String)],
    dt_ms: f64,
    max_time_ms: f64,
) -> Orchestrator {
    let dir = workspace_example_dir(example_dir);
    let graph = load_dir(&dir);
    let compiler = ModelCompiler::new(graph).with_source_dir(&dir);
    let base_ctx = eval_context_seed::context_from_graph(compiler.graph());
    let precompiled = Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));
    compiler
        .build_workspace_orchestrator(
            base_ctx,
            Some(precompiled),
            None,
            None,
            None,
            overrides,
            Some(dt_ms),
            Some(max_time_ms),
        )
        .expect("workspace orchestrator should build")
}
