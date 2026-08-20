//! JSON schema round-trip contract tests for the `sysml.orchestrate.*` namespace.
//!
//! Commands covered:
//! - sysml.orchestrate.start
//! - sysml.orchestrate.step
//! - sysml.orchestrate.stop
//! - sysml.orchestrate.inject
//! - sysml.orchestrate.workspace.start

use std::path::Path;

use serde_json::json;
use sysml_runtime::orchestrator::ExecutionSnapshot;
use sysml_service::{execute_command, SysmlService};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

fn load_valve_model(service: &SysmlService) -> String {
    let valve_path = repo_root().join("examples/valve-gating/ValveGating.sysml");
    service.load_file(&valve_path).unwrap();
    service
        .loaded_uris()
        .into_iter()
        .find(|u| u.contains("ValveGating"))
        .expect("ValveGating URI")
}

// ---------------------------------------------------------------------------
// orchestrate.start
// ---------------------------------------------------------------------------

#[test]
fn contract_orchestrate_start() {
    let service = SysmlService::empty();
    let uri = load_valve_model(&service);

    let result = execute_command(
        &service,
        "sysml.orchestrate.start",
        json!({ "uri": uri }),
    )
    .unwrap();

    // Returns [session_key, ExecutionSnapshot].
    let arr = result.as_array().expect("orchestrate.start returns array");
    assert_eq!(arr.len(), 2);

    let key = arr[0].as_str().expect("session key is string");
    assert!(!key.is_empty());

    let snapshot: ExecutionSnapshot =
        serde_json::from_value(arr[1].clone()).expect("round-trip ExecutionSnapshot");
    let _ = snapshot.tick;
    let _ = snapshot.time_ms;
    let _ = snapshot.subsystem_states;
    let _ = snapshot.variables;
    let _ = snapshot.messages;
    let _ = snapshot.constraint_results;
    let _ = snapshot.guard_diagnoses;
    let _ = snapshot.causation_links;
    let _ = snapshot.completed;
}

// ---------------------------------------------------------------------------
// orchestrate.step
// ---------------------------------------------------------------------------

#[test]
fn contract_orchestrate_step() {
    let service = SysmlService::empty();
    let uri = load_valve_model(&service);

    let start_result = execute_command(
        &service,
        "sysml.orchestrate.start",
        json!({ "uri": uri }),
    )
    .unwrap();
    let key = start_result.as_array().unwrap()[0]
        .as_str()
        .unwrap()
        .to_owned();

    let result = execute_command(
        &service,
        "sysml.orchestrate.step",
        json!({ "session_key": key }),
    )
    .unwrap();
    let snapshot: ExecutionSnapshot =
        serde_json::from_value(result).expect("round-trip ExecutionSnapshot");
    assert!(snapshot.tick >= 1);
}

// ---------------------------------------------------------------------------
// orchestrate.inject
// ---------------------------------------------------------------------------

#[test]
fn contract_orchestrate_inject() {
    let service = SysmlService::empty();
    let uri = load_valve_model(&service);

    let start_result = execute_command(
        &service,
        "sysml.orchestrate.start",
        json!({ "uri": uri }),
    )
    .unwrap();
    let key = start_result.as_array().unwrap()[0]
        .as_str()
        .unwrap()
        .to_owned();

    let result = execute_command(
        &service,
        "sysml.orchestrate.inject",
        json!({
            "session_key": key,
            "subsystem": "ValveSM",
            "event": "close_valve"
        }),
    )
    .unwrap();
    let snapshot: ExecutionSnapshot =
        serde_json::from_value(result).expect("round-trip ExecutionSnapshot");
    let _ = snapshot.tick;
}

// ---------------------------------------------------------------------------
// orchestrate.stop
// ---------------------------------------------------------------------------

#[test]
fn contract_orchestrate_stop() {
    let service = SysmlService::empty();
    let uri = load_valve_model(&service);

    let start_result = execute_command(
        &service,
        "sysml.orchestrate.start",
        json!({ "uri": uri }),
    )
    .unwrap();
    let key = start_result.as_array().unwrap()[0]
        .as_str()
        .unwrap()
        .to_owned();

    let result = execute_command(
        &service,
        "sysml.orchestrate.stop",
        json!({ "session_key": key }),
    )
    .unwrap();
    assert!(result.is_null(), "orchestrate.stop returns ()");
}

// ---------------------------------------------------------------------------
// orchestrate.workspace.start
// ---------------------------------------------------------------------------

#[test]
fn contract_orchestrate_workspace_start() {
    let service = SysmlService::empty();
    let uri = load_valve_model(&service);

    let result = execute_command(
        &service,
        "sysml.orchestrate.workspace.start",
        json!({ "uri": uri }),
    )
    .unwrap();

    // Returns [session_key, ExecutionSnapshot].
    let arr = result.as_array().expect("workspace.start returns array");
    assert_eq!(arr.len(), 2);

    let key = arr[0].as_str().expect("session key is string");
    assert!(!key.is_empty());

    let snapshot: ExecutionSnapshot =
        serde_json::from_value(arr[1].clone()).expect("round-trip ExecutionSnapshot");
    let _ = snapshot.tick;
    let _ = snapshot.subsystem_states;
}

// ---------------------------------------------------------------------------
// orchestrate.workspace.step (not a registered command — workspace.start
// returns a session that is stepped via orchestrate.step)
// ---------------------------------------------------------------------------
// Note: The plan mentioned workspace.step but the actual service only has
// orchestrate.workspace.start.  Stepping is done via orchestrate.step on the
// returned key. Covered above.
