//! Integration test for Phase 5 `expose` consumption — when a
//! ViewRequest sets `expose = Some(id)`, generators should restrict
//! the canvas root to that single element.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::{self, ViewType};
use sysml_diagram::ViewRequest;

#[test]
fn expose_restricts_top_level_to_subject_only() {
    let mut graph = parse_sysml("package P { part def Engine; part def Gearbox; }");
    sysml_core::elaborate::elaborate(&mut graph);

    // Find Engine's id in the graph.
    let engine_id = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Engine"))
        .expect("Engine present")
        .id
        .clone();

    let request = ViewRequest::new(ViewType::General).with_expose(engine_id);
    let sg_full = smodel::to_smodel_with(&graph, &ViewRequest::new(ViewType::General));
    let sg_exposed = smodel::to_smodel_with(&graph, &request);

    // Full diagram has more nodes than the exposed-only diagram.
    let full_nodes = count_by_type(&sg_full.children, "node:");
    let exposed_nodes = count_by_type(&sg_exposed.children, "node:");
    assert!(
        exposed_nodes < full_nodes,
        "exposed view should drop sibling parts: full={full_nodes} exposed={exposed_nodes}"
    );

    // The exposed diagram still has at least one node (the subject).
    assert!(exposed_nodes > 0, "exposed view should not be empty");
}

#[test]
fn view_filter_with_false_condition_excludes_top_level_element() {
    // Build a graph with one part and one ElementFilterMembership whose
    // filterExpression is `false` — every element should be excluded
    // when the filter is attached.
    let mut graph = sysml_core::ModelGraph::new();
    let part = sysml_core::ElementFactory::create(sysml_core::ElementKind::PartUsage)
        .with_name("p");
    graph.add_element(part);
    let mut filter = sysml_core::ElementFactory::create(
        sysml_core::ElementKind::ElementFilterMembership,
    );
    filter.set_prop("filterExpression", "false");
    let filter_id = filter.id.clone();
    graph.add_element(filter);

    let request = ViewRequest::new(ViewType::General).with_filter(
        sysml_core::ViewFilter::new()
            .with_kinds([sysml_core::ElementKind::PartUsage])
            .with_expression(filter_id),
    );
    let sg = smodel::to_smodel_with(&graph, &request);
    assert_eq!(
        count_by_type(&sg.children, "node:"),
        0,
        "false viewCondition should exclude every element"
    );
}

#[test]
fn multiple_filter_expressions_compose_as_conjunction() {
    // Two filter expressions, one literal `true` and one literal
    // `false`. ElementFilterMembership composes as AND per spec, so
    // every element must be excluded — `false` dominates the
    // conjunction. The previous behaviour kept only the last filter,
    // which would have let the literal-false win by accident; this test
    // verifies the AND composition explicitly with the false placed
    // FIRST so order-dependent regressions are caught.
    let mut graph = sysml_core::ModelGraph::new();
    let part = sysml_core::ElementFactory::create(sysml_core::ElementKind::PartUsage)
        .with_name("p");
    graph.add_element(part);

    let mut f_false = sysml_core::ElementFactory::create(
        sysml_core::ElementKind::ElementFilterMembership,
    );
    f_false.set_prop("filterExpression", "false");
    let f_false_id = f_false.id.clone();
    graph.add_element(f_false);

    let mut f_true = sysml_core::ElementFactory::create(
        sysml_core::ElementKind::ElementFilterMembership,
    );
    f_true.set_prop("filterExpression", "true");
    let f_true_id = f_true.id.clone();
    graph.add_element(f_true);

    let request = ViewRequest::new(ViewType::General).with_filter(
        sysml_core::ViewFilter::new()
            .with_kinds([sysml_core::ElementKind::PartUsage])
            .with_expressions([f_false_id, f_true_id]),
    );
    let sg = smodel::to_smodel_with(&graph, &request);
    assert_eq!(
        count_by_type(&sg.children, "node:"),
        0,
        "AND of [false, true] must drop every element — found nodes \
         means a stale filter slot leaked through"
    );
}

