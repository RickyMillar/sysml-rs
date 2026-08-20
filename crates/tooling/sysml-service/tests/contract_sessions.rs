//! JSON schema round-trip contract tests for the `sysml.sessions.*` namespace.
//!
//! Each test dispatches through `execute_command` (the type-erased JSON path),
//! serializes the response to JSON, then deserializes back into the expected
//! response type.  If a field is removed, renamed, or changes type, these
//! tests break — that is the point.

use std::path::Path;

use serde_json::json;
use sysml_service::execution::{
    BucketUsage, SessionDetail, SessionDivergence, SessionKind, SessionQuota, SessionSummary,
    SessionTimelineDivergence, SubsystemSummary,
};
use sysml_service::{execute_command, ServiceError, SysmlService};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Locate the repo root (the parent of `crates/`).
fn repo_root() -> &'static Path {
    // The test binary runs from the workspace root.
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent() // crates/tooling
        .unwrap()
        .parent() // crates
        .unwrap()
        .parent() // repo root
        .unwrap()
}

/// Start a simulation session using the ValveGating example (simple 2-state SM).
fn start_sim_session(service: &SysmlService) -> String {
    let valve_path = repo_root().join("examples/valve-gating/ValveGating.sysml");
    let uri = service.loaded_uris().into_iter().find(|u| u.contains("ValveGating"));
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

    // simulate.start returns a tuple serialized as [session_key, StepResult].
    let arr = result.as_array().expect("simulate.start returns array");
    arr[0].as_str().expect("session key is string").to_owned()
}

/// Start an orchestrator session for testing.
fn start_orchestrator_session(service: &SysmlService) -> String {
    let valve_path = repo_root().join("examples/valve-gating/ValveGating.sysml");
    let uri = service.loaded_uris().into_iter().find(|u| u.contains("ValveGating"));
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
        "sysml.orchestrate.start",
        json!({ "uri": uri }),
    )
    .unwrap();

    let arr = result.as_array().expect("orchestrate.start returns array");
    arr[0].as_str().expect("session key is string").to_owned()
}

// ---------------------------------------------------------------------------
// sessions.list
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_list_empty() {
    let service = SysmlService::empty();
    let result = execute_command(&service, "sysml.sessions.list", json!({})).unwrap();
    let summaries: Vec<SessionSummary> =
        serde_json::from_value(result.clone()).expect("round-trip Vec<SessionSummary>");
    assert!(summaries.is_empty());
}

#[test]
fn contract_sessions_list_populated() {
    let service = SysmlService::empty();
    let _key = start_sim_session(&service);

    let result = execute_command(&service, "sysml.sessions.list", json!({})).unwrap();
    let summaries: Vec<SessionSummary> =
        serde_json::from_value(result).expect("round-trip Vec<SessionSummary>");
    assert_eq!(summaries.len(), 1);

    let s = &summaries[0];
    // Assert stable field presence.
    assert_eq!(s.kind, SessionKind::Simulation);
    assert!(!s.id.is_empty());
    assert!(s.created_at_ms > 0);
    assert!(!s.is_expired);
}

// ---------------------------------------------------------------------------
// sessions.info
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_info() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    let result = execute_command(
        &service,
        "sysml.sessions.info",
        json!({ "session_id": key }),
    )
    .unwrap();
    let detail: Option<SessionDetail> =
        serde_json::from_value(result).expect("round-trip Option<SessionDetail>");
    let detail = detail.expect("detail should be Some for live session");

    assert_eq!(detail.summary.id, key);
    assert_eq!(detail.summary.kind, SessionKind::Simulation);
    // subsystems field is present (may be empty or populated).
    let _ = detail.subsystems;
    // latest_snapshot is present and roundtrips.
    let _ = detail.latest_snapshot;
}

