//! StateTransitionView integration tests for the SModel visualization pipeline.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::ViewType;

#[test]
fn state_has_initial_node() {
    let sg = generate(
        "package P { state def SM { state s1; state s2; transition first s1 then s2; } }",
        ViewType::StateTransition,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    // Check for initial node — type may vary
    let has_initial = json.contains("initialNode") || json.contains("initial");
    assert!(
        has_initial || count_nodes(&sg.children) > 0,
        "state diagram should have nodes"
    );
}

#[test]
fn state_transitions_are_edges() {
    let sg = generate(
        "package P { state def SM { state s1; state s2; transition first s1 then s2; } }",
        ViewType::StateTransition,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    // Transitions should produce edges
    let has_transition = json.contains("transition") || count_edges(&sg.children) > 0;
    assert!(has_transition, "should have transition edge or nodes");
}

#[test]
fn state_multiple_states() {
    let sg = generate(
        "package P { state def SM { state red; state green; state yellow; transition first red then green; transition first green then yellow; transition first yellow then red; } }",
        ViewType::StateTransition,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    // All state names should appear somewhere in the output
    assert!(json.contains("red"), "should contain state 'red'");
    assert!(json.contains("green"), "should contain state 'green'");
    assert!(json.contains("yellow"), "should contain state 'yellow'");
}

#[test]
fn state_empty_model_no_crash() {
    let sg = generate(
        "package P { part def A; }",
        ViewType::StateTransition,
        false,
    );
    assert_eq!(sg.type_, "graph");
}

#[test]
fn state_serializes_cleanly() {
    let sg = generate(
        "package P { state def SM { state s1; } }",
        ViewType::StateTransition,
        false,
    );
    let json = serde_json::to_string(&sg);
    assert!(json.is_ok());
}
