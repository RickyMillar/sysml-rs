//! Expression compilation from ModelGraph elements and strings.
//!
//! This module provides two paths for compiling expressions:
//! 1. **AST-based compilation**: Walk parser-produced element children in the ModelGraph
//! 2. **String-based compilation**: Parse simple expression strings directly

#![allow(clippy::indexing_slicing)]
use super::ir::{BinOp, ExprIR, UnaryOp};
use sysml_core::{Element, ElementKind, ModelGraph, Value};
use sysml_span::Diagnostic;

/// Maximum recursion depth for expression compilation.
const MAX_COMPILE_DEPTH: usize = 128;

/// Compile an expression from a model element.
///
/// **Primary API for user model expressions.** Walks the element's children
/// in the ModelGraph to build an [`ExprIR`] from parser-produced expression
/// elements (OperatorExpression, FeatureReferenceExpression, LiteralInteger, …).
/// If the AST walk finds no compilable children, falls back to the legacy
/// `"expr"` or `"constraint"` string property via [`compile_simple_expression`].
/// Never falls back to `unresolved_value` — that path was removed in Phase 6D.
///
/// For runtime-generated expression strings that don't come from the model
/// graph, use [`compile_simple_expression`] directly instead.
pub fn compile_expression(
    element: &Element,
    graph: &ModelGraph,
) -> Result<ExprIR, Vec<Diagnostic>> {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        element_id = %element.id,
        element_kind = ?element.kind,
        element_name = element.name.as_deref().unwrap_or("<unnamed>"),
        "compiling expression from element"
    );

    // Path 1: AST-based compilation (primary path, Phase 6C).
    match compile_expression_ast(element, graph) {
        Ok(ir) => Ok(ir),
        Err(ast_err) => {
            // Path 2: legacy string-prop fallback for runtime-synthesized
            // expressions and hand-crafted test graphs. Only `expr` /
            // `constraint` string props are honored; never `unresolved_value`.
            let expr_str = element
                .get_prop("expr")
                .or_else(|| element.get_prop("constraint"));

            match expr_str {
                Some(Value::String(s)) => {
                    #[cfg(feature = "tracing")]
                    tracing::trace!(
                        element_id = %element.id,
                        expr_len = s.len(),
                        "AST walk empty; falling back to string expression property"
                    );
                    compile_simple_expression(s)
                }
                Some(_) => Err(vec![Diagnostic::error(format!(
                    "element `{}` has a non-string expression property",
                    element.name.as_deref().unwrap_or("<unnamed>")
                ))]),
                None => Err(ast_err),
            }
        }
    }
}

/// Compile an expression by walking parser-produced element children.
///
/// The pest parser creates literal elements (LiteralBoolean, LiteralInteger,
/// LiteralRational, LiteralString) with a `"value"` property, and expression
/// elements (OperatorExpression, FeatureReferenceExpression, NullExpression)
/// with `"operator"` and child argument elements.
///
/// This function recursively walks the element tree, mapping each expression
/// element to the corresponding [`ExprIR`] node.
pub fn compile_expression_ast(
    element: &Element,
    graph: &ModelGraph,
) -> Result<ExprIR, Vec<Diagnostic>> {
    compile_expression_ast_inner(element, graph, 0)
}

/// Internal AST compiler with depth tracking.
fn compile_expression_ast_inner(
    element: &Element,
    graph: &ModelGraph,
    depth: usize,
) -> Result<ExprIR, Vec<Diagnostic>> {
    if depth > MAX_COMPILE_DEPTH {
        return Err(vec![Diagnostic::error(format!(
            "expression AST too deeply nested (max depth: {})",
            MAX_COMPILE_DEPTH
        ))]);
    }

    #[cfg(feature = "tracing")]
    tracing::trace!(
        element_id = %element.id,
        element_kind = ?element.kind,
        "compiling expression from AST children"
    );

    // Check if the element itself is directly compilable
    if let Some(ir) = try_compile_element(element, graph, depth) {
        return ir;
    }

    // Walk children looking for expression elements
    for child in graph.children_of(&element.id) {
        if let Some(ir) = try_compile_element(child, graph, depth) {
            return ir;
        }
    }

    // Phase 2c: the AST path no longer falls back to `unresolved_value`
    // string parsing. The parser's `process_expression` covers the full
    // Tier 1 grammar (100% of the example corpus); if no compilable child
    // is found, that is a real model gap. Legacy callers that explicitly
    // need the string parser can still invoke `compile_simple_expression`
    // directly — e.g., for runtime-synthesized expression strings.
    Err(vec![Diagnostic::error(format!(
        "element `{}` has no compilable expression children",
        element.name.as_deref().unwrap_or("<unnamed>")
    ))])
}

/// Try to compile an element to ExprIR based on its kind.
/// Returns `Some(result)` if the element is an expression element, `None` otherwise.
fn try_compile_element(
    element: &Element,
    graph: &ModelGraph,
    depth: usize,
) -> Option<Result<ExprIR, Vec<Diagnostic>>> {
    // Literals
    if let Some(ir) = try_compile_literal(element) {
        return Some(Ok(ir));
    }

    match element.kind {
        // NullExpression -> LiteralNull
        ElementKind::NullExpression => Some(Ok(ExprIR::LiteralNull)),

        // FeatureReferenceExpression -> FeatureRef / FeatureChain / qualified-enum literal
        ElementKind::FeatureReferenceExpression => {
            let name = element.name.as_deref().or_else(|| {
                element
                    .get_prop("unresolved_reference")
                    .and_then(|v| v.as_str())
            });
            match name {
                Some(n) => {
                    // Per KerML spec (KMLExp:401-406, KVocab:194-196), `::` is
                    // the qualified-name separator. `Status::Active` is just a
                    // FeatureReferenceExpression whose memberElement resolves
                    // to the `Active` value declared in `Status`. Resolution
                    // happens at evaluation time, NOT here. Pass it through as
                    // a FeatureRef carrying the qualified name verbatim.
                    if n.contains('.') {
                        let parts: Vec<String> = n.split('.').map(String::from).collect();
                        Some(Ok(ExprIR::FeatureChain(parts)))
                    } else {
                        Some(Ok(ExprIR::FeatureRef(n.to_owned())))
                    }
                }
                None => {
                    // Check children for a name reference
                    for child in graph.children_of(&element.id) {
                        if let Some(n) = child.name.as_deref() {
                            return Some(Ok(ExprIR::FeatureRef(n.to_owned())));
                        }
                    }
                    None
                }
            }
        }

        // OperatorExpression -> BinaryOp, UnaryOp, or special forms
        ElementKind::OperatorExpression => Some(compile_operator_expression(element, graph, depth)),

        // Specialized operator expressions
        ElementKind::IndexExpression => Some(compile_index_expression(element, graph, depth)),
        ElementKind::SelectExpression => Some(compile_collection_ast_expression(
            element, graph, "select", depth,
        )),
        ElementKind::CollectExpression => Some(compile_collection_ast_expression(
            element, graph, "collect", depth,
        )),
        ElementKind::FeatureChainExpression => {
            Some(compile_feature_chain_expression(element, graph, depth))
        }

        // InvocationExpression -> FunctionCall
        ElementKind::InvocationExpression => {
            Some(compile_invocation_expression(element, graph, depth))
        }

        // MetadataAccessExpression -> MetaAccess (Tier 2)
        ElementKind::MetadataAccessExpression => {
            let args = collect_expression_children(element, graph, depth).ok()?;
            let is_double = element
                .get_prop("operator")
                .and_then(|v| v.as_str())
                .map(|op| op == "@@")
                .unwrap_or(false);
            let operand = args.into_iter().next().unwrap_or(ExprIR::LiteralNull);
            Some(Ok(ExprIR::MetaAccess {
                operand: Box::new(operand),
                is_double,
            }))
        }

        // ConstructorExpression (`new T(...)`) -> ConstructorCall (Tier 2)
        ElementKind::ConstructorExpression => {
            let type_name = element
                .name
                .clone()
                .or_else(|| {
                    element
                        .get_prop("type")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_owned())
                })
                .unwrap_or_else(|| "<unknown>".to_owned());
            // A-structured: collect the NAMED arguments (`field = value`),
            // keyed by feature name, so the evaluator builds a `Value::Map`
            // whose keys match the definition's features and a receiver guard
            // can read `payload.field`. Positional (unnamed) arguments bind by
            // the type's feature order — a documented follow-up; they are not
            // added to the map here (so they do not appear under invented keys).
            match collect_named_args(element, graph, depth) {
                Ok(named_args) => Some(Ok(ExprIR::ConstructorCall {
                    type_name,
                    named_args,
                })),
                Err(e) => Some(Err(e)),
            }
        }

        _ => None,
    }
}

