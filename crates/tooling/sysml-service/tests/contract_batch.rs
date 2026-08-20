//! End-to-end JSON contract tests for the `sysml.batch.*` command family
//! (R5.0).
//!
//! Every test drives the service through the type-erased
//! [`execute_command`] path — the same code path the MCP and REST
//! transports use — and asserts on the externally-visible JSON shapes.
//! Any change in field name, enum representation, or status transition
//! breaks the tests — that is the point.

use serde_json::{json, Value};
use sysml_service::{execute_command, SysmlService};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A minimal model with a trivial state machine and a handful of
/// numeric parameters that batch overrides can target. Kept in one
/// place so all tests share the same fixture shape.
const BENCH_SOURCE: &str = r#"
    package Bench {
        attribute mass = 1.0;
        attribute tolerance = 0.1;
        state def BenchSM {
            entry; then S1;
            state S1;
        }
    }
"#;

/// Build a fresh service with the Bench model loaded. Returns the
/// URI key used to address the model.
fn service_with_bench() -> (SysmlService, String) {
    let service = SysmlService::empty();
    service
        .load_source("bench.sysml", BENCH_SOURCE)
        .unwrap();
    (service, "bench.sysml".to_owned())
}

/// Create a batch of N children parameterised over `mass ∈ 1.0..=N`.
/// Returns the full JSON response from `sysml.batch.create`.
fn create_mass_sweep(service: &SysmlService, uri: &str, n: usize) -> Value {
    let params: Vec<Value> = (1..=n)
        .map(|i| json!({ "mass": i as f64 }))
        .collect();
    let params_str = serde_json::to_string(&params).unwrap();
    execute_command(
        service,
        "sysml.batch.create",
        json!({
            "kind": "sweep",
            "uri": uri,
            "subsystem_name": "BenchSM",
            "children_params": params_str,
            "label": "mass sweep",
        }),
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// Core contract: 3-child batch full lifecycle
// ---------------------------------------------------------------------------

#[test]
fn contract_batch_three_child_lifecycle_with_archive() {
    let (service, uri) = service_with_bench();

    // -- create --
    let created = create_mass_sweep(&service, &uri, 3);
    let batch_id = created["batch_id"].as_str().unwrap().to_owned();
    let child_ids: Vec<String> = created["child_session_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();
    assert_eq!(child_ids.len(), 3);
    assert!(!batch_id.is_empty());

    // -- status: Pending (no child has stopped yet) --
    let status = execute_command(
        &service,
        "sysml.batch.status",
        json!({ "batch_id": batch_id }),
    )
    .unwrap();
    assert_eq!(status["batch"]["kind"], "sweep");
    assert_eq!(status["batch"]["label"], "mass sweep");
    assert_eq!(status["batch"]["children"].as_array().unwrap().len(), 3);
    assert_eq!(status["batch"]["status"]["status"], "pending");
    for (idx, child) in status["batch"]["children"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        assert_eq!(child["index"], idx as u64);
        assert_eq!(child["status"]["status"], "pending");
        assert_eq!(child["params"]["mass"], (idx as f64) + 1.0);
    }

    // -- stop child 0 --
    execute_command(
        &service,
        "sysml.sessions.stop",
        json!({ "session_id": child_ids[0] }),
    )
    .unwrap();
    let status = execute_command(
        &service,
        "sysml.batch.status",
        json!({ "batch_id": batch_id }),
    )
    .unwrap();
    assert_eq!(status["batch"]["status"]["status"], "running");
    assert_eq!(status["batch"]["status"]["running"], 0);
    assert_eq!(status["batch"]["status"]["completed"], 1);
    // Child 0 flipped to Complete; 1 and 2 remain Pending.
    let children = status["batch"]["children"].as_array().unwrap();
    assert_eq!(children[0]["status"]["status"], "complete");
    assert_eq!(children[1]["status"]["status"], "pending");
    assert_eq!(children[2]["status"]["status"], "pending");

    // -- stop remaining children → batch Complete --
    for sid in &child_ids[1..] {
        execute_command(
            &service,
            "sysml.sessions.stop",
            json!({ "session_id": sid }),
        )
        .unwrap();
    }
    let status = execute_command(
        &service,
        "sysml.batch.status",
        json!({ "batch_id": batch_id }),
    )
    .unwrap();
    assert_eq!(status["batch"]["status"]["status"], "complete");

    // -- archive: every child stored under origin=sweep --
    let archive_list = execute_command(
        &service,
        "sysml.sessions.archive.list",
        json!({ "origin": "sweep" }),
    )
    .unwrap();
    let archive_entries = archive_list["entries"].as_array().unwrap();
    assert_eq!(archive_entries.len(), 3);
    for entry in archive_entries {
        assert_eq!(entry["origin"], "sweep");
    }
}

// ---------------------------------------------------------------------------
// Result projection — include_verdicts flag
// ---------------------------------------------------------------------------

#[test]
fn contract_batch_results_include_verdicts_toggle() {
    let (service, uri) = service_with_bench();
    let created = create_mass_sweep(&service, &uri, 2);
    let batch_id = created["batch_id"].as_str().unwrap().to_owned();

    // include_verdicts omitted (defaults to false in JSON dispatch)
    let bare = execute_command(
        &service,
        "sysml.batch.results",
        json!({ "batch_id": batch_id, "include_verdicts": false }),
    )
    .unwrap();
    let bare_children = bare["children"].as_array().unwrap();
    assert_eq!(bare_children.len(), 2);
    for child in bare_children {
        // With skip_serializing_if on the Vec, missing == empty.
        assert!(child.get("verdicts").is_none() || child["verdicts"].as_array().unwrap().is_empty());
    }

    // include_verdicts = true → still empty (no verdicts were recorded)
    // but the field shape is stable when present.
    let full = execute_command(
        &service,
        "sysml.batch.results",
        json!({ "batch_id": batch_id, "include_verdicts": true }),
    )
    .unwrap();
    assert_eq!(full["children"].as_array().unwrap().len(), 2);
}

// ---------------------------------------------------------------------------
// Slice filters
// ---------------------------------------------------------------------------

#[test]
fn contract_batch_slice_filters_combine_with_and() {
    let (service, uri) = service_with_bench();
    let created = create_mass_sweep(&service, &uri, 3);
    let batch_id = created["batch_id"].as_str().unwrap().to_owned();
    let child_ids: Vec<String> = created["child_session_ids"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect();

    // Empty filter returns all 3 children.
    let all = execute_command(
        &service,
        "sysml.batch.slice",
        json!({ "batch_id": batch_id, "filter": {} }),
    )
    .unwrap();
    assert_eq!(all["children"].as_array().unwrap().len(), 3);

    // Stop child 1 (index 1, mass = 2.0).
    execute_command(
        &service,
        "sysml.sessions.stop",
        json!({ "session_id": child_ids[1] }),
    )
    .unwrap();

    // only_status = complete → 1 child
    let completes = execute_command(
        &service,
        "sysml.batch.slice",
        json!({
            "batch_id": batch_id,
            "filter": { "only_status": "complete" },
        }),
    )
    .unwrap();
    let c = completes["children"].as_array().unwrap();
    assert_eq!(c.len(), 1);
    assert_eq!(c[0]["index"], 1);

    // param_predicate mass >= 2.0 → 2 children (idx 1, 2)
    let heavy = execute_command(
        &service,
        "sysml.batch.slice",
        json!({
            "batch_id": batch_id,
            "filter": {
                "param_predicate": { "param": "mass", "op": "ge", "value": 2.0 },
            },
        }),
    )
    .unwrap();
    assert_eq!(heavy["children"].as_array().unwrap().len(), 2);

    // AND: complete AND mass >= 2.0 → only child 1
    let combo = execute_command(
        &service,
        "sysml.batch.slice",
        json!({
            "batch_id": batch_id,
            "filter": {
                "only_status": "complete",
                "param_predicate": { "param": "mass", "op": "ge", "value": 2.0 },
            },
        }),
    )
    .unwrap();
    let combo_c = combo["children"].as_array().unwrap();
    assert_eq!(combo_c.len(), 1);
    assert_eq!(combo_c[0]["index"], 1);
}

// ---------------------------------------------------------------------------
// Validation / error paths
// ---------------------------------------------------------------------------

#[test]
fn contract_batch_create_unknown_kind_is_invalid_input() {
    let (service, uri) = service_with_bench();
    let err = execute_command(
        &service,
        "sysml.batch.create",
        json!({
            "kind": "bogus",
            "uri": uri,
            "subsystem_name": "BenchSM",
            "children_params": "[]",
        }),
    )
    .unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("unknown batch kind"), "{rendered}");
}

#[test]
fn contract_batch_create_non_array_children_params_is_invalid_input() {
    let (service, uri) = service_with_bench();
    let err = execute_command(
        &service,
        "sysml.batch.create",
        json!({
            "kind": "sweep",
            "uri": uri,
            "subsystem_name": "BenchSM",
            "children_params": "not a json array",
        }),
    )
    .unwrap_err();
    let rendered = format!("{err}");
    assert!(
        rendered.contains("children_params must be a JSON array"),
        "{rendered}"
    );
}

#[test]
fn contract_batch_status_unknown_id_is_element_not_found() {
    let service = SysmlService::empty();
    let err = execute_command(
        &service,
        "sysml.batch.status",
        json!({ "batch_id": "no-such-batch" }),
    )
    .unwrap_err();
    let rendered = format!("{err}");
    assert!(rendered.contains("no batch"), "{rendered}");
}

#[test]
fn contract_batch_monte_carlo_archives_with_origin_monte_carlo() {
    let (service, uri) = service_with_bench();
    let created = execute_command(
        &service,
        "sysml.batch.create",
        json!({
            "kind": "monte_carlo",
            "uri": uri,
            "subsystem_name": "BenchSM",
            "children_params": "[{\"mass\": 1.5}]",
        }),
    )
    .unwrap();
    let child_id = created["child_session_ids"][0].as_str().unwrap().to_owned();
    execute_command(
        &service,
        "sysml.sessions.stop",
        json!({ "session_id": child_id }),
    )
    .unwrap();

    let got = execute_command(
        &service,
        "sysml.sessions.archive.get",
        json!({ "id": child_id }),
    )
    .unwrap();
    assert_eq!(got["entry"]["origin"], "monte_carlo");
}

#[test]
fn contract_batch_trade_study_archives_with_origin_trade_study() {
    let (service, uri) = service_with_bench();
    let created = execute_command(
        &service,
        "sysml.batch.create",
        json!({
            "kind": "trade_study",
            "uri": uri,
            "subsystem_name": "BenchSM",
            "children_params": "[{\"tolerance\": 0.2}]",
        }),
    )
    .unwrap();
    let child_id = created["child_session_ids"][0].as_str().unwrap().to_owned();
    execute_command(
        &service,
        "sysml.sessions.stop",
        json!({ "session_id": child_id }),
    )
    .unwrap();

    let got = execute_command(
        &service,
        "sysml.sessions.archive.get",
        json!({ "id": child_id }),
    )
    .unwrap();
    assert_eq!(got["entry"]["origin"], "trade_study");
}
