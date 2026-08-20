//! espresso-production-cell — determinism, slot ownership, override (Stage D).
//!
//! CELL-DET-01 (DET-RUN): two builds from the same elaborated graph produce
//!   byte-identical per-tick snapshots (differential run-to-run equality, NOT a
//!   stored golden trajectory).
//! CELL-SLOT-01 (RT-SLOT): writer-ownership invariants; per-instance ODE state
//!   slots have distinct owning writers and no multi-writer conflicts.
//! RES-OVERRIDE: a qualified per-instance override resolves and changes only
//!   that instance's behaviour.

mod common;
use common::{load_example_graph, load_example_orchestrator, workspace_example_dir};
use std::collections::BTreeMap;
use sysml_core::ModelGraph;
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::orchestrator::{ExecutionSnapshot, Orchestrator};

const FIXTURE: &str = "espresso-production-cell";

fn build(graph: &ModelGraph) -> Orchestrator {
    let compiler = ModelCompiler::new(graph.clone()).with_source_dir(&workspace_example_dir(FIXTURE));
    let base = sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
    let pre = std::sync::Arc::new(sysml_runtime::constraints::extract_and_precompile(
        compiler.graph(),
    ));
    compiler
        .build_workspace_orchestrator(base, Some(pre), None, None, None, &[], Some(100.0), Some(30_000.0))
        .expect("orchestrator builds")
}

/// Project a snapshot to a comparable map (slot values + SM states), excluding
/// the inherently-ambiguous flat config-overlay keys.
fn project(s: &ExecutionSnapshot) -> BTreeMap<String, String> {
    let mut m = BTreeMap::new();
    for (k, v) in s.variables.iter() {
        if k.starts_with("config.") {
            continue;
        }
        m.insert(format!("var:{k}"), format!("{v:?}"));
    }
    for (n, st) in s.subsystem_states.iter() {
        m.insert(format!("sm:{n}"), st.current_state.clone());
    }
    m
}

/// CELL-DET-01 — two builds of the same elaborated graph agree byte-for-byte at
/// every tick (parse/elaborate once, build twice, step in lockstep).
#[test]
fn cell_two_builds_are_identical_every_tick() {
    let graph = load_example_graph(FIXTURE); // elaborate ONCE
    let mut a = build(&graph);
    let mut b = build(&graph);
    assert_eq!(a.subsystem_count(), b.subsystem_count(), "subsystem count must match");
    for step in 0..250 {
        let pa = project(&a.step());
        let pb = project(&b.step());
        assert!(
            pa == pb,
            "builds diverged at step {step}: {:?}",
            pa.iter().filter(|(k, v)| pb.get(*k) != Some(*v)).take(5).collect::<Vec<_>>()
        );
    }
}

/// CELL-SLOT-01 — no multi-writer conflicts, and each station's ODE temperature
/// slot is owned by a distinct writer (per-instance independence at the slot
/// plane).
#[test]
fn cell_slot_writer_ownership_is_clean() {
    let graph = load_example_graph(FIXTURE);
    let orch = build(&graph);
    let store = orch.slot_store();
    assert!(!store.is_empty(), "slot table must be minted");
    assert!(
        store.multi_writer_conflicts().is_empty(),
        "no slot may have competing writers: {:?}",
        store.multi_writer_conflicts()
    );
    // Both per-instance temperature slots exist and are distinct slots.
    let s1 = store.slot_by_name("station1.temp");
    let s2 = store.slot_by_name("station2.temp");
    assert!(s1.is_some() && s2.is_some(), "per-instance temp slots must exist");
    assert_ne!(s1, s2, "each station's temperature is its own slot");
}

/// RES-OVERRIDE — a qualified override on one station changes only that
/// station's steady state; the other station is unaffected (independence).
///
/// Un-ignored by the `rtgaps` override-unification landing: a qualified
/// per-instance override key (`station1.P_heater`) resolves against that
/// instance's slot via the build-time prefix-strip and changes ONLY that
/// instance; the bare key (`P_heater`) remains a documented broadcast to every
/// station. An unknown/typo'd target now hard-errors instead of silently
/// no-op'ing (workspace-path validation in `build_workspace_orchestrator`).
#[test]
fn cell_qualified_override_is_isolated() {
    // Baseline: both stations settle to the same temperature.
    let settle = |orch: &mut Orchestrator| {
        let mut last = orch.step();
        for _ in 0..400 {
            last = orch.step();
        }
        last
    };
    let f = |s: &ExecutionSnapshot, k: &str| match s.variables.get(k) {
        Some(sysml_core::Value::Float(x)) => *x,
        other => panic!("{k}: expected float, got {other:?}"),
    };

    let mut base = load_example_orchestrator(FIXTURE, &[], 100.0, 60_000.0);
    let bs = settle(&mut base);
    let base1 = f(&bs, "station1.temp");
    let base2 = f(&bs, "station2.temp");
    assert!((base1 - base2).abs() < 1e-6, "stations identical at baseline: {base1} vs {base2}");

    // Override station1's heater power upward; station2 must be unchanged.
    let mut over =
        load_example_orchestrator(FIXTURE, &[("station1.P_heater".into(), "20.0".into())], 100.0, 60_000.0);
    let os = settle(&mut over);
    let over1 = f(&os, "station1.temp");
    let over2 = f(&os, "station2.temp");
    assert!(over1 > base1 + 1.0, "override must raise station1 temp: {base1} -> {over1}");
    assert!((over2 - base2).abs() < 1e-6, "station2 must be unaffected: {base2} -> {over2}");
}