#[test]
fn contract_sessions_info_missing() {
    let service = SysmlService::empty();
    let result = execute_command(
        &service,
        "sysml.sessions.info",
        json!({ "session_id": "no-such-id" }),
    )
    .unwrap();
    let detail: Option<SessionDetail> =
        serde_json::from_value(result).expect("round-trip None");
    assert!(detail.is_none());
}

// ---------------------------------------------------------------------------
// sessions.step
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_step() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    let result = execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": key, "event": null }),
    )
    .unwrap();
    let summary: SessionSummary =
        serde_json::from_value(result).expect("round-trip SessionSummary");
    assert_eq!(summary.id, key);
    assert!(summary.tick >= 1);
    assert!(summary.history_len >= 1);
}

// ---------------------------------------------------------------------------
// sessions.inject
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_inject() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    let result = execute_command(
        &service,
        "sysml.sessions.inject",
        json!({ "session_id": key, "subsystem": "ValveSM", "event": "close_valve" }),
    )
    .unwrap();
    let summary: SessionSummary =
        serde_json::from_value(result).expect("round-trip SessionSummary");
    assert_eq!(summary.id, key);
}

// ---------------------------------------------------------------------------
// sessions.reset
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_reset() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    // Step once to build history.
    execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": key, "event": null }),
    )
    .unwrap();

    let result = execute_command(
        &service,
        "sysml.sessions.reset",
        json!({ "session_id": key }),
    )
    .unwrap();
    let summary: SessionSummary =
        serde_json::from_value(result).expect("round-trip SessionSummary");
    assert_eq!(summary.id, key);
    assert_eq!(summary.history_len, 0);
    assert!(!summary.is_expired);
}

// ---------------------------------------------------------------------------
// sessions.fork
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_fork() {
    let service = SysmlService::empty();
    let parent_key = start_sim_session(&service);

    // Step parent so there's a snapshot to seed the child.
    execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": parent_key, "event": null }),
    )
    .unwrap();

    let result = execute_command(
        &service,
        "sysml.sessions.fork",
        json!({ "session_id": parent_key }),
    )
    .unwrap();
    let child: SessionSummary =
        serde_json::from_value(result).expect("round-trip SessionSummary (fork child)");
    assert_ne!(child.id, parent_key);
    assert_eq!(child.kind, SessionKind::Simulation);
    assert!(child.fork_point_tick.is_some());
}

// ---------------------------------------------------------------------------
// sessions.fork_with_overrides
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_fork_with_overrides() {
    let service = SysmlService::empty();
    let parent_key = start_sim_session(&service);

    execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": parent_key, "event": null }),
    )
    .unwrap();

    // RSC-2.5 (RS002): override targets must resolve to a runtime slot alias or
    // an existing context variable — unknown names are a hard error (silent
    // creation was removed in `3934fe84`). `tick` is the clock variable the
    // orchestrator seeds on every step (orchestrator.rs), so after the step
    // above it is guaranteed present and resolvable. (Value-parsing semantics
    // are pinned separately by the lib.rs unit tests.)
    let result = execute_command(
        &service,
        "sysml.sessions.fork_with_overrides",
        json!({
            "session_id": parent_key,
            "overrides": [["tick", "5"]]
        }),
    )
    .unwrap();
    let child: SessionSummary =
        serde_json::from_value(result).expect("round-trip SessionSummary (fork+override)");
    assert_ne!(child.id, parent_key);
    assert!(child.fork_point_tick.is_some());
}