/// Compile an OperatorExpression element.
/// Has an `"operator"` property and child argument expressions.
#[allow(clippy::expect_used)] // Expects are guarded by arity checks on args.len()
fn compile_operator_expression(
    element: &Element,
    graph: &ModelGraph,
    depth: usize,
) -> Result<ExprIR, Vec<Diagnostic>> {
    let Some(operator) = element.get_prop("operator").and_then(|v| v.as_str()) else {
        return Err(vec![Diagnostic::error(format!(
            "OperatorExpression `{}` has no operator property",
            element.name.as_deref().unwrap_or("<unnamed>")
        ))]);
    };

    // Collect argument children (expression elements that are children)
    let args = collect_expression_children(element, graph, depth)?;

    // Map operator + argument count to ExprIR
    match (operator, args.len()) {
        // N-ary sequence literal: `(a, b, c)` emits operator "," with all
        // operands flattened. The parser also emits this for single-element
        // trailing-comma sequences like `(x,)`.
        (",", _) => Ok(ExprIR::Sequence(args)),

        // Unary operators
        ("not", 1) | ("~", 1) => Ok(ExprIR::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(
                args.into_iter()
                    .next()
                    .expect("invariant: arity checked for 1 arg"),
            ),
        }),
        ("-", 1) => Ok(ExprIR::UnaryOp {
            op: UnaryOp::Negate,
            operand: Box::new(
                args.into_iter()
                    .next()
                    .expect("invariant: arity checked for 1 arg"),
            ),
        }),

        // Binary operators
        (op, 2) => {
            let mut iter = args.into_iter();
            let left = iter.next().expect("invariant: arity checked for 2 args");
            let right = iter.next().expect("invariant: arity checked for 2 args");
            let bin_op = match op {
                "+" => BinOp::Add,
                "-" => BinOp::Subtract,
                "*" => BinOp::Multiply,
                "/" => BinOp::Divide,
                "%" => BinOp::Remainder,
                "**" | "^" => BinOp::Power,
                "==" => BinOp::Equal,
                "!=" => BinOp::NotEqual,
                "===" => BinOp::ReferenceEqual,
                "!==" => BinOp::ReferenceNotEqual,
                "<" => BinOp::LessThan,
                "<=" => BinOp::LessEqual,
                ">" => BinOp::GreaterThan,
                ">=" => BinOp::GreaterEqual,
                "and" | "&" => BinOp::And,
                "or" | "|" => BinOp::Or,
                "xor" => BinOp::Xor,
                "implies" => BinOp::Implies,
                "hastype" => BinOp::HasType,
                "istype" => BinOp::IsType,
                "as" => BinOp::As,
                "meta" => BinOp::Meta,
                ".." => {
                    return Ok(ExprIR::Range {
                        lower: Box::new(left),
                        upper: Box::new(right),
                    });
                }
                "??" => {
                    return Ok(ExprIR::NullCoalescing {
                        expr: Box::new(left),
                        default: Box::new(right),
                    });
                }
                _ => {
                    return Err(vec![Diagnostic::error(format!(
                        "unsupported binary operator: `{}`",
                        op
                    ))]);
                }
            };
            Ok(ExprIR::BinaryOp {
                op: bin_op,
                left: Box::new(left),
                right: Box::new(right),
            })
        }

        // Conditional: if/? with 3 arguments (condition, then, else)
        ("if", 3) | ("?", 3) => {
            let mut iter = args.into_iter();
            let condition = iter.next().expect("invariant: arity checked for 3 args");
            let then_expr = iter.next().expect("invariant: arity checked for 3 args");
            let else_expr = iter.next().expect("invariant: arity checked for 3 args");
            Ok(ExprIR::Conditional {
                condition: Box::new(condition),
                then_expr: Box::new(then_expr),
                else_expr: Box::new(else_expr),
            })
        }

        _ => Err(vec![Diagnostic::error(format!(
            "unsupported operator `{}` with {} arguments",
            operator,
            args.len()
        ))]),
    }
}

/// Compile an IndexExpression (operator "#").
#[allow(clippy::expect_used)] // Expects are guarded by args.len() == 2 check
fn compile_index_expression(
    element: &Element,
    graph: &ModelGraph,
    depth: usize,
) -> Result<ExprIR, Vec<Diagnostic>> {
    let args = collect_expression_children(element, graph, depth)?;
    if args.len() == 2 {
        let mut iter = args.into_iter();
        let sequence = iter.next().expect("invariant: arity checked for 2 args");
        let index = iter.next().expect("invariant: arity checked for 2 args");
        Ok(ExprIR::Index {
            sequence: Box::new(sequence),
            index: Box::new(index),
        })
    } else {
        Err(vec![Diagnostic::error(format!(
            "IndexExpression expects 2 arguments, got {}",
            args.len()
        ))])
    }
}

