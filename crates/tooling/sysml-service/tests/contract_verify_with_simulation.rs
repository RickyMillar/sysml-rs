//! Contract gate for the `sysml.verify_with_simulation` demotion (B4 second
//! verdict-path removal).
//!
//! The command used to flat-evaluate model constraints to `overall_satisfied:
//! bool` — a SECOND verdict path competing with `VerificationRunner` (the B4
//! violation the verification invariants forbid). It now runs the ODE/SM
//! simulation, overlays the run's settled values, and routes every declared
//! verification case through the ONE verdict engine, returning a `VerdictKind`.
//!
//! This gate pins that contract on a real hybrid SM+ODE run:
//!   - the output carries a `verdict` (VerdictKind string), NOT `overall_satisfied`;
//!   - no `sim_*` magic keys leak (B1) — the response has no `constraints` block;
//!   - declared verification cases are discovered and routed through the engine
//!     (`cases` is populated).
//!
//! The verdict-flips-with-the-simulated-value semantics are gated at the runtime
//! seam by `sysml_runtime::observables::tests::measured_observable_drives_verification_verdict`.
//!
//! `run_simulation` only engages on a *hybrid* SM+ODE model, and the only such
//! fixtures in the corpus carry verification cases — so the no-case "honest
//! Inconclusive" twin and the value-driven flip live in the runtime gate above;
//! this gate proves the service wiring (discovery + routing + shape) end to end.

use std::path::PathBuf;

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

fn pump_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("examples/espresso-pump-hybrid")
}

#[test]
fn sim_verdict_routes_through_verification_runner() {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(pump_root()))
        .expect("open pump workspace");

    // A real file URI in the project so the service resolves the project root
    // (for `@DataSource` CSV lookup tables) — `verify_with_simulation` still
    // elaborates the whole workspace graph from it.
    let uri = service
        .loaded_uris()
        .into_iter()
        .find(|u| u.contains("PumpCycle"))
        .expect("PumpCycle URI");

    // The pump physics runs at a dt≈2.0 ms step; a horizon of a few hundred
    // ticks reaches a verdict for every declared case.
    let result = execute_command(
        &service,
        "sysml.verify_with_simulation",
        json!({
            "uri": uri,
            "sm_name": "PumpCycle",
            "overrides": [],
            "dt_ms": 2.0,
            "max_time_ms": 6000.0,
        }),
    )
    .expect("verify_with_simulation runs");

    // B4: the verdict is a VerdictKind string from VerificationRunner.
    let verdict = result
        .get("verdict")
        .and_then(|v| v.as_str())
        .expect("response carries a `verdict` string");
    assert!(
        ["pass", "fail", "inconclusive", "error"].contains(&verdict),
        "verdict must be a VerdictKind, got {verdict:?}"
    );

    // §2.1a(d) / study §3.4: the verdict-bearing wire ALWAYS labels its
    // evaluation mode; a live ODE run is `trajectory`.
    assert_eq!(
        result.get("evaluation_mode").and_then(|v| v.as_str()),
        Some("trajectory"),
        "verify_with_simulation is a trajectory-mode verdict"
    );

    // The old second-verdict-path surface is gone: no bool, no flat constraints.
    assert!(
        result.get("overall_satisfied").is_none(),
        "the `overall_satisfied` bool (second verdict path) must be removed"
    );
    assert!(
        result.get("constraints").is_none(),
        "the flat constraint block must be removed"
    );

    // The new shape is present and the declared verification case was discovered
    // and routed through VerificationRunner.
    assert!(result.get("simulation").is_some(), "keeps the simulation block");
    let cases = result
        .get("cases")
        .and_then(|c| c.as_array())
        .expect("carries a `cases` array");
    assert!(
        !cases.is_empty(),
        "the pump fixture declares verification cases — a case must be discovered and routed"
    );
    assert!(
        result
            .get("requirement_results")
            .and_then(|r| r.as_array())
            .is_some(),
        "carries a `requirement_results` array"
    );
}

/// The time-series trace variant is the same trajectory evaluation surfaced for
/// charting; it must carry the same `evaluation_mode` label (§2.1a(d), §3.4).
#[test]
fn sim_trace_carries_trajectory_mode() {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(pump_root()))
        .expect("open pump workspace");

    let uri = service
        .loaded_uris()
        .into_iter()
        .find(|u| u.contains("PumpCycle"))
        .expect("PumpCycle URI");

    let result = execute_command(
        &service,
        "sysml.verify_with_simulation_trace",
        json!({
            "uri": uri,
            "sm_name": "PumpCycle",
            "overrides": [],
            "dt_ms": 2.0,
            "max_time_ms": 6000.0,
        }),
    )
    .expect("verify_with_simulation_trace runs");

    assert_eq!(
        result.get("evaluation_mode").and_then(|v| v.as_str()),
        Some("trajectory"),
        "verify_with_simulation_trace is a trajectory-mode surface"
    );
    assert!(
        result.get("time_series").is_some(),
        "keeps the time_series block"
    );
}
