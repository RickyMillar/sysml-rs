//! BrowserView integration tests for the SModel visualization pipeline.

mod smodel_common;
use smodel_common::*;

use sysml_diagram::smodel::ViewType;

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

// ═══════════════════════════════════════════════════════════════════════════
// C12/C13 — no "unnamed" leaks + deterministic source ordering (Round 2)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn c12_browser_transition_rows_synthesize_source_then_target() {
    let sg = generate(
        "package P { state def Cycle { state idle; state driving; transition first idle then driving; } }",
        ViewType::Browser,
        true,
    );
    let json = serde_json::to_string(&sg).unwrap();
    assert!(
        json.contains("idle \u{2192} driving"),
        "browser transition row must read `idle → driving`, got: {}",
        json
    );
    assert!(
        !json.contains("\u{00bb} unnamed"),
        "no browser row may read 'unnamed', got: {}",
        json
    );
}

#[test]
fn c13_browser_members_follow_source_declaration_order() {
    let src = "package P { part def Box { \
        attribute zeta : Real; \
        attribute alpha : Real; \
        attribute mid : Real; } }";
    let sg = generate(src, ViewType::Browser, true);
    let json = serde_json::to_string(&sg).unwrap();
    let iz = json.find("zeta").expect("zeta row");
    let ia = json.find("alpha").expect("alpha row");
    let im = json.find("mid").expect("mid row");
    assert!(
        iz < ia && ia < im,
        "browser rows must follow source order, got indices {} {} {}",
        iz,
        ia,
        im
    );
}

#[test]
fn c13_tree_model_children_follow_source_declaration_order() {
    let src = "package P { part def Box { \
        attribute zeta : Real; \
        attribute alpha : Real; \
        attribute mid : Real; } }";
    let mut graph = parse_sysml(src);
    sysml_core::elaborate::elaborate(&mut graph);
    let tree = sysml_diagram::tree::to_tree_model(&graph, None);
    // P → Box → [zeta, alpha, mid]
    let pkg = &tree.roots[0];
    let boxdef = &pkg.children[0];
    let labels: Vec<&str> = boxdef.children.iter().map(|c| c.label.as_str()).collect();
    assert_eq!(
        labels,
        vec!["zeta", "alpha", "mid"],
        "tree children must match source declaration order"
    );
}
