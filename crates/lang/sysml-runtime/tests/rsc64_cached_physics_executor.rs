//! RSC-6.4 cached-physics-executor equivalence gate.
//!
//! `build_workspace_orchestrator` can now take a pre-built physics executor
//! (the salsa-memoized `Arc<PhysicsExecutor>` threaded via
//! `ModelCompiler::with_cached_physics_executor`) and clone it
//! (`PhysicsExecutor::clone_concrete`) instead of reconstructing it from the
//! graph at step 7. This is a perf lift (the executor is built once per graph
//! version rather than once per orchestrator build) and MUST be byte-identical
//! in observable behaviour to the inline build.
//!
//! This test builds the same workspace orchestrator two ways — inline (no
//! cache) and via the cached executor — and asserts they produce identical
//! subsystem structure AND identical step-by-step simulation output. The
//! espresso production cell is a multi-instance PowerBond physics model (its
//! compile produces a `physics` subsystem).
//!
//! The cached executor is built with `PhysicsExecutor::from_graph`, which
//! classifies links via `build_port_flow_resources().registry` ==
//! `compile_ports()` — exactly the registry step 7's inline path uses — so the
//! two executors are constructed from identical inputs.

use std::path::PathBuf;
use std::sync::Arc;

use sysml_core::{elaborate, ModelGraph};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::compiler::ModelCompiler;
use sysml_runtime::physics::executor::PhysicsExecutor;

fn espresso_cell_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("..")
        .join("examples")
        .join("espresso-production-cell")
}

fn collect_sysml_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect_sysml_files(&path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("sysml") {
                out.push(path);
            }
        }
    }
}

fn load_espresso_cell() -> ModelGraph {
    let dir = espresso_cell_dir();
    assert!(
        dir.exists(),
        "espresso production cell dir not found: {}",
        dir.display()
    );

    let mut sysml_files = Vec::new();
    collect_sysml_files(&dir, &mut sysml_files);
    sysml_files.sort();
    assert!(
        !sysml_files.is_empty(),
        "no .sysml files in {}",
        dir.display()
    );

    let parser = TreeSitterParser::new();
    let files: Vec<SysmlFile> = sysml_files
        .iter()
        .map(|path| {
            let source = std::fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("read {}: {}", path.display(), e));
            let name = path.file_name().unwrap().to_str().unwrap().to_owned();
            SysmlFile::new(name, source)
        })
        .collect();

    let mut merged = parser.parse(&files).graph;
    elaborate::elaborate(&mut merged);
    merged
}

/// Stringify a `Value` order-INDEPENDENTLY: `List` elements are sorted before
/// joining. Orchestrator-internal bookkeeping lists (`__recent_transitions`,
/// `__active_substates`, ...) collect entries across subsystems in HashMap
/// iteration order, so their ORDER is nondeterministic across builds even
/// though their CONTENT is identical. Physics observables (slot currents/
/// voltages, ODE state vars) are scalars, never lists, so normalizing list
/// order cannot mask a physics-cache divergence — it only neutralizes the
/// scheduler's nondeterministic list ordering.
fn norm_value(v: &sysml_core::Value) -> String {
    match v {
        sysml_core::Value::List(items) => {
            let mut parts: Vec<String> = items.iter().map(norm_value).collect();
            parts.sort();
            format!("List[{}]", parts.join(","))
        }
        other => format!("{other:?}"),
    }
}

/// A comparable projection of an `ExecutionSnapshot` — every shared variable
/// plus each subsystem's current state, keyed for diffing.
fn project(
    snap: &sysml_runtime::orchestrator::ExecutionSnapshot,
) -> std::collections::BTreeMap<String, String> {
    let mut rows = std::collections::BTreeMap::new();
    for (k, v) in snap.variables.iter() {
        rows.insert(format!("var:{k}"), norm_value(v));
    }
    for (name, state) in snap.subsystem_states.iter() {
        rows.insert(format!("sm:{name}"), state.current_state.clone());
    }
    rows
}

