//! S4.T3 — `sysml.workspace.capabilities` wire contract.
//!
//! Locks the JSON shape and behaviour for the backend-owned successor
//! to the simulation-app's FE walk:
//!
//! - Empty (no workspace loaded): every flag `false`, every name list
//!   empty. The default is "non-capable" so the FE renders a quiet
//!   shell rather than guessing.
//! - Loaded workspace: flags reflect the workspace's elaborated
//!   content; field names match `WorkspaceCapabilitiesResult` (which
//!   maps onto the FE's `Capabilities` interface 1-to-1).

use serde_json::{json, Value};
use sysml_service::{execute_command, SysmlService};

#[test]
fn empty_workspace_is_non_capable() {
    let service = SysmlService::empty();
    let resp = execute_command(&service, "sysml.workspace.capabilities", json!({}))
        .expect("sysml.workspace.capabilities");
    let obj = resp.as_object().expect("response is an object");

    // All known flags must default to false.
    for flag in [
        "has_state_machines",
        "has_action_flows",
        "has_ode_dynamics",
        "has_port_flows",
        "has_multiple_subsystems",
        "has_constraints",
        "has_requirements",
        "has_trade_studies",
    ] {
        assert_eq!(
            obj.get(flag),
            Some(&Value::Bool(false)),
            "{flag} should default to false on an empty workspace"
        );
    }
    for arr_field in [
        "state_machine_names",
        "action_flow_names",
        "trade_study_names",
    ] {
        let arr = obj
            .get(arr_field)
            .and_then(Value::as_array)
            .unwrap_or_else(|| panic!("{arr_field} should be an array"));
        assert!(
            arr.is_empty(),
            "{arr_field} should be empty on an empty workspace; got {arr:?}"
        );
    }
}

#[test]
fn detects_state_and_constraint_and_requirement() {
    // Inline source keeps the test isolated from fixture drift. The
    // model includes a named state def (drives the SM flag + names),
    // a constraint def (drives has_constraints), and a requirement
    // def (drives has_requirements).
    let src = r#"
        package CapTest {
            state def Tank;
            requirement def MustBeFull;
            constraint def CapacityCheck {
                1 > 0
            }
        }
    "#;
    let service = SysmlService::empty();
    service
        .load_source("file:///cap_test.sysml", src)
        .expect("load_source");

    let resp = execute_command(&service, "sysml.workspace.capabilities", json!({}))
        .expect("sysml.workspace.capabilities");
    let obj = resp.as_object().expect("response is an object");

    assert_eq!(obj.get("has_state_machines"), Some(&Value::Bool(true)));
    assert_eq!(obj.get("has_constraints"), Some(&Value::Bool(true)));
    assert_eq!(obj.get("has_requirements"), Some(&Value::Bool(true)));

    // Negative controls — model has none of these.
    assert_eq!(obj.get("has_action_flows"), Some(&Value::Bool(false)));
    assert_eq!(obj.get("has_trade_studies"), Some(&Value::Bool(false)));
    assert_eq!(obj.get("has_port_flows"), Some(&Value::Bool(false)));

    // Single-file workspace ⇒ no multi-file orchestrator promotion.
    assert_eq!(obj.get("has_ode_dynamics"), Some(&Value::Bool(false)));

    let sm_names = obj
        .get("state_machine_names")
        .and_then(Value::as_array)
        .expect("state_machine_names array");
    assert!(
        sm_names
            .iter()
            .filter_map(Value::as_str)
            .any(|n| n == "Tank"),
        "state_machine_names should contain Tank; got {sm_names:?}"
    );

    // Round-trip the JSON unchanged so wire-format drift is caught.
    let s = serde_json::to_string(&resp).expect("serialize");
    let parsed: Value = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(resp, parsed, "workspace.capabilities JSON should round-trip");
}

#[test]
fn multi_file_with_sm_implies_orchestrator_mode() {
    // Two files, both with a named state def → the FE's heuristic
    // (workspace size > 1 && state_def_count > 0) promotes
    // has_ode_dynamics so the orchestrator panel can drive cross-file
    // execution.
    let service = SysmlService::empty();
    service
        .load_source(
            "file:///a.sysml",
            "package A { state def SubsystemA; }",
        )
        .expect("load a");
    service
        .load_source(
            "file:///b.sysml",
            "package B { state def SubsystemB; }",
        )
        .expect("load b");

    let resp = execute_command(&service, "sysml.workspace.capabilities", json!({}))
        .expect("sysml.workspace.capabilities");
    let obj = resp.as_object().expect("response is an object");

    assert_eq!(obj.get("has_state_machines"), Some(&Value::Bool(true)));
    assert_eq!(
        obj.get("has_ode_dynamics"),
        Some(&Value::Bool(true)),
        "multi-file workspace with state defs should promote has_ode_dynamics"
    );
    assert_eq!(
        obj.get("has_multiple_subsystems"),
        Some(&Value::Bool(true)),
        "two state machines should imply has_multiple_subsystems"
    );
}
