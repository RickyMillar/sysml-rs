//! B2.1 gate — the per-requirement "evaluated contract" read (R18).
//!
//! Purpose-built spec-faithful fixture (NOT a corpus no-regression check):
//! one requirement carrying the full contract — subject parameter, an
//! inline assume constraint, an inline require constraint, a reference-form
//! require constraint, an owned attribute with a static value, an actor,
//! and a Rationale metadata annotation. Asserts through
//! `sysml.workspace.requirement_detail`:
//!
//! 1. the id accepted is a ROW id from `requirement_rows` (id parity —
//!    the workbench selects a row, then fetches its detail),
//! 2. subject resolves to an ElementId link ref (W1's `Value::Ref` shape,
//!    never a name string),
//! 3. assume/require constraints split by role, inline bodies as verbatim
//!    text, reference form linking its definition,
//! 4. actors surface as link refs (W2's elaborated list), separate from
//!    the verdict inputs,
//! 5. owned attribute values render as display text,
//! 6. rationale text surfaces, and
//! 7. non-requirement ids fail hard (no degraded detail view).

use std::path::PathBuf;

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

const FIXTURE: &str = r#"package B21DetailFixture {
	private import ModelingMetadata::*;

	part def Breaker;
	part def Driver;
	constraint def MassLimit { 1 <= 2 }

	requirement def <'REQ-010'> TripReq {
		doc /* The breaker shall trip within 40 ms of a fault. */
		@Rationale { text = "Threshold from the 2025 trade study."; }
		subject breaker : Breaker;
		attribute maxTripTime = 40;
		assume constraint { maxTripTime > 0 }
		require constraint { maxTripTime <= 40 }
		require constraint : MassLimit;
		actor driver : Driver;
	}

	requirement tripReq : TripReq;
}
"#;

fn fixture_workspace() -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("sysml-b21-detail-fixture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("B21DetailFixture.sysml"), FIXTURE).expect("write fixture");
    dir
}

fn open_fixture() -> SysmlService {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(fixture_workspace()))
        .expect("open B2.1 fixture workspace");
    service
}

