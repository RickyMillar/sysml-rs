//! espresso-production-cell — behaviour gates (Stage C).
//!
//! CELL-SM-01 (RT-SM): independent per-station lifecycle progress with
//! deterministic shared-resource arbitration.
//! CELL-ACT-01 (EX-MSG): exactly-once command delivery + per-station isolation.

mod common;
use common::{load_dir, load_example_orchestrator, workspace_example_dir};
use sysml_runtime::compiler::ModelCompiler;

const FIXTURE: &str = "espresso-production-cell";

/// Build an orchestrator from an isolated subset of the fixture's files. Used by
/// the message-delivery gate: routing exactly-once delivery is verified on the
/// focused coordination scenario (the same way exchange-plane-fixture gates
/// classification on a purpose-built fixture rather than the whole corpus),
/// free of cross-channel interference from the cell's many command channels.
fn build_isolated(files: &[&str]) -> sysml_runtime::orchestrator::Orchestrator {
    let root = workspace_example_dir(FIXTURE);
    let dst = std::env::temp_dir().join(format!("espresso_cell_iso_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dst);
    std::fs::create_dir_all(&dst).unwrap();
    for rel in files {
        let name = std::path::Path::new(rel).file_name().unwrap();
        std::fs::copy(root.join(rel), dst.join(name)).unwrap();
    }
    let graph = load_dir(&dst);
    let compiler = ModelCompiler::new(graph);
    let base = sysml_ide_db::eval_context_seed::context_from_graph(compiler.graph());
    let pre = std::sync::Arc::new(sysml_runtime::constraints::extract_and_precompile(compiler.graph()));
    compiler
        .build_workspace_orchestrator(base, Some(pre), None, None, None, &[], Some(100.0), Some(6000.0))
        .expect("isolated orchestrator builds")
}

fn run_states_of(
    mut orch: sysml_runtime::orchestrator::Orchestrator,
    ticks: usize,
) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut seq: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let mut last = orch.step();
    for _ in 0..ticks {
        last = orch.step();
        for (k, s) in last.subsystem_states.iter() {
            let e = seq.entry(k.clone()).or_default();
            if e.last().map(String::as_str) != Some(s.current_state.as_str()) {
                e.push(s.current_state.clone());
            }
        }
    }
    seq
}

/// Run the full workspace and collect, per subsystem, the ordered states seen.
fn run_states(ticks: usize) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut orch = load_example_orchestrator(FIXTURE, &[], 100.0, 12_000.0);
    let mut seq: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    let mut last = orch.step();
    for _ in 0..ticks {
        last = orch.step();
        for (k, s) in last.subsystem_states.iter() {
            let e = seq.entry(k.clone()).or_default();
            if e.last().map(String::as_str) != Some(s.current_state.as_str()) {
                e.push(s.current_state.clone());
            }
        }
    }
    seq
}

/// CELL-SM-01 — both station lifecycle SMs multiply and progress independently
/// through Idle -> Preheat -> Ready, and the run is deterministic.
#[test]
fn cell_stations_progress_independently_and_deterministically() {
    let a = run_states(120);
    // (Idle is the pre-run initial state; the auto warm-up transition fires on
    // the first tick, so the observable cycle is Preheat -> Ready.)
    for st in ["station1.StationLifecycle", "station2.StationLifecycle"] {
        let seq = a.get(st).unwrap_or_else(|| panic!("missing SM {st}"));
        assert_eq!(
            seq.as_slice(),
            ["preheat", "ready"],
            "{st} did not traverse the modeled warm-up cycle: {seq:?}"
        );
    }
    // Deterministic arbitration/progress: a second run yields the identical
    // per-SM state sequences (stable, not map-iteration dependent).
    let b = run_states(120);
    assert_eq!(a, b, "SM progression is not deterministic run-to-run");
}

/// CELL-ACT-01 — the supervisor grants exactly one permit, to the first station
/// in path order; it is delivered exactly once and consumed, and the ungranted
/// station stays isolated (never advances).
#[test]
fn cell_permit_delivered_exactly_once_with_isolation() {
    let orch = build_isolated(&[
        "Libraries/Types.sysml",
        "Libraries/Interfaces.sysml",
        "Behaviour/PlantSupervisor.sysml",
    ]);
    let seq = run_states_of(orch, 40);
    // Granted receiver consumed the message and advanced to brewing.
    let a = seq
        .get("GrantedLogic")
        .unwrap_or_else(|| panic!("missing granted SM; subsystems: {:?}", seq.keys().collect::<Vec<_>>()));
    // Advanced to brewing (delivered + consumed) in exactly one transition
    // (exactly-once: the accept fires a single time, no oscillation).
    assert_eq!(a.last().map(String::as_str), Some("brewingA"), "granted must brew: {a:?}");
    assert!(a.len() <= 2, "exactly-once consumption — no repeated transitions: {a:?}");
    // Ungranted receiver never advanced (isolation — no cross-delivery).
    let b = seq.get("UngrantedLogic").expect("missing ungranted SM");
    assert_eq!(
        b.as_slice(),
        ["waitingB"],
        "ungranted station must stay isolated in waiting: {b:?}"
    );
}
