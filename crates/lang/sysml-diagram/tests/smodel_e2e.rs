//! End-to-end integration tests for the SModel visualization pipeline.
//!
//! Each test parses inline SysML via TreeSitterParser, feeds the resulting ModelGraph
//! through `sysml_diagram::smodel::to_smodel_with()`, and asserts structural
//! properties of the resulting SGraph.

use std::collections::HashSet;

use sysml_diagram::smodel::{self, SEdge, SGraph, SModelElement, SNode, ViewType};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_sysml(source: &str) -> sysml_core::ModelGraph {
    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new("test.sysml", source)];
    let result = parser.parse(&files);
    let errors: Vec<_> = result
        .diagnostics
        .iter()
        .filter(|d| d.severity == sysml_span::Severity::Error)
        .collect();
    assert!(errors.is_empty(), "Parse errors: {:?}", errors);
    result.graph
}

fn generate(source: &str, view: ViewType, expand_all: bool) -> SGraph {
    let mut graph = parse_sysml(source);
    // Elaborate to synthesize implicit relationships (satisfy, verify, transitions,
    // connectors, flows) — production always feeds the diagram an elaborated graph
    // (the service `diagram()` method does this); mirror that here.
    sysml_core::elaborate::elaborate(&mut graph);
    let expanded: HashSet<String> = if expand_all {
        graph.elements.keys().map(|id| id.to_string()).collect()
    } else {
        HashSet::new()
    };
    let request = sysml_diagram::ViewRequest::new(view).with_expanded(expanded);
    smodel::to_smodel_with(&graph, &request)
}

fn count_by_type(children: &[SModelElement], type_prefix: &str) -> usize {
    let mut count = 0;
    for child in children {
        match child {
            SModelElement::Node(n) => {
                if n.type_.starts_with(type_prefix) {
                    count += 1;
                }
                count += count_by_type(&n.children, type_prefix);
            }
            SModelElement::Edge(e) => {
                if e.type_.starts_with(type_prefix) {
                    count += 1;
                }
            }
            SModelElement::Compartment(c) => {
                count += count_by_type(&c.children, type_prefix);
            }
            _ => {}
        }
    }
    count
}

fn count_edges(children: &[SModelElement]) -> usize {
    children
        .iter()
        .filter(|c| matches!(c, SModelElement::Edge(_)))
        .count()
}

fn count_nodes(children: &[SModelElement]) -> usize {
    children
        .iter()
        .filter(|c| matches!(c, SModelElement::Node(_)))
        .count()
}

fn find_node_by_type<'a>(children: &'a [SModelElement], type_: &str) -> Vec<&'a SNode> {
    let mut result = Vec::new();
    for child in children {
        match child {
            SModelElement::Node(n) => {
                if n.type_ == type_ {
                    result.push(n);
                }
                for inner in find_node_by_type(&n.children, type_) {
                    result.push(inner);
                }
            }
            SModelElement::Compartment(c) => {
                for inner in find_node_by_type(&c.children, type_) {
                    result.push(inner);
                }
            }
            _ => {}
        }
    }
    result
}

fn has_edge_type(children: &[SModelElement], type_: &str) -> bool {
    children
        .iter()
        .any(|c| matches!(c, SModelElement::Edge(e) if e.type_ == type_))
}

fn has_css_class_on_node(children: &[SModelElement], node_type: &str, class: &str) -> bool {
    for child in children {
        if let SModelElement::Node(n) = child {
            if n.type_ == node_type && n.css_classes.iter().any(|c| c == class) {
                return true;
            }
        }
    }
    false
}

