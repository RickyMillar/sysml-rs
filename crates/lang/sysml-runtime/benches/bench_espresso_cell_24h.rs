//! Benchmark: espresso-production-cell warm-start + 1000-tick simulation.
//!
//! Measures steady-state orchestrator throughput after compilation is
//! complete on a multi-instance workspace (PERF-STEP / PERF-MEM). The model is
//! loaded and compiled once (outside the bench loop), then 1000 ticks are
//! stepped per iteration. Clean-room replacement for the former
//! multi-subsystem 24h bench.

use std::path::PathBuf;
use std::sync::Arc;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sysml_core::ModelGraph;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;

/// Path to the espresso-production-cell example directory.
fn cell_example_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("..")
        .join("examples")
        .join("espresso-production-cell")
}

/// Recursively collect all .sysml files from a directory (sorted for
/// deterministic parse/elaborate order).
fn collect_sysml_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
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

/// Load and parse all .sysml files from the espresso cell directory.
fn load_cell() -> ModelGraph {
    let dir = cell_example_dir();
    let mut sysml_files = Vec::new();
    collect_sysml_files(&dir, &mut sysml_files);

    let parser = TreeSitterParser::new();
    let files: Vec<SysmlFile> = sysml_files
        .iter()
        .map(|path| {
            let source = std::fs::read_to_string(path).unwrap();
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            SysmlFile::new(name, source)
        })
        .collect();

    let result = parser.parse(&files);
    let mut graph = result.graph;
    sysml_core::elaborate::elaborate(&mut graph);
    graph
}

fn bench_cell_1000_ticks(c: &mut Criterion) {
    let dir = cell_example_dir();
    let graph = load_cell();
    let compiler = ModelCompiler::from_arc(Arc::new(graph)).with_source_dir(&dir);
    let base_ctx = sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
    let precompiled = Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));

    // Pre-build once to verify it works (skip bench if model doesn't compile).
    let test_orch = compiler.build_workspace_orchestrator(
        base_ctx.alias_live(),
        Some(Arc::clone(&precompiled)),
        None,
        None,
        None,
        &[],
        Some(100.0),
        Some(1_000_000.0),
    );
    if let Err(e) = &test_orch {
        eprintln!("SKIP: espresso cell orchestrator failed to compile: {e}");
        return;
    }

    let mut group = c.benchmark_group("runtime/espresso_cell");
    group.sample_size(10); // Each iteration is expensive

    group.bench_function("warm_start_1000_ticks", |b| {
        b.iter_batched(
            || {
                // Setup: build a fresh orchestrator (warm start — model already parsed).
                compiler
                    .build_workspace_orchestrator(
                        base_ctx.alias_live(),
                        Some(Arc::clone(&precompiled)),
                        None,
                        None,
                        None,
                        &[],
                        Some(100.0),
                        Some(1_000_000.0),
                    )
                    .expect("orchestrator should compile")
            },
            |mut orch| {
                // Measured: step 1000 ticks.
                for _ in 0..1000 {
                    black_box(orch.step());
                }
            },
            criterion::BatchSize::PerIteration,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_cell_1000_ticks);
criterion_main!(benches);
