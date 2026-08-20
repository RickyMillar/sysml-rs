//! v2 §7.2 gate — buffer-writeback field edits.
//!
//! The service COMPUTES guarded TextEdits; the client applies them. This
//! gate proves the full loop the FE will run: fetch rows → compute edit →
//! verify `expected_old_text` against the buffer → splice → REPARSE → the
//! field reads back changed. Also pins the fail-hard boundaries: absent
//! targets are creation actions (error, never a guessed insertion), the
//! StatusKind vocabulary is closed, and blank/multi-line values die at the
//! boundary.

use std::path::PathBuf;

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::position::position_to_offset;
use sysml_service::{execute_command, SysmlService};

const FIXTURE: &str = r#"package FieldEditFixture {
	private import ModelingMetadata::*;

	part def Breaker;

	requirement def <'REQ-020'> TripReq {
		doc /* The breaker shall trip within 40 ms. */
		@StatusInfo { status = StatusKind::tbd; }
		subject breaker : Breaker;
		attribute maxTripTime = 40;
	}

	requirement bare;
}
"#;

fn fixture_workspace(tag: &str, content: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sysml-field-edit-fixture-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("FieldEditFixture.sysml"), content).expect("write fixture");
    dir
}

fn open_workspace(tag: &str, content: &str) -> SysmlService {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(fixture_workspace(tag, content)))
        .expect("open field-edit fixture workspace");
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

/// The client-side splice contract: locate the range via line/col, VERIFY
/// the buffer slice equals `expected_old_text` (the stale-buffer guard),
/// then replace.
fn apply(content: &str, computed: &serde_json::Value) -> String {
    let edit = &computed["edit"];
    let start = position_to_offset(
        edit["line_start"].as_u64().expect("line_start") as u32,
        edit["col_start"].as_u64().expect("col_start") as u32,
        content,
    );
    let end = position_to_offset(
        edit["line_end"].as_u64().expect("line_end") as u32,
        edit["col_end"].as_u64().expect("col_end") as u32,
        content,
    );
    let expected = edit["expected_old_text"]
        .as_str()
        .expect("every field edit carries the staleness guard");
    assert_eq!(
        &content[start..end],
        expected,
        "guard must describe exactly the text being replaced"
    );
    format!(
        "{}{}{}",
        &content[..start],
        edit["new_text"].as_str().expect("new_text"),
        &content[end..]
    )
}

#[test]
fn doc_edit_round_trips_through_reparse() {
    let service = open_workspace("doc", FIXTURE);
    let id = row_id(&service, "TripReq");

    let computed = execute_command(
        &service,
        "sysml.workspace.edit_requirement_doc",
        json!({ "element_id": id, "new_text": "The breaker shall trip within 35 ms." }),
    )
    .expect("doc edit computes");
    assert_eq!(computed["field"], "doc");
    assert_eq!(
        computed["edit"]["expected_old_text"],
        " The breaker shall trip within 40 ms. "
    );

    let new_source = apply(FIXTURE, &computed);
    assert!(new_source.contains("doc /* The breaker shall trip within 35 ms. */"));

    let reparsed = open_workspace("doc-reparsed", &new_source);
    let rows = execute_command(
        &reparsed,
        "sysml.workspace.requirement_rows",
        json!({ "spec": {} }),
    )
    .expect("rows after reparse");
    let row = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "TripReq")
        .expect("TripReq row");
    assert_eq!(
        row["text"].as_str().unwrap(),
        "The breaker shall trip within 35 ms.",
        "reparsed statement must carry the edited body"
    );
}

