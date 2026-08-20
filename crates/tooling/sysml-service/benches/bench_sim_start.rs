//! Benchmark: `sysml.simulate.start` and per-tick / per-diagnostics cost.
//!
//! Stands up the per-tick microbench harness anticipated by ADR-011 §3 / S3.T3.
//! Anchors the M3 = 39.9 ms `simulate.start` median; surfaces per-tick T14
//! `ref_resolve_cache` cache-hit speedup and per-diagnostics T11 physics-health
//! cache-hit speedup (both invisible to `bench_orchestrate_workspace_start_warm`
//! since the orchestrator-build hot path doesn't touch those caches).
//!
//! Three benches on the espresso-pump-hybrid workspace:
//!
//! - `sim_start_warm` — mirrors the inline body of `SysmlService::simulate_start`
//!   *minus* `insert_session` (so the bench loop doesn't exhaust
//!   `execution::MAX_SESSIONS`). Each iteration constructs a fresh
//!   `Snapshot`, compiles the state machine, builds a single-SM
//!   `Orchestrator`, wraps it in `RuntimeSession`, and steps once. The
//!   workspace graph is cached outside the loop — same shape as
//!   `bench_orchestrate_workspace_start_warm`. Anchors ADR-011 M3 = 39.9 ms.
//! - `sim_step_warm` — builds + primes the session once outside the loop,
//!   then measures one `RuntimeSession::step` per iteration. T14's
//!   `ref_resolve_cache` snapshot-scoped hits land here (per-step
//!   expression resolution reuses the workspace-scoped compiled
//!   `ExprIR` cache).
//! - `diagnostics_warm` — primes the diagnostics pipeline, then
//!   measures one `SysmlService::diagnostics` call per iteration.
//!   T11's physics-health tracked-query cache-hit speedup lives here.

use std::path::PathBuf;

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sysml_service::SysmlService;

/// Workspace state machine to drive sim-start/step. `PumpCycle` is the
/// reciprocating-pump lifecycle SM in the espresso-pump-hybrid fixture
/// (examples/espresso-pump-hybrid/Behaviour/PumpCycle.sysml).
const SM_NAME: &str = "PumpCycle";

fn espresso_pump_dir() -> PathBuf {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .join("..")
        .join("..")
        .join("..")
        .join("examples")
        .join("espresso-pump-hybrid")
}

fn load_service() -> SysmlService {
    let service = SysmlService::empty();
    service
        .load_workspace(&espresso_pump_dir())
        .expect("load espresso-pump-hybrid");
    service
}

/// Workspace graph fetched the same way the orchestrate bench does it —
/// via `eval_context_with_overrides`, which exposes the underlying
/// elaborated `Arc<ModelGraph>` and primes the salsa cache.
fn workspace_graph(service: &SysmlService) -> std::sync::Arc<sysml_core::ModelGraph> {
    service
        .eval_context_with_overrides(&[])
        .expect("workspace eval_context")
        .graph
        .as_ref()
        .expect("EvalContext has graph attached")
        .clone()
}

fn bench_sim_start_warm(c: &mut Criterion) {
    let service = load_service();
    let graph = workspace_graph(&service);

    // Warmup: confirm the SM compiles before the bench loop. Mirrors what
    // a real `simulate.start` call does the first time on this graph.
    {
        let snap = sysml_ide_db::Snapshot::new(graph.clone());
        let _ = snap
            .compile_state_machine(SM_NAME)
            .expect("warmup compile_state_machine");
    }

    c.bench_function("service/sim_start_warm", |b| {
        b.iter(|| {
            let snap = sysml_ide_db::Snapshot::new(graph.clone());
            let ir = snap
                .compile_state_machine(SM_NAME)
                .expect("compile_state_machine");
            let runner = sysml_runtime::statemachine::StateMachineRunner::new(ir);
            let mut orch =
                sysml_runtime::orchestrator::Orchestrator::new(Default::default());
            orch.add_state_machine(SM_NAME, runner);
            let mut session = sysml_service::execution::RuntimeSession::new(
                orch,
                "__bench__".to_owned(),
                sysml_service::execution::SessionKind::Simulation,
                Some(SM_NAME.to_owned()),
            );
            black_box(session.step());
        });
    });
}

fn bench_sim_step_warm(c: &mut Criterion) {
    let service = load_service();
    let graph = workspace_graph(&service);

    let snap = sysml_ide_db::Snapshot::new(graph);
    let ir = snap
        .compile_state_machine(SM_NAME)
        .expect("compile_state_machine");
    let runner = sysml_runtime::statemachine::StateMachineRunner::new(ir);
    let mut orch = sysml_runtime::orchestrator::Orchestrator::new(Default::default());
    orch.add_state_machine(SM_NAME, runner);
    let mut session = sysml_service::execution::RuntimeSession::new(
        orch,
        "__bench__".to_owned(),
        sysml_service::execution::SessionKind::Simulation,
        Some(SM_NAME.to_owned()),
    );
    // Prime one step so the bench measures steady-state per-tick cost,
    // not the first-step initialization spike.
    let _ = session.step();

    c.bench_function("service/sim_step_warm", |b| {
        b.iter(|| {
            black_box(session.step());
        });
    });
}

fn bench_diagnostics_warm(c: &mut Criterion) {
    let service = load_service();
    // Pick any loaded URI; the diagnostics pipeline walks the full
    // workspace internally regardless of which file URI is named.
    let uri = service
        .loaded_uris()
        .into_iter()
        .next()
        .expect("at least one loaded URI in espresso-pump-hybrid");
    // Prime salsa cache: parse + resolve + validate + physics health.
    let _ = service.diagnostics(&uri).expect("warmup diagnostics");

    c.bench_function("service/diagnostics_warm", |b| {
        b.iter(|| {
            black_box(service.diagnostics(&uri).expect("diagnostics"));
        });
    });
}

criterion_group!(
    benches,
    bench_sim_start_warm,
    bench_sim_step_warm,
    bench_diagnostics_warm,
);
criterion_main!(benches);