/// Compile a SelectExpression or CollectExpression from AST.
#[allow(clippy::expect_used)] // Expects are guarded by args.len() >= 2 check
fn compile_collection_ast_expression(
    element: &Element,
    graph: &ModelGraph,
    kind: &str,
    depth: usize,
) -> Result<ExprIR, Vec<Diagnostic>> {
    let args = collect_expression_children(element, graph, depth)?;
    // Collection expressions have source as first arg, body as second
    if args.len() >= 2 {
        let mut iter = args.into_iter();
        let source = iter.next().expect("invariant: arity checked for >= 2 args");
        let body = iter.next().expect("invariant: arity checked for >= 2 args");
        // Try to get binding name from element properties
        let binding = element
            .get_prop("binding")
            .and_then(|v| v.as_str())
            .unwrap_or("it")
            .to_owned();
        match kind {
            "select" => Ok(ExprIR::Select {
                source: Box::new(source),
                binding,
                predicate: Box::new(body),
            }),
            "collect" => Ok(ExprIR::Collect {
                source: Box::new(source),
                binding,
                transform: Box::new(body),
            }),
            _ => Err(vec![Diagnostic::error(format!(
                "unsupported collection operation: `{}`",
                kind
            ))]),
        }
    } else {
        Err(vec![Diagnostic::error(format!(
            "collection expression expects at least 2 arguments, got {}",
            args.len()
        ))])
    }
}

/// Compile a FeatureChainExpression (operator ".").
#[allow(clippy::expect_used)] // Expects are guarded by length checks
fn compile_feature_chain_expression(
    element: &Element,
    graph: &ModelGraph,
    depth: usize,
) -> Result<ExprIR, Vec<Diagnostic>> {
    let args = collect_expression_children(element, graph, depth)?;
    // Flatten to chain of names if possible
    let mut parts = Vec::new();
    for arg in &args {
        match arg {
            ExprIR::FeatureRef(name) => parts.push(name.clone()),
            ExprIR::FeatureChain(chain) => parts.extend(chain.iter().cloned()),
            _ => {
                // Non-trivial chain element; can't flatten to simple names
                if args.len() == 1 {
                    return Ok(args
                        .into_iter()
                        .next()
                        .expect("invariant: single arg confirmed"));
                }
                return Err(vec![Diagnostic::error(
                    "FeatureChainExpression with non-name arguments".to_owned(),
                )]);
            }
        }
    }
    if parts.len() == 1 {
        Ok(ExprIR::FeatureRef(
            parts
                .into_iter()
                .next()
                .expect("invariant: single part confirmed"),
        ))
    } else {
        Ok(ExprIR::FeatureChain(parts))
    }
}

/// Compile an InvocationExpression (function call).
fn compile_invocation_expression(
    element: &Element,
    graph: &ModelGraph,
    depth: usize,
) -> Result<ExprIR, Vec<Diagnostic>> {
    let name = element
        .name
        .as_deref()
        .or_else(|| element.get_prop("function").and_then(|v| v.as_str()))
        .unwrap_or("<unknown>")
        .to_owned();
    let args = collect_expression_children(element, graph, depth)?;
    Ok(ExprIR::FunctionCall { name, args })
}

/// Collect child expression elements and recursively compile them.
///
/// `ModelGraph::children_of` iterates a `FxHashSet` whose order is
/// non-deterministic. To recover argument order, the parser tags each
/// emitted expression child with an `argIndex` integer property. We sort by
/// it before compiling. Children without `argIndex` (e.g., synthesized
/// elsewhere) sort last, in iteration order.
fn collect_expression_children(
    element: &Element,
    graph: &ModelGraph,
    depth: usize,
) -> Result<Vec<ExprIR>, Vec<Diagnostic>> {
    let mut tagged: Vec<(i64, &Element)> = Vec::new();
    for child in graph.children_of(&element.id) {
        if is_expression_element(&child.kind) {
            let idx = child
                .get_prop("argIndex")
                .and_then(|v| v.as_int())
                .unwrap_or(i64::MAX);
            tagged.push((idx, child));
        }
    }
    tagged.sort_by_key(|(idx, _)| *idx);

    let mut args = Vec::with_capacity(tagged.len());
    for (_, child) in tagged {
        let ir = compile_expression_ast_inner(child, graph, depth + 1)?;
        args.push(ir);
    }
    Ok(args)
}

/// Collect the NAMED arguments of an instantiation/constructor expression as
/// `(feature_name, value_ir)` pairs, ordered by `argIndex`. The parser stamps
/// `argName` on each named argument's projected value element (A-structured);
/// positional arguments carry no `argName` and are skipped here (binding them
/// by the type's feature order is a documented follow-up). The names become the
/// keys of the evaluated `Value::Map` payload.
fn collect_named_args(
    element: &Element,
    graph: &ModelGraph,
    depth: usize,
) -> Result<Vec<(String, Box<ExprIR>)>, Vec<Diagnostic>> {
    let mut tagged: Vec<(i64, &Element)> = Vec::new();
    for child in graph.children_of(&element.id) {
        if is_expression_element(&child.kind) {
            let idx = child
                .get_prop("argIndex")
                .and_then(|v| v.as_int())
                .unwrap_or(i64::MAX);
            tagged.push((idx, child));
        }
    }
    tagged.sort_by_key(|(idx, _)| *idx);

    let mut named = Vec::new();
    for (_, child) in tagged {
        let Some(name) = child.get_prop("argName").and_then(|v| v.as_str()) else {
            continue;
        };
        let ir = compile_expression_ast_inner(child, graph, depth + 1)?;
        named.push((name.to_owned(), Box::new(ir)));
    }
    Ok(named)
}

/// Check if an ElementKind is an expression element.
fn is_expression_element(kind: &ElementKind) -> bool {
    matches!(
        kind,
        ElementKind::LiteralBoolean
            | ElementKind::LiteralInteger
            | ElementKind::LiteralRational
            | ElementKind::LiteralString
            | ElementKind::LiteralInfinity
            | ElementKind::LiteralExpression
            | ElementKind::NullExpression
            | ElementKind::OperatorExpression
            | ElementKind::InvocationExpression
            | ElementKind::FeatureReferenceExpression
            | ElementKind::FeatureChainExpression
            | ElementKind::SelectExpression
            | ElementKind::CollectExpression
            | ElementKind::IndexExpression
            | ElementKind::MetadataAccessExpression
            | ElementKind::ConstructorExpression
    )
}