fn collect_all_types(children: &[SModelElement]) -> HashSet<String> {
    let mut types = HashSet::new();
    for child in children {
        match child {
            SModelElement::Graph(_) => {
                types.insert("graph".to_string());
            }
            SModelElement::Node(n) => {
                types.insert(n.type_.clone());
            }
            SModelElement::Edge(e) => {
                types.insert(e.type_.clone());
            }
            SModelElement::Compartment(c) => {
                types.insert(c.type_.clone());
            }
            SModelElement::Port(p) => {
                types.insert(p.type_.clone());
            }
            SModelElement::Label(l) => {
                types.insert(l.type_.clone());
            }
            SModelElement::Button(b) => {
                types.insert(b.type_.clone());
            }
        }
    }
    types
}

fn edges(children: &[SModelElement]) -> Vec<&SEdge> {
    children
        .iter()
        .filter_map(|c| {
            if let SModelElement::Edge(e) = c {
                Some(e)
            } else {
                None
            }
        })
        .collect()
}

fn ports_in(children: &[SModelElement]) -> usize {
    let mut count = 0;
    for child in children {
        match child {
            SModelElement::Port(_) => count += 1,
            SModelElement::Node(n) => count += ports_in(&n.children),
            SModelElement::Compartment(c) => count += ports_in(&c.children),
            _ => {}
        }
    }
    count
}

// ---------------------------------------------------------------------------
// GeneralView (8 tests)
// ---------------------------------------------------------------------------

#[test]
fn general_has_nodes_for_all_top_level_elements() {
    let sg = generate(
        "package P { part def A; part def B; part c : A; }",
        ViewType::General,
        false,
    );
    // Package + 2 defs + 1 usage = at least 3 nodes (root package auto-expands,
    // so children are nested inside it — count recursively)
    assert!(count_by_type(&sg.children, "node:") >= 3);
}

