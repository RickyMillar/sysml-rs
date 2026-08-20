//! End-to-end contract tests for the `sysml.sessions.archive.*` command
//! family + archive-backed `sysml.verify.timeline` (R4.1).
//!
//! These tests drive the service through the type-erased JSON dispatch
//! path (the same code path the MCP and REST transports use) and assert
//! on the externally-visible JSON shapes. Any change in field name, type,
//! or enum representation breaks the tests — that is the point.

use std::sync::Arc;

use serde_json::json;
use sysml_service::{execute_command, SysmlService};
use sysml_store::{
    ArchivedEvidence, ArchivedSession, ArchivedVerdict, InMemorySessionArchive, SessionArchive,
    SessionOrigin,
};

fn sample_session(
    id: &str,
    workspace: &str,
    origin: SessionOrigin,
    created_at: i64,
    verdicts: Vec<ArchivedVerdict>,
) -> ArchivedSession {
    ArchivedSession {
        id: id.to_owned(),
        label: None,
        origin,
        workspace_uri: workspace.to_owned(),
        created_at,
        ended_at: created_at + 1_000,
        ticks: 10,
        overrides: Vec::new(),
        verdicts,
        snapshots: Vec::new(),
        snapshot_value_units: None,
        golden: None,
        provenance: None,
    }
}

fn verdict(case: &str, v: &str, ts: i64, tick: Option<u64>) -> ArchivedVerdict {
    ArchivedVerdict::trajectory(
        case,
        v,
        ts,
        tick.map(|t| ArchivedEvidence {
            time_ms: None,
            session_id: "src-session".to_owned(),
            tick: t,
            element_id: Some("Req::X".to_owned()),
        }),
        None,
    )
}

fn service_with_three_sessions() -> (SysmlService, Arc<InMemorySessionArchive>) {
    let archive = Arc::new(InMemorySessionArchive::new());
    archive
        .record(sample_session(
            "s-run",
            "file:///w",
            SessionOrigin::Run,
            100,
            vec![],
        ))
        .unwrap();
    archive
        .record(sample_session(
            "s-verify-a",
            "file:///w",
            SessionOrigin::Verify,
            200,
            vec![
                verdict("CaseA", "pass", 210, Some(3)),
                verdict("CaseB", "fail", 220, Some(4)),
            ],
        ))
        .unwrap();
    archive
        .record(sample_session(
            "s-verify-b",
            "file:///w",
            SessionOrigin::Verify,
            300,
            vec![verdict("CaseA", "pass", 310, None)],
        ))
        .unwrap();
    let service = SysmlService::with_archive(archive.clone());
    (service, archive)
}

// -- archive.list -------------------------------------------------------

#[test]
fn archive_list_returns_all_newest_first() {
    let (service, _archive) = service_with_three_sessions();
    let result = execute_command(
        &service,
        "sysml.sessions.archive.list",
        json!({}),
    )
    .unwrap();
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3);
    // newest (created_at=300) first
    assert_eq!(entries[0]["id"], "s-verify-b");
    assert_eq!(entries[1]["id"], "s-verify-a");
    assert_eq!(entries[2]["id"], "s-run");
}

#[test]
fn archive_list_summary_shape_and_verdict_counts() {
    let (service, _archive) = service_with_three_sessions();
    let result = execute_command(
        &service,
        "sysml.sessions.archive.list",
        json!({}),
    )
    .unwrap();
    let entries = result["entries"].as_array().unwrap();
    let verify_a = entries.iter().find(|e| e["id"] == "s-verify-a").unwrap();
    // Required fields on summary.
    assert!(verify_a.get("origin").is_some());
    assert_eq!(verify_a["origin"], "verify");
    assert!(verify_a.get("workspace_uri").is_some());
    assert!(verify_a.get("created_at").is_some());
    assert!(verify_a.get("ended_at").is_some());
    assert!(verify_a.get("ticks").is_some());
    assert!(verify_a.get("snapshot_count").is_some());
    // Verdict counts breakdown.
    let counts = &verify_a["verdict_counts"];
    assert_eq!(counts["pass"], 1);
    assert_eq!(counts["fail"], 1);
    assert_eq!(counts["inconclusive"], 0);
    assert_eq!(counts["error"], 0);
    // Must NOT include the full verdicts/snapshots payloads.
    assert!(verify_a.get("verdicts").is_none());
    assert!(verify_a.get("snapshots").is_none());
    assert!(verify_a.get("overrides").is_none());
}

