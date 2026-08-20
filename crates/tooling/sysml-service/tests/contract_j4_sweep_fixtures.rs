//! J4 Analyze → Sweep, driven end-to-end over the two checked-in ODE
//! fixtures that the workflow could not launch.
//!
//! Both failed at the same place — building runtime subsystems — with two
//! different messages, and neither failure was specific to batching: a plain
//! `sysml.orchestrate.workspace.start` on either workspace failed identically.
//! The sweep was simply the first screen that asked these fixtures to run.
//!
//!   radiation-cooling  `internal: ODE detection 'RadiatingBody' has no
//!                       registered subsystem at slot-mint time (RSC-4.2
//!                       mint-gap)`
//!   damped-oscillator  `no state machines, ODE, discrete, or action
//!                       subsystems found in the workspace graph`
//!
//! Three defects, all in the shared detect → register → mint path:
//!
//!   1. The string-expression compiler knew `**` but not `^` for
//!      exponentiation, though KerML defines them as one operator
//!      (`ExponentiationOperator : '**' | '^'`). `RadiatingBody`'s
//!      Stefan-Boltzmann RHS uses `^`, so its solver failed to build.
//!   2. `specializes_name` recognised only the DEFINITION spellings of a
//!      specialization edge, so `action dynamics :> StateSpaceDynamics` — the
//!      usage form, which `damped-oscillator` alone uses — specialized nothing
//!      as far as any SSR detector could tell.
//!   3. Discrete state-space subsystems registered without minting their state
//!      vector as slots, so the first writeback hit an unrouted strict route.
//!      Latent behind (2) here, live on `digital-filter` today.
//!
//! Each test drives the sequence `useSweepRunner.start` drives — create,
//! then per child bulk-step → verify → stop, then read the rollup — through
//! `execute_command`, the same dispatch the HTTP transport uses.
//!
//! `damped-oscillator` is now REFUSED rather than run: it declares two state
//! variables against one scalar `getNextState`, and the detector used to close
//! that gap by broadcasting the single expression across both states — which
//! made a zeta sweep return five identical numbers with every child reporting
//! `complete`. See `an_under_determined_model_is_refused_rather_than_broadcast`.
//!
//! What these tests do NOT assert: anything about the physics being right.
//! The fixtures are synthetic and the point here is that a configured sweep
//! either reaches evaluation and reports coherently, or refuses with a
//! diagnostic that says why.

use serde_json::{json, Value};
use sysml_service::{execute_command, SysmlService};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Load one of the repo's `examples/` workspaces, read-only.
fn workspace(name: &str) -> SysmlService {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../examples")
        .join(name);
    let service = SysmlService::empty();
    service
        .load_workspace(&root)
        .unwrap_or_else(|e| panic!("load {name}: {e}"));
    service
}

/// `sysml.batch.create` with the sweep's own wire shape: `children_params` is
/// a JSON-ENCODED STRING, not an array (the frontend stringifies at the
/// boundary — see `useSweepRunner`), and no `subsystem_name`, so children take
/// the workspace-orchestrator path.
fn create_sweep(service: &SysmlService, points: &[Value]) -> Value {
    execute_command(
        service,
        "sysml.batch.create",
        json!({
            "kind": "sweep",
            "uri": "__workspace__",
            "children_params": serde_json::to_string(points).unwrap(),
            "label": "j4 regression",
        }),
    )
    .unwrap_or_else(|e| panic!("batch.create: {e}"))
}

/// Bulk-step one child, as the sweep runner does — ONE `sessions.step` with a
/// horizon, not N single-tick calls.
///
/// A step error is a panic here. The runner swallows it so one bad child
/// cannot wedge a batch, but a test that swallowed it would pass on a batch
/// that evaluated nothing, which is precisely the state this suite exists to
/// detect.
fn step_child(service: &SysmlService, session_id: &str, ticks: u64) {
    execute_command(
        service,
        "sysml.sessions.step",
        json!({ "session_id": session_id, "ticks": ticks }),
    )
    .unwrap_or_else(|e| panic!("step {session_id}: {e}"));
}

