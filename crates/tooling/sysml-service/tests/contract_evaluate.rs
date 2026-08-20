//! JSON schema round-trip contract tests for the `sysml.evaluate.*` namespace.
//!
//! Commands covered:
//! - sysml.evaluate.constraints
//! - sysml.evaluate.verification_cases
//! - sysml.evaluate.analysis_cases
//! - sysml.evaluate.calculations
//!
//! The evaluate commands return ad-hoc `serde_json::Value` arrays. The
//! contract tests assert on the field names present in each array element
//! rather than deserializing into a typed struct. If a field is removed or
//! renamed, these tests break.

use std::path::Path;

use serde_json::json;
use sysml_core::ElementKind;
use sysml_service::{execute_command, SysmlService};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

/// Find the first loaded element with the given name and kind.
fn find_element_id(service: &SysmlService, uri: &str, name: &str, kind: ElementKind) -> String {
    let graph = service.require_graph(uri).expect("loaded graph");
    graph
        .elements
        .values()
        .find(|element| element.name.as_deref() == Some(name) && element.kind == kind)
        .map(|element| element.id.to_string())
        .expect("named element")
}

/// Load the ValveGating example (contains constraints via the state machine).
fn load_valve_model(service: &SysmlService) -> String {
    let valve_path = repo_root().join("examples/valve-gating/ValveGating.sysml");
    service.load_file(&valve_path).unwrap();
    service
        .loaded_uris()
        .into_iter()
        .find(|u| u.contains("ValveGating"))
        .expect("ValveGating URI")
}


// ---------------------------------------------------------------------------
// evaluate.expression
// ---------------------------------------------------------------------------

#[test]
fn contract_evaluate_expression() {
    let service = SysmlService::empty();
    service
        .load_source("test.sysml", "package P { attribute x = 42; }")
        .unwrap();
    let element_id = find_element_id(&service, "test.sysml", "x", ElementKind::AttributeUsage);

    let result = execute_command(
        &service,
        "sysml.evaluate.expression",
        json!({ "uri": "test.sysml", "element_id": element_id, "overrides": [] }),
    )
    .unwrap();

    // Returns {element_id, element_name, source, value, display, value_type,
    //          verdict, symbols, context, diagnostics}.
    let obj = result
        .as_object()
        .expect("evaluate.expression returns object");
    assert!(obj.contains_key("element_id"), "missing element_id field");
    assert!(obj.contains_key("element_name"), "missing element_name field");
    assert!(obj.contains_key("source"), "missing source field");
    assert!(obj.contains_key("value"), "missing value field");
    assert!(obj.contains_key("display"), "missing display field");
    assert!(obj.contains_key("value_type"), "missing value_type field");
    assert!(obj.contains_key("verdict"), "missing verdict field");
    assert!(obj.contains_key("symbols"), "missing symbols field");
    assert!(obj.contains_key("context"), "missing context field");
    assert!(obj.contains_key("diagnostics"), "missing diagnostics field");

    let json_str = serde_json::to_string(&result).expect("serialize");
    let _: serde_json::Value = serde_json::from_str(&json_str).expect("deserialize");
}

// ---------------------------------------------------------------------------
// evaluate.constraints
// ---------------------------------------------------------------------------

#[test]
fn contract_evaluate_constraints() {
    let service = SysmlService::empty();
    let uri = load_valve_model(&service);

    let result = execute_command(
        &service,
        "sysml.evaluate.constraints",
        json!({ "uri": uri }),
    )
    .unwrap();

    // Returns Vec<{element_id, satisfied, detail, ast, verdict, error,
    //              name, kind, expr}>.
    // The `error` field is the eval-result-loss collapse (P2,
    // C-soft-fallback-eval-result-loss): structured EvalError display string
    // on the failure path, `null` on success. See
    // `contract_eval_result_loss.rs` for the per-row semantic invariants;
    // this contract only pins the schema.
    // B8: name/kind/expr enrichment lives on the service row so transports
    // (LSP/MCP/CLI) don't re-walk the graph.
    let arr = result.as_array().expect("evaluate.constraints returns array");
    // Model may or may not have constraints; assert schema if non-empty.
    for item in arr {
        let obj = item.as_object().expect("constraint result is object");
        assert!(
            obj.contains_key("element_id"),
            "missing element_id field"
        );
        assert!(
            obj.contains_key("satisfied"),
            "missing satisfied field"
        );
        assert!(obj.contains_key("detail"), "missing detail field");
        assert!(obj.contains_key("ast"), "missing ast field");
        assert!(obj.contains_key("verdict"), "missing verdict field");
        assert!(obj.contains_key("error"), "missing error field");
        assert!(obj.contains_key("name"), "missing name field (B8)");
        assert!(obj.contains_key("kind"), "missing kind field (B8)");
        assert!(obj.contains_key("expr"), "missing expr field (B8)");
        // name/kind must always be strings (never null) — fallback to
        // "<unnamed>" / "ConstraintUsage" on the service side.
        assert!(
            obj.get("name").and_then(|v| v.as_str()).is_some(),
            "name must be a string"
        );
        assert!(
            obj.get("kind").and_then(|v| v.as_str()).is_some(),
            "kind must be a string"
        );
        // expr is Option<String> — String or null.
        let expr = obj.get("expr").unwrap();
        assert!(
            expr.is_string() || expr.is_null(),
            "expr must be string or null, got {:?}",
            expr
        );
    }

    // Verify the JSON round-trips through serde_json.
    let json_str = serde_json::to_string(&result).expect("serialize");
    let _: serde_json::Value = serde_json::from_str(&json_str).expect("deserialize");
}