/// Build a [`ExprIR::LiteralQuantity`] from a numeric literal element that
/// carries a `unit` prop (parser fix D-5.0.5: `num [unit]` folds magnitude +
/// unit). Returns `None` when there is no unit prop or the unit is not in the
/// ISQ unit table — the caller then keeps the plain numeric literal (the unit
/// is dropped), so a `LiteralQuantity` only ever carries a known unit.
fn quantity_literal(element: &Element, value: f64) -> Option<ExprIR> {
    let unit_ref = element.get_prop("unit")?.as_str()?;
    // The parser may store a qualified name (`SI::mA`); the unit table is keyed
    // by the bare name (mirrors `infer_m_ref`).
    let bare = unit_ref.rsplit("::").next().unwrap_or(unit_ref);
    let entry = super::units::lookup_unit(bare)?;
    Some(ExprIR::LiteralQuantity {
        value,
        dimension: entry.dimension,
        unit: bare.to_owned(),
    })
}

/// Try to compile a literal element to an ExprIR.
fn try_compile_literal(element: &Element) -> Option<ExprIR> {
    match element.kind {
        ElementKind::LiteralBoolean => {
            let val = element.get_prop("value")?.as_bool()?;
            Some(ExprIR::LiteralBool(val))
        }
        ElementKind::LiteralInteger => {
            let val = element.get_prop("value")?.as_int()?;
            // `num [unit]` quantity literal (D-5.0.5): if the parser folded a
            // resolvable unit onto the literal, carry it as a quantity.
            if let Some(q) = quantity_literal(element, val as f64) {
                return Some(q);
            }
            Some(ExprIR::LiteralInt(val))
        }
        ElementKind::LiteralRational => {
            let val = element.get_prop("value")?.as_float()?;
            if let Some(q) = quantity_literal(element, val) {
                return Some(q);
            }
            Some(ExprIR::LiteralReal(val))
        }
        ElementKind::LiteralString => {
            let val = element.get_prop("value")?.as_str()?;
            Some(ExprIR::LiteralString(val.to_owned()))
        }
        ElementKind::LiteralInfinity => Some(ExprIR::LiteralReal(f64::INFINITY)),
        _ => None,
    }
}

/// **Synthesized Expression Parser** — compile a runtime-generated expression
/// string to an [`ExprIR`].
///
/// This is the designated API for compiling expression strings that are
/// synthesized at runtime (e.g., constraint IR display strings, ODE
/// derivative expressions, trade-study objective functions, state-machine
/// guard expressions). It is **not** intended for compiling user-facing
/// model expressions — those should go through [`compile_expression`] which
/// walks the AST subtree in the ModelGraph.
///
/// # When to use this vs `compile_expression`
///
/// - **User model expressions**: Use [`compile_expression(element, graph)`]
/// - **Runtime-generated strings**: Use this function
/// - **Test helpers**: Use this function for concise expression construction
///
/// Supported patterns:
/// - Binary operators: `var < num`, `var > num`, `var == num`, `var + num`, ...
/// - Boolean/integer/float/string literals
/// - Variable references and feature chains: `speed`, `vehicle.speed`
/// - Parenthesized expressions: `(x + y) * 2`
/// - Function calls: `size(items)`, `abs(x - y)`, `sqrt(x + y)`
/// - Conditional: `if flag ? x else y`
/// - Collection operations: `items->select{|x| x > 0}`, `items->collect{|it| it + 1}`
/// - Range: `start..end`
/// - Index: `arr#(idx)`
/// - Type operations: `x hastype Integer`, `x istype Real`, `x as Boolean`
/// - Unary: `not x`, `-x`
/// - Null coalescing: `x ?? default`
pub fn compile_simple_expression(expr: &str) -> Result<ExprIR, Vec<Diagnostic>> {
    compile_simple_expression_inner(expr, 0)
}