#[test]
fn archive_list_filters_by_origin() {
    let (service, _archive) = service_with_three_sessions();
    let result = execute_command(
        &service,
        "sysml.sessions.archive.list",
        json!({ "origin": "verify" }),
    )
    .unwrap();
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2);
    for entry in entries {
        assert_eq!(entry["origin"], "verify");
    }
}

#[test]
fn archive_list_filters_by_since() {
    let (service, _archive) = service_with_three_sessions();
    let result = execute_command(
        &service,
        "sysml.sessions.archive.list",
        json!({ "since": 250 }),
    )
    .unwrap();
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "s-verify-b");
}

#[test]
fn archive_list_filters_by_workspace_uri() {
    let archive = Arc::new(InMemorySessionArchive::new());
    archive
        .record(sample_session(
            "s-a",
            "file:///a",
            SessionOrigin::Run,
            100,
            vec![],
        ))
        .unwrap();
    archive
        .record(sample_session(
            "s-b",
            "file:///b",
            SessionOrigin::Run,
            100,
            vec![],
        ))
        .unwrap();
    let service = SysmlService::with_archive(archive);
    let result = execute_command(
        &service,
        "sysml.sessions.archive.list",
        json!({ "workspace_uri": "file:///a" }),
    )
    .unwrap();
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "s-a");
}

#[test]
fn archive_list_rejects_unknown_origin_with_invalid_input() {
    let (service, _archive) = service_with_three_sessions();
    let err = execute_command(
        &service,
        "sysml.sessions.archive.list",
        json!({ "origin": "bogus" }),
    )
    .unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("unknown session origin"), "{rendered}");
}

#[test]
fn archive_list_accepts_snake_case_multiword_origins() {
    let archive = Arc::new(InMemorySessionArchive::new());
    archive
        .record(sample_session(
            "s-mc",
            "file:///w",
            SessionOrigin::MonteCarlo,
            100,
            vec![],
        ))
        .unwrap();
    archive
        .record(sample_session(
            "s-ts",
            "file:///w",
            SessionOrigin::TradeStudy,
            200,
            vec![],
        ))
        .unwrap();
    let service = SysmlService::with_archive(archive);
    let mc = execute_command(
        &service,
        "sysml.sessions.archive.list",
        json!({ "origin": "monte_carlo" }),
    )
    .unwrap();
    assert_eq!(mc["entries"].as_array().unwrap().len(), 1);
    assert_eq!(mc["entries"][0]["origin"], "monte_carlo");

    let ts = execute_command(
        &service,
        "sysml.sessions.archive.list",
        json!({ "origin": "trade_study" }),
    )
    .unwrap();
    assert_eq!(ts["entries"].as_array().unwrap().len(), 1);
    assert_eq!(ts["entries"][0]["origin"], "trade_study");
}

// -- archive.get --------------------------------------------------------

#[test]
fn archive_get_returns_full_entry() {
    let (service, _archive) = service_with_three_sessions();
    let result = execute_command(
        &service,
        "sysml.sessions.archive.get",
        json!({ "id": "s-verify-a" }),
    )
    .unwrap();
    let entry = &result["entry"];
    assert!(!entry.is_null());
    assert_eq!(entry["id"], "s-verify-a");
    // Full entries DO include verdicts & snapshots arrays.
    assert!(entry["verdicts"].is_array());
    assert_eq!(entry["verdicts"].as_array().unwrap().len(), 2);
    assert_eq!(entry["verdicts"][0]["verdict"], "pass");
    assert_eq!(entry["verdicts"][0]["evidence"]["tick"], 3);
}

#[test]
fn archive_get_missing_returns_null_entry() {
    let (service, _archive) = service_with_three_sessions();
    let result = execute_command(
        &service,
        "sysml.sessions.archive.get",
        json!({ "id": "does-not-exist" }),
    )
    .unwrap();
    assert!(result["entry"].is_null());
}

// -- archive.mark_golden / unmark_golden -------------------------------