// ---------------------------------------------------------------------------
// evaluate.constraints — error round-trip (eval-result-loss closeout)
// ---------------------------------------------------------------------------

/// Closeout test for C-soft-fallback-eval-result-loss (P0=6b9f31ee, P1=307cff31,
/// P2=e3f8bfe7, P3=0d57e951). Complements the `#[ignore]`-gated baselines in
/// `contract_eval_result_loss.rs` by pinning the **canonical** evaluate-suite
/// contract: when a constraint references an undefined symbol, the resulting
/// row carries
///
///   - `error: "<non-empty EvalError display>"`        (structured)
///   - `verdict.actual: null`                          (no value computed)
///   - `verdict.verdict: "Error"` / `display: "ERR"`   (verdict band)
///
/// and the whole shape round-trips cleanly through serde_json. This lives in
/// `contract_evaluate.rs` (not `contract_eval_result_loss.rs`) so the
/// canonical evaluate-namespace contract suite includes the failure axis.
#[test]
fn contract_evaluate_constraints_error_round_trip() {
    let service = SysmlService::empty();
    let src = r#"
        package P {
            constraint c { undefined_variable_xyz > 0 }
        }
    "#;
    service
        .load_source("contract_evaluate_error.sysml", src)
        .unwrap();

    let result = execute_command(
        &service,
        "sysml.evaluate.constraints",
        json!({ "uri": "contract_evaluate_error.sysml" }),
    )
    .unwrap();

    let arr = result
        .as_array()
        .expect("evaluate.constraints returns array");
    assert!(
        !arr.is_empty(),
        "model declares a constraint -- expected at least one result even on eval failure"
    );

    let mut saw_failure = false;
    for item in arr {
        let obj = item.as_object().expect("constraint result is object");

        // Schema contract from contract_evaluate_constraints (above) — re-checked
        // here so this test is self-contained.
        assert!(obj.contains_key("element_id"), "missing element_id field");
        assert!(obj.contains_key("error"), "missing error field");
        assert!(obj.contains_key("verdict"), "missing verdict field");

        if let Some(serde_json::Value::String(msg)) = obj.get("error") {
            saw_failure = true;
            assert!(
                !msg.trim().is_empty(),
                "error message must be non-empty on the failure path"
            );

            // Failure axis: error string set ⇒ verdict.actual must be null
            // (the legacy bug stuffed the error message into actual; the
            // collapse made the two axes orthogonal).
            let verdict = obj
                .get("verdict")
                .and_then(|v| v.as_object())
                .expect("verdict object present");
            let actual = verdict.get("actual");
            assert!(
                matches!(actual, None | Some(serde_json::Value::Null)),
                "failure path must leave verdict.actual null (error lives in `error`), got {:?}",
                actual
            );
        }
    }

    assert!(
        saw_failure,
        "constraint referencing an undefined symbol must surface a non-null `error`"
    );

    // Whole-array JSON round-trip remains stable.
    let json_str = serde_json::to_string(&result).expect("serialize");
    let _: serde_json::Value = serde_json::from_str(&json_str).expect("deserialize");
}

// ---------------------------------------------------------------------------
// evaluate.verification_cases
// ---------------------------------------------------------------------------

