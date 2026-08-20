//! B2 gate — the Requirements-workbench table read, end to end.
//!
//! Purpose-built spec-faithful fixture (NOT a corpus no-regression check):
//! nested requirements (2 levels), multiple doc comments, a StatusInfo
//! maturity annotation, and all five link kinds — satisfy, verify (one
//! passing + one failing case), derive (explicit `Derivation` typing),
//! refine (`@Refinement` dependency) — with one requirement carrying no
//! verification at all. Asserts through `sysml.workspace.requirement_rows`:
//!
//! 1. rows follow DOCUMENT order (source order, never alphabetical) with
//!    outline depths counting requirement ancestors only,
//! 2. statement text is the doc bodies joined in source order,
//! 3. requirement IDs are the declared short names (spec §7.21.2),
//! 4. the three-state verification rollup: recorded fail / incomplete /
//!    all-pass,
//! 5. link arrays carry ElementId identity,
//! 6. paging via limit/cursor pages the same ordered row set.
//!
//! (`sysml.workspace.requirements_trace`, the legacy projection of the same
//! row builder, was deleted under debt-ledger L59 — `requirement_rows` is
//! the one requirement-row surface.)
//!
//! Opens with the bundled standard library: the Derive/Refine discriminators
//! resolve against library anchors (Derivation def / Refinement metadata def).

use std::path::PathBuf;

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

const FIXTURE: &str = r#"package B2RowsFixture {
	private import DerivationConnections::*;
	private import ModelingMetadata::*;
	private import VerificationCases::*;

	constraint def AlwaysHolds { 1 <= 2 }
	constraint def NeverHolds { 1 > 2 }

	requirement def <'REQ-001'> SystemReq {
		doc /* The system shall regulate output current. */
		doc /* It shall trip within 30 ms of a fault. */
		@StatusInfo { status = StatusKind::tbd; }
		require constraint : AlwaysHolds;
	}

	requirement def <'REQ-002'> FailingReq {
		require constraint : NeverHolds;
	}

	// Nested requirement: outline depth. NOT nested under a verified
	// requirement — a constraint-less sub-requirement honestly rolls its
	// parent's verification case to Inconclusive (sub-requirements are
	// part of what "verified" means), which is its own scenario, not this
	// fixture's pass-lane.
	requirement def <'REQ-003'> UnverifiedReq {
		requirement <'REQ-003.1'> nestedReq {
			doc /* Nested: the regulator inherits the trip budget. */
		}
	}

	requirement systemReq : SystemReq;
	requirement failingReq : FailingReq;

	part amplifier {
		satisfy SystemReq;
	}

	verification def PassTest {
		@VerificationMethod { kind = (VerificationMethodKind::test, VerificationMethodKind::demo); }
		objective {
			verify SystemReq;
		}
	}
	verification def FailTest {
		@VerificationMethod { kind = VerificationMethodKind::analyze; }
		objective {
			verify FailingReq;
		}
	}

	// Derive: explicit typing by the library Derivation connection def.
	// First end = original, rest = derived (KerML positional end rule).
	connection reqDerivation : Derivation {
		end ref origEnd references systemReq;
		end ref derivEnd references failingReq;
	}

	// Refine: dependency annotated with the library Refinement metadata.
	dependency refinesDep from failingReq to systemReq {
		@Refinement;
	}
}
"#;

fn fixture_workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("sysml-b2-rows-fixture-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("B2RowsFixture.sysml"), FIXTURE).expect("write fixture");
    dir
}

fn open_fixture() -> SysmlService {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(fixture_workspace()))
        .expect("open B2 fixture workspace");
    service
}

fn fetch_rows(service: &SysmlService, spec: serde_json::Value) -> serde_json::Value {
    execute_command(service, "sysml.workspace.requirement_rows", json!({ "spec": spec }))
        .expect("requirement_rows must succeed")
}

fn row_names(result: &serde_json::Value) -> Vec<String> {
    result["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .map(|r| r["name"].as_str().unwrap_or("<unnamed>").to_owned())
        .collect()
}

fn row<'a>(result: &'a serde_json::Value, name: &str) -> &'a serde_json::Value {
    result["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .find(|r| r["name"] == name)
        .unwrap_or_else(|| panic!("row {name} present"))
}

fn link_names(row: &serde_json::Value, field: &str) -> Vec<String> {
    row[field]
        .as_array()
        .unwrap_or_else(|| panic!("{field} is an array"))
        .iter()
        .map(|l| l["name"].as_str().unwrap_or("<unnamed>").to_owned())
        .collect()
}

