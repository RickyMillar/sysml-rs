//! Contract gates for the verification-EXECUTIONS lane
//! 2026-07-19).
//!
//! Purpose-built spec-faithful fixture (NOT a corpus no-regression check).
//! Pins, per the §6 backend-lane brief:
//!
//! 1. `sysml.verify.executions` is a PROJECTION over the session archive:
//!    only archived sessions carrying >=1 verdict are executions; a
//!    verdict-less run is excluded. Rows are newest-first, origin drives the
//!    execution-level `evaluation_mode`, and each per-case result carries the
//!    P6 `case_changed_since` stale flag.
//! 2. P6 wiring: `record_external` pins the case's own subtree digest at
//!    ingestion, so after a content edit inside the case the SAME archived
//!    record's `case_changed_since` flips false → true (an edit OUTSIDE the
//!    case never flips it).
//! 3. `sysml.verify.latest_status` is context-qualified per evaluation_mode
//!    (never one flat field): a trajectory execution and an external
//!    execution of the same case surface as distinct `latest.trajectory` /
//!    `latest.external` entries.
//! 4. P4 wire check: the approval sidecar (`sysml.workflow.set_approval` /
//!    `.state`) accepts a verification-case element id end-to-end — the
//!    sidecar keys on ElementId generically, no element-kind gate.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]

use std::fs;
use std::sync::Arc;

use serde_json::json;
use sysml_service::{execute_command, SysmlService};
use sysml_store::{
    ArchivedSession, ArchivedVerdict, InMemorySessionArchive, SessionArchive, SessionOrigin,
    SessionProvenance,
};
use tempfile::TempDir;

/// Case-with-verification fixture. V2 adds a `doc` INSIDE the verification
/// def — content within the case's own ownership subtree, so its subtree
/// digest changes while the constraint/requirement outside it do not.
const FIXTURE_V1: &str = r#"package TestMgmtFixture {
	private import VerificationCases::*;

	constraint def AlwaysHolds { 1 <= 2 }

	requirement def <'REQ-001'> SystemReq {
		require constraint : AlwaysHolds;
	}

	requirement systemReq : SystemReq;

	verification def CreepageInspection {
		@VerificationMethod { kind = VerificationMethodKind::inspect; }
		objective {
			verify SystemReq;
		}
	}
}
"#;

const FIXTURE_V2: &str = r#"package TestMgmtFixture {
	private import VerificationCases::*;

	constraint def AlwaysHolds { 1 <= 2 }

	requirement def <'REQ-001'> SystemReq {
		require constraint : AlwaysHolds;
	}

	requirement systemReq : SystemReq;

	verification def CreepageInspection {
		doc /* creepage inspection procedure, revised */
		@VerificationMethod { kind = VerificationMethodKind::inspect; }
		objective {
			verify SystemReq;
		}
	}
}
"#;

fn write_workspace(content: &str) -> (TempDir, std::path::PathBuf) {
    let dir = TempDir::new().expect("tempdir");
    let file = dir.path().join("TestMgmtFixture.sysml");
    fs::write(&file, content).expect("write fixture");
    (dir, file)
}

fn open_workspace(content: &str) -> (SysmlService, Arc<InMemorySessionArchive>, TempDir, std::path::PathBuf) {
    let archive = Arc::new(InMemorySessionArchive::new());
    let service = SysmlService::with_archive(archive.clone());
    let (dir, file) = write_workspace(content);
    service.load_workspace(dir.path()).expect("load fixture workspace");
    (service, archive, dir, file)
}

fn record_external(service: &SysmlService, verdict: &str) -> serde_json::Value {
    execute_command(
        service,
        "sysml.verify.record_external",
        json!({
            "tool": "pytest-7.4",
            "declared_digest": "declared-from-ci",
            "run_ref": "ci://job/7",
            "verdicts": [ { "case_id": "CreepageInspection", "verdict": verdict } ]
        }),
    )
    .expect("record_external must succeed")
}

fn latest_status(service: &SysmlService) -> serde_json::Value {
    execute_command(service, "sysml.verify.latest_status", json!({}))
        .expect("verify.latest_status")
}

fn executions(service: &SysmlService, case_name: Option<&str>) -> serde_json::Value {
    let body = match case_name {
        Some(name) => json!({ "case_name": name }),
        None => json!({}),
    };
    execute_command(service, "sysml.verify.executions", body).expect("verify.executions")
}