#[test]
fn contract_evaluate_verification_cases() {
    let service = SysmlService::empty();
    let uri = load_valve_model(&service);

    let result = execute_command(
        &service,
        "sysml.evaluate.verification_cases",
        json!({ "uri": uri }),
    )
    .unwrap();

    // Returns Vec<{element_id, case_id, case_name, subject, methods, verdict,
    //              total_requirements, passed_requirements, display,
    //              requirements, diagnostics}>. `methods` (B4) = the case's
    //              DECLARED @VerificationMethod kinds, [1..*] by spec —
    //              plural array, replacing the never-populated singular
    //              `method`.
    let arr = result
        .as_array()
        .expect("evaluate.verification_cases returns array");
    for item in arr {
        let obj = item.as_object().expect("verification case result is object");
        assert!(
            obj.contains_key("element_id"),
            "missing element_id field"
        );
        assert!(obj.contains_key("case_id"), "missing case_id field");
        assert!(
            obj.contains_key("case_name"),
            "missing case_name field"
        );
        assert!(obj.contains_key("subject"), "missing subject field");
        assert!(obj.contains_key("methods"), "missing methods field");
        assert!(
            obj["methods"].is_array(),
            "methods is the declared-kind array (spec [1..*])"
        );
        assert!(obj.contains_key("verdict"), "missing verdict field");
        assert!(
            obj.contains_key("total_requirements"),
            "missing total_requirements field"
        );
        assert!(
            obj.contains_key("passed_requirements"),
            "missing passed_requirements field"
        );
        assert!(obj.contains_key("display"), "missing display field");
        assert!(
            obj.contains_key("requirements"),
            "missing requirements field"
        );
        assert!(
            obj.contains_key("diagnostics"),
            "missing diagnostics field"
        );
    }

    let json_str = serde_json::to_string(&result).expect("serialize");
    let _: serde_json::Value = serde_json::from_str(&json_str).expect("deserialize");
}