/// Verify then stop, closing the child out. `sessions.stop` RELEASES the live
/// session (archiving it), so anything read from the live session must be read
/// before this runs.
///
/// `sessions.verify` is called for path fidelity and its result deliberately
/// ignored: neither fixture declares a verification case, so an empty or
/// failing verify is the honest answer, and the runner treats it the same way.
fn close_child(service: &SysmlService, session_id: &str) {
    let _ = execute_command(
        service,
        "sysml.sessions.verify",
        json!({ "session_id": session_id }),
    );
    execute_command(
        service,
        "sysml.sessions.stop",
        json!({ "session_id": session_id }),
    )
    .unwrap_or_else(|e| panic!("stop {session_id}: {e}"));
}

/// Final value of `var` in a live child's canonical time series. `None` when
/// the variable was never captured — which is itself a finding, so callers
/// assert on it rather than defaulting.
fn final_value(service: &SysmlService, session_id: &str, var: &str) -> Option<f64> {
    let series = execute_command(
        service,
        "sysml.sessions.timeseries",
        json!({ "session_id": session_id, "var": var }),
    )
    .unwrap_or_else(|e| panic!("timeseries {var}: {e}"));
    // `TimeSeriesResult::points` is a list of `{time_ms, value}` records.
    series["points"]
        .as_array()?
        .last()?
        .get("value")
        .and_then(Value::as_f64)
}

fn child_ids(created: &Value) -> Vec<String> {
    created["child_session_ids"]
        .as_array()
        .expect("child_session_ids array")
        .iter()
        .map(|v| v.as_str().expect("child id is a string").to_owned())
        .collect()
}

// ---------------------------------------------------------------------------
// A — radiation-cooling: 3 × 3 grid over emissivity and surfaceArea
// ---------------------------------------------------------------------------

/// The grid the J4 run configured: emissivity 0.5 → 0.9 step 0.2, surfaceArea
/// 0.1 → 0.9 step 0.4.
fn radiation_grid() -> Vec<Value> {
    let mut points = Vec::new();
    for e in [0.5, 0.7, 0.9] {
        for a in [0.1, 0.5, 0.9] {
            points.push(json!({ "emissivity": e, "surfaceArea": a }));
        }
    }
    points
}

#[test]
fn radiation_cooling_sweep_creates_nine_children() {
    let service = workspace("radiation-cooling");
    let created = create_sweep(&service, &radiation_grid());

    assert_eq!(
        child_ids(&created).len(),
        9,
        "a 3 × 3 grid must spawn nine children, not a partial batch"
    );
    assert!(
        created["batch_id"].as_str().is_some_and(|s| !s.is_empty()),
        "batch.create must return an addressable batch id"
    );
}

#[test]
fn radiation_cooling_children_integrate_the_stefan_boltzmann_rhs() {
    let service = workspace("radiation-cooling");
    let created = create_sweep(&service, &radiation_grid());
    let ids = child_ids(&created);

    // Extremes of the grid: least radiating (e 0.5, A 0.1) and most
    // (e 0.9, A 0.9). Index 0 and 8 by construction of `radiation_grid`.
    for id in [&ids[0], &ids[8]] {
        step_child(&service, id, 256);
    }

    // Read while the sessions are still live — `close_child` releases them.
    let coolest = final_value(&service, &ids[8], "temperature")
        .expect("the most-radiating child must capture a temperature series");
    let warmest = final_value(&service, &ids[0], "temperature")
        .expect("the least-radiating child must capture a temperature series");

    // The body starts at 1000 K and radiates into a 300 K ambient, so it can
    // only cool. If `temperature ^ 4` had failed to compile the ODE would not
    // exist at all — which is exactly how this fixture used to fail.
    assert!(
        warmest < 1000.0,
        "temperature must fall from its 1000 K initial value, got {warmest}"
    );
    // The whole point of a sweep: different factor values must produce
    // different outcomes, or the overrides never reached the solver. The
    // margin is asserted, not just the ordering — `<` alone would pass on two
    // runs separated by rounding noise.
    assert!(
        warmest - coolest > 0.01,
        "higher emissivity × area must cool measurably faster: e0.9/A0.9 \
         ended at {coolest} K, e0.5/A0.1 at {warmest} K"
    );
}