/// P1: only verdict-carrying sessions are executions; a pure run is not.
#[test]
fn executions_projection_excludes_verdict_less_sessions() {
    let (service, archive, _dir, _file) = open_workspace(FIXTURE_V1);

    // A real external ingest → one execution.
    record_external(&service, "pass");

    // A verdict-less simulation run recorded directly is NOT a verification
    // execution. Give it the same provenance root so scoping would include
    // it if verdicts were the wrong filter.
    let root = archive
        .list(sysml_store::ArchiveFilter::default())
        .first()
        .and_then(|s| archive.get(&s.id))
        .and_then(|e| e.provenance.and_then(|p| p.workspace_root))
        .expect("external record carries a provenance root");
    archive
        .record(ArchivedSession {
            id: "sim-only".to_owned(),
            label: None,
            origin: SessionOrigin::Run,
            workspace_uri: "__workspace__".to_owned(),
            created_at: 10,
            ended_at: 20,
            ticks: 100,
            overrides: Vec::new(),
            verdicts: Vec::new(),
            snapshots: Vec::new(),
            snapshot_value_units: None,
            golden: None,
            provenance: Some(SessionProvenance {
                model_digest: "x".to_owned(),
                git: None,
                workspace_root: Some(root),
                file_manifest: Vec::new(),
            }),
        })
        .unwrap();

    let result = executions(&service, None);
    let rows = result["executions"].as_array().expect("executions array");
    assert_eq!(rows.len(), 1, "verdict-less run is not an execution");
    assert_eq!(rows[0]["origin"], "external");
    assert_eq!(rows[0]["evaluation_mode"], "external");
    assert_eq!(rows[0]["counts"]["pass"], 1);
    let ext = &rows[0]["external"];
    assert_eq!(ext["tool"], "pytest-7.4");
    // declared digest is fabricated → stale vs the real current model.
    assert_eq!(ext["matches_current_model"], false);
    // Per-case result carries the P6 fields.
    let res = &rows[0]["results"][0];
    assert_eq!(res["case_id"], "CreepageInspection");
    assert_eq!(res["evaluation_mode"], "external");
    assert!(res["case_digest"].as_str().is_some(), "case subtree digest pinned at ingestion");
}

/// P6: the pinned case digest lets `case_changed_since` flip after an edit
/// INSIDE the case, and stay false for an edit outside it.
#[test]
fn case_changed_since_flips_on_in_case_edit_only() {
    let (service, _archive, dir, _file) = open_workspace(FIXTURE_V1);
    record_external(&service, "pass");

    // Freshly ingested against the current model → not changed.
    let before = executions(&service, None);
    let res_before = &before["executions"][0]["results"][0];
    assert_eq!(
        res_before["case_changed_since"], false,
        "pinned digest equals current subtree digest at ingestion"
    );
    let pinned = res_before["case_digest"].as_str().expect("pinned digest").to_owned();

    // Edit CONTENT inside the verification case (add a doc), reload.
    fs::write(dir.path().join("TestMgmtFixture.sysml"), FIXTURE_V2).unwrap();
    service.load_workspace(dir.path()).unwrap();

    let after = executions(&service, None);
    let res_after = &after["executions"][0]["results"][0];
    assert_eq!(
        res_after["case_changed_since"], true,
        "the case's subtree content changed since this execution"
    );
    // The STORED digest is unchanged (it is a record of the past); only the
    // server-computed comparison flipped.
    assert_eq!(res_after["case_digest"].as_str(), Some(pinned.as_str()));
}