#[test]
fn attribute_value_edit_round_trips_through_reparse() {
    let service = open_workspace("attr", FIXTURE);
    let id = row_id(&service, "TripReq");
    let detail = execute_command(
        &service,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": id }),
    )
    .expect("detail");
    let attr_id = detail["referenced_attributes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "maxTripTime")
        .expect("maxTripTime attribute")["id"]
        .as_str()
        .unwrap()
        .to_owned();

    let computed = execute_command(
        &service,
        "sysml.workspace.edit_attribute_value",
        json!({ "element_id": attr_id, "new_value": "35" }),
    )
    .expect("value edit computes");
    assert_eq!(computed["edit"]["expected_old_text"], "40");

    let new_source = apply(FIXTURE, &computed);
    assert!(new_source.contains("attribute maxTripTime = 35;"));

    let reparsed = open_workspace("attr-reparsed", &new_source);
    let rid = row_id(&reparsed, "TripReq");
    let detail = execute_command(
        &reparsed,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": rid }),
    )
    .expect("detail after reparse");
    let value = detail["referenced_attributes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["name"] == "maxTripTime")
        .expect("maxTripTime attribute")["value"]
        .clone();
    assert_eq!(
        value.as_str().unwrap_or_default(),
        "35",
        "reparsed attribute must carry the edited value"
    );
}

#[test]
fn maturity_edit_round_trips_through_reparse() {
    let service = open_workspace("maturity", FIXTURE);
    let id = row_id(&service, "TripReq");

    let computed = execute_command(
        &service,
        "sysml.workspace.edit_requirement_maturity",
        json!({ "element_id": id, "status": "done" }),
    )
    .expect("maturity edit computes");
    assert_eq!(computed["edit"]["expected_old_text"], "StatusKind::tbd");
    assert_eq!(computed["edit"]["new_text"], "StatusKind::done");

    let new_source = apply(FIXTURE, &computed);
    let reparsed = open_workspace("maturity-reparsed", &new_source);
    let rows = execute_command(
        &reparsed,
        "sysml.workspace.requirement_rows",
        json!({ "spec": {} }),
    )
    .expect("rows after reparse");
    let row = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "TripReq")
        .expect("TripReq row");
    assert_eq!(
        row["maturity"].as_str().unwrap(),
        "done",
        "reparsed maturity must carry the edited status"
    );
}

#[test]
fn create_requirement_round_trips_through_reparse() {
    let service = open_workspace("create", FIXTURE);
    // Parent = the package (find via the TripReq row's owning package id is
    // not exposed — use the requirement def itself as parent: nested clause).
    let parent = row_id(&service, "TripReq");

    let computed = execute_command(
        &service,
        "sysml.workspace.create_requirement",
        json!({
            "parent_id": parent,
            "name": "resetTime",
            "short_name": "REQ-021",
            "doc": "The breaker shall reset within 2 s."
        }),
    )
    .expect("create computes");
    assert_eq!(computed["field"], "create_requirement");

    let new_source = apply(FIXTURE, &computed);
    assert!(
        new_source.contains("requirement <'REQ-021'> resetTime {"),
        "skeleton present:\n{new_source}"
    );

    let reparsed = open_workspace("create-reparsed", &new_source);
    let rows = execute_command(
        &reparsed,
        "sysml.workspace.requirement_rows",
        json!({ "spec": {} }),
    )
    .expect("rows after reparse");
    let row = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "resetTime")
        .expect("created requirement is a row");
    assert_eq!(row["req_id"].as_str().unwrap(), "REQ-021");
    assert_eq!(
        row["text"].as_str().unwrap(),
        "The breaker shall reset within 2 s."
    );
}

#[test]
fn add_doc_converts_a_semicolon_body_and_round_trips() {
    let service = open_workspace("adddoc", FIXTURE);
    let bare = row_id(&service, "bare");

    let computed = execute_command(
        &service,
        "sysml.workspace.add_requirement_doc",
        json!({ "element_id": bare, "new_text": "Recovered from a bare declaration." }),
    )
    .expect("add_doc computes");
    assert_eq!(computed["edit"]["expected_old_text"], ";");

    let new_source = apply(FIXTURE, &computed);
    let reparsed = open_workspace("adddoc-reparsed", &new_source);
    let rows = execute_command(
        &reparsed,
        "sysml.workspace.requirement_rows",
        json!({ "spec": {} }),
    )
    .expect("rows after reparse");
    let row = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "bare")
        .expect("bare row");
    assert_eq!(
        row["text"].as_str().unwrap(),
        "Recovered from a bare declaration.",
        "the ;-declaration must grow a braced body carrying the doc"
    );

    // Adding where one exists routes to the EDIT command instead.
    let trip = row_id(&service, "TripReq");
    let err = execute_command(
        &service,
        "sysml.workspace.add_requirement_doc",
        json!({ "element_id": trip, "new_text": "x" }),
    )
    .expect_err("add on an element with a doc must fail");
    assert!(err.to_string().contains("already has a doc"), "{err}");
}

