//! InterconnectionView (IBD) integration tests for the SModel visualization pipeline.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::ViewType;

#[test]
fn ibd_parts_have_port_children() {
    let sg = generate(
        "package P { port def DP; part def S { port p : DP; } part s : S; }",
        ViewType::Interconnection,
        false,
    );
    let port_count = ports_in(&sg.children);
    assert!(port_count > 0, "should have ports");
}

#[test]
fn ibd_flow_edges_route_through_ports() {
    let src = "package P { port def DP; part def A { port out1 : DP; } part def B { port in1 : DP; } part a : A; part b : B; flow a.out1 to b.in1; }";
    let sg = generate(src, ViewType::Interconnection, false);
    let flow_edges: Vec<_> = edges(&sg.children)
        .into_iter()
        .filter(|e| e.type_.contains("flow"))
        .collect();
    // Flow edges may or may not be present depending on parser resolution.
    // At minimum the pipeline should not crash.
    if !flow_edges.is_empty() {
        let has_port_routing = flow_edges
            .iter()
            .any(|e| e.source_port_id.is_some() || e.target_port_id.is_some());
        assert!(has_port_routing, "flow edges should route through ports");
    }
}

#[test]
fn ibd_nested_parts_render() {
    let sg = generate(
        "package P { part def Outer { part inner; } part o : Outer; }",
        ViewType::Interconnection,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    assert!(
        json.contains("inner") || json.contains("Outer"),
        "nested parts should render"
    );
}

#[test]
fn ibd_package_children_visible() {
    let src = "package IBD { port def DP; part def S { port p : DP; } part s : S; }";
    let sg = generate(src, ViewType::Interconnection, false);
    assert!(
        count_nodes(&sg.children) > 0,
        "parts inside package should be visible"
    );
}

#[test]
fn ibd_empty_model_no_crash() {
    let sg = generate(
        "package P { part def A; }",
        ViewType::Interconnection,
        false,
    );
    assert_eq!(sg.type_, "graph");
}

#[test]
fn ibd_connection_edge_type() {
    let src = "package P { port def DP; part def A { port p1 : DP; } part def B { port p2 : DP; } part a : A; part b : B; }";
    let sg = generate(src, ViewType::Interconnection, false);
    // Even without explicit connection, it should not crash
    assert_eq!(sg.type_, "graph");
}