fn row_id(service: &SysmlService, name: &str) -> String {
    let rows = execute_command(
        service,
        "sysml.workspace.requirement_rows",
        json!({ "spec": {} }),
    )
    .expect("requirement_rows must succeed");
    rows["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("row {name} present"))["id"]
        .as_str()
        .expect("row id is a string")
        .to_owned()
}

fn names(refs: &serde_json::Value) -> Vec<String> {
    refs.as_array()
        .expect("link ref array")
        .iter()
        .map(|r| r["name"].as_str().unwrap_or("<unnamed>").to_owned())
        .collect()
}

#[test]
fn b21_requirement_detail_end_to_end() {
    let service = open_fixture();
    let id = row_id(&service, "TripReq");

    let detail = execute_command(
        &service,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": id }),
    )
    .expect("requirement_detail must succeed");

    // 1+2. Row-id parity, and the subject is an ElementId link ref.
    assert_eq!(detail["id"].as_str(), Some(id.as_str()));
    let subject = &detail["subject"];
    assert_eq!(subject["name"].as_str(), Some("breaker"));
    assert!(
        subject["id"].as_str().is_some_and(|s| !s.is_empty()),
        "subject carries ElementId identity, not a bare name: {subject}"
    );

    // 3. Constraint buckets: one assume (inline), two require (one inline,
    //    one reference-form linking MassLimit).
    let assumed = detail["assumed_constraints"]
        .as_array()
        .expect("assumed_constraints array");
    assert_eq!(assumed.len(), 1, "one assume constraint: {assumed:?}");
    assert_eq!(assumed[0]["text"].as_str(), Some("maxTripTime > 0"));

    let required = detail["required_constraints"]
        .as_array()
        .expect("required_constraints array");
    assert_eq!(required.len(), 2, "two require constraints: {required:?}");
    let inline = required
        .iter()
        .find(|c| c["text"].is_string())
        .expect("inline require constraint");
    assert_eq!(inline["text"].as_str(), Some("maxTripTime <= 40"));
    let reference = required
        .iter()
        .find(|c| c["text"].is_null())
        .expect("reference-form require constraint");
    assert_eq!(
        reference["referenced_definition"]["name"].as_str(),
        Some("MassLimit"),
        "reference form links its definition: {reference}"
    );

    // 4. Narrative bucket: the actor, separate from the verdict inputs.
    assert_eq!(names(&detail["actors"]), vec!["driver".to_owned()]);
    assert_eq!(names(&detail["stakeholders"]), Vec::<String>::new());

    // 5. Owned attribute with its static value as display text.
    let attrs = detail["referenced_attributes"]
        .as_array()
        .expect("referenced_attributes array");
    let trip_time = attrs
        .iter()
        .find(|a| a["name"] == "maxTripTime")
        .unwrap_or_else(|| panic!("maxTripTime attribute present: {attrs:?}"));
    assert_eq!(trip_time["value"].as_str(), Some("40"));

    // 6. Rationale text.
    assert_eq!(
        detail["rationale"].as_str(),
        Some("Threshold from the 2025 trade study.")
    );

    // 7. Reverse typing: the def lists its CONTENT instantiation (steward
    //    ruling 2026-07-16 — check occurrences would ride verified_by,
    //    never this list).
    let instantiated = detail["instantiated_by"]
        .as_array()
        .expect("instantiated_by array");
    assert_eq!(
        instantiated
            .iter()
            .map(|r| r["name"].as_str().unwrap_or("<unnamed>"))
            .collect::<Vec<_>>(),
        vec!["tripReq"]
    );
}

/// Inherited contract (steward ruling 2026-07-16): `tripReq : TripReq`
/// owns no constraints, so its verdict evaluates the def's — the detail
/// surfaces them in the inherited buckets (single FeatureTyping hop,
/// exactly the evaluator's rule), each row labeled with its provenance,
/// while the spec-named owned buckets stay honestly empty.
#[test]
fn b21_requirement_detail_surfaces_inherited_contract() {
    let service = open_fixture();
    let id = row_id(&service, "tripReq");

    let detail = execute_command(
        &service,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": id }),
    )
    .expect("requirement_detail must succeed");

    assert_eq!(detail["assumed_constraints"].as_array().map(Vec::len), Some(0));
    assert_eq!(detail["required_constraints"].as_array().map(Vec::len), Some(0));

    let inherited_assumed = detail["inherited_assumed_constraints"]
        .as_array()
        .expect("inherited_assumed_constraints array");
    assert_eq!(inherited_assumed.len(), 1, "{inherited_assumed:?}");
    assert_eq!(
        inherited_assumed[0]["text"].as_str(),
        Some("maxTripTime > 0")
    );
    assert_eq!(
        inherited_assumed[0]["inherited_from"]["name"].as_str(),
        Some("TripReq"),
        "provenance labeling is binding: {inherited_assumed:?}"
    );

    let inherited_required = detail["inherited_required_constraints"]
        .as_array()
        .expect("inherited_required_constraints array");
    assert_eq!(inherited_required.len(), 2, "{inherited_required:?}");
    assert!(inherited_required
        .iter()
        .all(|c| c["inherited_from"]["name"] == "TripReq"));
}

/// Non-requirement targets fail hard — a detail view over the wrong element
/// kind is a caller bug, not a degraded mode.
#[test]
fn b21_requirement_detail_rejects_non_requirements() {
    let service = open_fixture();
    let rows = execute_command(&service, "sysml.query", json!({
        "uri": "__workspace__",
        "spec": {
            "filter": { "type": "kind", "kinds": ["PartDefinition"] },
            "projection": "summary"
        }
    }));
    // Resolve a non-requirement id via the generic query primitive.
    let rows = rows.expect("query must succeed");
    let part_id = rows["rows"]
        .as_array()
        .and_then(|r| r.first())
        .and_then(|r| r["id"].as_str())
        .expect("a PartDefinition id")
        .to_owned();

    let err = execute_command(
        &service,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": part_id }),
    )
    .expect_err("detail over a part definition must fail");
    assert!(
        err.to_string().contains("not a requirement"),
        "error names the contract violation: {err}"
    );
}
