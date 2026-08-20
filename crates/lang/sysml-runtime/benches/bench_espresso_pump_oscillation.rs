//! Benchmark: espresso-pump-hybrid full-cycle oscillation.
//!
//! Measures hybrid SM + ODE overhead by running the espresso pump oscillator
//! model through a full oscillation cycle (~2000 ticks at 2ms step). The model
//! features a state machine (PumpCycle) driving an actuator ODE with
//! zero-crossing-located transitions, exercising the full mixed-mode execution
//! path (PERF-STEP). Clean-room replacement for the former oscillator-fixture
//! oscillation bench; physics/bounds are derived in
//! examples/espresso-pump-hybrid/README.md.

use std::path::PathBuf;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sysml_core::ModelGraph;
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;

/// Path to the espresso-pump-hybrid example directory.
fn pump_example_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("..")
        .join("examples")
        .join("espresso-pump-hybrid")
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

/// Load and parse all .sysml files from the pump example directory.
fn load_pump_model() -> ModelGraph {
    let dir = pump_example_dir();
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

fn bench_pump_full_oscillation(c: &mut Criterion) {
    let graph = load_pump_model();
    let dir = pump_example_dir();
    let compiler = ModelCompiler::new(graph).with_source_dir(&dir);
    let base_ctx = sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
    let precompiled = std::sync::Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));

    // dt = 2 ms, horizon = 4000 ms => ~2000 ticks (a full hybrid oscillation cycle).
    let test_orch = compiler.build_workspace_orchestrator(
        base_ctx.alias_live(),
        Some(std::sync::Arc::clone(&precompiled)),
        None,
        None,
        None,
        &[],
        Some(2.0),
        Some(4000.0),
    );
    if let Err(e) = &test_orch {
        eprintln!("SKIP: espresso pump orchestrator failed to compile: {e}");
        return;
    }

    let mut group = c.benchmark_group("runtime/espresso_pump_oscillation");
    group.sample_size(10);

    group.bench_function("full_cycle_2000_ticks", |b| {
        b.iter_batched(
            || {
                compiler
                    .build_workspace_orchestrator(
                        base_ctx.alias_live(),
                        Some(std::sync::Arc::clone(&precompiled)),
                        None,
                        None,
                        None,
                        &[],
                        Some(2.0),
                        Some(4000.0),
                    )
                    .expect("orchestrator should compile")
            },
            |mut orch| {
                for _ in 0..2000 {
                    black_box(orch.step());
                }
            },
            criterion::BatchSize::PerIteration,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_pump_full_oscillation);
criterion_main!(benches);
