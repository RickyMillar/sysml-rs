//! Requirement-notation integration tests for the SModel visualization
//! pipeline.
//!
//! The legacy Requirements view kind was retired (see
//! requirement notation (requirement nodes, «satisfy» edges) now renders
//! in the base `General` view, gated on element kind. These tests assert
//! that notation survives via the General path.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::{self, ViewType};
use sysml_diagram::ViewRequest;

#[test]
fn req_satisfy_edge_present() {
    let sg = generate(
        "package P { requirement def R1; requirement r : R1; part sys; satisfy r by sys; }",
        ViewType::General,
        false,
    );
    let edge_count = count_edges(&sg.children);
    assert!(
        edge_count > 0,
        "should have satisfy edge after elaboration (edges={})",
        edge_count,
    );
}

#[test]
fn req_requirement_nodes_visible() {
    let sg = generate(
        "package P { requirement def Speed; requirement speed : Speed; }",
        ViewType::General,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    assert!(
        json.contains("Speed") || json.contains("speed"),
        "requirement nodes should be visible"
    );
}

#[test]
fn req_connected_elements_included() {
    // The satisfying `part sys` is itself a top-level element, so it
    // renders in the General view independently. (The retired peer
    // generator's satisfy-endpoint auto-pull-in — a non-spec projection —
    // is intentionally dropped per doc §3.3; here `sys` is exposed by being
    // top-level, so the assertion still holds.)
    let sg = generate(
        "package P { requirement def R; requirement r : R; part sys; satisfy r by sys; }",
        ViewType::General,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    assert!(json.contains("sys"), "satisfying element should appear");
}

#[test]
fn req_empty_model_no_crash() {
    let sg = generate("package P { part def A; }", ViewType::General, false);
    assert_eq!(sg.type_, "graph");
}

#[test]
fn verify_edge_wins_over_typing_when_usage_is_off_canvas() {
    // D-N7 (C7): `verification def C { objective { verify requirement r :
    // ReqDef; } }` — the Verify target is the nested USAGE, which is not on
    // canvas. The «verify» edge must proxy to the rendered requirement
    // DEFINITION instead of self-suppressing (reroute onto its own case),
    // and the raw typing edge duplicating that pair must be dropped.
    let src = "package P { \
        requirement def TripAt5xRated; \
        verification def ComplianceCase { \
            objective fastTripObjective { \
                verify requirement fastTripCheck : TripAt5xRated; \
            } \
        } \
    }";
    let mut graph = parse_sysml(src);
    sysml_core::elaborate::elaborate(&mut graph);

    let sg = smodel::to_smodel_with(&graph, &ViewRequest::new(ViewType::General));
    let json = serde_json::to_string(&sg).unwrap();
    assert!(
        json.contains("\u{00ab}verify\u{00bb}"),
        "the semantic «verify» edge must render (proxied to the requirement def)"
    );

    // No typing edge may share the verify edge's (source, target) pair —
    // assert via the ViewModel IR where kinds are typed.
    let vm = sysml_diagram::to_view_model(&graph, &ViewRequest::new(ViewType::General));
    use sysml_diagram::ir::types::DiagramEdgeKind;
    use sysml_core::RelationshipKind;
    let verify_pairs: Vec<(String, String)> = vm
        .scene
        .edges
        .iter()
        .filter(|e| matches!(&e.kind, DiagramEdgeKind::Relationship(RelationshipKind::Verify)))
        .map(|e| (e.source_id.clone(), e.target_id.clone()))
        .collect();
    assert!(!verify_pairs.is_empty(), "expected a verify edge in the IR");
    for e in &vm.scene.edges {
        if matches!(&e.kind, DiagramEdgeKind::Relationship(RelationshipKind::TypeOf))
            || e.label == "typing"
        {
            assert!(
                !verify_pairs.contains(&(e.source_id.clone(), e.target_id.clone())),
                "typing edge duplicates the verify pair — the semantic edge must win"
            );
        }
    }
}

#[test]
fn stereotypes_use_declaration_text_keywords() {
    // D-N1 (C7): guillemet stereotypes read as declaration-text keywords —
    // «part def» / «requirement def» — never expanded metaclass names.
    let src = "package P { part def Vehicle; requirement def SafetyReq; part engine : Vehicle; }";
    let mut graph = parse_sysml(src);
    sysml_core::elaborate::elaborate(&mut graph);
    let sg = smodel::to_smodel_with(&graph, &ViewRequest::new(ViewType::General));
    let json = serde_json::to_string(&sg).unwrap();
    assert!(json.contains("\u{00ab}part def\u{00bb}"), "part definitions read «part def»");
    assert!(
        !json.contains("\u{00ab}part definition\u{00bb}"),
        "expanded metaclass names must not appear as stereotypes"
    );
    assert!(
        json.contains("\u{00ab}requirement def\u{00bb}"),
        "requirement definitions read «requirement def»"
    );
}
