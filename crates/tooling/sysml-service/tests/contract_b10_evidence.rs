//! B10 evidence-source taxonomy — contract gates
//! 2026-07-17).
//!
//! Purpose-built spec-faithful fixture (NOT a corpus no-regression check).
//! Pins, per the ruling's required list:
//! 1. `sysml.verify.record_external` happy path: synthetic archive entry
//!    (origin `external`, provenance captured at ingestion, verdicts carry
//!    `evaluation_mode: external` + the external evidence payload with the
//!    RESOLVED case ElementId — never name-only records).
//! 2. Every reject path is fail-hard and rejects the WHOLE batch: empty
//!    batch, unknown verdict string, blank tool/digest, unknown case name.
//! 3. Digest mismatch is recorded-not-rejected AND the staleness label
//!    (`matches_current_model`) renders on the `verify.timeline` read.
//! 4. `sysml.evaluate.verification_cases` rows carry
//!    `evaluation_mode: "static"` (§2.1a(d) — the case-view mode chip
//!    reads this wire).
//! 5. `sysml.workflow.attest_verification` appends an attestation and
//!    rejects a method outside the ONE canonical closed set
//!    (`sysml_core::metadata::VERIFICATION_METHOD_KINDS`) — manual
//!    verification is an attestation, never a verdict-store row.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};
use sysml_store::{InMemorySessionArchive, SessionArchive, SessionOrigin};

const FIXTURE: &str = r#"package B10EvidenceFixture {
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

fn fixture_workspace() -> PathBuf {
    // Every test in this binary shares one fixture directory and the harness
    // runs them in parallel, so rewriting the file per call truncates it under
    // a concurrent reader — which loads an empty model and fails looking for
    // elements the fixture declares. Write it exactly once per process.
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir =
            std::env::temp_dir().join(format!("sysml-b10-evidence-fixture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create fixture dir");
        std::fs::write(dir.join("B10EvidenceFixture.sysml"), FIXTURE).expect("write fixture");
        dir
    })
    .clone()
}

fn open_fixture_with_archive() -> (SysmlService, Arc<InMemorySessionArchive>) {
    let archive = Arc::new(InMemorySessionArchive::new());
    let service = SysmlService::with_archive(archive.clone());
    service
        .open_context(OpenTarget::Folder(fixture_workspace()))
        .expect("open B10 fixture workspace");
    (service, archive)
}

fn record_external(
    service: &SysmlService,
    body: serde_json::Value,
) -> Result<serde_json::Value, sysml_service::ServiceError> {
    execute_command(service, "sysml.verify.record_external", body)
}

#[test]
fn b10_record_external_happy_path_and_archive_shape() {
    let (service, archive) = open_fixture_with_archive();

    let result = record_external(
        &service,
        json!({
            "tool": "pytest-7.4",
            "declared_digest": "digest-from-ci",
            "run_ref": "ci://job/42",
            "artifacts": ["file:///run-report.xml"],
            "label": "nightly HIL run",
            "verdicts": [
                { "case_id": "CreepageInspection", "verdict": "pass",
                  "artifacts": ["file:///creepage-photos.zip"] }
            ]
        }),
    )
    .expect("valid external batch must record");

    assert_eq!(result["recorded"], 1);
    assert_eq!(result["declared_digest"], "digest-from-ci");
    // The fixture digest is real and differs from the fabricated claim —
    // recorded honestly, flagged immediately, never rejected.
    assert_eq!(result["matches_current_model"], false);
    let current_digest = result["current_digest"].as_str().expect("current digest present");
    assert!(!current_digest.is_empty());

    let session_id = result["session_id"].as_str().expect("session id");
    let entry = archive.get(session_id).expect("archive entry recorded");
    assert_eq!(entry.origin, SessionOrigin::External);
    assert_eq!(entry.label.as_deref(), Some("nightly HIL run"));
    assert_eq!(entry.ticks, 0, "external entries are degenerate sessions");
    assert!(entry.snapshots.is_empty());
    let prov = entry.provenance.as_ref().expect("provenance captured at ingestion");
    assert_eq!(prov.model_digest, current_digest, "B6 corroborating digest = ingestion state");

    assert_eq!(entry.verdicts.len(), 1);
    let v = &entry.verdicts[0];
    assert_eq!(v.case_id, "CreepageInspection");
    assert_eq!(v.evaluation_mode, sysml_store::EvaluationMode::External);
    assert!(v.evidence.is_none(), "external records never carry session evidence");
    let ext = v.external.as_ref().expect("external evidence payload");
    assert_eq!(ext.tool, "pytest-7.4");
    assert_eq!(ext.declared_digest, "digest-from-ci");
    assert_eq!(ext.run_ref.as_deref(), Some("ci://job/42"));
    // Run-level + per-verdict artifacts, in that order.
    assert_eq!(
        ext.artifacts,
        vec!["file:///run-report.xml".to_owned(), "file:///creepage-photos.zip".to_owned()]
    );
    // Steward amendment: the resolution already happened — the resolved
    // identity is captured, never thrown away into a name-only record.
    assert!(ext.element_id.is_some(), "resolved case ElementId captured on evidence");

    // Declaring the digest the server just reported → an exact match.
    let matched = record_external(
        &service,
        json!({
            "tool": "pytest-7.4",
            "declared_digest": current_digest,
            "verdicts": [ { "case_id": "CreepageInspection", "verdict": "fail" } ]
        }),
    )
    .expect("matching-digest batch records");
    assert_eq!(matched["matches_current_model"], true);
}