/// Triage follow-up (P5 escalation #1): an RS002-class override-resolution
/// failure is caller-input-shaped (a mistyped or stale target/value), never
/// an internal invariant violation — it must map to `ServiceError::InvalidInput`
/// (HTTP 400 in `sysml-api`'s `service_err_response`), not the generic
/// `ServiceError::Execution` catch-all (HTTP 500). This was the actual bug:
/// every override failure surfaced as a 500 regardless of cause. Covers all
/// three `apply_overrides_with_aliases` call sites' error path via the
/// `sessions.step` one; `sessions.inject`/`sessions.fork_with_overrides` share
/// the identical `.map_err` line.
#[test]
fn contract_sessions_step_unknown_override_is_invalid_input_not_execution() {
    let service = SysmlService::empty();
    let parent_key = start_sim_session(&service);

    let err = execute_command(
        &service,
        "sysml.sessions.step",
        json!({
            "session_id": parent_key,
            "event": null,
            "overrides": [["rsc2NoSuchVariableXyz", "1.0"]],
        }),
    )
    .expect_err("unknown override target must fail");

    assert!(
        matches!(err, ServiceError::InvalidInput(_)),
        "RS002-class override errors must map to InvalidInput (HTTP 400), \
         not the Execution catch-all (HTTP 500); got {err:?}"
    );
    assert!(
        err.to_string().contains("RS002"),
        "the underlying RS002 message must still be present: {err}"
    );
}

/// R4: omitting `at_tick` (null) preserves the old fork-from-current
/// behaviour — backward compatibility guard.
#[test]
fn contract_sessions_fork_with_overrides_at_tick_null_is_backward_compat() {
    let service = SysmlService::empty();
    let parent_key = start_sim_session(&service);
    execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": parent_key, "event": null }),
    )
    .unwrap();

    // RSC-2.5 (RS002): `tick` is the guaranteed post-step context variable used
    // as a resolvable override target (see the sibling test above).
    let result = execute_command(
        &service,
        "sysml.sessions.fork_with_overrides",
        json!({
            "session_id": parent_key,
            "overrides": [["tick", "5"]],
            "at_tick": null,
        }),
    )
    .unwrap();
    let child: SessionSummary = serde_json::from_value(result).unwrap();
    assert_ne!(child.id, parent_key);
    assert!(child.fork_point_tick.is_some());
}

/// R4: asking for a tick past the parent's current tick produces a
/// structured `FutureTick` error serialised as tagged JSON.
#[test]
fn contract_sessions_fork_with_overrides_future_tick_is_structured() {
    let service = SysmlService::empty();
    let parent_key = start_sim_session(&service);
    execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": parent_key, "event": null }),
    )
    .unwrap();

    let err = execute_command(
        &service,
        "sysml.sessions.fork_with_overrides",
        json!({
            "session_id": parent_key,
            "overrides": [],
            "at_tick": 9_999u64,
        }),
    )
    .expect_err("future tick must error");
    let text = err.to_string();
    // The Display impl renders the ForkAtTickError JSON payload.
    let payload: serde_json::Value =
        serde_json::from_str(&text).expect("error Display must be JSON");
    assert_eq!(payload["kind"], "FutureTick");
    assert_eq!(payload["tick"], 9_999u64);
}