/// Internal string expression compiler with depth tracking.
#[allow(clippy::indexing_slicing)] // String slicing with bounds checks throughout
fn compile_simple_expression_inner(expr: &str, depth: usize) -> Result<ExprIR, Vec<Diagnostic>> {
    if depth > MAX_COMPILE_DEPTH {
        return Err(vec![Diagnostic::error(format!(
            "expression too deeply nested (max depth: {})",
            MAX_COMPILE_DEPTH
        ))]);
    }

    let expr = expr.trim();
    #[cfg(feature = "tracing")]
    tracing::trace!(expr_len = expr.len(), "compiling simple expression");

    // Boolean literals
    if expr == "true" {
        return Ok(ExprIR::LiteralBool(true));
    }
    if expr == "false" {
        return Ok(ExprIR::LiteralBool(false));
    }

    // Null
    if expr == "null" {
        return Ok(ExprIR::LiteralNull);
    }

    // Integer literal
    if let Ok(n) = expr.parse::<i64>() {
        return Ok(ExprIR::LiteralInt(n));
    }

    // Float literal
    if let Ok(f) = expr.parse::<f64>() {
        return Ok(ExprIR::LiteralReal(f));
    }

    // String literal
    if expr.starts_with('"') && expr.ends_with('"') && expr.len() >= 2 {
        return Ok(ExprIR::LiteralString(expr[1..expr.len() - 1].to_owned()));
    }

    // Parenthesized expression: strip outer parens if they wrap the entire expr
    if expr.starts_with('(')
        && expr.ends_with(')')
        && matching_close_paren(expr, 0) == Some(expr.len() - 1)
    {
        return compile_simple_expression_inner(&expr[1..expr.len() - 1], depth + 1);
    }

    // Conditional: if condition ? then_expr else else_expr
    if let Some(rest) = expr.strip_prefix("if ") {
        if let Some(cond_end) = find_operator_outside_parens_and_braces(rest, "?") {
            let condition_str = rest[..cond_end].trim();
            let after_q = &rest[cond_end + 1..];
            if let Some(else_pos) = find_operator_outside_parens_and_braces(after_q, " else ") {
                let then_str = after_q[..else_pos].trim();
                let else_str = after_q[else_pos + 6..].trim();
                if !condition_str.is_empty() && !then_str.is_empty() && !else_str.is_empty() {
                    let condition = compile_simple_expression_inner(condition_str, depth + 1)?;
                    let then_expr = compile_simple_expression_inner(then_str, depth + 1)?;
                    let else_expr = compile_simple_expression_inner(else_str, depth + 1)?;
                    return Ok(ExprIR::Conditional {
                        condition: Box::new(condition),
                        then_expr: Box::new(then_expr),
                        else_expr: Box::new(else_expr),
                    });
                }
            }
        }
    }

    // Null coalescing: expr ?? default (lowest precedence after conditional).
    // Left-associative per KerMLExpressions.xtext `NullCoalescingExpression`
    // (`ImpliesExpression (op ImpliesExpression)*`), so split at the rightmost
    // top-level `??` to build `(a ?? b) ?? c`.
    if let Some(pos) = rfind_operator_outside_parens_and_braces(expr, "??") {
        let left = expr[..pos].trim();
        let right = expr[pos + 2..].trim();
        if !left.is_empty() && !right.is_empty() {
            let expr_ir = compile_simple_expression_inner(left, depth + 1)?;
            let default_ir = compile_simple_expression_inner(right, depth + 1)?;
            return Ok(ExprIR::NullCoalescing {
                expr: Box::new(expr_ir),
                default: Box::new(default_ir),
            });
        }
    }

    // Binary operators are handled below via `find_binary_split`, which walks
    // the precedence classes (lowest → highest) and honours per-class
    // associativity. Postfix operators (highest precedence): ->, #( must be checked before
    // binary operators so that `-` in `->` is not mistaken for subtraction.

    // Collection operations: source->method{|binding| body}
    if let Some(arrow_pos) = find_operator_outside_parens_and_braces(expr, "->") {
        let source_str = expr[..arrow_pos].trim();
        let after_arrow = expr[arrow_pos + 2..].trim();
        if !source_str.is_empty() && !after_arrow.is_empty() {
            if let Some(brace_start) = after_arrow.find('{') {
                let method = after_arrow[..brace_start].trim();
                let brace_body = after_arrow[brace_start..].trim();
                if brace_body.ends_with('}') {
                    let inner = brace_body[1..brace_body.len() - 1].trim();
                    // Parse |binding| body
                    if let Some(rest) = inner.strip_prefix('|') {
                        if let Some(pipe_end) = rest.find('|') {
                            let binding = rest[..pipe_end].trim().to_owned();
                            let body_str = rest[pipe_end + 1..].trim();
                            if !binding.is_empty() && !body_str.is_empty() {
                                let source =
                                    compile_simple_expression_inner(source_str, depth + 1)?;
                                let body = compile_simple_expression_inner(body_str, depth + 1)?;
                                return match method {
                                    "select" => Ok(ExprIR::Select {
                                        source: Box::new(source),
                                        binding,
                                        predicate: Box::new(body),
                                    }),
                                    "collect" => Ok(ExprIR::Collect {
                                        source: Box::new(source),
                                        binding,
                                        transform: Box::new(body),
                                    }),
                                    "reject" => Ok(ExprIR::Reject {
                                        source: Box::new(source),
                                        binding,
                                        predicate: Box::new(body),
                                    }),
                                    "forAll" => Ok(ExprIR::ForAll {
                                        source: Box::new(source),
                                        binding,
                                        predicate: Box::new(body),
                                    }),
                                    "exists" => Ok(ExprIR::Exists {
                                        source: Box::new(source),
                                        binding,
                                        predicate: Box::new(body),
                                    }),
                                    _ => Err(vec![Diagnostic::error(format!(
                                        "unknown collection operation: `{}`",
                                        method
                                    ))]),
                                };
                            }
                        }
                    }
                }
            }
        }
    }

    // Indexing: sequence#(index)
    if let Some(hash_pos) = find_operator_outside_parens_and_braces(expr, "#(") {
        let seq_str = expr[..hash_pos].trim();
        let after_hash = &expr[hash_pos + 1..]; // skip '#', keep '('
        if !seq_str.is_empty() && after_hash.starts_with('(') {
            if let Some(close) = matching_close_paren(after_hash, 0) {
                let index_str = after_hash[1..close].trim();
                if !index_str.is_empty() {
                    let sequence = compile_simple_expression_inner(seq_str, depth + 1)?;
                    let index = compile_simple_expression_inner(index_str, depth + 1)?;
                    return Ok(ExprIR::Index {
                        sequence: Box::new(sequence),
                        index: Box::new(index),
                    });
                }
            }
        }
    }

    // Range: lower..upper (between comparison and additive in KerML precedence)
    // Check before binary operators so `..` isn't parsed as two `.` tokens.
    if let Some(pos) = find_operator_outside_parens_and_braces(expr, "..") {
        let left = expr[..pos].trim();
        let right = expr[pos + 2..].trim();
        if !left.is_empty() && !right.is_empty() {
            let lower = compile_simple_expression_inner(left, depth + 1)?;
            let upper = compile_simple_expression_inner(right, depth + 1)?;
            return Ok(ExprIR::Range {
                lower: Box::new(lower),
                upper: Box::new(upper),
            });
        }
    }

    // Split at the lowest-precedence top-level binary operator, honouring
    // per-class associativity (KerMLExpressions.xtext). See `find_binary_split`.
    if let Some((pos, op_str, bin_op)) = find_binary_split(expr) {
        let left = expr[..pos].trim();
        let right = expr[pos + op_str.len()..].trim();
        if !left.is_empty() && !right.is_empty() {
            let left_ir = compile_simple_expression_inner(left, depth + 1)?;
            let right_ir = compile_simple_expression_inner(right, depth + 1)?;
            return Ok(ExprIR::BinaryOp {
                op: bin_op,
                left: Box::new(left_ir),
                right: Box::new(right_ir),
            });
        }
    }

    // Unary not
    if let Some(rest) = expr.strip_prefix("not ") {
        let operand = compile_simple_expression_inner(rest, depth + 1)?;
        return Ok(ExprIR::UnaryOp {
            op: UnaryOp::Not,
            operand: Box::new(operand),
        });
    }

    // Metadata access: @@ (check before single @)
    if let Some(rest) = expr.strip_prefix("@@") {
        let operand = compile_simple_expression_inner(rest.trim(), depth + 1)?;
        return Ok(ExprIR::MetaAccess {
            operand: Box::new(operand),
            is_double: true,
        });
    }
    if let Some(rest) = expr.strip_prefix('@') {
        let operand = compile_simple_expression_inner(rest.trim(), depth + 1)?;
        return Ok(ExprIR::MetaAccess {
            operand: Box::new(operand),
            is_double: false,
        });
    }

    // Unary negation
    if expr.starts_with('-') && expr.len() > 1 {
        let operand = compile_simple_expression_inner(&expr[1..], depth + 1)?;
        return Ok(ExprIR::UnaryOp {
            op: UnaryOp::Negate,
            operand: Box::new(operand),
        });
    }

    // Function call: name(args)
    if let Some(paren_start) = expr.find('(') {
        let name = expr[..paren_start].trim();
        if !name.is_empty()
            && name.chars().all(|c| c.is_alphanumeric() || c == '_')
            && expr.ends_with(')')
        {
            let inner = &expr[paren_start + 1..expr.len() - 1];
            let args = split_args(inner, depth)?;
            return Ok(ExprIR::FunctionCall {
                name: name.to_owned(),
                args,
            });
        }
    }

    // Qualified enum reference: `TypeName::Variant` → enum literal string
    if let Some((_, variant)) = expr.split_once("::") {
        if variant.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Ok(ExprIR::LiteralString(variant.to_owned()));
        }
    }

    // Simple identifier (variable reference)
    if expr
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '.')
    {
        if expr.contains('.') {
            let parts: Vec<String> = expr.split('.').map(String::from).collect();
            return Ok(ExprIR::FeatureChain(parts));
        }
        return Ok(ExprIR::FeatureRef(expr.to_owned()));
    }

    #[cfg(feature = "tracing")]
    tracing::debug!(expr = expr, "simple expression parse failed");
    Err(vec![Diagnostic::error(format!(
        "cannot parse expression: `{}`",
        expr
    ))])
}