/// P2: latest_status is context-qualified per evaluation_mode — a trajectory
/// execution and an external execution of one case are distinct latest
/// entries, never merged into one flat verdict.
#[test]
fn latest_status_is_per_mode() {
    let (service, archive, _dir, _file) = open_workspace(FIXTURE_V1);

    // External execution (real mint).
    record_external(&service, "fail");

    // Read the pinned digest + provenance root off the external record so
    // the seeded trajectory execution shares scope and current-digest.
    let ext_entry = {
        let id = archive.list(sysml_store::ArchiveFilter::default())[0].id.clone();
        archive.get(&id).unwrap()
    };
    let root = ext_entry.provenance.as_ref().unwrap().workspace_root.clone();
    let pinned = ext_entry.verdicts[0].case_digest.clone();

    // Seed a trajectory-origin execution of the SAME case (the projection's
    // trajectory branch; the trajectory MINT path is exercised by
    // contract_sessions_verify). Same root + current digest → not changed.
    archive
        .record(ArchivedSession {
            id: "traj-exec".to_owned(),
            label: Some("nightly sim".to_owned()),
            origin: SessionOrigin::Verify,
            workspace_uri: "__workspace__".to_owned(),
            created_at: 99,
            ended_at: 100,
            ticks: 42,
            overrides: Vec::new(),
            verdicts: vec![ArchivedVerdict::trajectory(
                "CreepageInspection",
                "pass",
                99,
                None,
                pinned.clone(),
            )],
            snapshots: Vec::new(),
            snapshot_value_units: None,
            golden: None,
            provenance: Some(SessionProvenance {
                model_digest: "current-model".to_owned(),
                git: None,
                workspace_root: root,
                file_manifest: Vec::new(),
            }),
        })
        .unwrap();

    let status = execute_command(&service, "sysml.verify.latest_status", json!({}))
        .expect("verify.latest_status");
    let cases = status["cases"].as_array().expect("cases array");
    let case = cases
        .iter()
        .find(|c| c["case_id"] == "CreepageInspection")
        .expect("case present");
    assert!(
        case["case_element_id"].as_str().is_some(),
        "case still resolves → element id surfaced"
    );

    let traj = &case["latest"]["trajectory"];
    assert_eq!(traj["verdict"], "pass");
    assert_eq!(traj["execution_id"], "traj-exec");
    assert_eq!(traj["model_digest"], "current-model");

    let external = &case["latest"]["external"];
    assert_eq!(external["verdict"], "fail");
    assert_eq!(external["tool"], "pytest-7.4");
    // Distinct per-mode entries — not one consolidated verdict.
    assert_ne!(traj["verdict"], external["verdict"]);
}

/// The `case_name` filter keeps only executions touching that case.
#[test]
fn executions_case_name_filter() {
    let (service, _archive, _dir, _file) = open_workspace(FIXTURE_V1);
    record_external(&service, "pass");

    let hit = executions(&service, Some("CreepageInspection"));
    assert_eq!(hit["executions"].as_array().unwrap().len(), 1);

    let miss = executions(&service, Some("NoSuchCase"));
    assert!(miss["executions"].as_array().unwrap().is_empty());
}

/// P4 wire check: the approval sidecar accepts a verification-case element
/// id end-to-end. The sidecar keys on ElementId generically (no element-kind
/// gate), so a verification case is an approvable element like any other.
#[test]
fn approval_sidecar_accepts_verification_case_element() {
    // `workflow.state` folds against the snapshot store, so this wire check
    // uses a store-backed service (the archive is irrelevant to approval).
    let store: Arc<std::sync::RwLock<dyn sysml_store::Store + Send + Sync>> =
        Arc::new(std::sync::RwLock::new(sysml_store::InMemoryStore::new()));
    let service = SysmlService::with_store(store);
    let (dir, _file) = write_workspace(FIXTURE_V1);
    service.load_workspace(dir.path()).expect("load fixture workspace");

    // Resolve the verification case element id.
    let rows = execute_command(&service, "sysml.evaluate.verification_cases", json!({}))
        .expect("evaluate verification cases");
    let case_element_id = rows.as_array().expect("rows")[0]["element_id"]
        .as_str()
        .expect("case element id")
        .to_owned();

    // Transition draft → in_review → approved on the CASE element.
    let ev = execute_command(
        &service,
        "sysml.workflow.set_approval",
        json!({ "project": "testmgmt", "element_id": case_element_id, "to": "in_review", "actor": "ricky" }),
    )
    .expect("set_approval on a verification case must succeed");
    assert_eq!(ev["kind"], "approval_state_changed");
    assert_eq!(ev["to"], "in_review");

    execute_command(
        &service,
        "sysml.workflow.set_approval",
        json!({ "project": "testmgmt", "element_id": case_element_id, "to": "approved", "actor": "ricky" }),
    )
    .expect("second transition succeeds");

    // The folded state reflects the case's current approval.
    let state = execute_command(
        &service,
        "sysml.workflow.state",
        json!({ "project": "testmgmt", "element_id": case_element_id }),
    )
    .expect("workflow.state read");
    assert_eq!(
        state["approval"][0], "approved",
        "verification case approval folds end-to-end"
    );
    assert_eq!(state["orphaned"], false);
}