/// R4: asking for a tick that has been evicted from the archive
/// produces a structured `SnapshotMissing` error.
#[test]
fn contract_sessions_fork_with_overrides_snapshot_missing_is_structured() {
    use sysml_service::execution::DEFAULT_SNAPSHOT_RETENTION_TICKS;

    let service = SysmlService::empty();
    let parent_key = start_sim_session(&service);

    // Step enough ticks to guarantee tick 0..=k eviction. Force a small
    // retention window on the live session via the infra-level knob so
    // we don't have to step DEFAULT_SNAPSHOT_RETENTION_TICKS+1 times.
    service
        .set_session_snapshot_retention(&parent_key, 2)
        .expect("set_session_snapshot_retention should find the session");
    // This test exercises RETENTION/eviction, not the archive-cadence
    // feature (UX closeout arc #7, default cadence = 100 ticks) — force
    // every-tick archiving so the two orthogonal knobs don't interact.
    service
        .set_session_archive_cadence(&parent_key, 1)
        .expect("set_session_archive_cadence should find the session");

    // Step 5 ticks — archive retains only the last 2.
    for _ in 0..5 {
        execute_command(
            &service,
            "sysml.sessions.step",
            json!({ "session_id": parent_key, "event": null }),
        )
        .unwrap();
    }
    let _ = DEFAULT_SNAPSHOT_RETENTION_TICKS; // referenced for doc clarity

    let err = execute_command(
        &service,
        "sysml.sessions.fork_with_overrides",
        json!({
            "session_id": parent_key,
            "overrides": [],
            "at_tick": 1u64,
        }),
    )
    .expect_err("evicted tick must error");
    let payload: serde_json::Value =
        serde_json::from_str(&err.to_string()).expect("error Display must be JSON");
    assert_eq!(payload["kind"], "SnapshotMissing");
    assert_eq!(payload["tick"], 1);
    // earliest_available must be reported — at least 3 (5 ticks, retain 2).
    assert!(payload["earliest_available"].is_u64());
    // UX closeout arc #7 §2.3: FAIL-HARD carries the exact valid ticks, not
    // a silent clamp — with retention=2, exactly two consecutive ticks
    // (the two most recent) are retained: `earliest_available` and its
    // successor.
    let earliest = payload["earliest_available"].as_u64().unwrap();
    assert_eq!(
        payload["valid_ticks"],
        serde_json::json!([earliest, earliest + 1]),
        "valid_ticks must list the exact forkable ticks, not a nearest-match suggestion"
    );
}

/// R4: the service advertises fork-at-tick support via the new
/// `sysml.system.capabilities` command.
#[test]
fn contract_system_capabilities_reports_fork_at_tick() {
    let service = SysmlService::empty();
    let result = execute_command(&service, "sysml.system.capabilities", json!({})).unwrap();
    assert_eq!(result["has_fork_at_tick"], true);
    assert!(
        result["snapshot_retention_ticks"].is_u64(),
        "snapshot_retention_ticks must be a number",
    );
    assert_eq!(result["snapshot_retention_ticks"], 256u64);
}

// ---------------------------------------------------------------------------
// sessions.rename
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_rename() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    // rename returns () which serializes as null.
    let result = execute_command(
        &service,
        "sysml.sessions.rename",
        json!({ "session_id": key, "label": "test label" }),
    )
    .unwrap();
    assert!(result.is_null(), "rename returns ()");

    // Verify label is set via info.
    let info_result = execute_command(
        &service,
        "sysml.sessions.info",
        json!({ "session_id": key }),
    )
    .unwrap();
    let detail: Option<SessionDetail> = serde_json::from_value(info_result).unwrap();
    assert_eq!(
        detail.unwrap().summary.label.as_deref(),
        Some("test label")
    );
}

// ---------------------------------------------------------------------------
// sessions.subsystems
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_subsystems() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    let result = execute_command(
        &service,
        "sysml.sessions.subsystems",
        json!({ "session_id": key }),
    )
    .unwrap();
    let subs: Vec<SubsystemSummary> =
        serde_json::from_value(result).expect("round-trip Vec<SubsystemSummary>");
    // simulate.start creates one subsystem.
    assert!(!subs.is_empty());
    // Assert field presence on the first entry.
    let first = &subs[0];
    assert!(!first.name.is_empty());
    assert!(!first.kind_label.is_empty());
    let _ = &first.current_state;
    let _ = first.completed;
    let _ = &first.available_transitions;
}

// ---------------------------------------------------------------------------
// sessions.diff
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_diff() {
    let service = SysmlService::empty();
    let parent_key = start_sim_session(&service);

    // Step and fork.
    execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": parent_key, "event": null }),
    )
    .unwrap();
    let fork_result = execute_command(
        &service,
        "sysml.sessions.fork",
        json!({ "session_id": parent_key }),
    )
    .unwrap();
    let child: SessionSummary = serde_json::from_value(fork_result).unwrap();

    let result = execute_command(
        &service,
        "sysml.sessions.diff",
        json!({ "a_id": parent_key, "b_id": child.id }),
    )
    .unwrap();
    let diff: SessionDivergence =
        serde_json::from_value(result).expect("round-trip SessionDivergence");
    assert_eq!(diff.a_id, parent_key);
    assert_eq!(diff.b_id, child.id);
    let _ = diff.current_tick_a;
    let _ = diff.current_tick_b;
    let _ = diff.subsystem_diffs;
    let _ = diff.variable_diffs;
}