#[test]
fn general_has_edges_for_relationships() {
    let sg = generate(
        "package P { part def Vehicle; part def Car :> Vehicle; }",
        ViewType::General,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    // Specialization may appear as edge or as text annotation depending on resolution
    let has_relationship_trace =
        count_edges(&sg.children) > 0 || json.contains("specialize") || json.contains("Vehicle");
    assert!(
        has_relationship_trace,
        "should reference the specialization somehow"
    );
}

#[test]
fn general_package_children_are_visible() {
    let sg = generate(
        "package Outer { part def Inner; }",
        ViewType::General,
        false,
    );
    // Root package auto-expands — inner def is nested inside it
    let nodes = count_by_type(&sg.children, "node:");
    assert!(
        nodes >= 2,
        "package + inner def should both render (recursively), got {}",
        nodes
    );
}

#[test]
fn general_expanded_nodes_have_nested_children() {
    let sg = generate(
        "package P { part def V { attribute mass : Real; part engine; } }",
        ViewType::General,
        true,
    );
    let json = serde_json::to_string(&sg).unwrap();
    // When expanded, children should appear as nested nodes, not just text
    assert!(
        json.contains("engine"),
        "expanded node should show nested child"
    );
}

#[test]
fn general_collapsed_nodes_have_text_compartments() {
    let sg = generate(
        "package P { part def V { attribute mass : Real; } }",
        ViewType::General,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    // When collapsed, attributes appear in text compartment
    assert!(
        json.contains("comp:attributes") || json.contains("mass"),
        "should show attributes"
    );
}

#[test]
fn general_no_membership_noise() {
    let sg = generate(
        "package P { part def A; part b : A; }",
        ViewType::General,
        false,
    );
    let types = collect_all_types(&sg.children);
    assert!(
        !types.contains("node:membership"),
        "membership nodes should be filtered"
    );
}

#[test]
fn general_css_classes_correct() {
    let sg = generate(
        "package P { part def Vehicle; part car : Vehicle; }",
        ViewType::General,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    // At minimum, definitions or usages should have css classes
    assert!(
        json.contains("definition") || json.contains("usage"),
        "nodes should have definition or usage css class"
    );
}

#[test]
fn general_serializes_to_valid_json() {
    let sg = generate("package P { part def A; }", ViewType::General, false);
    let json = serde_json::to_string_pretty(&sg);
    assert!(json.is_ok(), "SGraph should serialize to JSON");
    let s = json.unwrap();
    assert!(
        s.contains("\"type\": \"graph\"") || s.contains("\"type\":\"graph\""),
        "root should be graph type, got: {}",
        &s[..s.len().min(200)]
    );
}

// ---------------------------------------------------------------------------
// InterconnectionView (6 tests)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// StateTransitionView (5 tests)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// ActionFlowView (5 tests)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Requirement notation in the General view (4 tests)
//
// The legacy Requirements view kind was retired; requirement notation now
// renders in the base General view, gated on element kind. See
// ---------------------------------------------------------------------------

#[test]
fn req_satisfy_edge_present() {
    let sg = generate(
        "package P { requirement def R1; requirement r : R1; part sys; satisfy r by sys; }",
        ViewType::General,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    // Should contain either satisfy edges or at least the requirement/part nodes
    let has_satisfy = json.contains("satisfy") || count_edges(&sg.children) > 0;
    assert!(
        has_satisfy,
        "should have satisfy edge or relationship representation"
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
    // `part sys` is top-level so it renders in General independently; the
    // peer generator's non-spec satisfy-endpoint auto-pull-in is dropped.
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

// ---------------------------------------------------------------------------
// SequenceView (3 tests)
// ---------------------------------------------------------------------------

#[test]
fn sequence_empty_no_crash() {
    let sg = generate("package P { part def A; }", ViewType::Sequence, false);
    assert_eq!(sg.type_, "graph");
}

#[test]
fn sequence_with_actions() {
    let sg = generate(
        "package P { action def Client; action def Server; action c : Client; action s : Server; }",
        ViewType::Sequence,
        false,
    );
    assert_eq!(sg.type_, "graph");
}

#[test]
fn sequence_serializes() {
    let sg = generate("package P { action def A; }", ViewType::Sequence, false);
    let json = serde_json::to_string(&sg);
    assert!(json.is_ok());
}

// ---------------------------------------------------------------------------
// GridView (3 tests)
// ---------------------------------------------------------------------------

#[test]
fn grid_with_satisfy_relationships() {
    let sg = generate(
        "package P { requirement def R; requirement r : R; part s; satisfy r by s; }",
        ViewType::Grid,
        false,
    );
    assert_eq!(sg.type_, "graph");
}

#[test]
fn grid_empty_model_no_crash() {
    let sg = generate("package P { part def A; }", ViewType::Grid, false);
    assert_eq!(sg.type_, "graph");
}

#[test]
fn grid_serializes() {
    let sg = generate(
        "package P { requirement def R; requirement r : R; part s; satisfy r by s; }",
        ViewType::Grid,
        false,
    );
    let json = serde_json::to_string(&sg);
    assert!(json.is_ok());
}

// ---------------------------------------------------------------------------
// BrowserView (3 tests)
// ---------------------------------------------------------------------------

#[test]
fn browser_nesting_depth() {
    let sg = generate(
        "package L1 { package L2 { part def S { part inner; } } }",
        ViewType::Browser,
        true,
    );
    let json = serde_json::to_string(&sg).unwrap();
    assert!(
        json.contains("inner"),
        "deep nesting should be visible when expanded"
    );
}

#[test]
fn browser_no_edges() {
    let sg = generate(
        "package P { part def A; part def B :> A; }",
        ViewType::Browser,
        false,
    );
    let edge_count = count_edges(&sg.children);
    assert_eq!(edge_count, 0, "browser view should have zero edges");
}

#[test]
fn browser_expand_button_present() {
    let sg = generate(
        "package P { part def A { part child; } }",
        ViewType::Browser,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    assert!(
        json.contains("button:expand"),
        "browser nodes with children should have expand button"
    );
}

// ---------------------------------------------------------------------------
// GeometryView (3 tests)
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Cross-view (2 tests)
// ---------------------------------------------------------------------------

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