#[test]
fn view_filter_with_true_condition_keeps_matching_elements() {
    let mut graph = sysml_core::ModelGraph::new();
    let part = sysml_core::ElementFactory::create(sysml_core::ElementKind::PartUsage)
        .with_name("p");
    graph.add_element(part);
    let mut filter = sysml_core::ElementFactory::create(
        sysml_core::ElementKind::ElementFilterMembership,
    );
    filter.set_prop("filterExpression", "true");
    let filter_id = filter.id.clone();
    graph.add_element(filter);

    let request = ViewRequest::new(ViewType::General).with_filter(
        sysml_core::ViewFilter::new()
            .with_kinds([sysml_core::ElementKind::PartUsage])
            .with_expression(filter_id),
    );
    let sg = smodel::to_smodel_with(&graph, &request);
    assert!(
        count_by_type(&sg.children, "node:") > 0,
        "true viewCondition should keep elements"
    );
}

#[test]
fn requirement_view_filter_parses_and_evaluates() {
    // GAP 1 + GAP 2 end-to-end. A `filter @A or @B;` written INSIDE a
    // view-def body must:
    //  (GAP 1) parse into a real `ElementFilterMembership` child of the
    //          view — `filter_decl` is now reachable from `definition_body`
    //          / `function_body` (the shared `_body_member` choice), not
    //          mis-parsed as a `result_expression` with `filter` lexed as
    //          an identifier; and
    //  (GAP 2) capture the FULL `@A or @B` chain — the tail operand of
    //          `filter_classification_lead` now accepts a second leading-`@`
    //          classification, so the chain no longer truncates at an ERROR.
    let src = "package P { \
        public import SysML::*; \
        view def RV :> GeneralView { \
            filter @SysML::RequirementUsage or @SysML::RequirementDefinition; \
        } \
        requirement r1; \
        part p1; \
    }";
    let mut graph = parse_sysml(src);
    sysml_core::elaborate::elaborate(&mut graph);

    // GAP 1: the view owns exactly one ElementFilterMembership.
    let index = sysml_core::view_index::build_view_index(&graph);
    let rv = index
        .iter()
        .find(|s| s.name.as_deref() == Some("RV"))
        .expect("RV view present in index");
    assert_eq!(
        rv.filters.len(),
        1,
        "a view-def-body `filter` must mint exactly one ElementFilterMembership (GAP 1)"
    );

    // GAP 2: the captured filter text carries BOTH `@`-operands.
    let filter_id = rv.filters[0].clone();
    let filter_elem = graph
        .get_element(&filter_id)
        .expect("filter membership element present");
    let expr = filter_elem
        .get_prop("filterExpression")
        .and_then(|v| match v {
            sysml_core::Value::String(s) => Some(s.clone()),
            _ => None,
        })
        .expect("filterExpression text captured");
    assert!(
        expr.contains("@SysML::RequirementUsage"),
        "filter must keep the first operand; got {expr:?}"
    );
    assert!(
        expr.contains("@SysML::RequirementDefinition"),
        "filter must keep the second `@` operand past `or` (GAP 2); got {expr:?}"
    );

    // The filter actually filters: requirement selected, part excluded.
    let req = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("r1"))
        .expect("requirement usage r1 present");
    let part = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("p1"))
        .expect("part usage p1 present");
    assert!(
        sysml_runtime::view_condition::evaluate_view_condition(&graph, &filter_id, req),
        "requirement r1 ({:?}) must pass the requirement filter",
        req.kind
    );
    assert!(
        !sysml_runtime::view_condition::evaluate_view_condition(&graph, &filter_id, part),
        "part p1 ({:?}) must be excluded by the requirement filter",
        part.kind
    );
}

#[test]
fn expose_with_unrelated_id_yields_empty_top_level() {
    let mut graph = parse_sysml("package P { part def Engine; }");
    sysml_core::elaborate::elaborate(&mut graph);

    // Use a fresh, never-used ElementId.
    let bogus = sysml_core::ElementId::new_v4();
    let request = ViewRequest::new(ViewType::General).with_expose(bogus);
    let sg = smodel::to_smodel_with(&graph, &request);

    let nodes = count_by_type(&sg.children, "node:");
    assert_eq!(nodes, 0, "no node should match a bogus exposed id");
}