#[test]
fn radiation_cooling_batch_reports_a_coherent_rollup() {
    let service = workspace("radiation-cooling");
    let created = create_sweep(&service, &radiation_grid());
    let batch_id = created["batch_id"].as_str().unwrap().to_owned();
    let ids = child_ids(&created);

    for id in &ids {
        step_child(&service, id, 8);
        close_child(&service, id);
    }

    let status = execute_command(
        &service,
        "sysml.batch.status",
        json!({ "batch_id": batch_id }),
    )
    .expect("batch.status");
    assert_eq!(
        status["batch"]["status"]["status"], "complete",
        "every child terminated, so the rollup must read complete: {}",
        status["batch"]["status"]
    );

    let results = execute_command(
        &service,
        "sysml.batch.results",
        json!({ "batch_id": batch_id, "include_verdicts": false }),
    )
    .expect("batch.results");
    let children = results["children"].as_array().expect("children array");
    assert_eq!(children.len(), 9, "all nine children must be reported");
    for child in children {
        // `ChildStatus` is a serde-tagged enum, so the wire shape is
        // `{"status": "complete"}` — the `Failed` variant carries a message
        // alongside the tag, which is why it is not a bare string.
        assert_eq!(
            child["status"]["status"], "complete",
            "child {} reported {}",
            child["index"], child["status"]
        );
    }
    // The descriptors carry the point each child stands for — without them a
    // results table cannot label its own rows.
    let emissivities: Vec<f64> = children
        .iter()
        .filter_map(|c| c["params"]["emissivity"].as_f64())
        .collect();
    assert_eq!(emissivities.len(), 9, "every child must retain its params");
}

// ---------------------------------------------------------------------------
// B — usage-form dynamics: detected, and only run when well-formed
// ---------------------------------------------------------------------------

/// The grid the J4 run configured for `damped-oscillator`.
fn zeta_grid() -> Vec<Value> {
    [0.05, 0.1, 0.2, 0.4, 0.8]
        .iter()
        .map(|z| json!({ "zeta": z }))
        .collect()
}

/// The usage-form specialization fix is what makes this workspace VISIBLE.
/// Pinned on a well-formed model, so the fix stays covered by something that
/// passes rather than only by the refusal below.
#[test]
fn a_usage_form_dynamics_action_registers_a_subsystem() {
    // `action dynamics :> …` — the usage spelling, not `action def X :> …`.
    // One state variable, one `getNextState` return: well-formed.
    const USAGE_FORM: &str = r#"
package Decay {
    private import StateSpaceRepresentation::*;
    part def DecayModel {
        attribute rate : ScalarValues::Real = 0.5;
        action dynamics :> StateSpaceDynamics {
            in attribute input : Input;
            attribute stateSpace : StateSpace;
            out attribute level : ScalarValues::Real = 10.0;
            calc :>> getNextState : GetNextState {
                in input = dynamics::input;
                in stateSpace = dynamics::stateSpace;
                in timeStep = 0.25;
                return result = level - rate;
            }
        }
    }
}
"#;
    let service = SysmlService::empty();
    service.load_source("decay.sysml", USAGE_FORM).expect("load");

    let started = execute_command(
        &service,
        "sysml.orchestrate.workspace.start",
        json!({ "uri": "__workspace__" }),
    )
    .expect("a usage-form `action dynamics :> StateSpaceDynamics` must register a subsystem");

    let states = started[1]["subsystem_states"]
        .as_object()
        .expect("subsystem_states map");
    assert!(
        states.contains_key("DecayModel"),
        "the dynamics action must appear as a subsystem, got: {:?}",
        states.keys().collect::<Vec<_>>()
    );
}