#[test]
fn b2_requirement_rows_end_to_end() {
    let service = open_fixture();
    let result = fetch_rows(&service, json!({}));

    // ── 1. Document order + outline depth ──
    assert_eq!(
        row_names(&result),
        vec![
            "SystemReq",
            "FailingReq",
            "UnverifiedReq",
            "nestedReq",
            "systemReq",
            "failingReq",
        ],
        "rows must follow source order, and stdlib requirements must not leak"
    );
    assert_eq!(result["total_estimate"], 6);

    let sys = row(&result, "SystemReq");
    let nested = row(&result, "nestedReq");
    let failing = row(&result, "FailingReq");
    let unverified = row(&result, "UnverifiedReq");
    let sys_usage = row(&result, "systemReq");
    let failing_usage = row(&result, "failingReq");

    assert_eq!(sys["outline_depth"], 0);
    assert_eq!(nested["outline_depth"], 1, "nested requirement is depth 1");
    assert_eq!(failing["outline_depth"], 0);

    // ── 2. Statement text: doc bodies joined in source order ──
    assert_eq!(
        sys["text"].as_str(),
        Some(
            "The system shall regulate output current.\n\nIt shall trip within 30 ms of a fault."
        ),
        "both doc comments join in source order"
    );
    assert_eq!(
        nested["text"].as_str(),
        Some("Nested: the regulator inherits the trip budget.")
    );
    assert_eq!(unverified["text"], serde_json::Value::Null);

    // ── 3. Requirement IDs = declared short names ──
    assert_eq!(sys["req_id"], "REQ-001");
    assert_eq!(nested["req_id"], "REQ-003.1");
    assert_eq!(failing["req_id"], "REQ-002");
    assert_eq!(unverified["req_id"], "REQ-003");

    // Maturity from @StatusInfo.
    assert_eq!(sys["maturity"], "tbd");
    assert_eq!(failing["maturity"], serde_json::Value::Null);

    // Kind + package context ride each row.
    assert_eq!(sys["kind"], "RequirementDefinition");
    assert_eq!(sys_usage["kind"], "RequirementUsage");
    assert_eq!(sys["owning_package"]["name"], "B2RowsFixture");
    assert_eq!(
        sys["qualified_name"].as_str(),
        Some("B2RowsFixture::SystemReq")
    );

    // ── 4. Three-state verification rollup ──
    assert_eq!(link_names(sys, "verified_by"), vec!["PassTest"]);
    assert_eq!(sys["verification"]["state"], "pass", "all linked cases pass");
    assert_eq!(sys["verification"]["cases_total"], 1);
    assert_eq!(sys["verification"]["cases_passed"], 1);

    assert_eq!(link_names(failing, "verified_by"), vec!["FailTest"]);
    assert_eq!(
        failing["verification"]["state"], "fail",
        "a recorded fail wins the rollup"
    );
    assert_eq!(failing["verification"]["cases_passed"], 0);

    assert_eq!(
        unverified["verification"]["state"], "incomplete",
        "no verify links is incomplete, never pass"
    );
    assert_eq!(unverified["verification"]["cases_total"], 0);

    // ── 4b. Declared verification methods (B4) — model intent off the
    // cases' @VerificationMethod annotations, distinct from
    // evaluation_mode (what the tool computed). Tuple keeps declaration
    // order; qualified references normalize to the enum literal.
    assert_eq!(
        sys["verification_methods"],
        json!(["test", "demo"]),
        "PassTest declares kind = (test, demo)"
    );
    assert_eq!(failing["verification_methods"], json!(["analyze"]));
    assert_eq!(
        unverified["verification_methods"],
        json!([]),
        "no verifying case → no declared method, never a default"
    );

    // ── 5. Link arrays: satisfy + derive + refine, ElementId identity ──
    assert_eq!(link_names(sys, "satisfied_by"), vec!["amplifier"]);

    assert_eq!(link_names(failing_usage, "derived_from"), vec!["systemReq"]);
    assert_eq!(link_names(sys_usage, "derives"), vec!["failingReq"]);
    assert_eq!(link_names(failing_usage, "refines"), vec!["systemReq"]);
    assert_eq!(
        failing_usage["derived_from"][0]["id"], sys_usage["id"],
        "links carry ElementId identity, not just names"
    );

    // Every row id is a real element id string.
    for r in result["rows"].as_array().unwrap() {
        assert!(
            r["id"].as_str().is_some_and(|s| !s.is_empty()),
            "row id present"
        );
        assert!(r["source_span"]["file"].as_str().is_some(), "span present");
    }
}