// ---------------------------------------------------------------------------
// sessions.diff_timeline
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_diff_timeline() {
    let service = SysmlService::empty();
    let parent_key = start_sim_session(&service);

    execute_command(
        &service,
        "sysml.sessions.step",
        json!({ "session_id": parent_key, "event": null }),
    )
    .unwrap();
    let fork_result = execute_command(
        &service,
        "sysml.sessions.fork",
        json!({ "session_id": parent_key }),
    )
    .unwrap();
    let child: SessionSummary = serde_json::from_value(fork_result).unwrap();

    let result = execute_command(
        &service,
        "sysml.sessions.diff_timeline",
        json!({ "a_id": parent_key, "b_id": child.id }),
    )
    .unwrap();
    let tl: SessionTimelineDivergence =
        serde_json::from_value(result).expect("round-trip SessionTimelineDivergence");
    assert_eq!(tl.a_id, parent_key);
    assert_eq!(tl.b_id, child.id);
    let _ = tl.shared_start_tick;
    let _ = tl.shared_end_tick;
    let _ = tl.first_divergence_tick;
    let _ = tl.tick_diffs;
    let _ = tl.history_truncated;
}

// ---------------------------------------------------------------------------
// sessions.stop
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_stop() {
    let service = SysmlService::empty();
    let key = start_sim_session(&service);

    let result = execute_command(
        &service,
        "sysml.sessions.stop",
        json!({ "session_id": key }),
    )
    .unwrap();
    assert!(result.is_null(), "stop returns ()");

    // Verify session is gone.
    let list_result =
        execute_command(&service, "sysml.sessions.list", json!({})).unwrap();
    let summaries: Vec<SessionSummary> = serde_json::from_value(list_result).unwrap();
    assert!(summaries.is_empty());
}

// ---------------------------------------------------------------------------
// sessions.reap
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_reap() {
    let service = SysmlService::empty();
    // No expired sessions — reap returns 0.
    let result = execute_command(&service, "sysml.sessions.reap", json!({})).unwrap();
    let count: usize = serde_json::from_value(result).expect("round-trip usize");
    assert_eq!(count, 0);
}

// ---------------------------------------------------------------------------
// sessions.quota
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_quota() {
    let service = SysmlService::empty();
    let result = execute_command(&service, "sysml.sessions.quota", json!({})).unwrap();
    let quota: SessionQuota =
        serde_json::from_value(result).expect("round-trip SessionQuota");
    // Assert the bucket shape.
    assert_field_bucket(&quota.simulation);
    assert_field_bucket(&quota.action);
    assert_field_bucket(&quota.orchestrator);
}

fn assert_field_bucket(b: &BucketUsage) {
    let _ = b.used;
    assert!(b.cap > 0);
}

// ---------------------------------------------------------------------------
// sessions.topology
// ---------------------------------------------------------------------------

#[test]
fn contract_sessions_topology() {
    let service = SysmlService::empty();
    let key = start_orchestrator_session(&service);

    let result = execute_command(
        &service,
        "sysml.sessions.topology",
        json!({ "session_id": key }),
    )
    .unwrap();
    // SystemTopology only derives Serialize, not Deserialize.
    // Assert field presence on the Value directly.
    let obj = result.as_object().expect("topology returns object");
    assert!(obj.contains_key("root_label"), "missing root_label field");
    assert!(obj.contains_key("modules"), "missing modules field");
    assert!(
        obj.contains_key("domain_summaries"),
        "missing domain_summaries field"
    );
}
