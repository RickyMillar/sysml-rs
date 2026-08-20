//! Benchmark: `sysml.constraint.check` per-call cost after S3.T13 perf-wire.
//!
//! Post-S3.T13 (commit `553b0833`) `check_constraints` routes through the
//! salsa-cached `workspace_eval_context_with_library` query. The
//! ~14k-binding `EvalContext` seed walk is built once per graph revision
//! and reused on every subsequent call — per ADR-011 §3 the per-eval
//! command headline payoff.
//!
//! This bench measures the warm-call cost on the espresso-production-cell
//! workspace (stdlib-heavy: ISQ + SI + ScalarValues populate the seed
//! with thousands of `Value::Ref` bindings). The model is loaded once
//! outside the bench loop; the first `check_constraints` call primes the
//! salsa cache; subsequent calls measured by criterion are pure cache
//! hits on the context construction side.
//!
//! Four benches:
//!
//! - `eval_context_cached` — `service.eval_context_with_overrides(uri, &[])`,
//!   the salsa-cached path. Isolates the seed-walk caching effect:
//!   this is the operation the S3.T13 perf-wire moved.
//! - `seed_walk_uncached` — `sysml_ide_db::eval_context_seed::context_from_graph`
//!   called directly on the elaborated workspace graph each iteration.
//!   Represents the pre-fix per-call cost of context construction; the
//!   delta vs `eval_context_cached` is the perf payoff.
//! - `check_constraints_warm` — full `service.check_constraints` call.
//!   Dominated by per-call constraint-precompile + per-constraint
//!   scoped-ctx clone + evaluate — those steps are not cached.
//! - `check_constraints_with_overrides_warm` — same with one override;
//!   confirms overrides don't bust the salsa cache.

use std::path::PathBuf;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sysml_service::SysmlService;

fn espresso_cell_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("..")
        .join("examples")
        .join("espresso-production-cell")
}

fn load_service() -> SysmlService {
    let service = SysmlService::empty();
    service
        .load_workspace(&espresso_cell_dir())
        .expect("load espresso-production-cell");
    service
}

fn bench_eval_context_cached(c: &mut Criterion) {
    let service = load_service();
    let _ = service
        .eval_context_with_overrides(&[])
        .expect("warmup");

    c.bench_function("service/eval_context_cached", |b| {
        b.iter(|| {
            black_box(
                service
                    .eval_context_with_overrides(&[])
                    .expect("eval_context_with_overrides"),
            );
        });
    });
}

fn bench_seed_walk_uncached(c: &mut Criterion) {
    let service = load_service();
    // Reach the elaborated workspace graph the same way `check_constraints`
    // does. Cached once outside the bench loop; the bench timed below is
    // the per-call seed-walk cost on this graph, with no salsa caching.
    let cached_ctx = service
        .eval_context_with_overrides(&[])
        .expect("eval_context_with_overrides");
    let graph = cached_ctx
        .graph
        .as_ref()
        .expect("EvalContext has graph attached")
        .clone();

    c.bench_function("service/seed_walk_uncached", |b| {
        b.iter(|| {
            black_box(sysml_ide_db::eval_context_seed::context_from_graph(
                &graph,
            ));
        });
    });
}

fn bench_check_constraints_warm(c: &mut Criterion) {
    let service = load_service();
    // Prime the salsa cache.
    let _ = service
        .check_constraints("__workspace__", &[])
        .expect("warmup");

    c.bench_function("service/check_constraints_warm", |b| {
        b.iter(|| {
            black_box(
                service
                    .check_constraints("__workspace__", &[])
                    .expect("check_constraints"),
            );
        });
    });
}

fn bench_check_constraints_with_overrides_warm(c: &mut Criterion) {
    let service = load_service();
    let overrides = vec![("dummy_var".to_string(), "1.0".to_string())];
    let _ = service
        .check_constraints("__workspace__", &overrides)
        .expect("warmup");

    c.bench_function("service/check_constraints_with_overrides_warm", |b| {
        b.iter(|| {
            black_box(
                service
                    .check_constraints("__workspace__", &overrides)
                    .expect("check_constraints"),
            );
        });
    });
}

/// Measure what `sysml.orchestrate.workspace.start` pays per call after
/// the S3.T13 perf-wire (commit `a618039a`) cached the seed walk.
///
/// Calls `Snapshot::new(graph) + build_workspace_orchestrator(base_ctx, ...)`
/// directly so the bench doesn't allocate session-cap slots each iteration
/// (the service command would otherwise hit `MAX_SESSIONS` quickly). Both
/// `graph` (Arc clone, free) and `base_ctx` (Arc-wrapped HashMap clone,
/// free) are reused across iterations — same shape as the cached path
/// the service command now takes.
fn bench_orchestrate_workspace_start_warm(c: &mut Criterion) {
    let service = load_service();
    let graph = service
        .eval_context_with_overrides(&[])
        .expect("warmup eval_context")
        .graph
        .as_ref()
        .expect("EvalContext has graph attached")
        .clone();
    // `base_ctx` is fetched fresh per iteration below: `EvalContext` is no
    // longer `Clone` (copy sites must choose `alias_live` vs `scratch_snapshot`),
    // and `build_workspace_orchestrator` needs a WRITABLE context to mint slots,
    // which the read-only `scratch_snapshot` cannot provide. The fetch is a
    // salsa cache hit (the seed walk is memoised — see `bench_eval_context_cached`),
    // so the timed cost is still dominated by the orchestrator build.

    let precompiled = service
        .workspace_precompiled_constraints()
        .expect("precompiled constraints");
    let port_flow = service
        .workspace_port_flow_resources()
        .expect("port_flow resources");
    let gated = service
        .workspace_gated_expressions()
        .expect("gated expressions");
    let ref_cache = service
        .workspace_ref_resolve_cache()
        .expect("ref resolve cache");

    c.bench_function("service/orchestrate_workspace_start_warm", |b| {
        b.iter(|| {
            // Thread the source dir so the cell's `@DataSource` SampledFunctions
            // (data/generated_ambient.csv, generated_demand.csv) resolve during
            // the orchestrator build — the espresso cell samples external data
            // where the old multi-circuit fixture had none.
            let snap = sysml_ide_db::Snapshot::new(graph.clone())
                .with_source_dir(espresso_cell_dir());
            let base_ctx = service
                .eval_context_with_overrides(&[])
                .expect("eval_context (salsa cache hit)");
            black_box(
                snap.build_workspace_orchestrator(
                    base_ctx,
                    Some(precompiled.clone()),
                    Some(port_flow.clone()),
                    Some(gated.clone()),
                    Some(ref_cache.clone()),
                    &[],
                    None,
                    None,
                )
                .expect("orchestrator should build"),
            );
        });
    });
}

criterion_group!(
    benches,
    bench_eval_context_cached,
    bench_seed_walk_uncached,
    bench_check_constraints_warm,
    bench_orchestrate_workspace_start_warm,
    bench_check_constraints_with_overrides_warm
);
criterion_main!(benches);
