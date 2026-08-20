//! Contract gate for `sysml.sessions.create` — session-KIND inference for the
//! single-state-machine → **Simulation** branch (execution-entry-unification P4).
//!
//! `sessions.create` infers the `SessionKind` from the model + optional target
//! and always returns a `SessionSummary`. This file pins the ONE branch no
//! espresso fixture can exercise: a pure single-state-machine model (no ODE)
//! must route to a **Simulation** session. Both espresso replacement fixtures
//! (espresso-pump-hybrid, espresso-production-cell) are ODE-coupled and always
//! infer **Orchestrator**, so the Simulation-kind branch is gated here on the
//! generic, provenance-cleared `valve-gating` teaching fixture (AUD ef11f326:
//! generic Valve/Pipe, ISQ/SI, NOT Basis-derived — kept).
//!
//! The remaining `sessions.create` obligations — no-target → orchestrator,
//! state-machine-in-multi-subsystem → orchestrator, unknown-target hard error,
//! and provenance capture / fork inheritance / stop-archive — are covered on
//! public espresso fixtures by `espresso_pump_service.rs`
//! (svc_session_create_infers_kind_and_bulk_steps_exactly,
//! svc_session_unknown_target_is_hard_error, svc_timeseries_archive_and_fork_round_trip)
//! and `espresso_cell_service.rs`
//! (cell_session_is_orchestrator_with_provenance_and_exact_bulk_step). Those
//! were migrated off the legacy oscillator product model (MIG-07); the
//! oscillator-driven cases that used to live here retired against that coverage.

use std::path::Path;

use serde_json::json;
use sysml_service::execution::{SessionKind, SessionSummary};
use sysml_service::{execute_command, SysmlService};

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

fn create(service: &SysmlService, params: serde_json::Value) -> SessionSummary {
    let v = execute_command(service, "sysml.sessions.create", params)
        .expect("sessions.create runs");
    serde_json::from_value(v).expect("sessions.create returns a SessionSummary")
}

/// Single-SM, no-ODE model → simulation session targeting that SM. Driven on
/// the generic `valve-gating` fixture — the one construct shape (a pure state
/// machine with no continuous dynamics) that infers `SessionKind::Simulation`,
/// which neither ODE-coupled espresso fixture can produce.
#[test]
fn create_single_state_machine_yields_simulation() {
    let service = SysmlService::empty();
    let valve = repo_root().join("examples/valve-gating/ValveGating.sysml");
    service.load_file(&valve).expect("load ValveGating");
    let uri = service
        .loaded_uris()
        .into_iter()
        .find(|u| u.contains("ValveGating"))
        .expect("ValveGating URI");

    let summary = create(&service, json!({ "uri": uri, "target": "ValveSM" }));
    assert_eq!(summary.kind, SessionKind::Simulation);
    assert_eq!(summary.subsystem_name.as_deref(), Some("ValveSM"));

    // Parity: the legacy simulate.start produces the same kind + subsystem.
    let legacy = execute_command(
        &service,
        "sysml.simulate.start",
        json!({ "uri": uri, "sm_name": "ValveSM" }),
    )
    .expect("simulate.start runs");
    // simulate.start returns [session_key, StepResult]; assert the legacy
    // command still works alongside the unified create path above.
    assert!(legacy.get(0).and_then(|v| v.as_str()).is_some());
}
