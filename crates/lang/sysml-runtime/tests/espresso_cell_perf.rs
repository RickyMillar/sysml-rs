//! espresso-production-cell — performance/memory sanity budgets (Stage D).
//!
//! CELL-PERF-01 (PERF-LOAD / PERF-STEP / PERF-MEM): the smoke-scale workspace
//! builds and steps inside a measured, generously-headroomed budget. These are
//! debug-build sanity bounds (guarding gross regressions), not release
//! benchmarks; the numbers printed here are recorded in the README. Release
//! criterion benchmarks at ci/stress scale are the migration-track perf gates.

mod common;
use common::load_example_orchestrator;
use std::time::Instant;

const FIXTURE: &str = "espresso-production-cell";

/// PERF-LOAD + PERF-STEP — build the smoke workspace and step it, asserting both
/// stay inside a generous debug-mode budget; print the measured numbers.
#[test]
fn cell_build_and_step_within_budget() {
    let t0 = Instant::now();
    let mut orch = load_example_orchestrator(FIXTURE, &[], 100.0, 120_000.0);
    let build_ms = t0.elapsed().as_secs_f64() * 1e3;

    const STEPS: usize = 500;
    let t1 = Instant::now();
    for _ in 0..STEPS {
        orch.step();
    }
    let step_ms = t1.elapsed().as_secs_f64() * 1e3;
    let per_step_us = step_ms * 1e3 / STEPS as f64;

    eprintln!(
        "PERF (smoke, debug): build={build_ms:.1} ms, {STEPS} steps={step_ms:.1} ms, \
         per-step={per_step_us:.1} us"
    );

    // The per-step budget is profile-aware. In release the assertion is tight
    // (the real performance gate); in debug it is a loose sanity ceiling only —
    // debug per-step varies widely across hosts (measured 7.7-24 ms on loaded
    // machines), so a tight debug bound would be flaky. The build bound is the
    // same in both profiles.
    assert!(build_ms < 15_000.0, "smoke build {build_ms:.0} ms exceeds 15 s budget");
    let per_step_budget_us = if cfg!(debug_assertions) { 60_000.0 } else { 2_000.0 };
    assert!(
        per_step_us < per_step_budget_us,
        "per-step {per_step_us:.0} us exceeds {:.0} us budget ({} profile)",
        per_step_budget_us,
        if cfg!(debug_assertions) { "debug/sanity" } else { "release" }
    );
}