#[test]
fn add_maturity_round_trips_through_reparse() {
    let service = open_workspace("addmat", FIXTURE);
    let bare = row_id(&service, "bare");

    let computed = execute_command(
        &service,
        "sysml.workspace.add_requirement_maturity",
        json!({ "element_id": bare, "status": "open" }),
    )
    .expect("add_maturity computes");

    let new_source = apply(FIXTURE, &computed);
    let reparsed = open_workspace("addmat-reparsed", &new_source);
    let rows = execute_command(
        &reparsed,
        "sysml.workspace.requirement_rows",
        json!({ "spec": {} }),
    )
    .expect("rows after reparse");
    let row = rows["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "bare")
        .expect("bare row");
    assert_eq!(row["maturity"].as_str().unwrap(), "open");

    // Duplicate add fails toward the edit command.
    let trip = row_id(&service, "TripReq");
    let err = execute_command(
        &service,
        "sysml.workspace.add_requirement_maturity",
        json!({ "element_id": trip, "status": "open" }),
    )
    .expect_err("add on an element with @StatusInfo must fail");
    assert!(err.to_string().contains("already has @StatusInfo"), "{err}");
}

#[test]
fn create_requirement_rejects_bad_parents_and_names() {
    let service = open_workspace("createfail", FIXTURE);
    let trip = row_id(&service, "TripReq");

    for bad_name in ["", "4x", "a b", "x;"] {
        execute_command(
            &service,
            "sysml.workspace.create_requirement",
            json!({ "parent_id": trip, "name": bad_name }),
        )
        .expect_err("invalid identifier must fail");
    }

    // A part def is not a requirement container.
    let detail = execute_command(
        &service,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": trip }),
    )
    .expect("detail");
    let subject_target = detail["subject"]["id"].as_str();
    if let Some(part_id) = subject_target {
        let err = execute_command(
            &service,
            "sysml.workspace.create_requirement",
            json!({ "parent_id": part_id, "name": "x" }),
        )
        .expect_err("non-container parent must fail");
        assert!(
            err.to_string().contains("pick a package or requirement"),
            "{err}"
        );
    }
}

#[test]
fn absent_targets_and_invalid_inputs_fail_hard() {
    let service = open_workspace("failhard", FIXTURE);
    let bare = row_id(&service, "bare");
    let trip = row_id(&service, "TripReq");

    // No doc comment → creation territory, not an edit.
    let err = execute_command(
        &service,
        "sysml.workspace.edit_requirement_doc",
        json!({ "element_id": bare, "new_text": "x" }),
    )
    .expect_err("doc edit on doc-less element must fail");
    assert!(err.to_string().contains("no doc comment"), "{err}");

    // No @StatusInfo → creation territory.
    let err = execute_command(
        &service,
        "sysml.workspace.edit_requirement_maturity",
        json!({ "element_id": bare, "status": "done" }),
    )
    .expect_err("maturity edit without @StatusInfo must fail");
    assert!(err.to_string().contains("no @StatusInfo"), "{err}");

    // Closed vocabulary.
    let err = execute_command(
        &service,
        "sysml.workspace.edit_requirement_maturity",
        json!({ "element_id": trip, "status": "aproved" }),
    )
    .expect_err("invalid StatusKind must die at the boundary");
    assert!(err.to_string().contains("invalid StatusKind"), "{err}");

    // Blank / multi-statement values.
    let detail = execute_command(
        &service,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": trip }),
    )
    .expect("detail");
    let attr_id = detail["referenced_attributes"].as_array().unwrap()[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    for bad in ["", "40; part def Evil"] {
        execute_command(
            &service,
            "sysml.workspace.edit_attribute_value",
            json!({ "element_id": attr_id, "new_value": bad }),
        )
        .expect_err("blank / multi-statement values must fail");
    }
}

// ─── R5 link writing (§7.6) — satisfy/verify/derive round trips ───────────

const LINKS_SPEC: &str = r#"package LinkSpecPkg {
	requirement def <'REQ-100'> tripReq {
		doc /* Trip fast. */
	}
	requirement def <'REQ-101'> speedReq;
}
"#;

const LINKS_IMPL: &str = r#"package LinkImplPkg {
	part def Breaker;
	part breaker : Breaker;
	part legacyBreaker : Breaker {
		satisfy LinkSpecPkg::tripReq;
	}

	verification def TripTest {
		objective {
			verify LinkSpecPkg::speedReq;
		}
	}

	verification def BareTest;
}
"#;

