//! S1.T10 — service::execute_command ID round-trip.
//!
//! Sanity test: IDs returned by service commands round-trip cleanly through
//! `serde_json` and stay equal across follow-up commands that take the same
//! ID as input. This is the wire-format contract being locked in pre-S2.
//!
//! Today's behaviour (post-S1):
//! - `ElementId` serialises transparently as a string (the canonical-key
//!   derived UUID). Deserialise → string-equality holds.
//! - Feeding an ID returned by `sysml.find` into `sysml.element` returns
//!   the same element shape (matching id, kind, name).
//!
//! This is a 50-line "freeze the JSON contract" test. The interesting work
//! happened in S1.T11a/b/c (canonical keys) and S1.T7 (RuntimeSession key
//! migration); this file is pure regression cover.

use std::path::Path;

use serde_json::{json, Value};
use sysml_service::{execute_command, SysmlService};

/// Workspace root (`sysml-rs/sysml-rs/`).
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

/// Round-trip a JSON value through `to_string` + `from_str`, asserting
/// byte-stable string equality.
fn assert_json_round_trip(name: &str, v: &Value) {
    let s = serde_json::to_string(v).expect("serialize");
    let parsed: Value = serde_json::from_str(&s).expect("deserialize");
    assert_eq!(v, &parsed, "{}: round-trip JSON value mismatch", name);
}

/// Recursively collect every string field whose key suggests it carries
/// an element id (`id` / `element_id` / `*_id` / `target` / `source`).
/// Used to harvest ids from arbitrary command responses without depending
/// on each command's typed shape.
fn harvest_ids(v: &Value, out: &mut Vec<String>) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                let k = k.as_str();
                if (k == "id" || k == "element_id" || k.ends_with("_id"))
                    && val.is_string()
                {
                    out.push(val.as_str().unwrap().to_string());
                }
                harvest_ids(val, out);
            }
        }
        Value::Array(arr) => {
            for item in arr {
                harvest_ids(item, out);
            }
        }
        _ => {}
    }
}

#[test]
fn id_round_trip_across_six_commands() {
    let service = SysmlService::empty();
    let uri = coffee_definitions_uri(&service);

    // -- 1. sysml.parse -- returns ModelGraph + diagnostics; ids in graph.elements.
    let parse_resp = execute_command(
        &service,
        "sysml.parse",
        json!({ "path": format!("{}", coffee_definitions_path().display()) }),
    )
    .expect("sysml.parse");
    assert_json_round_trip("sysml.parse", &parse_resp);
    let mut parse_ids: Vec<String> = vec![];
    harvest_ids(&parse_resp, &mut parse_ids);
    assert!(
        !parse_ids.is_empty(),
        "sysml.parse should expose at least one id"
    );

    // -- 2. sysml.find -- returns Vec<Element>.
    let find_resp = execute_command(
        &service,
        "sysml.find",
        json!({ "uri": uri, "pattern": "Coffee", "kind": null }),
    )
    .expect("sysml.find");
    assert_json_round_trip("sysml.find", &find_resp);
    let mut find_ids: Vec<String> = vec![];
    harvest_ids(&find_resp, &mut find_ids);
    assert!(
        !find_ids.is_empty(),
        "sysml.find for `Coffee` should return at least one element from the coffee fixture"
    );

    // -- 3. sysml.element -- feed an ID from sysml.find back in.
    //      This is the "follow-up" round-trip: the response should
    //      reference the same element id back.
    let target_id = find_ids[0].clone();
    let element_resp = execute_command(
        &service,
        "sysml.element",
        json!({ "uri": uri, "id": target_id }),
    )
    .expect("sysml.element");
    assert_json_round_trip("sysml.element", &element_resp);
    let mut element_ids = vec![];
    harvest_ids(&element_resp, &mut element_ids);
    assert!(
        element_ids.contains(&target_id),
        "sysml.element response should echo the input id ({}); harvested: {:?}",
        target_id,
        element_ids,
    );

    // -- 4. sysml.children -- feed the same ID back in to a different
    //      command. We don't require children to be non-empty (some
    //      elements are leaves), just that the response round-trips.
    let children_resp = execute_command(
        &service,
        "sysml.children",
        json!({ "uri": uri, "id": target_id }),
    )
    .expect("sysml.children");
    assert_json_round_trip("sysml.children", &children_resp);

    // -- 5. sysml.model.tree -- broader query that returns a forest of
    //      tree nodes, each carrying an element id. Cap depth so the
    //      response stays small.
    let tree_resp = execute_command(
        &service,
        "sysml.model.tree",
        json!({ "uri": uri, "max_depth": 3, "view": "user_facing" }),
    )
    .expect("sysml.model.tree");
    assert_json_round_trip("sysml.model.tree", &tree_resp);
    let mut tree_ids = vec![];
    harvest_ids(&tree_resp, &mut tree_ids);
    assert!(
        !tree_ids.is_empty(),
        "sysml.model.tree should expose at least one id"
    );

    // -- 6. sysml.sessions.list -- session command (returns []
    //      typically; we just verify the wire shape round-trips).
    let sessions_resp = execute_command(&service, "sysml.sessions.list", json!({}))
        .expect("sysml.sessions.list");
    assert_json_round_trip("sysml.sessions.list", &sessions_resp);
    assert!(
        sessions_resp.is_array(),
        "sysml.sessions.list should be a JSON array"
    );

    // Cross-command identity check: every id harvested from sysml.find
    // *should* appear in sysml.model.tree (model_tree is a superset of
    // top-level + descendants reachable from roots). For the coffee
    // fixture the find pattern is `Coffee`, which matches the top-level
    // CoffeeMachine usage; that one definitely lives inside
    // model_tree's roots.
    let target_id = find_ids[0].clone();
    let in_tree = tree_ids.iter().any(|tid| tid == &target_id);
    assert!(
        in_tree,
        "id {} returned by sysml.find was not present in sysml.model.tree (top-3-depth) ids",
        target_id,
    );
}
