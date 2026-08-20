//! Create-time scenario overrides on `sysml.sessions.create` — the paths the
//! espresso-pump fixture cannot reach.
//!
//! `espresso_pump_service.rs` covers the orchestrator happy path on a real
//! coupled workspace. What it cannot cover is the REFUSAL: that fixture has ODE
//! dynamics, so even naming its state machine routes to the orchestrator. This
//! suite uses a deliberately ODE-free model so `sessions.create` actually picks
//! the single-SM builder, which has no override seeding.
//!
//! The rule under test is "fail hard, never drop": a scenario override that is
//! accepted and then quietly ignored is the worst outcome available here — the
//! run looks configured, produces baseline numbers, and reads as evidence.

use serde_json::json;
use sysml_service::{execute_command, ServiceError, SysmlService};

/// One state machine, one attribute, no continuous dynamics — so
/// `workspace_capabilities().has_ode_dynamics` is false and a named target
/// selects `Pick::Simulation`.
const ODE_FREE: &str = r#"
package Gate {
    attribute openThreshold : ScalarValues::Real default 4.0;

    state def GateCycle {
        entry; then closed;
        state closed;
        state open;
        transition closed then open;
    }
}
"#;

fn service() -> SysmlService {
    let service = SysmlService::empty();
    service
        .load_source("gate.sysml", ODE_FREE)
        .expect("load the ODE-free model");
    service
}

fn overrides() -> Vec<(String, String)> {
    vec![("openThreshold".to_string(), "9.0".to_string())]
}

#[test]
fn overrides_on_a_single_sm_target_are_refused_not_dropped() {
    let service = service();

    // Sanity: without overrides this target really does select the single-SM
    // builder. If this ever becomes an orchestrator the refusal below stops
    // testing what it claims to.
    let plain = service
        .sessions_create("gate.sysml", Some("GateCycle"), None, None, None)
        .expect("create a plain single-SM session");
    assert_eq!(
        plain.kind,
        sysml_service::execution::SessionKind::Simulation,
        "fixture must stay ODE-free for this suite to mean anything",
    );

    let err = service
        .sessions_create("gate.sysml", Some("GateCycle"), None, None, Some(&overrides()))
        .expect_err("a scenario the builder cannot seed must be refused");
    match err {
        ServiceError::InvalidInput(msg) => {
            // The message has to tell the caller what to do instead, or the
            // fail-hard just moves the dead end.
            assert!(
                msg.contains("orchestrator"),
                "error should name the supported session kind: {msg}",
            );
            assert!(
                msg.contains("sessions.step"),
                "error should point at the mid-run alternative: {msg}",
            );
        }
        other => panic!("expected InvalidInput, got {other:?}"),
    }
}

#[test]
fn an_empty_override_list_is_not_a_scenario() {
    let service = service();
    // `Some(&[])` must behave exactly like `None` — an empty list is "no
    // scenario", so it must not trip the non-orchestrator refusal.
    let empty: Vec<(String, String)> = Vec::new();
    let s = service
        .sessions_create("gate.sysml", Some("GateCycle"), None, None, Some(&empty))
        .expect("an empty override list is not a scenario request");
    assert!(s.create_overrides.is_empty());
}

#[test]
fn create_overrides_round_trip_through_json_dispatch() {
    let service = SysmlService::empty();
    service
        .load_source("gate.sysml", ODE_FREE)
        .expect("load");

    // The palette / REST / MCP transports all arrive as JSON, so the parameter
    // has to deserialize from the wire shape, not just the Rust signature.
    let out = execute_command(
        &service,
        "sysml.sessions.create",
        json!({
            "uri": "gate.sysml",
            "target": "GateCycle",
            "overrides": [["openThreshold", "9.0"]],
        }),
    );
    let err = out.expect_err("refusal must survive the JSON boundary");
    assert!(
        format!("{err:?}").contains("orchestrator"),
        "expected the create-time-override refusal, got {err:?}",
    );
}
