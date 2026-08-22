//! S4.T2 — `sysml.get_source` wire contract.
//!
//! Locks the JSON shape and behaviour for the new sneak-peek command:
//!
//! - Known element: response has a non-empty `text`, byte offsets that
//!   agree with the slice length, and `text` matches the source file's
//!   bytes in that range.
//! - Unknown id: response is JSON `null` (Option::None).
//! - Unknown URI: command returns an error (`ServiceError::ElementNotFound`).
//!
//! Powered by `sysml-ide-db::file_source_query::file_source_at`; this
//! file is the transport-side regression cover.

use std::path::Path;

use serde_json::{json, Value};
use sysml_service::{execute_command, SysmlService};

fn workspace_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn coffee_definitions_path() -> std::path::PathBuf {
    workspace_root().join("examples/the-book-corpus/coffee-machine/definitions.sysml")
}

fn coffee_definitions_uri(service: &SysmlService) -> String {
    let path = coffee_definitions_path();
    service.load_file(&path).expect("load coffee_definitions");
    service
        .loaded_uris()
        .into_iter()
        .find(|u| u.contains("definitions.sysml"))
        .expect("coffee definitions URI")
}

fn harvest_first_id(v: &Value) -> Option<String> {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                if (k == "id" || k == "element_id") && val.is_string() {
                    return Some(val.as_str().unwrap().to_owned());
                }
                if let Some(found) = harvest_first_id(val) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(arr) => arr.iter().find_map(harvest_first_id),
        _ => None,
    }
}

#[test]
fn get_source_returns_slice_for_known_element() {
    let service = SysmlService::empty();
    let uri = coffee_definitions_uri(&service);

    let find_resp = execute_command(
        &service,
        "sysml.find",
        json!({ "uri": uri, "pattern": "Coffee", "kind": null }),
    )
    .expect("sysml.find");
    let id = harvest_first_id(&find_resp)
        .expect("sysml.find should expose at least one element id for `Coffee`");

    let resp = execute_command(
        &service,
        "sysml.get_source",
        json!({ "uri": uri, "id": id }),
    )
    .expect("sysml.get_source");

    let obj = resp
        .as_object()
        .unwrap_or_else(|| panic!("get_source should return a JSON object, got {resp:?}"));

    let text = obj
        .get("text")
        .and_then(|v| v.as_str())
        .expect("text field");
    let start = obj
        .get("start")
        .and_then(|v| v.as_u64())
        .expect("start field") as usize;
    let end = obj
        .get("end")
        .and_then(|v| v.as_u64())
        .expect("end field") as usize;

    assert!(!text.is_empty(), "slice text should be non-empty");
    assert_eq!(
        text.len(),
        end - start,
        "text length should match (end - start)"
    );
    assert!(end > start, "end must be strictly greater than start");

    // Cross-check the slice against the actual file bytes.
    let file_bytes = std::fs::read_to_string(coffee_definitions_path()).expect("read fixture");
    assert_eq!(
        &file_bytes[start..end],
        text,
        "get_source text should match the file's byte range"
    );
}

#[test]
fn get_source_returns_null_for_unknown_element() {
    let service = SysmlService::empty();
    let uri = coffee_definitions_uri(&service);

    let stranger = sysml_service::ElementId::new_v4().to_string();
    let resp = execute_command(
        &service,
        "sysml.get_source",
        json!({ "uri": uri, "id": stranger }),
    )
    .expect("sysml.get_source");

    assert!(
        resp.is_null(),
        "unknown id should yield JSON null (Option::None), got {resp:?}"
    );
}

#[test]
fn get_source_errors_on_unloaded_uri() {
    let service = SysmlService::empty();
    let stranger = sysml_service::ElementId::new_v4().to_string();

    let err = execute_command(
        &service,
        "sysml.get_source",
        json!({ "uri": "file:///nowhere.sysml", "id": stranger }),
    )
    .expect_err("unloaded URI should error");

    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("no graph") || msg.to_lowercase().contains("not found"),
        "error should mention missing URI, got: {msg}"
    );
}