/// Find the position of the closing `)` that matches the `(` at `open_pos`.
/// Returns `None` if parens are unbalanced.
#[allow(clippy::indexing_slicing)] // Byte indexing within loop bounds
fn matching_close_paren(s: &str, open_pos: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth = 0;
    for (i, &byte) in bytes.iter().enumerate().skip(open_pos) {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Find the first occurrence of `op` in `s` that is not inside parentheses or braces.
#[allow(clippy::indexing_slicing)] // Byte indexing with bounds checks
fn find_operator_outside_parens_and_braces(s: &str, op: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let op_bytes = op.as_bytes();
    let mut paren_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => paren_depth += 1,
            b')' => paren_depth -= 1,
            b'{' => brace_depth += 1,
            b'}' => brace_depth -= 1,
            _ => {}
        }
        if paren_depth == 0
            && brace_depth == 0
            && i + op_bytes.len() <= bytes.len()
            && &bytes[i..i + op_bytes.len()] == op_bytes
        {
            return Some(i);
        }
        i += 1;
    }
    None
}

/// Like [`find_operator_outside_parens_and_braces`] but returns the LAST
/// (rightmost) top-level occurrence. Used for left-associative operators so
/// the rightmost operator becomes the tree root (`a op b op c` => `(a op b) op c`).
#[allow(clippy::indexing_slicing)] // Byte indexing with bounds checks
fn rfind_operator_outside_parens_and_braces(s: &str, op: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let op_bytes = op.as_bytes();
    let mut paren_depth: i32 = 0;
    let mut brace_depth: i32 = 0;
    let mut found = None;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => paren_depth += 1,
            b')' => paren_depth -= 1,
            b'{' => brace_depth += 1,
            b'}' => brace_depth -= 1,
            _ => {}
        }
        if paren_depth == 0
            && brace_depth == 0
            && i + op_bytes.len() <= bytes.len()
            && &bytes[i..i + op_bytes.len()] == op_bytes
        {
            found = Some(i);
            i += op_bytes.len();
            continue;
        }
        i += 1;
    }
    found
}

/// Associativity of a binary-operator precedence class.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Assoc {
    Left,
    Right,
}

/// Binary-operator precedence classes, ordered from LOWEST to HIGHEST binding.
/// Operators within a class are listed longest-match-first (so `<=` is matched
/// before `<`, `!==`/`===` before `!=`/`==`).
///
/// Precedence and associativity follow `KerMLExpressions.xtext`:
/// `ImpliesExpression`, `OrExpression`, `XorExpression`, `AndExpression`,
/// `EqualityExpression`, `RelationalExpression`, `AdditiveExpression` and
/// `MultiplicativeExpression` all have the shape `head (op operand)*` and are
/// therefore LEFT-associative. `ExponentiationExpression` has the shape
/// `head (op ExponentiationExpression)?` — its right operand recurses into the
/// same rule, making it RIGHT-associative — and it is the operand of
/// `MultiplicativeExpression`, so it binds TIGHTER than `*`/`/`/`%`.
/// (`ClassificationExpression` operators do not chain; they are listed for
/// completeness with left association, which is a no-op for a single operator.)
#[allow(clippy::type_complexity)]
const PRECEDENCE_CLASSES: &[(&[(&str, BinOp)], Assoc)] = &[
    (&[("implies", BinOp::Implies)], Assoc::Left),
    (&[(" or ", BinOp::Or)], Assoc::Left),
    (&[(" xor ", BinOp::Xor)], Assoc::Left),
    (&[(" and ", BinOp::And)], Assoc::Left),
    (
        &[
            ("!==", BinOp::ReferenceNotEqual),
            ("===", BinOp::ReferenceEqual),
            ("!=", BinOp::NotEqual),
            ("==", BinOp::Equal),
        ],
        Assoc::Left,
    ),
    (
        &[
            ("<=", BinOp::LessEqual),
            (">=", BinOp::GreaterEqual),
            ("<", BinOp::LessThan),
            (">", BinOp::GreaterThan),
        ],
        Assoc::Left,
    ),
    (
        &[
            (" as ", BinOp::As),
            (" meta ", BinOp::Meta),
            (" hastype ", BinOp::HasType),
            (" istype ", BinOp::IsType),
        ],
        Assoc::Left,
    ),
    (&[("+", BinOp::Add), ("-", BinOp::Subtract)], Assoc::Left),
    (
        &[
            ("*", BinOp::Multiply),
            ("/", BinOp::Divide),
            ("%", BinOp::Remainder),
        ],
        Assoc::Left,
    ),
    // KerML `ExponentiationOperator : '**' | '^'` (KerMLExpressions.xtext:277).
    // Both spellings are the SAME operator at the SAME precedence, so they
    // belong in one class; `^` was missing here while the AST-driven compiler
    // has always mapped it (see the `"**" | "^" => BinOp::Power` arm above),
    // which left the two expression front-ends disagreeing on a spelling the
    // grammar treats as interchangeable.
    (
        &[("**", BinOp::Power), ("^", BinOp::Power)],
        Assoc::Right,
    ),
];

/// Locate the split point for the lowest-precedence top-level binary operator
/// in `expr`, honouring per-class associativity: LEFT-associative classes split
/// at the RIGHTMOST occurrence (`a - b - c` => `(a - b) - c`), RIGHT-associative
/// classes (exponentiation) at the LEFTMOST (`a ** b ** c` => `a ** (b ** c)`).
/// Returns `(byte_pos, op_str, op)`.
///
/// Operators inside parentheses/brackets/braces are skipped, as are `+`/`-` in
/// unary position and the `*` characters of a `**` token.
#[allow(clippy::indexing_slicing)] // Byte indexing with bounds checks
fn find_binary_split(expr: &str) -> Option<(usize, &'static str, BinOp)> {
    let bytes = expr.as_bytes();
    for (ops, assoc) in PRECEDENCE_CLASSES {
        let mut chosen: Option<(usize, &'static str, BinOp)> = None;
        let mut depth: i32 = 0;
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'(' | b'[' | b'{' => depth += 1,
                b')' | b']' | b'}' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                if let Some((op_str, bin_op)) = match_class_operator(expr, i, ops) {
                    // A binary split needs an operand on each side.
                    if i > 0 && i + op_str.len() < bytes.len() {
                        match assoc {
                            Assoc::Left => chosen = Some((i, op_str, bin_op)),
                            Assoc::Right => {
                                if chosen.is_none() {
                                    chosen = Some((i, op_str, bin_op));
                                }
                            }
                        }
                    }
                    // Skip past the matched operator so its bytes are not
                    // re-examined (e.g. the second `*` of a `**` token).
                    i += op_str.len();
                    continue;
                }
            }
            i += 1;
        }
        if chosen.is_some() {
            return chosen;
        }
    }
    None
}