#[test]
fn expose_prunes_edges_to_non_canvas_root_endpoints() {
    // Two parts plus a Subclassification edge between them at the
    // package level. Exposing only Engine should keep Engine on the
    // canvas and drop the edge that lands on the unexposed Wheel
    // sibling — payloads should NOT carry every workspace relationship
    // when expose narrows the canvas.
    let src = "package P { \
        part def Engine; \
        part def Wheel; \
        part def Hub :> Wheel; \
    }";
    let mut graph = parse_sysml(src);
    sysml_core::elaborate::elaborate(&mut graph);

    let engine_id = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Engine"))
        .expect("Engine present")
        .id
        .clone();

    let request = ViewRequest::new(ViewType::General).with_expose(engine_id);
    let sg = smodel::to_smodel_with(&graph, &request);

    // The Wheel <- Hub specialization edge has neither endpoint inside
    // the Engine subtree, so it must be pruned out of the SGraph entirely.
    let edges = count_by_type(&sg.children, "edge:");
    assert_eq!(
        edges, 0,
        "expose narrowing should drop edges whose endpoints sit outside the canvas subject — found {edges} edge(s)"
    );
}

#[test]
fn expose_fences_state_view_to_exposed_definition() {
    // Two state defs; exposing one must fence the other out — before the
    // `in_exposed_scope` fence the state generator swept in EVERY
    // StateDefinition in the merged graph (D-B1).
    let mut graph = parse_sysml(
        "package P { \
            state def ExposedMachine { state s1; state s2; transition first s1 then s2; } \
            state def LeakyMachine { state o1; } \
        }",
    );
    sysml_core::elaborate::elaborate(&mut graph);

    let sm_id = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("ExposedMachine"))
        .expect("ExposedMachine present")
        .id
        .clone();

    let request = ViewRequest::new(ViewType::StateTransition).with_expose(sm_id);
    let sg = smodel::to_smodel_with(&graph, &request);
    let json = serde_json::to_string(&sg).unwrap();
    assert!(
        json.contains("ExposedMachine"),
        "exposed state def must render"
    );
    assert!(
        !json.contains("LeakyMachine"),
        "unexposed sibling state def must be fenced out of the scene"
    );
}

#[test]
fn expose_fences_action_view_to_exposed_definition() {
    let mut graph = parse_sysml(
        "package P { \
            action def ExposedFlow { action a; action b; first a then b; } \
            action def LeakyFlow { action c; } \
        }",
    );
    sysml_core::elaborate::elaborate(&mut graph);

    let flow_id = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("ExposedFlow"))
        .expect("ExposedFlow present")
        .id
        .clone();

    let request = ViewRequest::new(ViewType::ActionFlow).with_expose(flow_id);
    let sg = smodel::to_smodel_with(&graph, &request);
    let json = serde_json::to_string(&sg).unwrap();
    assert!(json.contains("ExposedFlow"), "exposed action def must render");
    assert!(
        !json.contains("LeakyFlow"),
        "unexposed sibling action def must be fenced out of the scene"
    );
}

#[test]
fn expose_via_owning_package_admits_contained_definitions() {
    // Exposing a PACKAGE must still admit the definitions inside it —
    // the fence is "exposed element or ownership-descendant of one".
    let mut graph = parse_sysml(
        "package Outer { package Inner { state def SMInside { state s1; } } }",
    );
    sysml_core::elaborate::elaborate(&mut graph);

    let inner_id = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Inner"))
        .expect("Inner present")
        .id
        .clone();

    let request = ViewRequest::new(ViewType::StateTransition).with_expose(inner_id);
    let sg = smodel::to_smodel_with(&graph, &request);
    let json = serde_json::to_string(&sg).unwrap();
    assert!(
        json.contains("SMInside"),
        "a state def inside the exposed package must render"
    );
}

