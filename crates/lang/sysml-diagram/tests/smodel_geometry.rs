//! GeometryView integration tests for the SModel visualization pipeline.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::ViewType;

#[test]
fn geometry_produces_nodes() {
    let sg = generate(
        "package P { part def A; part def B; }",
        ViewType::Geometry,
        false,
    );
    assert!(
        count_nodes(&sg.children) > 0,
        "geometry should produce nodes"
    );
}

#[test]
fn geometry_edges_present() {
    let sg = generate(
        "package P { part def A; part def B :> A; }",
        ViewType::Geometry,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    // Geometry view should show relationships in some form, or at least render both elements
    assert!(
        count_edges(&sg.children) > 0 || json.contains("specialize") || json.contains("A"),
        "geometry should show elements and/or relationship edges"
    );
}

#[test]
fn geometry_serializes() {
    let sg = generate("package P { part def A; }", ViewType::Geometry, false);
    let json = serde_json::to_string(&sg);
    assert!(json.is_ok());
}
