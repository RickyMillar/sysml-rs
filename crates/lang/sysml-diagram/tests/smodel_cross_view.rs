//! Cross-view integration tests for the SModel visualization pipeline.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::ViewType;

#[test]
fn all_views_no_panic() {
    let src = "package P { part def A { attribute x : Real; } part a : A; requirement def R; requirement r : R; satisfy r by a; state def S { state s1; } action def Act { action step; } }";
    let views = [
        ViewType::General,
        ViewType::Interconnection,
        ViewType::StateTransition,
        ViewType::ActionFlow,
        ViewType::Browser,
        ViewType::Sequence,
        ViewType::Grid,
        ViewType::Geometry,
    ];
    for view in views {
        let sg = generate(src, view, false);
        assert_eq!(sg.type_, "graph", "{:?} should produce valid graph", view);
        let json = serde_json::to_string(&sg);
        assert!(json.is_ok(), "{:?} should serialize to JSON", view);
    }
}

#[test]
fn all_views_expand_all_no_panic() {
    let src = "package P { part def A { attribute x : Real; part inner; } }";
    let views = [
        ViewType::General,
        ViewType::Interconnection,
        ViewType::Browser,
        ViewType::Geometry,
    ];
    for view in views {
        let sg = generate(src, view, true);
        assert_eq!(sg.type_, "graph");
    }
}

// ---------------------------------------------------------------------------
// Suppress unused warnings for helpers used conditionally
// ---------------------------------------------------------------------------

#[test]
fn helpers_compile() {
    // Ensure all helpers are used at least once to avoid dead-code warnings
    let sg = generate("package P { part def A; }", ViewType::General, false);
    let _ = count_by_type(&sg.children, "node:");
    let _ = count_edges(&sg.children);
    let _ = count_nodes(&sg.children);
    let _ = find_node_by_type(&sg.children, "node:block");
    let _ = has_edge_type(&sg.children, "edge:specialize");
    let _ = has_css_class_on_node(&sg.children, "node:block", "definition");
    let _ = collect_all_types(&sg.children);
    let _ = edges(&sg.children);
    let _ = ports_in(&sg.children);
}