/// Two-file workspace for the cross-file link tests: the spec document and
/// the implementation/verification context live in separate files (the
/// satisfy/verify insertions target the PICKED element's file).
fn links_workspace(tag: &str, spec: &str, impl_src: &str) -> SysmlService {
    let dir = std::env::temp_dir().join(format!(
        "sysml-link-edit-fixture-{tag}-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("LinkSpec.sysml"), spec).expect("write spec fixture");
    std::fs::write(dir.join("LinkImpl.sysml"), impl_src).expect("write impl fixture");
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(dir))
        .expect("open link fixture workspace");
    service
}

/// The element id of a named non-requirement pick target (part / case),
/// via sysml.query.
fn element_id_by_name(service: &SysmlService, name: &str) -> String {
    let result = execute_command(
        service,
        "sysml.query",
        json!({ "uri": "__workspace__", "spec": { "filter": { "type": "name_match", "name_match": { "exact": name } }, "projection": "summary" } }),
    )
    .expect("query must succeed");
    result["rows"]
        .as_array()
        .expect("query rows")
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("element {name} present"))["id"]
        .as_str()
        .expect("element id")
        .to_owned()
}

fn row_link_names(service: &SysmlService, row_name: &str, field: &str) -> Vec<String> {
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
        .find(|r| r["name"] == row_name)
        .unwrap_or_else(|| panic!("row {row_name} present"))[field]
        .as_array()
        .unwrap_or_else(|| panic!("{field} is an array"))
        .iter()
        .filter_map(|l| l["name"].as_str().map(str::to_owned))
        .collect()
}

#[test]
fn satisfy_link_round_trips_cross_file_with_qualified_ref() {
    let service = links_workspace("satisfy", LINKS_SPEC, LINKS_IMPL);
    let req = row_id(&service, "tripReq");
    let subject = element_id_by_name(&service, "breaker");

    let computed = execute_command(
        &service,
        "sysml.workspace.add_satisfy_link",
        json!({ "requirement_id": req, "subject_id": subject }),
    )
    .expect("satisfy link computes");
    assert_eq!(computed["field"], "add_satisfy_link");
    assert!(
        computed["uri"].as_str().unwrap().ends_with("LinkImpl.sysml"),
        "the edit targets the SUBJECT's file: {}",
        computed["uri"]
    );

    // Cross-package target → fully qualified reference; the `;` declaration
    // grows a braced body.
    let new_impl = apply(LINKS_IMPL, &computed);
    assert!(
        new_impl.contains("satisfy LinkSpecPkg::tripReq;"),
        "{new_impl}"
    );

    let reparsed = links_workspace("satisfy-reparsed", LINKS_SPEC, &new_impl);
    assert!(
        row_link_names(&reparsed, "tripReq", "satisfied_by").contains(&"breaker".to_owned()),
        "the qualified satisfy reference must round-trip into the Satisfy edge"
    );
}

