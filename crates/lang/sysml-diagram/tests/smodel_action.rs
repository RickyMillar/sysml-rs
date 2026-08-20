//! ActionFlowView integration tests for the SModel visualization pipeline.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::ViewType;

#[test]
fn action_produces_graph() {
    let sg = generate(
        "package P { action def Process { action step1; action step2; succession first step1 then step2; } }",
        ViewType::ActionFlow,
        false,
    );
    // At minimum, should not crash and produce a valid graph
    assert_eq!(sg.type_, "graph");
}

#[test]
fn action_empty_model_no_crash() {
    let sg = generate("package P { part def A; }", ViewType::ActionFlow, false);
    assert_eq!(sg.type_, "graph");
}

#[test]
fn action_serializes_cleanly() {
    let sg = generate(
        "package P { action def A { action step; } }",
        ViewType::ActionFlow,
        false,
    );
    let json = serde_json::to_string(&sg);
    assert!(json.is_ok());
}

#[test]
fn action_with_multiple_steps() {
    let sg = generate(
        "package P { action def Pipeline { action a1; action a2; action a3; succession first a1 then a2; succession first a2 then a3; } }",
        ViewType::ActionFlow,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    assert!(json.contains("graph"));
}

#[test]
fn action_flow_has_valid_root() {
    let sg = generate(
        "package P { action def A { action s1; action s2; succession first s1 then s2; } }",
        ViewType::ActionFlow,
        false,
    );
    assert_eq!(sg.id, "root");
}