/// A context key whose VALUE (not just order) is set nondeterministically by
/// the instance-config overlay seed, NOT by any executor.
///
/// `build_workspace_orchestrator`'s `seed_instance_config_into` walks a
/// `HashMap` of instances and writes flat keys into the `config` namespace;
/// when several multiplied parts declare the same-named attribute, they collide
/// last-writer-wins in iteration order (a documented overlay-contract limit,
/// orthogonal to physics — each value IS one of the legitimate per-instance
/// configs, just not deterministically chosen, e.g. `config.stationName` =
/// "station1" vs "station2", `stationN.config` = one map or another). These are
/// genuine value differences, so unlike the nondeterministically-ORDERED
/// bookkeeping lists (handled by `norm_value`) they cannot be normalized and
/// must be excluded. The physics executor writes NONE of these (it writes
/// physical slots like `<assembly>.<port>.current`), so excluding them does not
/// weaken the physics-equivalence check — a regression would surface on a
/// physics slot / ODE var / SM state, all of which are retained.
fn is_nondeterministic_seed_key(k: &str) -> bool {
    k.starts_with("var:config.") || k.ends_with(".config")
}

/// Keys (excluding the nondeterministic seed overlay) whose value differs, or is
/// present in only one, between two projections.
fn diff_keys(
    a: &std::collections::BTreeMap<String, String>,
    b: &std::collections::BTreeMap<String, String>,
) -> std::collections::BTreeSet<String> {
    let mut keys = std::collections::BTreeSet::new();
    for (k, v) in a {
        if !is_nondeterministic_seed_key(k) && b.get(k) != Some(v) {
            keys.insert(k.clone());
        }
    }
    for k in b.keys() {
        if !is_nondeterministic_seed_key(k) && !a.contains_key(k) {
            keys.insert(k.clone());
        }
    }
    keys
}

/// Build an espresso-cell workspace orchestrator, optionally threading a
/// pre-built physics executor (RSC-6.4 cached path).
fn build_orch(
    graph: &ModelGraph,
    cached: Option<Arc<PhysicsExecutor>>,
) -> sysml_runtime::orchestrator::Orchestrator {
    let mut compiler = ModelCompiler::new(graph.clone()).with_source_dir(&espresso_cell_dir());
    if let Some(exec) = cached {
        compiler = compiler.with_cached_physics_executor(exec);
    }
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

#[test]
fn cached_physics_executor_matches_inline_build() {
    // Parse + elaborate ONCE and clone for every build — a second parse would
    // mint fresh ElementId UUIDs, so `Ref(ElementId(..))` context values would
    // differ for reasons unrelated to the cache.
    let graph = load_espresso_cell();

    let mut orch_inline = build_orch(&graph, None);

    // Build the cached executor exactly as the salsa query will: from the
    // elaborated graph, classifying links the same way step 7's inline path
    // does (`build_port_flow_resources().registry == compile_ports()`).
    let cached_exec = {
        let probe = ModelCompiler::new(graph.clone());
        let (executor, _diags) = PhysicsExecutor::from_graph(probe.graph())
            .expect("espresso cell must yield a PowerBond physics executor");
        Arc::new(executor)
    };
    let mut orch_cached = build_orch(&graph, Some(Arc::clone(&cached_exec)));

    // --- structural equivalence ------------------------------------------
    assert_eq!(
        orch_inline.subsystem_count(),
        orch_cached.subsystem_count(),
        "subsystem count must match between inline and cached builds"
    );
    let mut subs_inline = orch_inline.subsystem_names();
    let mut subs_cached = orch_cached.subsystem_names();
    subs_inline.sort();
    subs_cached.sort();
    assert_eq!(subs_inline, subs_cached, "subsystem names must match");
    assert!(
        subs_inline.iter().any(|n| n == "physics"),
        "the gate is only meaningful if a physics subsystem is present; got {subs_inline:?}"
    );

    // --- behavioural equivalence -----------------------------------------
    // Step both in lockstep. Every key OUTSIDE the nondeterministic
    // instance-config seed overlay (see `is_nondeterministic_seed_key`) — i.e.
    // every physics slot, ODE state var, SM state, and port value — must match
    // exactly at every tick. A physics-cache regression would surface here.
    for step in 0..200 {
        let pi = project(&orch_inline.step());
        let pc = project(&orch_cached.step());
        let diverged = diff_keys(&pi, &pc);
        assert!(
            diverged.is_empty(),
            "RSC-6.4: cached physics executor diverged from inline build at step {step} \
             on {} non-seed key(s):\n{:#?}",
            diverged.len(),
            diverged
                .iter()
                .map(|k| format!("{k}: inline={:?} cached={:?}", pi.get(k), pc.get(k)))
                .collect::<Vec<_>>(),
        );
    }
}