#[test]
fn b10_record_external_reject_paths_are_fail_hard() {
    let (service, archive) = open_fixture_with_archive();

    // Empty batch.
    assert!(record_external(
        &service,
        json!({ "tool": "t", "declared_digest": "d", "verdicts": [] })
    )
    .is_err());

    // Unknown verdict string.
    assert!(record_external(
        &service,
        json!({ "tool": "t", "declared_digest": "d",
                "verdicts": [ { "case_id": "CreepageInspection", "verdict": "passed" } ] })
    )
    .is_err());

    // Blank tool / blank digest.
    assert!(record_external(
        &service,
        json!({ "tool": "  ", "declared_digest": "d",
                "verdicts": [ { "case_id": "CreepageInspection", "verdict": "pass" } ] })
    )
    .is_err());
    assert!(record_external(
        &service,
        json!({ "tool": "t", "declared_digest": "",
                "verdicts": [ { "case_id": "CreepageInspection", "verdict": "pass" } ] })
    )
    .is_err());

    // Unknown case name rejects the WHOLE batch — including the valid row.
    assert!(record_external(
        &service,
        json!({ "tool": "t", "declared_digest": "d",
                "verdicts": [
                    { "case_id": "CreepageInspection", "verdict": "pass" },
                    { "case_id": "NoSuchCase", "verdict": "pass" }
                ] })
    )
    .is_err());

    // Nothing leaked into the archive from any rejected batch.
    assert!(
        archive.list(sysml_store::ArchiveFilter::default()).is_empty(),
        "rejected batches must not partially record"
    );
}

#[test]
fn b10_timeline_renders_external_lane_with_staleness_label() {
    let (service, _archive) = open_fixture_with_archive();

    let first = record_external(
        &service,
        json!({
            "tool": "hil-bench-2",
            "declared_digest": "stale-digest",
            "verdicts": [ { "case_id": "CreepageInspection", "verdict": "fail" } ]
        }),
    )
    .expect("record stale batch");
    let current_digest = first["current_digest"].as_str().expect("digest").to_owned();
    record_external(
        &service,
        json!({
            "tool": "hil-bench-2",
            "declared_digest": current_digest,
            "verdicts": [ { "case_id": "CreepageInspection", "verdict": "pass" } ]
        }),
    )
    .expect("record fresh batch");

    let timeline = execute_command(&service, "sysml.verify.timeline", json!({}))
        .expect("timeline read succeeds");
    let entries = timeline["entries"].as_array().expect("entries array");
    assert_eq!(entries.len(), 2, "both external runs appear as timeline entries");
    for entry in entries {
        assert_eq!(entry["evaluation_mode"], "external");
        assert_eq!(entry["case_id"], "CreepageInspection");
        assert!(entry.get("evidence").is_none(), "no session deep-link on external entries");
    }
    // The staleness label is server-computed on the read: the stale run
    // says false, the fresh run says true (steward-required pin — the
    // mismatch is recorded and LABELED, never rejected or hidden).
    let stale = entries.iter().find(|e| e["verdict"] == "fail").expect("stale entry");
    assert_eq!(stale["external"]["matches_current_model"], false);
    assert_eq!(stale["external"]["declared_digest"], "stale-digest");
    assert_eq!(stale["external"]["tool"], "hil-bench-2");
    let fresh = entries.iter().find(|e| e["verdict"] == "pass").expect("fresh entry");
    assert_eq!(fresh["external"]["matches_current_model"], true);
}