/// Library base features must never surface as case rows. The stdlib's
/// abstract vocabulary (`VerificationCase`, `verificationCases`, `self`,
/// `subVerificationCases`) satisfies `is_verification_case_kind` and was
/// leaking into the workspace-scoped read as perpetual INCONCLUSIVE noise.
#[test]
fn contract_evaluate_verification_cases_excludes_library_rows() {
    let dir = std::env::temp_dir().join(format!(
        "sysml-evaluate-stdlib-filter-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(
        dir.join("StdlibFilterFixture.sysml"),
        r#"
        package StdlibFilterFixture {
            part def Bench;
            requirement def ReqDef { attribute m : ScalarValues::Real; require constraint { m > 0.0 } }
            requirement req : ReqDef { attribute m = 1.0; }
            verification def OnlyUserCase {
                subject bench : Bench;
                objective { verify req; }
            }
        }
        "#,
    )
    .expect("write fixture");

    let service = SysmlService::empty();
    service
        .open_context(sysml_project::discovery::OpenTarget::Folder(dir))
        .expect("open fixture workspace");

    let result = execute_command(&service, "sysml.evaluate.verification_cases", json!({}))
        .expect("workspace-scoped evaluate.verification_cases");
    let names: Vec<&str> = result
        .as_array()
        .expect("array")
        .iter()
        .filter_map(|r| r["case_name"].as_str())
        .collect();

    assert!(
        names.contains(&"OnlyUserCase"),
        "the user's case must be present, got {names:?}"
    );
    for library_name in ["VerificationCase", "verificationCases", "subVerificationCases", "self"] {
        assert!(
            !names.contains(&library_name),
            "stdlib base feature '{library_name}' leaked into the rows: {names:?}"
        );
    }
}

/// ONE digest identity space (live-caught 2026-07-19): `workspace.verify`'s
/// `model_digest` must equal the digest `record_external` compares staleness
/// against (`current_digest`, the B6 capture path) — otherwise a fresh
/// external ingest whose client read the frame chip's digest labels stale.
#[test]
fn workspace_verify_digest_matches_external_staleness_space() {
    let dir = std::env::temp_dir().join(format!(
        "sysml-digest-space-{}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    std::fs::write(
        dir.join("DigestSpaceFixture.sysml"),
        r#"
        package DigestSpaceFixture {
            part def Bench;
            requirement def ReqDef { attribute m : ScalarValues::Real; require constraint { m > 0.0 } }
            requirement req : ReqDef { attribute m = 1.0; }
            verification def OnlyUserCase {
                subject bench : Bench;
                objective { verify req; }
            }
        }
        "#,
    )
    .expect("write fixture");

    let service = SysmlService::empty();
    service
        .open_context(sysml_project::discovery::OpenTarget::Folder(dir))
        .expect("open fixture workspace");

    let verify = execute_command(&service, "sysml.workspace.verify", json!({}))
        .expect("workspace.verify");
    let frame_digest = verify["model_digest"]
        .as_str()
        .expect("model_digest present")
        .to_owned();
    assert!(!frame_digest.is_empty());

    let recorded = execute_command(
        &service,
        "sysml.verify.record_external",
        json!({
            "tool": "digest-space-probe",
            "declared_digest": frame_digest,
            "verdicts": [{ "case_id": "OnlyUserCase", "verdict": "pass" }]
        }),
    )
    .expect("record_external");

    assert_eq!(
        recorded["current_digest"].as_str(),
        Some(frame_digest.as_str()),
        "workspace.verify's model_digest and record_external's current_digest \
         must live in ONE identity space"
    );
    assert_eq!(
        recorded["matches_current_model"].as_bool(),
        Some(true),
        "an ingest declaring the frame chip's digest must read FRESH, never stale"
    );
}

// ---------------------------------------------------------------------------
// evaluate.analysis_cases
// ---------------------------------------------------------------------------

#[test]
fn contract_evaluate_analysis_cases() {
    let service = SysmlService::empty();
    let uri = load_valve_model(&service);

    let result = execute_command(
        &service,
        "sysml.evaluate.analysis_cases",
        json!({ "uri": uri }),
    )
    .unwrap();

    // Returns Vec<{element_id, case_name, display, subject, objective,
    //              tool_name, tool_uri, parameters, constraints,
    //              result_expression, diagnostics}>.
    let arr = result
        .as_array()
        .expect("evaluate.analysis_cases returns array");
    for item in arr {
        let obj = item.as_object().expect("analysis case result is object");
        assert!(
            obj.contains_key("element_id"),
            "missing element_id field"
        );
        assert!(
            obj.contains_key("case_name"),
            "missing case_name field"
        );
        assert!(obj.contains_key("display"), "missing display field");
        assert!(obj.contains_key("subject"), "missing subject field");
        assert!(obj.contains_key("objective"), "missing objective field");
        assert!(
            obj.contains_key("tool_name"),
            "missing tool_name field"
        );
        assert!(obj.contains_key("tool_uri"), "missing tool_uri field");
        assert!(
            obj.contains_key("parameters"),
            "missing parameters field"
        );
        assert!(
            obj.contains_key("constraints"),
            "missing constraints field"
        );
        assert!(
            obj.contains_key("result_expression"),
            "missing result_expression field"
        );
        assert!(
            obj.contains_key("diagnostics"),
            "missing diagnostics field"
        );
    }

    let json_str = serde_json::to_string(&result).expect("serialize");
    let _: serde_json::Value = serde_json::from_str(&json_str).expect("deserialize");
}

// ---------------------------------------------------------------------------
// evaluate.calculations
// ---------------------------------------------------------------------------

#[test]
fn contract_evaluate_calculations() {
    let service = SysmlService::empty();
    let uri = load_valve_model(&service);

    let result = execute_command(
        &service,
        "sysml.evaluate.calculations",
        json!({ "uri": uri }),
    )
    .unwrap();

    // Returns Vec<{element_id, display, ast, name, kind, expr}>.
    // B8: name/kind/expr enrichment supplied server-side so transports don't
    // re-walk the graph.
    let arr = result
        .as_array()
        .expect("evaluate.calculations returns array");
    for item in arr {
        let obj = item.as_object().expect("calculation result is object");
        assert!(
            obj.contains_key("element_id"),
            "missing element_id field"
        );
        assert!(obj.contains_key("display"), "missing display field");
        assert!(obj.contains_key("ast"), "missing ast field");
        assert!(obj.contains_key("name"), "missing name field (B8)");
        assert!(obj.contains_key("kind"), "missing kind field (B8)");
        assert!(obj.contains_key("expr"), "missing expr field (B8)");
        assert!(
            obj.get("name").and_then(|v| v.as_str()).is_some(),
            "name must be a string"
        );
        assert!(
            obj.get("kind").and_then(|v| v.as_str()).is_some(),
            "kind must be a string"
        );
        let expr = obj.get("expr").unwrap();
        assert!(
            expr.is_string() || expr.is_null(),
            "expr must be string or null, got {:?}",
            expr
        );
    }

    let json_str = serde_json::to_string(&result).expect("serialize");
    let _: serde_json::Value = serde_json::from_str(&json_str).expect("deserialize");
}