#[test]
fn verify_link_inserts_into_existing_objective() {
    let service = links_workspace("verify", LINKS_SPEC, LINKS_IMPL);
    let req = row_id(&service, "tripReq");
    let case = element_id_by_name(&service, "TripTest");

    let computed = execute_command(
        &service,
        "sysml.workspace.add_verify_link",
        json!({ "requirement_id": req, "case_id": case }),
    )
    .expect("verify link computes");

    let new_impl = apply(LINKS_IMPL, &computed);
    assert!(
        new_impl.contains("verify LinkSpecPkg::tripReq;"),
        "{new_impl}"
    );
    assert_eq!(
        new_impl.matches("objective").count(),
        1,
        "the existing objective is reused, never a second block: {new_impl}"
    );

    let reparsed = links_workspace("verify-reparsed", LINKS_SPEC, &new_impl);
    assert!(
        row_link_names(&reparsed, "tripReq", "verified_by").contains(&"TripTest".to_owned()),
        "the new verify link must round-trip"
    );
    assert!(
        row_link_names(&reparsed, "speedReq", "verified_by").contains(&"TripTest".to_owned()),
        "the pre-existing verify link must survive the insertion"
    );
}

#[test]
fn verify_link_synthesizes_objective_when_absent() {
    let service = links_workspace("verify-bare", LINKS_SPEC, LINKS_IMPL);
    let req = row_id(&service, "tripReq");
    let case = element_id_by_name(&service, "BareTest");

    let computed = execute_command(
        &service,
        "sysml.workspace.add_verify_link",
        json!({ "requirement_id": req, "case_id": case }),
    )
    .expect("verify link computes");
    assert!(
        computed["edit"]["new_text"]
            .as_str()
            .unwrap()
            .contains("objective {"),
        "a case without an objective gains the whole block: {}",
        computed["edit"]["new_text"]
    );

    let new_impl = apply(LINKS_IMPL, &computed);
    let reparsed = links_workspace("verify-bare-reparsed", LINKS_SPEC, &new_impl);
    assert!(
        row_link_names(&reparsed, "tripReq", "verified_by").contains(&"BareTest".to_owned()),
        "the synthesized objective must round-trip into the Verify edge"
    );
}

#[test]
fn derive_link_round_trips_and_carries_the_import() {
    let service = links_workspace("derive", LINKS_SPEC, LINKS_IMPL);
    let derived = row_id(&service, "speedReq");
    let original = row_id(&service, "tripReq");

    let computed = execute_command(
        &service,
        "sysml.workspace.add_derive_link",
        json!({ "requirement_id": derived, "original_id": original }),
    )
    .expect("derive link computes");
    assert!(
        computed["uri"].as_str().unwrap().ends_with("LinkSpec.sysml"),
        "the connection lands in the DERIVED requirement's package: {}",
        computed["uri"]
    );
    let new_text = computed["edit"]["new_text"].as_str().unwrap();
    assert!(
        new_text.contains("private import RequirementDerivation::*;"),
        "the load-bearing import rides the same insertion: {new_text}"
    );
    assert!(new_text.contains("end #original ::> tripReq;"), "{new_text}");
    assert!(new_text.contains("end #derive ::> speedReq;"), "{new_text}");

    let new_spec = apply(LINKS_SPEC, &computed);
    let reparsed = links_workspace("derive-reparsed", &new_spec, LINKS_IMPL);
    assert!(
        row_link_names(&reparsed, "speedReq", "derived_from").contains(&"tripReq".to_owned()),
        "the derivation connection must round-trip into the Derive edge"
    );
    assert!(
        row_link_names(&reparsed, "tripReq", "derives").contains(&"speedReq".to_owned()),
        "the original's incoming side must show too"
    );

    // Second add of the SAME link on the reparsed workspace = duplicate →
    // fail hard.
    let derived2 = row_id(&reparsed, "speedReq");
    let original2 = row_id(&reparsed, "tripReq");
    let err = execute_command(
        &reparsed,
        "sysml.workspace.add_derive_link",
        json!({ "requirement_id": derived2, "original_id": original2 }),
    )
    .expect_err("duplicate derive link must fail hard");
    assert!(err.to_string().contains("already derives"), "{err}");
}