#[test]
fn interconnection_part_expose_renders_internal_structure_expanded() {
    // Exposing a part def in an Interconnection view makes it the IBD context
    // block: internal part usages present AND the frame marked expanded —
    // without the flag the renderer showed a collapsed bare box (D-B4).
    let mut graph = parse_sysml(
        "package P { \
            part def Engine; part def Gearbox; \
            part def Vehicle { \
                part engine : Engine; \
                part gearbox : Gearbox; \
                connect engine to gearbox; \
            } \
        }",
    );
    sysml_core::elaborate::elaborate(&mut graph);

    let vehicle_id = graph
        .elements
        .values()
        .find(|e| e.name.as_deref() == Some("Vehicle") && e.kind.to_string().contains("Definition"))
        .expect("Vehicle def present")
        .id
        .clone();

    let request = ViewRequest::new(ViewType::Interconnection).with_expose(vehicle_id);
    let vm = sysml_diagram::to_view_model(&graph, &request);
    assert_eq!(vm.scene.nodes.len(), 1, "one IBD context block");
    let ctx_node = &vm.scene.nodes[0];
    assert_eq!(
        ctx_node.expanded,
        Some(true),
        "IBD context frame must render expanded — it exists to show internal structure"
    );
    let child_names: Vec<String> = ctx_node
        .children
        .iter()
        .filter_map(|c| match c {
            sysml_diagram::ir::types::DiagramChild::Node(n) => Some(n.name.clone()),
            _ => None,
        })
        .collect();
    assert!(
        child_names.iter().any(|n| n.contains("engine")),
        "internal part usages must be children of the context block; got {child_names:?}"
    );
}

#[test]
fn interconnection_constraint_exposes_render_constraint_blocks() {
    // A parametric diagram = Interconnection view exposing constraint defs.
    // EVERY expose contributes (two constraints here) and a constraint
    // subject renders as a constraint block — both were dropped before
    // (first-expose-only + no constraint arm → 0 nodes, D-B4).
    let mut graph = parse_sysml(
        "package P { \
            constraint def MaxPower { assert constraint { 1 <= 2 } } \
            constraint def Balance { assert constraint { 2 <= 3 } } \
        }",
    );
    sysml_core::elaborate::elaborate(&mut graph);

    let find = |name: &str| {
        graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("{name} present"))
            .id
            .clone()
    };
    let request = ViewRequest::new(ViewType::Interconnection)
        .with_exposes(vec![find("MaxPower"), find("Balance")]);
    let vm = sysml_diagram::to_view_model(&graph, &request);
    let names: Vec<_> = vm.scene.nodes.iter().map(|n| n.name.clone()).collect();
    assert!(
        names.iter().any(|n| n.contains("MaxPower")),
        "first exposed constraint must render; got {names:?}"
    );
    assert!(
        names.iter().any(|n| n.contains("Balance")),
        "SECOND exposed constraint must render too; got {names:?}"
    );
}

/// Collect every node + port id emitted in a scene (recursively).
fn scene_shape_ids(ir: &sysml_diagram::ir::types::DiagramIR) -> std::collections::HashSet<String> {
    use sysml_diagram::ir::types::{DiagramChild, DiagramNode, DiagramPort};
    fn ports(p: &DiagramPort, ids: &mut std::collections::HashSet<String>) {
        ids.insert(p.element_id.clone());
        for sp in &p.sub_ports {
            ports(sp, ids);
        }
    }
    fn walk(n: &DiagramNode, ids: &mut std::collections::HashSet<String>) {
        ids.insert(n.element_id.clone());
        for p in &n.ports {
            ports(p, ids);
        }
        for c in &n.children {
            match c {
                DiagramChild::Node(cn) => walk(cn, ids),
                DiagramChild::Island { subtree, expanded, .. } if *expanded => {
                    for sn in &subtree.nodes {
                        walk(sn, ids);
                    }
                }
                _ => {}
            }
        }
    }
    let mut ids = std::collections::HashSet::new();
    for n in &ir.nodes {
        walk(n, &mut ids);
    }
    ids
}

