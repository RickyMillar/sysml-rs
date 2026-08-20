//! P0 baseline tests for the sessions.* unified-shape collapse (top-3).
//!
//! These tests pin the TARGET (post-collapse) shape of the
//! `sysml.sessions.step` and `sysml.sessions.inject` commands:
//!
//!   * exactly ONE command per concept (no `_with_overrides` twin)
//!   * `overrides` is an OPTIONAL arg on the unified command name
//!
//! All four tests are `#[ignore]`'d at P0 — Phase 1 will land the actual
//! collapse and un-ignore them.
//!
//! Fixture-load pattern, SysmlService construction, and assertion style are
//! intentionally copied from `contract_sessions.rs` to keep the contract
//! suite uniform.

use std::path::Path;

use serde_json::json;
use sysml_service::execution::{SessionKind, SessionSummary};
use sysml_service::{execute_command, SysmlService};

// ---------------------------------------------------------------------------
// Helpers (mirrors contract_sessions.rs)
// ---------------------------------------------------------------------------

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/tooling
        .unwrap()
        .parent() // crates
        .unwrap()
        .parent() // repo root
        .unwrap()
}

fn start_sim_session(service: &SysmlService) -> String {
    let valve_path = repo_root().join("examples/valve-gating/ValveGating.sysml");
    let uri = service
        .loaded_uris()
        .into_iter()
        .find(|u| u.contains("ValveGating"));
    let uri = if let Some(u) = uri {
        u
    } else {
        service.load_file(&valve_path).unwrap();
        service
            .loaded_uris()
            .into_iter()
            .find(|u| u.contains("ValveGating"))
            .expect("ValveGating URI")
    };

    let result = execute_command(
        service,
        "sysml.simulate.start",
        json!({ "uri": uri, "sm_name": "ValveSM" }),
    )
    .unwrap();

    let arr = result.as_array().expect("simulate.start returns array");
    arr[0].as_str().expect("session key is string").to_owned()
}

// ---------------------------------------------------------------------------
// sessions.step — unified shape (no overrides)
// ---------------------------------------------------------------------------

/// After the collapse, `sysml.sessions.step` MUST accept
/// `{ session_id, event? }` with no `overrides` field and return a
/// SessionSummary whose tick has advanced by 1.
#[test]
fn contract_sessions_step_unified_no_overrides() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    // Capture pre-step tick from sessions.list.
    let list_before = execute_command(&service, "sysml.sessions.list", json!({})).unwrap();
    let before: Vec<SessionSummary> = serde_json::from_value(list_before).unwrap();
    let pre_tick = before
        .iter()
        .find(|s| s.id == key)
        .expect("session present")
        .tick;

    let result = execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": key, "event": null }),
    )
    .unwrap();
    let summary: SessionSummary =
        serde_json::from_value(result).expect("round-trip SessionSummary");
    assert_eq!(summary.id, key);
    assert_eq!(summary.kind, SessionKind::Simulation);
    assert_eq!(
        summary.tick,
        pre_tick + 1,
        "unified sessions.step must advance tick by exactly 1"
    );
}

// ---------------------------------------------------------------------------
// sessions.step — unified shape (WITH overrides)
// ---------------------------------------------------------------------------

/// After the collapse, the SAME `sysml.sessions.step` command MUST accept
/// an optional `overrides` arg — there is no separate `step_with_overrides`.
#[test]
fn contract_sessions_step_unified_with_overrides() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    let list_before = execute_command(&service, "sysml.sessions.list", json!({})).unwrap();
    let before: Vec<SessionSummary> = serde_json::from_value(list_before).unwrap();
    let pre_tick = before
        .iter()
        .find(|s| s.id == key)
        .expect("session present")
        .tick;

    let result = execute_command(
        &service,
        "sysml.sessions.step",
        json!({
            "session_id": key,
            "event": null,
            // `tick` is a real, resolvable override target (a runtime context
            // variable seeded into every session). RS002 fail-hard rejects
            // unknown names, so this must name something that actually exists
            // in the ValveSM session — ValveSM has no model AttributeUsages of
            // its own, so `tick` is the genuine in-scope target.
            "overrides": [["tick", "2.0"]],
        }),
    )
    .unwrap();
    let summary: SessionSummary =
        serde_json::from_value(result).expect("round-trip SessionSummary");
    assert_eq!(summary.id, key);
    assert_eq!(
        summary.tick,
        pre_tick + 1,
        "unified sessions.step (with overrides) must still advance tick by 1"
    );
}

// ---------------------------------------------------------------------------
// sessions.inject — unified shape (no overrides)
// ---------------------------------------------------------------------------

/// After the collapse, `sysml.sessions.inject` MUST accept
/// `{ session_id, subsystem, event }` with no `overrides` field and return
/// a SessionSummary.
#[test]
fn contract_sessions_inject_unified_no_overrides() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    let result = execute_command(
        &service,
        "sysml.sessions.inject",
        json!({
            "session_id": key,
            "subsystem": "ValveSM",
            "event": "close_valve",
        }),
    )
    .unwrap();
    let summary: SessionSummary =
        serde_json::from_value(result).expect("round-trip SessionSummary");
    assert_eq!(summary.id, key);
    assert_eq!(summary.kind, SessionKind::Simulation);
}

// ---------------------------------------------------------------------------
// sessions.inject — unified shape (WITH overrides)
// ---------------------------------------------------------------------------

/// After the collapse, the SAME `sysml.sessions.inject` command MUST accept
/// an optional `overrides` arg — there is no separate `inject_with_overrides`.
#[test]
fn contract_sessions_inject_unified_with_overrides() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    let result = execute_command(
        &service,
        "sysml.sessions.inject",
        json!({
            "session_id": key,
            "subsystem": "ValveSM",
            "event": "close_valve",
            // `tick` is a real, resolvable override target (see the step test);
            // RS002 fail-hard rejects unknown names.
            "overrides": [["tick", "3.0"]],
        }),
    )
    .unwrap();
    let summary: SessionSummary =
        serde_json::from_value(result).expect("round-trip SessionSummary");
    assert_eq!(summary.id, key);
    assert_eq!(summary.kind, SessionKind::Simulation);
}