#[test]
fn link_duplicates_and_bad_targets_fail_hard() {
    let service = links_workspace("link-failhard", LINKS_SPEC, LINKS_IMPL);
    let trip = row_id(&service, "tripReq");
    let speed = row_id(&service, "speedReq");
    let legacy = element_id_by_name(&service, "legacyBreaker");
    let case = element_id_by_name(&service, "TripTest");
    let part_def = element_id_by_name(&service, "Breaker");

    // Duplicate satisfy (the fixture's qualified `satisfy LinkSpecPkg::tripReq;`
    // already minted the edge — which also pins qualified-name resolution).
    let err = execute_command(
        &service,
        "sysml.workspace.add_satisfy_link",
        json!({ "requirement_id": trip, "subject_id": legacy }),
    )
    .expect_err("duplicate satisfy link must fail hard");
    assert!(err.to_string().contains("already satisfies"), "{err}");

    // Duplicate verify.
    let err = execute_command(
        &service,
        "sysml.workspace.add_verify_link",
        json!({ "requirement_id": speed, "case_id": case }),
    )
    .expect_err("duplicate verify link must fail hard");
    assert!(err.to_string().contains("already verifies"), "{err}");

    // Verify against a non-case.
    let err = execute_command(
        &service,
        "sysml.workspace.add_verify_link",
        json!({ "requirement_id": trip, "case_id": part_def }),
    )
    .expect_err("verify against a non-case must fail hard");
    assert!(err.to_string().contains("not a verification case"), "{err}");

    // Satisfy/derive against a non-requirement.
    let err = execute_command(
        &service,
        "sysml.workspace.add_satisfy_link",
        json!({ "requirement_id": part_def, "subject_id": legacy }),
    )
    .expect_err("satisfy of a non-requirement must fail hard");
    assert!(err.to_string().contains("not a requirement"), "{err}");
    let err = execute_command(
        &service,
        "sysml.workspace.add_derive_link",
        json!({ "requirement_id": trip, "original_id": part_def }),
    )
    .expect_err("derive from a non-requirement must fail hard");
    assert!(err.to_string().contains("not a requirement"), "{err}");
}

#[test]
fn refine_link_round_trips_and_carries_the_import() {
    let service = links_workspace("refine", LINKS_SPEC, LINKS_IMPL);
    let refining = row_id(&service, "speedReq");
    let refined = row_id(&service, "tripReq");

    let computed = execute_command(
        &service,
        "sysml.workspace.add_refine_link",
        json!({ "requirement_id": refining, "refined_id": refined }),
    )
    .expect("refine link computes");
    assert_eq!(computed["field"], "add_refine_link");
    assert!(
        computed["uri"].as_str().unwrap().ends_with("LinkSpec.sysml"),
        "the dependency lands in the REFINING requirement's package: {}",
        computed["uri"]
    );
    let new_text = computed["edit"]["new_text"].as_str().unwrap();
    assert!(
        new_text.contains("private import ModelingMetadata::*;"),
        "the load-bearing import rides the same insertion: {new_text}"
    );
    assert!(
        new_text.contains("dependency from speedReq to tripReq"),
        "{new_text}"
    );
    assert!(new_text.contains("@Refinement;"), "{new_text}");

    let new_spec = apply(LINKS_SPEC, &computed);
    let reparsed = links_workspace("refine-reparsed", &new_spec, LINKS_IMPL);
    assert!(
        row_link_names(&reparsed, "speedReq", "refines").contains(&"tripReq".to_owned()),
        "the refinement dependency must round-trip into the Refine edge"
    );

    // Duplicate on the reparsed workspace = fail hard.
    let refining2 = row_id(&reparsed, "speedReq");
    let refined2 = row_id(&reparsed, "tripReq");
    let err = execute_command(
        &reparsed,
        "sysml.workspace.add_refine_link",
        json!({ "requirement_id": refining2, "refined_id": refined2 }),
    )
    .expect_err("duplicate refine link must fail hard");
    assert!(err.to_string().contains("already refines"), "{err}");

    // A refine target that is not a requirement fails hard.
    let part_def = element_id_by_name(&service, "Breaker");
    let err = execute_command(
        &service,
        "sysml.workspace.add_refine_link",
        json!({ "requirement_id": refining, "refined_id": part_def }),
    )
    .expect_err("refine of a non-requirement must fail hard");
    assert!(err.to_string().contains("not a requirement"), "{err}");
}