#[test]
fn mixed_expose_scene_is_self_consistent() {
    // MixedExposeView shape (D-B2): a General view exposing three elements
    // while an UNEXPOSED container (Vehicle) owns a connection between its
    // internal usages. The scene must never carry an edge whose endpoint is
    // not an emitted shape — that is a hard elk JsonImportException.
    let mut graph = parse_sysml(
        "package Showcase { \
            part def Engine { attribute power : Real; } \
            part def Gearbox { attribute ratio : Real; } \
            state def DriveStateMachine { state idle; state driving; } \
            part def Vehicle { \
                part engine : Engine; \
                part gearbox : Gearbox; \
                connect engine to gearbox; \
            } \
        }",
    );
    sysml_core::elaborate::elaborate(&mut graph);

    let find = |name: &str| {
        graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some(name))
            .unwrap_or_else(|| panic!("{name} present"))
            .id
            .clone()
    };
    let request = ViewRequest::new(ViewType::General).with_exposes(vec![
        find("Engine"),
        find("Gearbox"),
        find("DriveStateMachine"),
    ]);
    let vm = sysml_diagram::to_view_model(&graph, &request);
    let ids = scene_shape_ids(&vm.scene);

    let mut check = |edges: &[sysml_diagram::ir::types::DiagramEdge]| {
        for e in edges {
            assert!(
                ids.contains(&e.source_id),
                "edge {} ({}) references source {} not emitted as a shape",
                e.id, e.label, e.source_id
            );
            assert!(
                ids.contains(&e.target_id),
                "edge {} ({}) references target {} not emitted as a shape",
                e.id, e.label, e.target_id
            );
        }
    };
    check(&vm.scene.edges);
}

// ── Grammar defect #11: def-owned expose ─────────────────────────────────
//
// The pilot grammar (SysML.xtext) admits `Expose` only as a ViewBodyItem —
// view USAGE bodies; `ViewDefinitionBodyItem` has no Expose alternative. Our
// tree-sitter grammar admits expose_decl in every definition body, so
// `view def Bad { expose X; }` parses cleanly today AND elaboration carries
// the def-owned expose into `ViewSummary.exposed`. The grammar fix is
// scheduled with the next tree-sitter regen (rules/*.js are not edited
// out-of-band). An elaboration-side guard (def-owned expose contributes
// nothing) was assessed and DEFERRED: the example corpus
// (examples/view-showcase, examples/espresso-production-cell) authors
// def-owned expose throughout, so the guard is a corpus-wide behaviour
// change, not a contained fix.
//
// This test asserts the DESIRED shape and stays #[ignore]d until either fix
// lands (repo convention: contract_resolution_features_baseline.rs). When
// one does, drop the #[ignore] so the test pins it.
#[test]
#[ignore = "grammar defect #11: tree-sitter admits expose_decl in definition bodies; flips when the scheduled grammar regen (or an elaboration-side guard) makes def-owned expose fail or go inert"]
fn def_owned_expose_is_rejected_or_inert() {
    use sysml_parser_incremental::TreeSitterParser;
    use sysml_parser_trait::{Parser, SysmlFile};

    let src = "package P { part def X; view def Bad { expose X; } }";
    let parser = TreeSitterParser::new();
    let files = vec![SysmlFile::new("test.sysml", src)];
    let result = parser.parse(&files);

    let has_parse_error = result
        .diagnostics
        .iter()
        .any(|d| d.severity == sysml_span::Severity::Error);

    let mut graph = result.graph;
    sysml_core::elaborate::elaborate(&mut graph);
    let summaries = sysml_core::build_view_index(&graph);
    let bad = summaries
        .iter()
        .find(|s| s.name.as_deref() == Some("Bad"));
    let def_expose_inert = bad.map(|s| s.exposed.is_empty()).unwrap_or(true);

    assert!(
        has_parse_error || def_expose_inert,
        "def-owned expose must fail to parse or contribute nothing to \
         ViewSummary.exposed; got parse_error={has_parse_error}, \
         exposed={:?}",
        bad.map(|s| s.exposed.len())
    );
}