/// J5: the run behind a trajectory verdict must survive the projection.
///
/// `ArchivedVerdict::evidence` (session id + tick) has been stored since B10,
/// but neither `ExecutionResult` nor `LatestTrajectory` carried it — so a UI
/// drilling into a trajectory-backed verdict could see THAT a run existed and
/// not WHICH run, at what tick. "Latest run: PASS" with no locator is a claim
/// a reader cannot check.
#[test]
fn trajectory_evidence_survives_both_projections() {
    let (service, archive, _dir, _file) = open_workspace(FIXTURE_V1);
    record_external(&service, "fail");
    let ext_entry = {
        let id = archive.list(sysml_store::ArchiveFilter::default())[0].id.clone();
        archive.get(&id).unwrap()
    };
    let root = ext_entry.provenance.as_ref().unwrap().workspace_root.clone();
    let pinned = ext_entry.verdicts[0].case_digest.clone();

    archive
        .record(ArchivedSession {
            id: "traj-evidence".to_owned(),
            label: None,
            origin: SessionOrigin::Verify,
            workspace_uri: "__workspace__".to_owned(),
            created_at: 500,
            ended_at: 501,
            ticks: 3819,
            overrides: Vec::new(),
            verdicts: vec![ArchivedVerdict::trajectory(
                "CreepageInspection",
                "pass",
                500,
                Some(sysml_store::ArchivedEvidence {
                    time_ms: None,
                    session_id: "sess-fa911215".to_owned(),
                    tick: 3819,
                    element_id: Some("el-creepage".to_owned()),
                }),
                pinned.clone(),
            )],
            snapshots: Vec::new(),
            snapshot_value_units: None,
            golden: None,
            provenance: Some(SessionProvenance {
                model_digest: "current-model".to_owned(),
                git: None,
                workspace_root: root,
                file_manifest: Vec::new(),
            }),
        })
        .unwrap();

    // (a) the per-execution results projection
    let execs = executions(&service, Some("CreepageInspection"));
    let row = execs["executions"]
        .as_array()
        .expect("executions array")
        .iter()
        .find(|r| r["execution_id"] == "traj-evidence")
        .expect("the seeded trajectory execution is projected");
    let result = &row["results"][0];
    assert_eq!(result["evidence"]["session_id"], "sess-fa911215");
    assert_eq!(result["evidence"]["tick"], 3819);
    assert_eq!(result["evidence"]["element_id"], "el-creepage");

    // (b) the latest-status projection the case view actually reads
    let latest = latest_status(&service);
    let entry = latest["cases"]
        .as_array()
        .expect("cases array")
        .iter()
        .find(|c| c["case_id"] == "CreepageInspection")
        .expect("case present in latest status");
    let traj = &entry["latest"]["trajectory"];
    assert_eq!(traj["evidence"]["session_id"], "sess-fa911215");
    assert_eq!(traj["evidence"]["tick"], 3819);
}

/// A verdict with no stored evidence must project an explicit `null`, never a
/// MISSING key and never a zeroed session id.
///
/// The missing-key form was the original shape, and it made "this record has
/// no evidence" indistinguishable from "this server is too old to report
/// evidence". A brand-new run at tick 5001 was reported to a user as
/// "predates evidence capture" on exactly that ambiguity — the server
/// answering had been built before the field existed. An explicit null makes
/// absence something the server SAID, not something the client inferred from
/// silence.
#[test]
fn absent_evidence_is_explicit_null_never_a_missing_key() {
    let (service, archive, _dir, _file) = open_workspace(FIXTURE_V1);
    record_external(&service, "fail");
    let ext_entry = {
        let id = archive.list(sysml_store::ArchiveFilter::default())[0].id.clone();
        archive.get(&id).unwrap()
    };
    let root = ext_entry.provenance.as_ref().unwrap().workspace_root.clone();
    let pinned = ext_entry.verdicts[0].case_digest.clone();

    archive
        .record(ArchivedSession {
            id: "traj-pre-b10".to_owned(),
            label: None,
            origin: SessionOrigin::Verify,
            workspace_uri: "__workspace__".to_owned(),
            created_at: 600,
            ended_at: 601,
            ticks: 7,
            overrides: Vec::new(),
            verdicts: vec![ArchivedVerdict::trajectory(
                "CreepageInspection",
                "pass",
                600,
                None,
                pinned,
            )],
            snapshots: Vec::new(),
            snapshot_value_units: None,
            golden: None,
            provenance: Some(SessionProvenance {
                model_digest: "current-model".to_owned(),
                git: None,
                workspace_root: root,
                file_manifest: Vec::new(),
            }),
        })
        .unwrap();

    let execs = executions(&service, Some("CreepageInspection"));
    let row = execs["executions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["execution_id"] == "traj-pre-b10")
        .expect("projected");
    let evidence = row["results"][0]
        .get("evidence")
        .expect("the evidence key is always serialized, so absence is explicit");
    assert!(
        evidence.is_null(),
        "an absent run record is null, not an empty object or a zeroed id: {evidence}"
    );
}

