//! B1 gate — Derive / Refine / Trace elaboration through the service.
//!
//! Purpose-built spec-faithful fixture (NOT a corpus no-regression check):
//! one explicitly-typed `Derivation` connection (the normative
//! `DerivationConnections::Derivation` semantics the `#derivation` keyword
//! sugars over — the keyword itself is a filed parser gap, B1b), one
//! `ModelingMetadata::Refinement`-annotated dependency, one plain
//! dependency. Asserts:
//!
//! 1. the three `RelationshipKind` edges elaborate with the ruled
//!    endpoint conventions (Derive: derived → original; Refine/Trace:
//!    client → supplier), and
//! 2. `sysml.trace_matrix` answers each kind over `__workspace__`, and
//! 3. `sysml.workspace.requirement_rows` rows carry the `derived_from` /
//!    `refines` arrays and exclude stdlib requirement elements (the legacy
//!    `requirements_trace` projection was deleted under debt-ledger L59).
//!
//! The workspace opens with the bundled standard library because both
//! discriminators resolve against library anchors (Derivation def /
//! Refinement metadata def) — never name strings.

use std::path::PathBuf;

use serde_json::json;
use sysml_project::discovery::OpenTarget;
use sysml_service::{execute_command, SysmlService};

const FIXTURE: &str = r#"package B1TraceFixture {
	private import DerivationConnections::*;
	private import ModelingMetadata::*;

	requirement def <'REQ-001'> SystemReq;
	requirement def <'REQ-002'> SubsystemReq;

	requirement systemReq : SystemReq;
	requirement subsystemReq : SubsystemReq;

	part amplifier;

	// Derive: explicit typing by the library Derivation connection def.
	// End order is semantic (KerML implicit end redefinition): first end =
	// originalRequirement[1], rest = derivedRequirements[1..*].
	connection sysDerivation : Derivation {
		end ref origEnd references systemReq;
		end ref derivEnd references subsystemReq;
	}

	// Refine: dependency annotated with the library Refinement metadata.
	dependency refinesDep from subsystemReq to systemReq {
		@Refinement;
	}

	// Trace: plain dependency.
	dependency traceDep from amplifier to systemReq;
}
"#;

fn fixture_workspace() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "sysml-b1-trace-fixture-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(dir.join("B1TraceFixture.sysml"), FIXTURE).expect("write fixture");
    dir
}

fn matrix_rows(
    service: &SysmlService,
    source_kind: &str,
    rel_kind: &str,
    target_kind: &str,
) -> Vec<serde_json::Value> {
    let result = execute_command(
        service,
        "sysml.trace_matrix",
        json!({
            "uri": "__workspace__",
            "source_kind": source_kind,
            "rel_kind": rel_kind,
            "target_kind": target_kind,
        }),
    )
    .unwrap_or_else(|e| panic!("trace_matrix({rel_kind}) must succeed: {e:?}"));
    result
        .as_array()
        .unwrap_or_else(|| panic!("trace_matrix({rel_kind}) must return an array"))
        .clone()
}

fn row_names(rows: &[serde_json::Value]) -> Vec<(String, String)> {
    rows.iter()
        .map(|r| {
            (
                r["source_name"].as_str().unwrap_or("").to_owned(),
                r["target_name"].as_str().unwrap_or("").to_owned(),
            )
        })
        .collect()
}

#[test]
fn b1_derive_refine_trace_end_to_end() {
    let service = SysmlService::empty();
    service
        .open_context(OpenTarget::Folder(fixture_workspace()))
        .expect("open B1 fixture workspace");

    // ── Derive: subsystemReq (derived) → systemReq (original) ──
    let derive = matrix_rows(&service, "RequirementUsage", "derive", "RequirementUsage");
    assert_eq!(
        row_names(&derive),
        vec![("subsystemReq".to_owned(), "systemReq".to_owned())],
        "Derive: source = derived requirement, target = original"
    );

    // ── Refine: subsystemReq (client/refining) → systemReq (supplier) ──
    let refine = matrix_rows(&service, "RequirementUsage", "refine", "RequirementUsage");
    assert_eq!(
        row_names(&refine),
        vec![("subsystemReq".to_owned(), "systemReq".to_owned())],
        "Refine: source = client (refining), target = supplier (refined)"
    );

    // ── Trace: amplifier (client) → systemReq (supplier) ──
    let trace = matrix_rows(&service, "PartUsage", "trace", "RequirementUsage");
    assert_eq!(
        row_names(&trace),
        vec![("amplifier".to_owned(), "systemReq".to_owned())],
        "Trace: source = client, target = supplier"
    );
    // The Refine-classified dependency must NOT double-count as Trace.
    let req_trace = matrix_rows(&service, "RequirementUsage", "trace", "RequirementUsage");
    assert!(
        req_trace.is_empty(),
        "a Refine-classified dependency must not also mint Trace, got: {req_trace:?}"
    );

    // ── requirement_rows carry derived_from / refines ──
    // (was pinned through `sysml.workspace.requirements_trace`; that legacy
    // projection was deleted under debt-ledger L59 — `requirement_rows` is
    // the one requirement-row surface. This fixture's unique value here is
    // the explicit `Derivation` CONNECTION form reaching the rows wire.)
    let result = execute_command(
        &service,
        "sysml.workspace.requirement_rows",
        json!({ "spec": {} }),
    )
    .expect("requirement_rows must succeed");
    let rows = result["rows"].as_array().expect("rows array");

    // Stdlib requirement elements must not appear as rows.
    assert!(
        rows.iter().all(|r| {
            let n = r["name"].as_str().unwrap_or("");
            n != "originalRequirements"
                && n != "derivedRequirements"
                && n != "RequirementCheck"
        }),
        "library requirement elements leaked into requirement rows"
    );

    let sub = rows
        .iter()
        .find(|r| r["name"] == "subsystemReq")
        .expect("subsystemReq row");
    let derived_from: Vec<&str> = sub["derived_from"]
        .as_array()
        .expect("derived_from array")
        .iter()
        .filter_map(|v| v["name"].as_str())
        .collect();
    assert_eq!(
        derived_from,
        vec!["systemReq"],
        "subsystemReq derives from systemReq"
    );
    let refines: Vec<&str> = sub["refines"]
        .as_array()
        .expect("refines array")
        .iter()
        .filter_map(|v| v["name"].as_str())
        .collect();
    assert_eq!(refines, vec!["systemReq"], "subsystemReq refines systemReq");
}