#[test]
fn b10_evaluate_verification_cases_rows_carry_static_mode() {
    let (service, _archive) = open_fixture_with_archive();
    let rows = execute_command(&service, "sysml.evaluate.verification_cases", json!({}))
        .expect("evaluate verification cases");
    let rows = rows.as_array().expect("rows array");
    assert!(!rows.is_empty(), "fixture declares a verification case");
    for row in rows {
        // §2.1a(d): the label renders ALWAYS. This read recomputes against
        // the current graph — static, never archived.
        assert_eq!(row["evaluation_mode"], "static");
    }
    let case = rows
        .iter()
        .find(|r| r["case_name"] == "CreepageInspection")
        .expect("fixture case present");
    assert_eq!(case["methods"], json!(["inspect"]), "B4 layer-1 declared method intact");
}

#[test]
fn b10_attest_verification_appends_and_rejects_unknown_method() {
    let store: Arc<std::sync::RwLock<dyn sysml_store::Store + Send + Sync>> =
        Arc::new(std::sync::RwLock::new(sysml_store::InMemoryStore::new()));
    let service = SysmlService::with_store(store);
    service
        .open_context(OpenTarget::Folder(fixture_workspace()))
        .expect("open B10 fixture workspace");

    // Find the verification case element id via the rows read.
    let rows = execute_command(&service, "sysml.evaluate.verification_cases", json!({}))
        .expect("evaluate verification cases");
    let case_id = rows.as_array().expect("rows")[0]["element_id"]
        .as_str()
        .expect("element id")
        .to_owned();

    // Method outside the spec's closed set dies at the write boundary —
    // one canonical home (sysml_core::metadata::VERIFICATION_METHOD_KINDS).
    let err = execute_command(
        &service,
        "sysml.workflow.attest_verification",
        json!({
            "project": "b10", "element_id": case_id, "method": "inspekt",
            "statement": "checked it", "actor": "analyst"
        }),
    )
    .expect_err("unknown method must be rejected");
    assert!(err.to_string().contains("inspekt"), "error names the bad method: {err}");

    // Blank statement / actor are caller errors.
    assert!(execute_command(
        &service,
        "sysml.workflow.attest_verification",
        json!({ "project": "b10", "element_id": case_id, "method": "inspect",
                "statement": "  ", "actor": "analyst" }),
    )
    .is_err());
    assert!(execute_command(
        &service,
        "sysml.workflow.attest_verification",
        json!({ "project": "b10", "element_id": case_id, "method": "inspect",
                "statement": "visually inspected creepage distance", "actor": "" }),
    )
    .is_err());

    // Happy path: the event lands in the sidecar with the digest pin.
    let event = execute_command(
        &service,
        "sysml.workflow.attest_verification",
        json!({
            "project": "b10", "element_id": case_id, "method": "inspect",
            "statement": "visually inspected creepage distance", "actor": "analyst"
        }),
    )
    .expect("valid attestation appends");
    assert_eq!(event["kind"], "verification_attestation");
    assert_eq!(event["method"], "inspect");
    assert_eq!(event["actor"], "analyst");
    assert!(
        event["attested_commit"].as_str().is_some_and(|c| !c.is_empty()),
        "content digest pinned at attest time"
    );

    // The folded state surfaces it as an ATTESTATION record (workflow
    // sidecar) — verification_attestations, never a verdict store.
    let state = execute_command(
        &service,
        "sysml.workflow.state",
        json!({ "project": "b10", "element_id": case_id }),
    )
    .expect("workflow state read");
    // `ElementWorkflowState` flattens the folded store state to the top
    // level of the wire object.
    let attestations = state["verification_attestations"]
        .as_array()
        .expect("verification_attestations in folded state");
    assert_eq!(attestations.len(), 1);
    assert_eq!(attestations[0]["method"], "inspect");
    assert_eq!(
        attestations[0]["superseded"], false,
        "content unchanged since attest → attestation stands"
    );
}