/// Try to match one of `ops` (longest-first) at byte `pos` in `expr`, applying
/// per-operator validity guards.
#[allow(clippy::indexing_slicing)] // Byte indexing with bounds checks
fn match_class_operator(
    expr: &str,
    pos: usize,
    ops: &'static [(&'static str, BinOp)],
) -> Option<(&'static str, BinOp)> {
    let bytes = expr.as_bytes();
    for (op_str, bin_op) in ops {
        let ob = op_str.as_bytes();
        if pos + ob.len() <= bytes.len()
            && &bytes[pos..pos + ob.len()] == ob
            && operator_is_binary_at(expr, pos, op_str)
        {
            return Some((op_str, *bin_op));
        }
    }
    None
}

/// Reject operator matches that are not genuine binary operators at `pos`:
/// a `+`/`-` in unary position (start of expression or following another
/// operator), and a `*` that is really part of a `**` exponentiation token.
#[allow(clippy::indexing_slicing)] // Byte indexing with bounds checks
fn operator_is_binary_at(expr: &str, pos: usize, op_str: &str) -> bool {
    let bytes = expr.as_bytes();
    match op_str {
        "+" | "-" => {
            // Binary only if the preceding non-whitespace byte ends an operand.
            let mut j = pos;
            while j > 0 {
                j -= 1;
                match bytes[j] {
                    b' ' | b'\t' => continue,
                    b')' | b']' | b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'_' | b'.'
                    | b'"' => return true,
                    _ => return false, // follows another operator => unary
                }
            }
            false // nothing to the left => unary
        }
        "*" => {
            // Not one of the `*` bytes of a `**` token.
            let next_is_star = bytes.get(pos + 1) == Some(&b'*');
            let prev_is_star = pos > 0 && bytes[pos - 1] == b'*';
            !next_is_star && !prev_is_star
        }
        _ => true,
    }
}

/// Split a comma-separated argument list, respecting parentheses.
#[allow(clippy::indexing_slicing)] // Byte indexing within loop bounds
fn split_args(s: &str, compile_depth: usize) -> Result<Vec<ExprIR>, Vec<Diagnostic>> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let mut args = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    let bytes = trimmed.as_bytes();
    for i in 0..bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b',' if depth == 0 => {
                args.push(compile_simple_expression_inner(
                    &trimmed[start..i],
                    compile_depth + 1,
                )?);
                start = i + 1;
            }
            _ => {}
        }
    }
    args.push(compile_simple_expression_inner(
        &trimmed[start..],
        compile_depth + 1,
    )?);
    Ok(args)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod quantity_literal_tests {
    use super::*;
    use crate::expressions::{EvalContext, ExpressionEvaluator};
    use sysml_core::physics::DimensionVector;

    fn dim_current() -> DimensionVector {
        DimensionVector::new(0, 0, 0, 1, 0, 0, 0)
    }

    /// A `num [unit]` literal element (parser fix D-5.0.5 stores value + unit
    /// props) compiles to a unit-bearing `LiteralQuantity` and evaluates to a
    /// `Value::Quantity` carrying the resolved dimension/unit.
    #[test]
    fn unit_literal_compiles_and_evaluates_to_quantity() {
        let element = Element::new_with_kind(ElementKind::LiteralInteger)
            .with_prop("value", Value::Int(5))
            .with_prop("unit", Value::String("SI::mA".into()));

        let ir = try_compile_literal(&element).expect("literal compiles");
        match &ir {
            ExprIR::LiteralQuantity {
                value,
                dimension,
                unit,
            } => {
                assert_eq!(*value, 5.0);
                assert_eq!(*dimension, dim_current());
                assert_eq!(unit, "mA", "qualified name reduced to the bare unit");
            }
            other => panic!("expected LiteralQuantity, got {other:?}"),
        }

        let ev = ExpressionEvaluator::new();
        match ev.eval(&ir, &EvalContext::new()).expect("evaluates") {
            Value::Quantity {
                value,
                dimension,
                unit,
            } => {
                assert_eq!(value, 5.0);
                assert_eq!(dimension, dim_current());
                assert_eq!(unit.as_deref(), Some("mA"));
            }
            other => panic!("expected Quantity, got {other:?}"),
        }
    }

    /// An unknown unit is NOT carried — the literal stays a plain numeric
    /// (byte-identical to the no-unit case).
    #[test]
    fn unknown_unit_falls_back_to_plain_literal() {
        let element = Element::new_with_kind(ElementKind::LiteralRational)
            .with_prop("value", Value::Float(1.5))
            .with_prop("unit", Value::String("floops".into()));
        assert_eq!(
            try_compile_literal(&element),
            Some(ExprIR::LiteralReal(1.5)),
            "unknown unit drops to a plain real literal"
        );
    }

    /// No unit prop ⇒ plain numeric literal (the common case).
    #[test]
    fn no_unit_is_plain_literal() {
        let element =
            Element::new_with_kind(ElementKind::LiteralInteger).with_prop("value", Value::Int(7));
        assert_eq!(try_compile_literal(&element), Some(ExprIR::LiteralInt(7)));
    }
}

