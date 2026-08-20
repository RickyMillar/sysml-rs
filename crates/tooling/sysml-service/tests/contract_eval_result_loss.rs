//! P0 baseline contract tests for the eval-result-loss collapse.
//!
//! Audit cluster: **C-soft-fallback-eval-result-loss**.
//!
//! Today, when constraint evaluation fails (e.g. the expression references a
//! symbol that only exists inside a running session, or compilation produced
//! an `EvaluationError`), the failure is lossy: the evaluator's error string
//! is stuffed into `verdict.actual` as a `Value::String(...)`. There is no
//! structured `error` field on the per-constraint JSON. Consumers can't tell
//! "ran fine, value is the string 'foo'" from "eval blew up with reason 'foo'".
//!
//! The collapse (P1+P2) wires a real `error: Option<String>` field through
//! `ConstraintMeasurement` / the `sysml.evaluate.constraints` JSON, so the
//! success and failure axes are orthogonal:
//!
//!   - success: `actual = <Value>`, `error = null`
//!   - failure: `actual = null`,    `error = "<EvalError display>"`
//!
//! Tests in this file pin the **target** (post-collapse) shape and stay
//! `#[ignore]`-gated until P2 lands so the file compiles today without the
//! new field existing on the response.

use std::path::Path;

use serde_json::json;
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

/// Load the ValveGating example which carries constraint usages.
fn load_valve_model(service: &SysmlService) -> String {
    let valve_path = repo_root().join("examples/valve-gating/ValveGating.sysml");
    service.load_file(&valve_path).unwrap();
    service
        .loaded_uris()
        .into_iter()
        .find(|u| u.contains("ValveGating"))
        .expect("ValveGating URI")
}

/// Pluck the per-constraint JSON map.
fn as_obj(item: &serde_json::Value) -> &serde_json::Map<String, serde_json::Value> {
    item.as_object().expect("constraint result is object")
}

/// Read the `error` field via the JSON Value path so this test does not
/// reference a not-yet-existing Rust struct field. Returns:
///   - `None` if the key is absent or `null`
///   - `Some(s)` if the key is a non-null string
fn read_error_field(obj: &serde_json::Map<String, serde_json::Value>) -> Option<String> {
    match obj.get("error") {
        None => None,
        Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(other) => panic!(
            "P0 contract: `error` must be null or string, got {:?}",
            other
        ),
    }
}

// ---------------------------------------------------------------------------
// Test 1: clean-evaluation path — `actual` populated, `error` null/absent.
// ---------------------------------------------------------------------------

#[test]
fn contract_eval_result_loss_success_path_has_no_error() {
    let service = SysmlService::empty();
    // A model with a constraint that evaluates cleanly to a Bool.
    // `x = 10`, constraint `c { x > 0 }` → satisfied = true, actual = true.
    let src = r#"
        package P {
            attribute x : Real = 10.0;
            constraint c { x > 0.0 }
        }
    "#;
    service.load_source("eval_loss_success.sysml", src).unwrap();

    let result = execute_command(
        &service,
        "sysml.evaluate.constraints",
        json!({ "uri": "eval_loss_success.sysml" }),
    )
    .unwrap();

    let arr = result
        .as_array()
        .expect("evaluate.constraints returns array");
    assert!(
        !arr.is_empty(),
        "model declares a constraint -- expected at least one result"
    );

    for item in arr {
        let obj = as_obj(item);

        // Verdict object carries `actual` after R1.3.
        let verdict = obj
            .get("verdict")
            .and_then(|v| v.as_object())
            .expect("verdict object present");
        let actual = verdict.get("actual");
        assert!(
            matches!(actual, Some(v) if !v.is_null()),
            "success path must populate verdict.actual, got {:?}",
            actual
        );

        // P2 target: top-level `error` is either absent or null on success.
        let err = read_error_field(obj);
        assert!(
            err.is_none(),
            "success path must NOT carry an error string, got {:?}",
            err
        );
    }
}

// ---------------------------------------------------------------------------
// Test 2: failure path — `actual` null, `error` populated with EvalError msg.
// ---------------------------------------------------------------------------

#[test]
fn contract_eval_result_loss_failure_surfaces_error() {
    let service = SysmlService::empty();
    // A constraint that references an undefined symbol -- the expression
    // evaluator returns `EvaluationError::UndefinedSymbol(...)` (or similar),
    // which today is silently coerced into `actual: "ERROR..."`.
    //
    // Target post-P2 shape:
    //   - `verdict.actual` is null (no real value computed)
    //   - top-level `error` is Some(non-empty string)
    let src = r#"
        package P {
            constraint c { undefined_variable_xyz > 0 }
        }
    "#;
    service.load_source("eval_loss_failure.sysml", src).unwrap();

    let result = execute_command(
        &service,
        "sysml.evaluate.constraints",
        json!({ "uri": "eval_loss_failure.sysml" }),
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
        let obj = as_obj(item);
        let err = read_error_field(obj);
        if let Some(msg) = err {
            saw_failure = true;
            assert!(
                !msg.trim().is_empty(),
                "P2 target: error message must be non-empty"
            );

            // On the failure path, `verdict.actual` must NOT carry the
            // error string masquerading as a value -- it should be null,
            // and the message lives in the structured `error` field.
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
        "P2 target: at least one constraint with an unresolved symbol must surface a non-null `error`"
    );
}

// ---------------------------------------------------------------------------
// Test 3: ValveGating real-world fixture -- shape invariant holds across mixed
// success/failure rows on a realistic model.
// ---------------------------------------------------------------------------

#[test]
fn contract_eval_result_loss_valve_fixture_shape_invariant() {
    let service = SysmlService::empty();
    let uri = load_valve_model(&service);

    let result = execute_command(
        &service,
        "sysml.evaluate.constraints",
        json!({ "uri": uri }),
    )
    .unwrap();

    let arr = result
        .as_array()
        .expect("evaluate.constraints returns array");

    for item in arr {
        let obj = as_obj(item);
        let err = read_error_field(obj);
        let verdict = obj
            .get("verdict")
            .and_then(|v| v.as_object())
            .expect("verdict object present");
        let actual = verdict.get("actual");

        // Shape invariant (post-P2): success and failure are orthogonal axes.
        // If `error` is Some, `actual` must be null. If `error` is None,
        // `actual` may be anything (including null for inconclusive rows).
        if err.is_some() {
            assert!(
                matches!(actual, None | Some(serde_json::Value::Null)),
                "valve fixture: row with `error` set must leave verdict.actual null; got actual={:?}",
                actual
            );
        }
    }
}