#[test]
fn archive_mark_and_unmark_golden_roundtrip() {
    let (service, _archive) = service_with_three_sessions();

    // Mark golden.
    let marked = execute_command(
        &service,
        "sysml.sessions.archive.mark_golden",
        json!({ "id": "s-verify-a", "label": "baseline" }),
    )
    .unwrap();
    assert_eq!(marked["ok"], true);

    // Verify it's now tagged via get.
    let got = execute_command(
        &service,
        "sysml.sessions.archive.get",
        json!({ "id": "s-verify-a" }),
    )
    .unwrap();
    assert_eq!(got["entry"]["golden"]["label"], "baseline");
    assert!(got["entry"]["golden"]["marked_at"].is_i64());

    // only_golden filter now shows just this one.
    let goldens = execute_command(
        &service,
        "sysml.sessions.archive.list",
        json!({ "only_golden": true }),
    )
    .unwrap();
    let entries = goldens["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["id"], "s-verify-a");

    // Unmark it.
    let unmarked = execute_command(
        &service,
        "sysml.sessions.archive.unmark_golden",
        json!({ "id": "s-verify-a" }),
    )
    .unwrap();
    assert_eq!(unmarked["ok"], true);

    let got2 = execute_command(
        &service,
        "sysml.sessions.archive.get",
        json!({ "id": "s-verify-a" }),
    )
    .unwrap();
    assert!(got2["entry"]["golden"].is_null());
}

#[test]
fn archive_mark_golden_unknown_id_is_element_not_found() {
    let (service, _archive) = service_with_three_sessions();
    let err = execute_command(
        &service,
        "sysml.sessions.archive.mark_golden",
        json!({ "id": "nope", "label": "x" }),
    )
    .unwrap_err();
    let rendered = format!("{err}");
    assert!(
        rendered.contains("no archived session"),
        "expected not-found error, got: {rendered}"
    );
}

// -- archive-backed verify_timeline ------------------------------------

#[test]
fn verify_timeline_returns_populated_entries_from_archive() {
    let (service, _archive) = service_with_three_sessions();
    // `workspace_uri` was removed from the command (workspace scoping is
    // keyed server-side on session provenance since 2026-07-17); a legacy
    // caller still sending it must be tolerated — the field is ignored,
    // not an error.
    let result = execute_command(
        &service,
        "sysml.verify.timeline",
        json!({ "workspace_uri": "file:///w" }),
    )
    .unwrap();
    let entries = result["entries"].as_array().unwrap();
    // 3 verdicts total across the two verify sessions.
    assert_eq!(entries.len(), 3);
    // Sorted ascending by timestamp.
    let ts: Vec<i64> = entries
        .iter()
        .map(|e| e["timestamp"].as_i64().unwrap())
        .collect();
    let mut sorted = ts.clone();
    sorted.sort();
    assert_eq!(ts, sorted, "entries must be ascending by timestamp");
    // Verdict field lowercase.
    for e in entries {
        let v = e["verdict"].as_str().unwrap();
        assert!(
            matches!(v, "pass" | "fail" | "inconclusive" | "error"),
            "unexpected verdict wire string: {v}"
        );
    }
}

/// Workspace scoping is keyed server-side on session provenance
/// (`SessionProvenance::workspace_root`, B6). A service with no
/// resolvable root — nothing loaded, as here — has no identity to key
/// on, so the timeline honestly spans the whole archive (scope-collapse
/// W7b re-key, 2026-07-17).
#[test]
fn verify_timeline_no_resolvable_root_spans_archive() {
    let (service, _archive) = service_with_three_sessions();
    let result = execute_command(&service, "sysml.verify.timeline", json!({})).unwrap();
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 3, "no root to key on must span every archived run");
}

#[test]
fn verify_timeline_filters_case_ids_end_to_end() {
    let (service, _archive) = service_with_three_sessions();
    let result = execute_command(
        &service,
        "sysml.verify.timeline",
        json!({
            "case_ids": ["CaseB"],
        }),
    )
    .unwrap();
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["case_id"], "CaseB");
    assert_eq!(entries[0]["verdict"], "fail");
}

#[test]
fn verify_timeline_filters_since_end_to_end() {
    let (service, _archive) = service_with_three_sessions();
    let result = execute_command(
        &service,
        "sysml.verify.timeline",
        json!({
            "since_timestamp": 300,
        }),
    )
    .unwrap();
    let entries = result["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["session_id"], "s-verify-b");
}