#[test]
fn add_rationale_round_trips_through_reparse() {
    let service = open_workspace("rationale", FIXTURE);
    let id = row_id(&service, "TripReq");

    // Nested-parent insert: TripReq has no rationale yet.
    let computed = execute_command(
        &service,
        "sysml.workspace.add_rationale",
        json!({ "element_id": id, "text": "Threshold from the 2025 trade study." }),
    )
    .expect("rationale add computes");
    assert_eq!(computed["field"], "add_rationale");
    let new_text = computed["edit"]["new_text"].as_str().unwrap();
    assert!(
        new_text.contains("@Rationale { text = \"Threshold from the 2025 trade study.\"; }"),
        "{new_text}"
    );

    let new_source = apply(FIXTURE, &computed);
    let reparsed = open_workspace("rationale-reparsed", &new_source);
    let rid = row_id(&reparsed, "TripReq");
    let detail = execute_command(
        &reparsed,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": rid }),
    )
    .expect("detail after reparse");
    assert_eq!(
        detail["rationale"].as_str().unwrap_or_default(),
        "Threshold from the 2025 trade study.",
        "the added @Rationale must round-trip into requirement_detail.rationale"
    );

    // Blank + multi-line text die at the boundary.
    for bad in ["", "line one\nline two"] {
        execute_command(
            &service,
            "sysml.workspace.add_rationale",
            json!({ "element_id": id, "text": bad }),
        )
        .expect_err("blank / multi-line rationale must fail");
    }
}

#[test]
fn add_attribute_round_trips_through_reparse() {
    let service = open_workspace("addattr", FIXTURE);
    let id = row_id(&service, "TripReq");

    let computed = execute_command(
        &service,
        "sysml.workspace.add_attribute",
        json!({ "element_id": id, "name": "resetTime", "value": "5" }),
    )
    .expect("attribute add computes");
    assert_eq!(computed["field"], "add_attribute");
    assert!(
        computed["edit"]["new_text"]
            .as_str()
            .unwrap()
            .contains("attribute resetTime = 5;"),
        "{}",
        computed["edit"]["new_text"]
    );

    let new_source = apply(FIXTURE, &computed);
    let reparsed = open_workspace("addattr-reparsed", &new_source);
    let rid = row_id(&reparsed, "TripReq");
    let detail = execute_command(
        &reparsed,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": rid }),
    )
    .expect("detail after reparse");
    let attrs = detail["referenced_attributes"].as_array().unwrap();
    let added = attrs
        .iter()
        .find(|a| a["name"] == "resetTime")
        .expect("the added attribute must round-trip into requirement_detail");
    assert_eq!(added["value"].as_str().unwrap(), "5");

    // Duplicate name, bad identifier, and `;`-bearing value fail hard.
    let err = execute_command(
        &service,
        "sysml.workspace.add_attribute",
        json!({ "element_id": id, "name": "maxTripTime", "value": "1" }),
    )
    .expect_err("duplicate attribute name must fail");
    assert!(err.to_string().contains("already has an attribute"), "{err}");
    for bad in [json!({"element_id": id, "name": "has space"}), json!({"element_id": id, "name": "ok", "value": "1; part def Evil"})] {
        execute_command(&service, "sysml.workspace.add_attribute", bad)
            .expect_err("bad identifier / multi-statement value must fail");
    }
}

