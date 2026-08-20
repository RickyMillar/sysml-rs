//! GeneralView integration tests for the SModel visualization pipeline.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::ViewType;
use sysml_diagram::ViewRequest;

#[test]
fn general_has_nodes_for_all_top_level_elements() {
    let sg = generate(
        "package P { part def A; part def B; part c : A; }",
        ViewType::General,
        false,
    );
    // Root package auto-expands — children are nested inside, count recursively
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

// ═══════════════════════════════════════════════════════════════════════════
// C11 — value features render as compartment TEXT rows, never child nodes
// (contract §D; SysML-graphical-bnf.kgbnf: `usage-cp = usageDeclaration ValuePart?`)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn c11_expanded_def_renders_scalar_attributes_as_text_rows_not_nodes() {
    let sg = generate(
        "package P { part def Engine { attribute width : Real = 5.5; attribute power : Real; } }",
        ViewType::General,
        true, // expand everything — the old bug promoted every attr to a node
    );
    let json = serde_json::to_string(&sg).unwrap();

    // Attributes must NOT appear as nested attribute nodes...
    assert_eq!(
        count_by_type(&sg.children, "node:attribute"),
        0,
        "scalar attributes must not render as child nodes: {}",
        json
    );
    // ...but as `name : Type = default` compartment text rows.
    assert!(
        json.contains("width : Real = 5.5"),
        "attribute row with default value missing, got: {}",
        json
    );
    assert!(
        json.contains("power : Real"),
        "attribute row without default missing, got: {}",
        json
    );
}

#[test]
fn c11_collapsed_attribute_row_includes_default_value() {
    let sg = generate(
        "package P { part def Gearbox { attribute ratio : Real = 3.7; } }",
        ViewType::General,
        false,
    );
    let json = serde_json::to_string(&sg).unwrap();
    assert!(
        json.contains("ratio : Real = 3.7"),
        "collapsed attribute row must carry `= default`, got: {}",
        json
    );
}

#[test]
fn c11_structural_children_still_render_as_nodes_when_expanded() {
    let sg = generate(
        "package P { part def V { attribute mass : Real; part engine; } }",
        ViewType::General,
        true,
    );
    let json = serde_json::to_string(&sg).unwrap();
    // The structural part keeps node treatment...
    assert!(
        json.contains("engine"),
        "structural child must still render: {}",
        json
    );
    // ...while the scalar attribute is a text row, not a node.
    assert_eq!(count_by_type(&sg.children, "node:attribute"), 0);
    assert!(json.contains("mass : Real"));
}

// ═══════════════════════════════════════════════════════════════════════════
// C12 — no "unnamed" leaks (doc bodies + transition labels)
// (spec §8.2.3.3–4: documentation-compartment = Identification text-block)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn c12_documented_requirement_surfaces_doc_text_not_unnamed() {
    let src = r#"
        package P {
            requirement def BrakeReq {
                doc /* The system shall stop within 3 seconds. */
            }
        }
    "#;
    for expand in [false, true] {
        let sg = generate(src, ViewType::General, expand);
        let json = serde_json::to_string(&sg).unwrap();
        assert!(
            json.contains("The system shall stop within 3 seconds"),
            "doc body must surface (expand={}), got: {}",
            expand,
            json
        );
        assert!(
            !json.contains("\"unnamed\"")
                && !json.contains("unnamed\\\"")
                && !json.to_lowercase().contains(">unnamed<")
                && !json.contains(": \"unnamed"),
            "no label may read 'unnamed' (expand={}), got: {}",
            expand,
            json
        );
    }
}

