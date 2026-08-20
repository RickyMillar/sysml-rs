//! espresso-production-cell — service/session gates (Stage D).
//!
//! SVC-SESSION: a multi-file workspace creates an orchestrator session with
//!   captured provenance, and bulk-step advances exactly N ticks (fail-hard cap).
//! SES-TS: the session's time-series buffer captures observables that evolve.
//! VER-SIM: declared verification cases route through the one VerificationRunner
//!   and return verdicts.

use std::path::PathBuf;

use serde_json::{json, Value};
use sysml_project::discovery::OpenTarget;
use sysml_service::execution::{SessionSummary, MAX_BULK_STEP_TICKS};
use sysml_service::{execute_command, ServiceError, SysmlService};

fn cell_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent().unwrap().parent().unwrap().parent().unwrap()
        .join("examples/espresso-production-cell")
}

fn open(service: &SysmlService) {
    service
        .open_context(OpenTarget::Folder(cell_root()))
        .expect("open espresso-production-cell workspace");
}

fn create_ws(service: &SysmlService) -> SessionSummary {
    let v = execute_command(service, "sysml.sessions.create", json!({ "uri": "__workspace__" }))
        .expect("sessions.create");
    serde_json::from_value(v).expect("SessionSummary")
}

/// SVC-SESSION — workspace session infers orchestrator kind, captures a model
/// digest matching store identity, and bulk-steps exactly N ticks / rejects
/// over-cap requests.
#[test]
fn cell_session_is_orchestrator_with_provenance_and_exact_bulk_step() {
    use sysml_service::execution::SessionKind;
    let service = SysmlService::empty();
    open(&service);

    let created = create_ws(&service);
    assert_eq!(created.kind, SessionKind::Orchestrator, "multi-file workspace => orchestrator");

    // Provenance identity matches the store's content digest.
    let expected = service.workspace_aware_graph().expect("graph").content_digest();
    assert_eq!(
        created.provenance.as_ref().expect("provenance").model_digest,
        expected,
        "session model digest must equal store identity"
    );

    // Bulk step advances exactly N ticks.
    let start = created.tick;
    let stepped: SessionSummary = serde_json::from_value(
        execute_command(&service, "sysml.sessions.step",
            json!({ "session_id": created.id, "ticks": 150 })).expect("step"),
    ).unwrap();
    assert_eq!(stepped.tick - start, 150, "bulk step must advance exactly 150 ticks");

    // Over the cap is a hard error, never clamped.
    let err = execute_command(&service, "sysml.sessions.step",
        json!({ "session_id": created.id, "ticks": MAX_BULK_STEP_TICKS + 1 }))
        .expect_err("over-cap must be rejected");
    assert!(matches!(err, ServiceError::InvalidInput(_)));
}

/// SES-TS — the session captures time-series observables that evolve as the
/// simulation advances.
#[test]
fn cell_timeseries_capture_evolving_observable() {
    let service = SysmlService::empty();
    open(&service);
    let created = create_ws(&service);

    execute_command(&service, "sysml.sessions.step",
        json!({ "session_id": created.id, "ticks": 200 })).expect("step");

    let names = execute_command(&service, "sysml.sessions.timeseries_names",
        json!({ "session_id": created.id })).expect("timeseries_names");
    let list: Vec<String> = names.get("names").and_then(|n| n.as_array()).unwrap()
        .iter().map(|v| v.as_str().unwrap().to_owned()).collect();
    assert!(!list.is_empty(), "some observables must be captured");

    // A station temperature observable evolves (rises from ambient toward the
    // brew band) over the run.
    let temp_var = list.iter().find(|n| n.contains("temp"))
        .unwrap_or_else(|| panic!("expected a temperature observable in {list:?}"));
    let series = execute_command(&service, "sysml.sessions.timeseries",
        json!({ "session_id": created.id, "var": temp_var })).expect("timeseries");
    let pts = series.get("points").and_then(|p| p.as_array()).expect("points");
    assert!(pts.len() >= 2, "an evolving series needs multiple points");
    let first = pts.first().unwrap().get("value").unwrap().as_f64().unwrap();
    let last = pts.last().unwrap().get("value").unwrap().as_f64().unwrap();
    assert!(last > first, "station temperature must rise over the run: {first} -> {last}");
}

/// VER-SIM — declared verification cases run through the single VerificationRunner
/// and produce verdicts (the requirements-to-scenario binding is exercised).
#[test]
fn cell_verification_cases_produce_verdicts() {
    let service = SysmlService::empty();
    open(&service);
    let created = create_ws(&service);
    execute_command(&service, "sysml.sessions.step",
        json!({ "session_id": created.id, "ticks": 200 })).expect("step");

    let raw = execute_command(&service, "sysml.sessions.verify",
        json!({ "session_id": created.id })).expect("sessions.verify");
    let cases = raw.as_array().expect("verify returns an array of cases");
    assert!(!cases.is_empty(), "declared verification cases must be evaluated");
    // Every case carries a verdict-kind string (routing is non-vacuous — binding
    // may resolve to any verdict kind including Inconclusive).
    for c in cases {
        let verdict = c.get("verdict").and_then(Value::as_str)
            .unwrap_or_else(|| panic!("case has no verdict: {c}"));
        assert!(
            ["Pass", "Fail", "Inconclusive", "Error", "pass", "fail", "inconclusive", "error"]
                .contains(&verdict),
            "unexpected verdict {verdict:?}"
        );
    }
}