#[test]
fn b2_requirement_rows_paging_walks_the_same_order() {
    let service = open_fixture();
    let full = fetch_rows(&service, json!({}));
    let expected = row_names(&full);

    let mut paged: Vec<String> = Vec::new();
    let mut cursor: Option<String> = None;
    for _ in 0..10 {
        let mut spec = json!({ "limit": 2 });
        if let Some(c) = &cursor {
            spec["cursor"] = json!(c);
        }
        let page = fetch_rows(&service, spec);
        assert_eq!(page["cursor_invalidated"], false);
        paged.extend(row_names(&page));
        match page["cursor"].as_str() {
            Some(c) => cursor = Some(c.to_owned()),
            None => break,
        }
    }
    assert_eq!(paged, expected, "paged walk reproduces the full ordered set");
}

/// Declaration-form verify — the pilot-canonical shape used by the
/// legacy oscillator fixture (`objective { verify requirement check :
/// ReqDef; }`). Two identical requirements, one verified by the reference
/// form, one by the declaration form; the two rows must agree:
///
/// - the Verify edge from the declaration form targets the membership-owned
///   check-usage (spec referencedConstraint branch 2), so the check-usage row
///   carries the direct link;
/// - the def row reports the same case through the shared rollup
///   (`elements_verifying`) and rolls up to pass;
/// - the verify form mints NO Satisfy edge (the pre-fix
///   SatisfyRequirementUsage kind did).
const DECL_FIXTURE: &str = r#"package B2DeclVerify {
	constraint def AlwaysHolds { 1 <= 2 }

	requirement def <'REQ-A'> RefForm {
		require constraint : AlwaysHolds;
	}
	requirement def <'REQ-B'> DeclForm {
		require constraint : AlwaysHolds;
	}

	verification def RefTest {
		objective {
			verify RefForm;
		}
	}
	verification def DeclTest {
		objective {
			verify requirement declCheck : DeclForm;
		}
	}
}
"#;

#[test]
fn b2_declaration_form_verify_rolls_up_to_the_def_row() {
    let dir = std::env::temp_dir().join(format!(
        "sysml-b2-decl-verify-fixture-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("B2DeclVerify.sysml"), DECL_FIXTURE).expect("write fixture");
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(dir))
        .expect("open declaration-form fixture workspace");

    let result = fetch_rows(&service, json!({}));

    // Both forms verify their def identically — the tool must not disagree
    // with itself on the two spellings.
    let ref_form = row(&result, "RefForm");
    assert_eq!(link_names(ref_form, "verified_by"), vec!["RefTest"]);
    assert_eq!(ref_form["verification"]["state"], "pass");

    let decl_form = row(&result, "DeclForm");
    assert_eq!(
        link_names(decl_form, "verified_by"),
        vec!["DeclTest"],
        "declaration form def row reports its case via the shared rollup"
    );
    assert_eq!(
        decl_form["verification"]["state"], "pass",
        "declaration form def row rolls up to pass"
    );
    assert_eq!(decl_form["verification"]["cases_total"], 1);
    assert_eq!(decl_form["verification"]["cases_passed"], 1);

    // Role-based rows (steward ruling 2026-07-16, NARROWING — not
    // reversing — the earlier "the fixture gains 4 check rows" behavior): the
    // membership-owned check occurrence is verification bookkeeping
    // ("a record of the evaluations", VerificationCases.sysml), so it is
    // NOT a peer row by default…
    assert!(
        !row_names(&result).contains(&"declCheck".to_owned()),
        "check occurrence must not be a default row: {:?}",
        row_names(&result)
    );

    // …but it remains a real, kind-correct RequirementUsage: the reveal
    // flag lists it, and the Verify edge still targets it directly (the
    // rollup mechanics the 2026-07-16 B2 follow-up shipped are untouched).
    let revealed = fetch_rows(&service, json!({ "include_verification_occurrences": true }));
    let check = row(&revealed, "declCheck");
    assert_eq!(
        link_names(check, "verified_by"),
        vec!["DeclTest"],
        "the Verify edge targets the local check-usage, not the def"
    );

    // The verify form must never mint Satisfy edges (pre-fix defect shape).
    assert_eq!(link_names(decl_form, "satisfied_by"), Vec::<String>::new());
    assert_eq!(link_names(check, "satisfied_by"), Vec::<String>::new());
}