/// Operator associativity + precedence for the string expression compiler.
///
/// Spec: `KerMLExpressions.xtext`. `AdditiveExpression`,
/// `MultiplicativeExpression`, `RelationalExpression`, `EqualityExpression`,
/// `And`/`Or`/`Xor`/`ImpliesExpression` and `NullCoalescingExpression` all have
/// the shape `head (op operand)*` and are LEFT-associative;
/// `ExponentiationExpression` is `head (op ExponentiationExpression)?` — RIGHT-
/// associative — and binds tighter than `MultiplicativeExpression`.
#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod associativity_tests {
    use super::*;
    use crate::expressions::{EvalContext, ExpressionEvaluator};

    fn eval_f(expr: &str) -> f64 {
        let ir = compile_simple_expression(expr).expect("compiles");
        let ev = ExpressionEvaluator::new();
        match ev.eval(&ir, &EvalContext::new()).expect("evaluates") {
            Value::Float(f) => f,
            Value::Int(n) => n as f64,
            other => panic!("expected numeric, got {other:?}"),
        }
    }

    fn eval_b(expr: &str) -> bool {
        let ir = compile_simple_expression(expr).expect("compiles");
        let ev = ExpressionEvaluator::new();
        match ev.eval(&ir, &EvalContext::new()).expect("evaluates") {
            Value::Bool(b) => b,
            other => panic!("expected bool, got {other:?}"),
        }
    }

    // --- Additive: LEFT-associative (KerML AdditiveExpression) -------------

    #[test]
    fn subtraction_is_left_associative() {
        // (10 - 2) - 3 = 5, NOT 10 - (2 - 3) = 11
        assert_eq!(eval_f("10.0 - 2.0 - 3.0"), 5.0);
    }

    #[test]
    fn subtraction_chain_four_terms_left_associative() {
        // ((20 - 5) - 3) - 2 = 10, NOT 20 - (5 - (3 - 2)) = 16
        assert_eq!(eval_f("20.0 - 5.0 - 3.0 - 2.0"), 10.0);
    }

    #[test]
    fn mixed_add_subtract_left_associative() {
        // ((10 + 2) - 3) - 4 = 5
        assert_eq!(eval_f("10.0 + 2.0 - 3.0 - 4.0"), 5.0);
        // (10 - 2) + 3 = 11
        assert_eq!(eval_f("10.0 - 2.0 + 3.0"), 11.0);
    }

    // --- Multiplicative: LEFT-associative (KerML MultiplicativeExpression) -

    #[test]
    fn division_is_left_associative() {
        // (8 / 4) / 2 = 1, NOT 8 / (4 / 2) = 4
        assert_eq!(eval_f("8.0 / 4.0 / 2.0"), 1.0);
        // (16 / 4) / 2 = 2, NOT 16 / (4 / 2) = 8
        assert_eq!(eval_f("16.0 / 4.0 / 2.0"), 2.0);
    }

    #[test]
    fn remainder_is_left_associative() {
        // (10 % 4) % 2 = 0, NOT 10 % (4 % 2) = NaN
        assert_eq!(eval_f("10.0 % 4.0 % 2.0"), 0.0);
    }

    #[test]
    fn mixed_multiply_divide_left_associative() {
        // (100 / 5) * 2 = 40; ((100 * 2) / 5) = 40
        assert_eq!(eval_f("100.0 / 5.0 * 2.0"), 40.0);
        assert_eq!(eval_f("100.0 * 2.0 / 5.0"), 40.0);
    }

    // --- Exponentiation: RIGHT-associative, binds tightest -----------------

    #[test]
    fn exponentiation_is_right_associative() {
        // 2 ** (3 ** 2) = 2 ** 9 = 512, NOT (2 ** 3) ** 2 = 64
        assert_eq!(eval_f("2.0 ** 3.0 ** 2.0"), 512.0);
    }

    #[test]
    fn exponentiation_binds_tighter_than_multiplication() {
        // 2 * (3 ** 2) = 18, NOT (2 * 3) ** 2 = 36
        assert_eq!(eval_f("2.0 * 3.0 ** 2.0"), 18.0);
        // (2 ** 2) * 3 = 12, NOT 2 ** (2 * 3) = 64
        assert_eq!(eval_f("2.0 ** 2.0 * 3.0"), 12.0);
    }

    #[test]
    fn exponentiation_binds_tighter_than_addition() {
        // 12 - (2 ** 3) = 4
        assert_eq!(eval_f("12.0 - 2.0 ** 3.0"), 4.0);
    }

    // --- Unary-minus interplay must survive the rightmost-split scan -------

    #[test]
    fn binary_minus_before_unary_minus() {
        // 10 - (-2) = 12: the second `-` is unary and must be skipped so the
        // split lands on the (only) binary `-`.
        assert_eq!(eval_f("10.0 - -2.0"), 12.0);
    }

    #[test]
    fn multiply_by_negative_literal() {
        // 10 * (-2) = -20: the `-` is unary (follows `*`), so the additive
        // class finds no split and the multiplicative `*` is used.
        assert_eq!(eval_f("10.0 * -2.0"), -20.0);
    }

    // --- Parentheses still override associativity --------------------------

    #[test]
    fn parentheses_override_left_associativity() {
        assert_eq!(eval_f("10.0 - (2.0 - 3.0)"), 11.0);
        assert_eq!(eval_f("8.0 / (4.0 / 2.0)"), 4.0);
    }

    // --- Comparison / logical chains: LEFT-associative ---------------------

    #[test]
    fn logical_and_chain_left_associative() {
        assert!(!eval_b("true and false and true"));
        assert!(eval_b("true and true and true"));
    }

    #[test]
    fn null_coalescing_is_left_associative() {
        // (null ?? null) ?? 3 = 3; associativity is spec-mandated even though
        // the value is stable under either grouping for `??`.
        assert_eq!(eval_f("null ?? null ?? 3.0"), 3.0);
        assert_eq!(eval_f("null ?? 2.0 ?? 3.0"), 2.0);
    }

    // --- Exponentiation: `**` and `^` are ONE operator ---------------------
    //
    // `ExponentiationOperator : '**' | '^'` (KerMLExpressions.xtext:277).
    // Only `**` was in the precedence table, so `temperature ^ 4` — the
    // Stefan-Boltzmann term in `examples/radiation-cooling` — did not compile
    // and that fixture's ODE silently failed to build.

    #[test]
    fn caret_is_exponentiation() {
        assert_eq!(eval_f("2.0 ^ 3.0"), 8.0);
    }

    #[test]
    fn caret_and_double_star_agree() {
        assert_eq!(eval_f("3.0 ^ 4.0"), eval_f("3.0 ** 4.0"));
    }

    #[test]
    fn caret_is_right_associative() {
        // 2 ^ (3 ^ 2) = 2 ^ 9 = 512, NOT (2 ^ 3) ^ 2 = 64.
        assert_eq!(eval_f("2.0 ^ 3.0 ^ 2.0"), 512.0);
    }

    #[test]
    fn caret_binds_tighter_than_multiplication() {
        // 2 * (3 ^ 2) = 18, NOT (2 * 3) ^ 2 = 36. This is the grouping the
        // radiating-body RHS depends on: `e * s * A * (T ^ 4 - Tamb ^ 4)`.
        assert_eq!(eval_f("2.0 * 3.0 ^ 2.0"), 18.0);
    }

    #[test]
    fn caret_binds_tighter_than_subtraction_inside_a_difference() {
        // The exact shape of the Stefan-Boltzmann bracket.
        assert_eq!(eval_f("10.0 ^ 2.0 - 3.0 ^ 2.0"), 91.0);
    }
}