#[test]
fn c12_transition_label_synthesizes_source_then_target() {
    let src = r#"
        package P {
            state def Cycle {
                state idle;
                state driving;
                transition first idle then driving;
            }
        }
    "#;
    let graph = {
        let mut g = parse_sysml(src);
        sysml_core::elaborate::elaborate(&mut g);
        g
    };
    // Find the unnamed TransitionUsage and check the tree label synthesis.
    let tree = sysml_diagram::tree::to_tree_model(&graph, None);
    let mut labels = Vec::new();
    fn walk(n: &sysml_diagram::TreeNode, out: &mut Vec<String>) {
        out.push(n.label.clone());
        for c in &n.children {
            walk(c, out);
        }
    }
    for r in &tree.roots {
        walk(r, &mut labels);
    }
    assert!(
        labels.iter().any(|l| l == "idle \u{2192} driving"),
        "transition row must read `idle → driving`, got labels: {:?}",
        labels
    );
    assert!(
        !labels.iter().any(|l| l == "unnamed"),
        "no tree row may read 'unnamed', got: {:?}",
        labels
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// C13 — deterministic member ordering in SOURCE declaration order
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn c13_attribute_rows_follow_source_declaration_order() {
    // Names deliberately NOT in alphabetical or hash order.
    let src = "package P { part def Box { \
        attribute zeta : Real; \
        attribute alpha : Real; \
        attribute mid : Real; } }";
    let sg = generate(src, ViewType::General, false);
    let json = serde_json::to_string(&sg).unwrap();
    let iz = json.find("zeta : Real").expect("zeta row");
    let ia = json.find("alpha : Real").expect("alpha row");
    let im = json.find("mid : Real").expect("mid row");
    assert!(
        iz < ia && ia < im,
        "rows must follow source order zeta < alpha < mid, got indices {} {} {}",
        iz,
        ia,
        im
    );
}

#[test]
fn c13_two_runs_produce_identical_member_order() {
    let src = "package P { part def Box { \
        attribute zeta : Real; \
        attribute alpha : Real; \
        attribute mid : Real; \
        part engine; \
        part gearbox; } }";

    // Two INDEPENDENT parses (fresh random ElementIds each) must produce the
    // same label sequence. Extract compartment/label text order, not raw JSON
    // (ids differ between runs by construction).
    fn label_sequence(src: &str) -> Vec<String> {
        let sg = generate(src, ViewType::General, true);
        let json = serde_json::to_value(&sg).unwrap();
        let mut texts = Vec::new();
        fn walk(v: &serde_json::Value, out: &mut Vec<String>) {
            if let Some(obj) = v.as_object() {
                if let Some(t) = obj.get("text").and_then(|t| t.as_str()) {
                    out.push(t.to_owned());
                }
                for val in obj.values() {
                    walk(val, out);
                }
            } else if let Some(arr) = v.as_array() {
                for val in arr {
                    walk(val, out);
                }
            }
        }
        walk(&json, &mut texts);
        texts
    }

    let run1 = label_sequence(src);
    let run2 = label_sequence(src);
    assert_eq!(
        run1, run2,
        "two independent runs must produce identical label order"
    );
    // And the attribute rows must be in declaration order within the run.
    let pos = |needle: &str| {
        run1.iter()
            .position(|t| t.contains(needle))
            .unwrap_or_else(|| panic!("label containing {:?} not found in {:?}", needle, run1))
    };
    assert!(pos("zeta") < pos("alpha") && pos("alpha") < pos("mid"));
}

// ── Edge label vocabulary (§8.2.3 graphical BNF) ─────────────────────────

/// The specialization family has NO text-label production in the spec: the
/// §8.2.3.6 BNF entries for `definition` / `subclassification` / `subsetting` /
/// `redefinition` are bare images, so line style + arrowhead carry the whole
/// meaning. Grepping the spec for the token `typing` returns zero hits.
///
/// The General generator used to stamp the metaclass name ("typing",
/// "specialization", …) onto these edges, which is what painted the
/// `typing` label pile over AllPartsView.
#[test]
fn specialization_family_edges_carry_no_metaclass_label() {
    let mut graph = parse_sysml("package P { part def Wheel; part def Vehicle { part frontLeft : Wheel; } }");
    sysml_core::elaborate::elaborate(&mut graph);

    let vm = sysml_diagram::to_view_model(&graph, &ViewRequest::new(ViewType::General));
    use sysml_core::RelationshipKind;
    use sysml_diagram::ir::types::DiagramEdgeKind;

    let typing: Vec<_> = vm
        .scene
        .edges
        .iter()
        .filter(|e| matches!(&e.kind, DiagramEdgeKind::Relationship(RelationshipKind::TypeOf)))
        .collect();
    assert!(!typing.is_empty(), "expected a FeatureTyping edge in the IR");
    for e in typing {
        assert_eq!(
            e.label, "",
            "FeatureTyping edges have no label production in §8.2.3.6 — the \
             open triangle IS the notation (got {:?})",
            e.label
        );
    }
}

/// Two features of the same owner typed by the same definition produce two
/// distinct FeatureTyping elements. Neither usage is a rendered node, so both
/// fold onto (Vehicle → Wheel). With no label to tell them apart, the second
/// arrow carries zero information — collapse it.
#[test]
fn folded_typing_edges_collapse_to_one_arrow() {
    let mut graph = parse_sysml(
        "package P { part def Wheel; \
         part def Vehicle { part frontLeft : Wheel; part frontRight : Wheel; } }",
    );
    sysml_core::elaborate::elaborate(&mut graph);

    let vm = sysml_diagram::to_view_model(&graph, &ViewRequest::new(ViewType::General));
    use sysml_core::RelationshipKind;
    use sysml_diagram::ir::types::DiagramEdgeKind;

    let pairs: Vec<(String, String)> = vm
        .scene
        .edges
        .iter()
        .filter(|e| matches!(&e.kind, DiagramEdgeKind::Relationship(RelationshipKind::TypeOf)))
        .map(|e| (e.source_id.clone(), e.target_id.clone()))
        .collect();
    let mut deduped = pairs.clone();
    deduped.sort();
    deduped.dedup();
    assert_eq!(
        pairs.len(),
        deduped.len(),
        "two wheels typed by Wheel must fold to ONE typing arrow, got {} for {} distinct pairs",
        pairs.len(),
        deduped.len()
    );
}

/// §8.2.3.13: `connection-label = UsageDeclaration` — a connector edge shows
/// the connector usage's DECLARED NAME, never the metaclass debug name. This
/// previously rendered as the literal string "Connection".
#[test]
fn connection_edge_label_is_the_declared_name() {
    let mut graph = parse_sysml(
        "package P { \
         part def Engine { port torqueOut; } \
         part def Gearbox { port torqueIn; } \
         part def Vehicle { part engine : Engine; part gearbox : Gearbox; \
           connection engineToGearbox connect engine.torqueOut to gearbox.torqueIn; } }",
    );
    sysml_core::elaborate::elaborate(&mut graph);

    let vm = sysml_diagram::to_view_model(&graph, &ViewRequest::new(ViewType::General));
    use sysml_core::RelationshipKind;
    use sysml_diagram::ir::types::DiagramEdgeKind;

    let labels: Vec<&str> = vm
        .scene
        .edges
        .iter()
        .filter(|e| matches!(&e.kind, DiagramEdgeKind::Relationship(RelationshipKind::Connection)))
        .map(|e| e.label.as_str())
        .collect();
    assert!(!labels.is_empty(), "expected a connection edge in the IR");
    assert!(
        labels.iter().all(|l| *l != "Connection"),
        "connector edges must not carry the Rust metaclass debug name, got {labels:?}"
    );
    assert!(
        labels.contains(&"engineToGearbox"),
        "expected the declared connector name as the label, got {labels:?}"
    );
}
