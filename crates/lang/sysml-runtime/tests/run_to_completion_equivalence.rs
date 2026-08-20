//! OPT #3 gate (runtime-hotpath-perf-plan): `run_to_completion` was rewritten
//! to run intermediate ticks as light `advance()` ticks (skipping the five
//! purely-cosmetic snapshot fields) and to compute those fields only ONCE, on
//! the final snapshot. This gate proves the rewrite is behaviour-preserving:
//! the FINAL snapshot from the new advance-based path must be byte-identical
//! to the FINAL snapshot from the old full-`step()`-every-tick path.
//!
//! The two paths are exercised through the SAME `run_to_completion` entry
//! point (identical stop conditions) by toggling observer presence: with an
//! observer installed, `run_to_completion` stays on full `step()` every tick
//! (the streaming layer is owed every frame); without one, it uses the new
//! `advance()` + final-upgrade path. The espresso production cell is the stress
//! fixture — instance-multiplied ODEs (derivatives), state machines (causation /
//! transitions), and a port/physics network (port_values), so every deferred
//! field is non-trivial.

use std::collections::BTreeMap;
use std::sync::Arc;

use sysml_core::ModelGraph;
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::orchestrator::{ExecutionSnapshot, Orchestrator};

mod common;
// Task #8: blessed loader (common/mod.rs RULE) instead of a hand-rolled
// read_dir+parse+elaborate copy. Parse-ONCE / build-TWICE (advance-path vs
// full-step()-path from the SAME graph), so `load_example_graph` + the local
// `build_orch(&graph)` — NOT `load_example_orchestrator`, which re-parses per
// call and would give the two builds different ElementIds.
use common::load_example_graph;

fn build_orch(graph: &ModelGraph) -> Orchestrator {
    let dir = common::workspace_example_dir("espresso-production-cell");
    let compiler = ModelCompiler::new(graph.clone()).with_source_dir(&dir);
    let base_ctx = sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
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
            &[],
            Some(1.0),
            Some(50.0),
        )
        .expect("workspace orchestrator should build")
}

/// Project EVERY verdict-relevant snapshot field — including the five deferred
/// ones (`resolved_refs`, `causation_links`, `guard_diagnoses`, `port_values`,
/// `derivatives`) — into a comparable string map.
fn project_full(snap: &ExecutionSnapshot) -> BTreeMap<String, String> {
    let mut rows = BTreeMap::new();
    rows.insert("@tick".to_owned(), snap.tick.to_string());
    rows.insert("@time_ms".to_owned(), format!("{:?}", snap.time_ms));
    rows.insert("@completed".to_owned(), snap.completed.to_string());

    for (k, v) in snap.variables.iter() {
        rows.insert(format!("var:{k}"), format!("{v:?}"));
    }
    for (name, state) in snap.subsystem_states.iter() {
        rows.insert(
            format!("sm:{name}"),
            format!("{}|completed={}", state.current_state, state.completed),
        );
    }
    for r in &snap.constraint_results {
        rows.insert(format!("constraint:{}", r.name), format!("{:?}", r.verdict));
    }
    for (k, v) in &snap.resolved_refs {
        rows.insert(format!("ref:{k}"), format!("{v:?}"));
    }
    for (k, v) in &snap.derivatives {
        rows.insert(format!("deriv:{k}"), format!("{v:?}"));
    }
    for (k, feats) in &snap.port_values {
        for (f, v) in feats {
            rows.insert(format!("port:{k}.{f}"), format!("{v:?}"));
        }
    }
    // Causation/guard diagnoses: count + Debug, order-insensitive via sort.
    let mut causation: Vec<String> = snap.causation_links.iter().map(|c| format!("{c:?}")).collect();
    causation.sort();
    rows.insert("@causation".to_owned(), causation.join("\n"));
    let mut guards: Vec<String> = snap.guard_diagnoses.iter().map(|g| format!("{g:?}")).collect();
    guards.sort();
    rows.insert("@guard_diagnoses".to_owned(), guards.join("\n"));
    rows
}

/// The un-prefixed FLAT config-overlay keys (`config.<attr>`) are inherently
/// ambiguous: a per-instance attribute promoted to a bare name, which several
/// multiplied parts write with GENUINELY DIFFERENT values (last-writer-wins
/// under HashMap order). They differ between two *separate builds* of the same
/// graph regardless of how each is stepped — the same documented ambiguous-
/// overlay-key exclusion the two-build determinism gate applies. They are
/// orthogonal to the step-vs-advance equivalence this gate checks, so exclude
/// them; every prefixed verdict-relevant key (`<inst>.config.<attr>`), ODE
/// state, SM state, derivative, port value, etc. is still compared.
fn is_overlay_ambiguous_key(k: &str) -> bool {
    k.starts_with("var:config.")
}

fn diff(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> Vec<String> {
    let mut out = Vec::new();
    for (k, v) in a {
        if !is_overlay_ambiguous_key(k) && b.get(k) != Some(v) {
            out.push(format!("{k}: full={v:?} advance={:?}", b.get(k)));
        }
    }
    for k in b.keys() {
        if !is_overlay_ambiguous_key(k) && !a.contains_key(k) {
            out.push(format!("{k}: full=absent advance={:?}", b.get(k)));
        }
    }
    out
}

#[test]
fn run_to_completion_advance_matches_full_step_loop() {
    let graph = load_example_graph("espresso-production-cell");

    // Reference: full `step()` every tick (a no-op observer forces that path
    // through `run_to_completion`).
    let mut full = build_orch(&graph);
    full.set_snapshot_observer(Arc::new(|_: &ExecutionSnapshot| {}));
    let full_final = full
        .run_to_completion()
        .expect("reference run produces a final snapshot");

    // Subject: new advance-based path (no observer).
    let mut light = build_orch(&graph);
    let light_final = light
        .run_to_completion()
        .expect("advance run produces a final snapshot");

    let pf = project_full(&full_final);
    let pl = project_full(&light_final);
    let divergences = diff(&pf, &pl);
    assert!(
        divergences.is_empty(),
        "OPT #3: advance-based run_to_completion final snapshot diverged from the \
         full-step reference on {} field(s):\n{:#?}",
        divergences.len(),
        divergences
    );

    // Sanity: the run actually did non-trivial work (otherwise the gate is vacuous).
    assert!(full_final.tick > 1, "fixture should step many ticks");
    assert!(
        !pf.is_empty() && pf.values().any(|v| v.contains('.')),
        "fixture should populate verdict-relevant fields"
    );
}
