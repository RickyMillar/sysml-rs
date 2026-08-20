//! S1.T6 — reparse identity gate for the tree-sitter expression walker.
//!
//! Parses the same source twice through `ExpressionBuilder::process_with_key`
//! (the canonical-key path) and asserts that the IDs of every minted
//! expression element are identical across the two parses.
//!
//! This is the test that validates ADR-009's reparse-stability claim for
//! tree-sitter parsed expressions. It is the tree-sitter sibling of
//! S1.T5's batch-parser equivalent. While S1.T5/S1.T11 are still in flight
//! the public `ExpressionBuilder::process` (no parent_key) still produces
//! fresh UUIDs — that path remains tested by `expression_parity.rs`.

#![cfg(feature = "semantic")]

use std::collections::BTreeSet;

use sysml_core::{CanonicalKey, Element, ElementId, ElementKind, ModelGraph};
use sysml_parser_incremental::ExpressionBuilder;
use tree_sitter::{Node, Tree};

/// Parse `expr` once via `ExpressionBuilder::process_with_key`, returning the
/// set of element IDs minted under the synthetic holder element.
fn parse_collect_ids(expr: &str) -> BTreeSet<ElementId> {
    let src = format!("package T {{ constraint c {{ {expr} }} }}");

    let mut parser = tree_sitter::Parser::new();
    parser
        .set_language(&tree_sitter_sysml::language())
        .expect("set language");
    let tree: Tree = parser.parse(&src, None).expect("ts parse");

    let expr_node = find_first_kind(&tree.root_node(), "result_expression")
        .and_then(|n| n.named_child(0))
        .expect("result_expression with a child");

    // Build a fresh graph with a known canonical key for the holder so that
    // every descendant's ID is fully derived from the canonical key chain.
    let holder_key = CanonicalKey::for_named(
        &CanonicalKey::root("expression-id-stability-test"),
        "ConstraintUsage",
        "c",
    );
    let mut graph = ModelGraph::new();
    let holder = graph.add_element(Element::new_with_key(ElementKind::ConstraintUsage, &holder_key));

    let builder = ExpressionBuilder::new(&src, "test.sysml");
    let _root = builder
        .process_with_key(expr_node, holder.clone(), &holder_key, &mut graph)
        .expect("walker emits a root");

    // Collect every element id under the holder (including the root expr).
    fn collect(graph: &ModelGraph, root: &ElementId, out: &mut BTreeSet<ElementId>) {
        for child in graph.children_of(root) {
            out.insert(child.id.clone());
            collect(graph, &child.id, out);
        }
    }
    let mut ids = BTreeSet::new();
    collect(&graph, &holder, &mut ids);
    ids
}

fn find_first_kind<'t>(node: &Node<'t>, kind: &str) -> Option<Node<'t>> {
    if node.kind() == kind {
        return Some(*node);
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if let Some(f) = find_first_kind(&child, kind) {
            return Some(f);
        }
    }
    None
}

fn assert_id_stable(expr: &str) {
    let a = parse_collect_ids(expr);
    let b = parse_collect_ids(expr);
    assert!(
        !a.is_empty(),
        "expected at least one expression element for `{expr}` (got 0)",
    );
    assert_eq!(
        a, b,
        "reparse identity drifted for `{expr}`\n  parse A: {a:#?}\n  parse B: {b:#?}",
    );
}

// ---------------------------------------------------------------------------
// One stability test per parity fixture so the failure surface is granular.
// ---------------------------------------------------------------------------

#[test]
fn id_stable_integer_literal() {
    assert_id_stable("42");
}

#[test]
fn id_stable_real_literal() {
    assert_id_stable("3.14");
}

#[test]
fn id_stable_boolean_literal() {
    assert_id_stable("true");
}

#[test]
fn id_stable_feature_ref() {
    assert_id_stable("temp");
}

#[test]
fn id_stable_qualified_name() {
    assert_id_stable("Status::Active");
}

#[test]
fn id_stable_binary_add() {
    assert_id_stable("a + b");
}

#[test]
fn id_stable_binary_sub_left_assoc() {
    assert_id_stable("a - b - c");
}

#[test]
fn id_stable_mul_binds_tighter() {
    assert_id_stable("a + b * c");
}

#[test]
fn id_stable_comparison() {
    assert_id_stable("temp >= 90.0");
}

#[test]
fn id_stable_logical_and_chain() {
    assert_id_stable("temp >= 90 and temp <= 96");
}

#[test]
fn id_stable_unary_not() {
    assert_id_stable("not flag");
}

#[test]
fn id_stable_unary_neg_fold() {
    assert_id_stable("-15.0");
}

#[test]
fn id_stable_exponentiation_right_assoc() {
    assert_id_stable("a ** b ** c");
}

#[test]
fn id_stable_division() {
    assert_id_stable("a / b");
}

#[test]
fn id_stable_parenthesized() {
    assert_id_stable("(a + b) * c");
}

#[test]
fn id_stable_function_call() {
    assert_id_stable("sqrt(a + b)");
}

#[test]
fn id_stable_range() {
    assert_id_stable("1..10");
}

#[test]
fn id_stable_classification_at() {
    assert_id_stable("x @ Integer");
}

#[test]
fn id_stable_null_coalesce() {
    assert_id_stable("override ?? baseline");
}

#[test]
fn id_stable_if_cases() {
    assert_id_stable("if x > 0 ? 1 else 0");
}

/// Cross-fixture: structurally-distinct expressions (different element kinds
/// at root) produce disjoint ID sets.
///
/// Note: per ADR-009 the canonical key is `parent/kind[sibling_index]`, so
/// `a + b` and `a * b` legitimately share IDs (same kinds, same positions —
/// only the `operator` *property* differs, which is by design). To validate
/// that the canonical key actually encodes structure, contrast a literal
/// against a feature reference: different kinds → different IDs.
#[test]
fn id_stable_distinct_kinds_distinct_ids() {
    let lit = parse_collect_ids("42");
    let feat = parse_collect_ids("temp");
    let intersect: BTreeSet<_> = lit.intersection(&feat).collect();
    assert!(
        intersect.is_empty(),
        "expected disjoint IDs for distinct element kinds, but {} IDs collided",
        intersect.len()
    );
}