/// End-to-end: the state a session reports follows the calc's RETURN.
///
/// Honest about its strength — this is a smoke test, not the gate. Element ids
/// are content-derived, so hash order is fixed per model, and on THIS model the
/// old any-child extractor happened to order the return first: mutation-testing
/// it (return-member preference removed) still passed. The sharp gate is
/// `sysml-runtime/tests/calc_result_selection.rs`, pinned on the fixture whose
/// ordering actually put the bound input first. Kept because it exercises the
/// whole path — detect, register, mint, tick, timeseries — on a usage-form
/// dynamics action, which nothing else does.
#[test]
fn the_next_state_comes_from_the_return_not_a_bound_input() {
    const USAGE_FORM: &str = r#"
package Decay {
    private import StateSpaceRepresentation::*;
    part def DecayModel {
        attribute rate : ScalarValues::Real = 0.5;
        action dynamics :> StateSpaceDynamics {
            in attribute input : Input;
            attribute stateSpace : StateSpace;
            out attribute level : ScalarValues::Real = 10.0;
            calc :>> getNextState : GetNextState {
                in input = dynamics::input;
                in stateSpace = dynamics::stateSpace;
                in timeStep = 0.25;
                return result = level - rate;
            }
        }
    }
}
"#;
    let service = SysmlService::empty();
    service.load_source("decay.sysml", USAGE_FORM).expect("load");
    let created = create_sweep(&service, &[json!({ "rate": 0.5 })]);
    let sid = &child_ids(&created)[0];
    step_child(&service, sid, 4);

    let level = final_value(&service, sid, "level").expect("level must be captured");
    // level(0)=10, minus rate=0.5 each tick. The seed tick counts, so after 4
    // steps the state has advanced 4 times: 10 - 4*0.5 = 8.
    assert!(
        (level - 8.0).abs() < 1e-6,
        "expected the RETURN expression (level - rate) to drive the state, got {level}. \
         A value near 0.25 means the bound `timeStep` input was used as the result."
    );
}

/// `damped-oscillator` declares TWO state variables (`x`, `v`) and one scalar
/// `getNextState`. The normative library (`StateSpaceRepresentation`, Domain
/// Libraries/Analysis) declares `StateSpace :> VectorQuantityValue` and
/// `GetNextState { return : StateSpace }` — the result is the whole next-state
/// vector, not one component to replicate.
///
/// The detector used to broadcast that single expression across both states.
/// The sweep then ran, reported five children `complete`, and returned the
/// SAME number for every zeta — numbers that were not the model's, presented
/// with no indication anything was wrong. Refusing is the honest outcome.
#[test]
fn an_under_determined_model_is_refused_rather_than_broadcast() {
    let service = workspace("damped-oscillator");
    let err = execute_command(
        &service,
        "sysml.batch.create",
        json!({
            "kind": "sweep",
            "uri": "__workspace__",
            "children_params": serde_json::to_string(&zeta_grid()).unwrap(),
        }),
    )
    .expect_err("a model with 2 states and 1 next-state return must not silently run");

    let message = err.to_string();
    // The diagnostic has to be actionable: what is wrong, where, and the counts.
    assert!(
        message.contains("2 state variables"),
        "the error must state the mismatch, got: {message}"
    );
    assert!(
        message.contains("GetNextState"),
        "the error must name the calc kind, got: {message}"
    );
    assert!(
        message.contains("one return per state variable"),
        "the error must say how to fix it, got: {message}"
    );
    // And it must prove detection worked at all — naming the subsystem means
    // the usage-form `:>` WAS resolved; this is not the old
    // "no subsystems found" failure wearing a new coat.
    assert!(
        message.contains("DampedOscillatorModel"),
        "the error must name the detected subsystem, got: {message}"
    );
}

// ---------------------------------------------------------------------------
// C — the diagnostic itself
// ---------------------------------------------------------------------------

/// An ODE whose solver cannot be built used to be skipped, and the skip
/// surfaced ~600 lines later as `internal: … no registered subsystem at
/// slot-mint time`, blaming the mint step for a failure that happened at
/// solver build and discarding the reason. Skipping was never survivable —
/// the mint step hard-fails on any unregistered detection — so the only thing
/// the swallow bought was a worse message.
#[test]
fn an_unbuildable_ode_reports_why_not_a_mint_gap() {
    const BAD_RHS: &str = r#"
package Broken {
    private import StateSpaceRepresentation::*;
    part def Decaying {
        attribute rate : ScalarValues::Real default 0.5;
        out attribute level : ScalarValues::Real default 10.0;
        calc def LevelDerivative :> GetDerivative {
            return dLevel = -rate * level ~~~ 3;
        }
    }
}
"#;
    let service = SysmlService::empty();
    service.load_source("broken.sysml", BAD_RHS).expect("load");

    let err = execute_command(
        &service,
        "sysml.orchestrate.workspace.start",
        json!({ "uri": "__workspace__" }),
    )
    .expect_err("an ODE with an uncompilable derivative must fail the build");

    let message = err.to_string();
    assert!(
        message.contains("could not be built"),
        "the error must name the stage that actually failed, got: {message}"
    );
    assert!(
        !message.contains("mint-gap"),
        "a solver-build failure must not be reported as a slot-mint gap: {message}"
    );
}
