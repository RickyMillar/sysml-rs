//! Phase 2.5c: spec-aligned Tier 2 expression forms.
//!
//! Asserts that the parser emits the spec-correct ElementKind for forms
//! that were previously stubbed as a flat-text FeatureReferenceExpression.

use sysml_core::{ElementKind, ModelGraph};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::expressions::{compile_expression_ast, ExprIR};

fn parse_constraint(expr: &str) -> Option<(ModelGraph, sysml_core::ElementId)> {
    let src = format!(
        r#"
        package P {{
            part def Holder {{
                attribute arr: Real;
                attribute idx: Integer;
                attribute x: Real;
                constraint k {{ {expr} }}
            }}
        }}
    "#,
        expr = expr,
    );
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile {
        path: "spec.sysml".into(),
        text: src,
    }]);
    let graph = result.graph;
    let id = graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::ConstraintUsage)?
        .id
        .clone();
    Some((graph, id))
}

fn first_descendant_kind(
    graph: &ModelGraph,
    root: &sysml_core::ElementId,
    kind: ElementKind,
) -> bool {
    fn walk(graph: &ModelGraph, id: &sysml_core::ElementId, kind: &ElementKind) -> bool {
        for child in graph.children_of(id) {
            if child.kind == *kind {
                return true;
            }
            if walk(graph, &child.id, kind) {
                return true;
            }
        }
        false
    }
    walk(graph, root, &kind)
}

#[test]
fn index_emits_index_expression() {
    let (graph, id) = parse_constraint("arr#(idx)").expect("parse");
    assert!(
        first_descendant_kind(&graph, &id, ElementKind::IndexExpression),
        "expected IndexExpression element under the constraint"
    );

    let elem = graph.elements.get(&id).unwrap();
    let ir = compile_expression_ast(elem, &graph).expect("compile");
    assert!(
        matches!(ir, ExprIR::Index { .. }),
        "expected ExprIR::Index, got {:?}",
        ir
    );
}

#[test]
fn arrow_call_with_lambda_body_emits_invocation_with_body_parameter() {
    // Spec form (KMLExp:372-387): `items->select{in x; x > 0}`
    // → InvocationExpression(name="select")
    //    operand[0] = source items
    //    operand[1] = BodyExpression containing
    //                   BodyParameterMember (Feature "x") +
    //                   result expression `x > 0`
    let (graph, id) = parse_constraint("arr->forAll{in x; x > 0}").expect("parse");
    let invocation = graph
        .elements
        .values()
        .find(|e| {
            e.kind == ElementKind::InvocationExpression && e.name.as_deref() == Some("forAll")
        })
        .expect("expected InvocationExpression named forAll");

    // The binding parameter should appear somewhere under the InvocationExpression
    // as a Feature element with declaredName "x" and a marker prop.
    let has_binding = {
        fn walk(graph: &ModelGraph, id: &sysml_core::ElementId) -> bool {
            for child in graph.children_of(id) {
                if child.kind == ElementKind::Feature
                    && child.name.as_deref() == Some("x")
                    && child
                        .get_prop("isBodyParameter")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                {
                    return true;
                }
                if walk(graph, &child.id) {
                    return true;
                }
            }
            false
        }
        walk(&graph, &invocation.id)
    };
    assert!(
        has_binding,
        "expected a Feature(name=\"x\", isBodyParameter=true) under the InvocationExpression"
    );

    // Drop unused binding to keep clippy happy when `id` is not referenced.
    let _ = id;
}

#[test]
fn arrow_call_emits_invocation_expression() {
    // `items->forAll(items > 0)` per spec (KMLExp:308-319) is an
    // InvocationExpression with name="forAll", operand[0]=items,
    // operand[1]=arg expression.
    let (graph, id) = parse_constraint("items->forAll(arr > 0)")
        .or_else(|| parse_constraint("arr->forAll(arr > 0)"))
        .expect("parse `arr->forAll(...)`");
    assert!(
        first_descendant_kind(&graph, &id, ElementKind::InvocationExpression),
        "expected InvocationExpression for `arr->forAll(...)`"
    );

    let invocation = graph
        .elements
        .values()
        .find(|e| e.kind == ElementKind::InvocationExpression)
        .expect("InvocationExpression should exist");
    assert_eq!(
        invocation.name.as_deref(),
        Some("forAll"),
        "InvocationExpression.name should be `forAll`"
    );
}

#[test]
fn metadata_dot_emits_metadata_access_expression() {
    // `elem.metadata` per KMLExp:411-412 is a `MetadataAccessExpression`
    // (the bare BaseExpression form).
    let (graph, id) = parse_constraint("arr.metadata == arr").expect("parse `arr.metadata == arr`");
    assert!(
        first_descendant_kind(&graph, &id, ElementKind::MetadataAccessExpression),
        "expected MetadataAccessExpression for `arr.metadata`"
    );
}

#[test]
fn qualified_enum_is_feature_ref_not_string_literal() {
    // Per spec: `Type::Variant` is a qualified-name FeatureReferenceExpression.
    // Resolution to enum value happens at evaluation, not at compile.
    let (graph, id) = parse_constraint("x == ColorEnum::Red").expect("parse");
    let elem = graph.elements.get(&id).unwrap();
    let ir = compile_expression_ast(elem, &graph).expect("compile");
    // The right-hand side of `==` must be a FeatureRef, not a LiteralString.
    if let ExprIR::BinaryOp { right, .. } = &ir {
        assert!(
            matches!(right.as_ref(), ExprIR::FeatureRef(_)),
            "expected RHS to be FeatureRef, got {:?}",
            right
        );
        if let ExprIR::FeatureRef(name) = right.as_ref() {
            assert!(
                name.contains("::"),
                "expected qualified `::` name, got `{}`",
                name
            );
        }
    } else {
        panic!("expected BinaryOp at top of expression, got {:?}", ir);
    }
}