#[test]
fn add_constraint_round_trips_through_reparse() {
    let service = open_workspace("addconstr", FIXTURE);
    let id = row_id(&service, "TripReq");

    let computed = execute_command(
        &service,
        "sysml.workspace.add_constraint",
        json!({ "element_id": id, "kind": "require", "name": "fastEnough", "expr": "maxTripTime <= 50" }),
    )
    .expect("constraint add computes");
    assert_eq!(computed["field"], "add_constraint");
    assert!(
        computed["edit"]["new_text"]
            .as_str()
            .unwrap()
            .contains("require constraint fastEnough { maxTripTime <= 50 }"),
        "{}",
        computed["edit"]["new_text"]
    );

    let new_source = apply(FIXTURE, &computed);
    let reparsed = open_workspace("addconstr-reparsed", &new_source);
    let rid = row_id(&reparsed, "TripReq");
    let detail = execute_command(
        &reparsed,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": rid }),
    )
    .expect("detail after reparse");
    let required = detail["required_constraints"].as_array().unwrap();
    assert!(
        required
            .iter()
            .any(|c| c["text"].as_str().unwrap_or_default().contains("maxTripTime <= 50")),
        "the added require constraint must round-trip into requirement_detail: {:?}",
        required
    );

    // Bad kind, braces/`;` in expr, blank expr all fail hard.
    execute_command(
        &service,
        "sysml.workspace.add_constraint",
        json!({ "element_id": id, "kind": "maybe", "expr": "x > 0" }),
    )
    .expect_err("bad kind must fail");
    for bad in ["x > 0; y < 1", "{ nested }", ""] {
        execute_command(
            &service,
            "sysml.workspace.add_constraint",
            json!({ "element_id": id, "kind": "require", "expr": bad }),
        )
        .expect_err("brace / `;` / blank expr must fail");
    }
}

#[test]
fn add_requirement_role_round_trips_and_validates_kind() {
    let service = open_workspace("role", FIXTURE);
    let trip = row_id(&service, "TripReq");
    let bare = row_id(&service, "bare");
    let breaker = element_id_by_name(&service, "Breaker"); // the fixture's part def

    // Actor: repeatable, part-definition target → round-trips into detail.actors.
    let computed = execute_command(
        &service,
        "sysml.workspace.add_requirement_role",
        json!({ "requirement_id": trip, "role": "actor", "type_id": breaker, "name": "operator" }),
    )
    .expect("actor add computes");
    assert!(
        computed["edit"]["new_text"]
            .as_str()
            .unwrap()
            .contains("actor operator : Breaker;"),
        "{}",
        computed["edit"]["new_text"]
    );
    let new_source = apply(FIXTURE, &computed);
    let reparsed = open_workspace("role-reparsed", &new_source);
    let rid = row_id(&reparsed, "TripReq");
    let detail = execute_command(
        &reparsed,
        "sysml.workspace.requirement_detail",
        json!({ "element_id": rid }),
    )
    .expect("detail after reparse");
    assert!(
        detail["actors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|a| a["name"] == "operator"),
        "the added actor must round-trip into requirement_detail.actors: {:?}",
        detail["actors"]
    );

    // Subject: singleton. `bare` has none → succeeds; TripReq already has one → fails.
    execute_command(
        &service,
        "sysml.workspace.add_requirement_role",
        json!({ "requirement_id": bare, "role": "subject", "type_id": breaker, "name": "unit" }),
    )
    .expect("subject on a subject-less requirement computes");
    let err = execute_command(
        &service,
        "sysml.workspace.add_requirement_role",
        json!({ "requirement_id": trip, "role": "subject", "type_id": breaker, "name": "dup" }),
    )
    .expect_err("second subject must fail");
    assert!(err.to_string().contains("already has a subject"), "{err}");

    // Kind validation: a concern must reference a concern definition; Breaker
    // (a part def) is rejected. And an unknown role string is rejected.
    let err = execute_command(
        &service,
        "sysml.workspace.add_requirement_role",
        json!({ "requirement_id": trip, "role": "concern", "type_id": breaker, "name": "c" }),
    )
    .expect_err("concern with a non-concern target must fail");
    assert!(err.to_string().contains("concern definition"), "{err}");
    execute_command(
        &service,
        "sysml.workspace.add_requirement_role",
        json!({ "requirement_id": trip, "role": "bogus", "type_id": breaker, "name": "x" }),
    )
    .expect_err("unknown role must fail");
}
