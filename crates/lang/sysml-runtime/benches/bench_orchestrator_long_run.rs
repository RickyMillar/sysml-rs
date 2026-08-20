//! Benchmark: 10k-tick synthetic multi-subsystem orchestrator session.
//!
//! Builds a synthetic orchestrator with 3 state machines and no ODE,
//! then steps 10,000 ticks. Measures baseline orchestrator overhead:
//! context sync, flow routing, constraint evaluation, snapshot recording.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use sysml_core::Value;
use sysml_runtime::orchestrator::{Orchestrator, OrchestratorConfig};
use sysml_runtime::statemachine::StateMachineRunner;
use sysml_runtime::{AssignmentIR, StateIR, StateMachineIR, TransitionActionIR, TransitionIR};

/// Build a simple 3-state cyclic state machine:
///   idle --tick--> running --tick--> cooldown --tick--> idle
/// with a counter variable incremented on each transition.
fn build_cyclic_sm(name: &str) -> StateMachineIR {
    StateMachineIR::new(name, "idle")
        .with_state(StateIR::new("idle"))
        .with_state(StateIR::new("running"))
        .with_state(StateIR::new("cooldown"))
        .with_transition(
            TransitionIR::new("idle", "running")
                .with_event("tick")
                .with_action(TransitionActionIR::structured(
                    vec![AssignmentIR::add("counter", Value::Float(1.0))],
                    Vec::new(),
                )),
        )
        .with_transition(
            TransitionIR::new("running", "cooldown")
                .with_event("tick")
                .with_action(TransitionActionIR::structured(
                    vec![AssignmentIR::add("counter", Value::Float(1.0))],
                    Vec::new(),
                )),
        )
        .with_transition(
            TransitionIR::new("cooldown", "idle")
                .with_event("tick")
                .with_action(TransitionActionIR::structured(
                    vec![AssignmentIR::add("counter", Value::Float(1.0))],
                    Vec::new(),
                )),
        )
}

/// Build an orchestrator with 3 cyclic state machines and a snapshot
/// interval of 10 (to keep memory reasonable over 10k ticks).
fn build_orchestrator() -> Orchestrator {
    let config = OrchestratorConfig {
        dt_ms: 1.0,
        max_ticks: 15_000,
        max_time_ms: 15_000.0,
        snapshot_interval: 10,
        ..Default::default()
    };
    let mut orch = Orchestrator::new(config);

    for i in 0..3 {
        let name = format!("subsystem_{i}");
        let sm_ir = build_cyclic_sm(&name);
        orch.add_state_machine(&name, StateMachineRunner::new(sm_ir));
    }

    // Seed context with initial counter values
    for i in 0..3 {
        orch.context
            .set(format!("subsystem_{i}.counter"), Value::Float(0.0));
    }

    orch
}

fn bench_orchestrator_10k_ticks(c: &mut Criterion) {
    let mut group = c.benchmark_group("runtime/orchestrator_long_run");
    group.sample_size(10);

    group.bench_function("3_subsystems_10k_ticks", |b| {
        b.iter_batched(
            build_orchestrator,
            |mut orch| {
                // Inject "tick" event to each subsystem to drive transitions
                for _ in 0..10_000 {
                    for i in 0..3 {
                        orch.inject_event(&format!("subsystem_{i}"), "tick");
                    }
                    black_box(orch.step());
                }
            },
            criterion::BatchSize::PerIteration,
        );
    });

    group.finish();
}

criterion_group!(benches, bench_orchestrator_10k_ticks);
criterion_main!(benches);
