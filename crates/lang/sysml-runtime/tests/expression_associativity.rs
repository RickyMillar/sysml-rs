//! Phase 2.5a: spec-correct operator associativity assertions.
//!
//! Per KerMLExpressions.xtext (lines 254-257, 263-266, 157-160, 236-238)
//! the `+`/`-`/`*`/`/`/`%`/`==`/`!=`/`<`/`>`/`<=`/`>=`/`and`/`or`/`xor`/`implies`/`??`
//! operators are **left-associative** (`X (op X)*` Kleene loop, Xtext
//! left-folds via tree rewriting).
//!
//! Per KMLExp:272-274, `**` and `^` (exponentiation) are **right-associative**
//! (right-recursive grammar rule).
//!
//! These shapes are observable when the operator is non-commutative
//! (`-`, `/`, `**`); for commutative operators the value is identical
//! either way but the spec still mandates the tree shape. We assert it
//! so that future refactors cannot silently regress.

use sysml_core::{ElementKind, ModelGraph};
use sysml_parser_incremental::TreeSitterParser;
use sysml_parser_trait::{Parser, SysmlFile};
use sysml_runtime::expressions::{compile_expression_ast, BinOp, ExprIR};

fn parse_constraint(expr: &str) -> Option<(ModelGraph, sysml_core::ElementId)> {
    let src = format!(
        r#"
        package P {{
            part def Holder {{
                attribute a: Real;
                attribute b: Real;
                attribute c: Real;
                attribute d: Real;
                attribute x: Boolean;
                attribute y: Boolean;
                attribute z: Boolean;
                constraint k {{ {expr} }}
            }}
        }}
    "#,
        expr = expr,
    );
    let parser = TreeSitterParser::new();
    let result = parser.parse(&[SysmlFile {
        path: "assoc.sysml".into(),
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

fn compile(expr: &str) -> ExprIR {
    let (graph, id) = parse_constraint(expr)
        .unwrap_or_else(|| panic!("failed to parse constraint body: {}", expr));
    let elem = graph.elements.get(&id).expect("constraint should exist");
    compile_expression_ast(elem, &graph)
        .unwrap_or_else(|e| panic!("compile_expression_ast({}) failed: {:?}", expr, e))
}

fn fr(name: &str) -> ExprIR {
    ExprIR::FeatureRef(name.into())
}

#[test]
fn additive_chain_is_left_associative() {
    // a - b - c → Sub(Sub(a, b), c)
    let ir = compile("a - b - c");
    assert_eq!(
        ir,
        ExprIR::BinaryOp {
            op: BinOp::Subtract,
            left: Box::new(ExprIR::BinaryOp {
                op: BinOp::Subtract,
                left: Box::new(fr("a")),
                right: Box::new(fr("b")),
            }),
            right: Box::new(fr("c")),
        }
    );

    // a + b + c → Add(Add(a, b), c)  — value identical to right-assoc but
    // shape spec-correct
    let ir = compile("a + b + c");
    assert_eq!(
        ir,
        ExprIR::BinaryOp {
            op: BinOp::Add,
            left: Box::new(ExprIR::BinaryOp {
                op: BinOp::Add,
                left: Box::new(fr("a")),
                right: Box::new(fr("b")),
            }),
            right: Box::new(fr("c")),
        }
    );
}

#[test]
fn multiplicative_chain_is_left_associative() {
    // a / b / c → Div(Div(a, b), c). Left-assoc ≠ right-assoc here:
    //   Left:  (a / b) / c
    //   Right: a / (b / c)
    // For a=8, b=4, c=2 → 1 vs 4. Spec demands left.
    let ir = compile("a / b / c");
    assert_eq!(
        ir,
        ExprIR::BinaryOp {
            op: BinOp::Divide,
            left: Box::new(ExprIR::BinaryOp {
                op: BinOp::Divide,
                left: Box::new(fr("a")),
                right: Box::new(fr("b")),
            }),
            right: Box::new(fr("c")),
        }
    );
}

#[test]
fn exponentiation_chain_is_right_associative() {
    // a ** b ** c → Pow(a, Pow(b, c))  per KMLExp:272-274 right-recursive rule.
    let ir = compile("a ** b ** c");
    assert_eq!(
        ir,
        ExprIR::BinaryOp {
            op: BinOp::Power,
            left: Box::new(fr("a")),
            right: Box::new(ExprIR::BinaryOp {
                op: BinOp::Power,
                left: Box::new(fr("b")),
                right: Box::new(fr("c")),
            }),
        }
    );
}

#[test]
fn logical_and_is_left_associative() {
    // x and y and z → And(And(x, y), z)
    let ir = compile("x and y and z");
    assert_eq!(
        ir,
        ExprIR::BinaryOp {
            op: BinOp::And,
            left: Box::new(ExprIR::BinaryOp {
                op: BinOp::And,
                left: Box::new(fr("x")),
                right: Box::new(fr("y")),
            }),
            right: Box::new(fr("z")),
        }
    );
}

#[test]
fn mixed_precedence_groups_correctly() {
    // a + b * c → Add(a, Mul(b, c))  precedence + within left-assoc
    let ir = compile("a + b * c");
    assert_eq!(
        ir,
        ExprIR::BinaryOp {
            op: BinOp::Add,
            left: Box::new(fr("a")),
            right: Box::new(ExprIR::BinaryOp {
                op: BinOp::Multiply,
                left: Box::new(fr("b")),
                right: Box::new(fr("c")),
            }),
        }
    );

    // a - b + c → Add(Sub(a, b), c) — left-assoc means - and + group L→R
    let ir = compile("a - b + c");
    assert_eq!(
        ir,
        ExprIR::BinaryOp {
            op: BinOp::Add,
            left: Box::new(ExprIR::BinaryOp {
                op: BinOp::Subtract,
                left: Box::new(fr("a")),
                right: Box::new(fr("b")),
            }),
            right: Box::new(fr("c")),
        }
    );
}
