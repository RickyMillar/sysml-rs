//! RSC-5.2 — quantity boundary diagnostics (design-doc D-5.0.8, two-tier
//! severity).
//!
//! A model-level scan that surfaces dimensional-conformance violations at
//! binding boundaries *before* simulation, as coded diagnostics. The
//! complementary eval-time hard-errors (add/sub/compare on resolved-incompatible
//! dimensions) already fire inside the expression evaluator (RSC-5.1b,
//! `eval_quantity_binary`); this static layer catches the same class of mistake
//! at the connector level, where the LSP can flag it live.
//!
//! Severity is graduated by *how much we know* (steward Q4 override — the
//! "all warning" recommendation was rejected because eval already fails hard):
//!  - **hard error** when BOTH endpoints resolve to a [`MeasurementRef`] with
//!    incompatible (non-zero, non-equal) dimensions — we have the facts;
//!  - **warning** when one endpoint is dimensioned and the other resolves to an
//!    *untyped* attribute — we lack the facts to fail hard, so surface the
//!    suspicion without blocking;
//!  - **silent** otherwise (same dimension — a scale difference is RSC-5.3's
//!    boundary auto-conversion, not a mismatch; or neither side carries a
//!    dimension; or an endpoint does not resolve).
//!
//! This module covers (RSC-5.2a) **UQ001** (binding connector `bind a = b`,
//! [`quantity_mismatch_health_diagnostics`]) and (RSC-5.2b) the
//! constraint-expression layer ([`quantity_expression_health_diagnostics`]):
//!  - **UQ003** — a cross-dimension *ordering* comparison (`<`, `<=`, `>`,
//!    `>=`) between two confidently-dimensioned operands. This is the static
//!    twin of the eval-time hard error shipped in RSC-5.1b
//!    (`eval_quantity_binary`, evaluator.rs); the predicate is identical
//!    (`ld != rd && !ld.is_zero() && !rd.is_zero()`), so the static code and the
//!    runtime error agree. `==`/`!=` are NOT flagged — the evaluator folds the
//!    dimension into the boolean result rather than failing, so neither does
//!    this layer.
//!  - **UQ004** — a dimensioned argument passed to a dimensionless-only
//!    (transcendental) function (`sin`, `cos`, `exp`, `ln`, …). `sqrt`, `abs`,
//!    `floor`, etc. are dimension-carrying and never flagged.
//!
//! Both fire only when the operand dimensions are *known* — a conservative
//! [`expr_dimension`] returns `None` for anything it cannot determine
//! (unresolved name, untyped attribute, function it does not model), and `None`
//! never produces a diagnostic. Operand names resolve **owner-scoped**
//! (RSC-3.1 D-3.0.6-B, [`owner_scoped_features`]), so an injected runtime
//! variable that is not a model attribute (e.g. `sim_time_ms`) resolves to
//! nothing and stays silent.
//!
//! UQ001 also covers (RSC-5.2b) the **value-binding form** — an attribute whose
//! declared ISQ dimension contradicts the dimension of its default *value
//! expression* (`attribute x : LengthValue = mass * 2`), emitted alongside the
//! binding-connector form by [`quantity_mismatch_health_diagnostics`].
//!
//! Still pending in a later step: UQ002 (link / flow payload endpoints).

use std::collections::HashMap;

use sysml_core::element_ordering::{primary_span, sort_elements_by_source_order};
use sysml_core::physics::DimensionVector;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph};
use sysml_span::Diagnostic;

use crate::compiler::{infer_m_ref, owner_scoped_features};
use crate::expressions::{compile_expression, BinOp, ExprIR, UnaryOp};
use crate::slots::MeasurementRef;

/// Binding-connector element kinds (both the abstract connector and the usage
/// form the parser produces from `bind a = b`).
const BINDING_KINDS: &[ElementKind] = &[
    ElementKind::BindingConnector,
    ElementKind::BindingConnectorAsUsage,
];

/// What we know about one binding endpoint's quantity dimension.
enum EndpointDim {
    /// Resolves to an attribute carrying a non-zero ISQ dimension.
    Dimensioned(MeasurementRef),
    /// Resolves to an attribute, but it carries no (non-zero) dimension — an
    /// untyped or dimensionless attribute. We lack the facts to fail hard.
    Untyped,
    /// The endpoint does not resolve to an attribute (a port, an action
    /// feature, or an unresolved name). Out of scope for a quantity check.
    Unknown,
}

/// Resolve one binding endpoint (`source_id` / `target_id`, stamped by
/// elaboration as a [`sysml_core::Value::Ref`]) into the target element and its
/// dimension classification. Returns `None` when the endpoint is absent or its
/// ref does not resolve — both mean "no facts", never a diagnostic.
fn endpoint<'g>(
    graph: &'g ModelGraph,
    element: &Element,
    prop: &str,
) -> Option<(&'g Element, EndpointDim)> {
    let id: &ElementId = element.get_prop(prop)?.as_ref()?;
    let target = graph.get_element(id)?;
    let dim = if target.kind == ElementKind::AttributeUsage {
        match infer_m_ref(graph, id) {
            Some(m) if !m.dimension.is_zero() => EndpointDim::Dimensioned(m),
            _ => EndpointDim::Untyped,
        }
    } else {
        EndpointDim::Unknown
    };
    Some((target, dim))
}

fn label(element: &Element) -> String {
    element
        .name
        .as_deref()
        .map(|n| format!("'{n}'"))
        .unwrap_or_else(|| "<anonymous>".to_owned())
}

/// RSC-5.2 / D-5.0.8 — dimensional-conformance diagnostics at binding
/// boundaries. See the module docstring for the severity rubric.
pub fn quantity_mismatch_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    let mut bindings: Vec<&Element> = BINDING_KINDS
        .iter()
        .flat_map(|kind| graph.elements_by_kind(kind))
        .collect();
    sort_elements_by_source_order(&mut bindings);

    let mut diagnostics = Vec::new();
    for element in bindings {
        let (Some((src_el, src_dim)), Some((tgt_el, tgt_dim))) = (
            endpoint(graph, element, "source_id"),
            endpoint(graph, element, "target_id"),
        ) else {
            // One or both endpoints unresolved — no facts, stay silent.
            continue;
        };

        let diag = match (&src_dim, &tgt_dim) {
            // Both dimensioned: hard error only on incompatible dimensions.
            // Equal dimensions (incl. a pure scale difference) are not a
            // mismatch — that is RSC-5.3 boundary conversion territory.
            (EndpointDim::Dimensioned(s), EndpointDim::Dimensioned(t)) => {
                if s.dimension != t.dimension {
                    Some(
                        Diagnostic::error(format!(
                            "binding {} connects incompatible quantity dimensions: \
                             {} [{}] and {} [{}]",
                            label(element),
                            label(src_el),
                            s.dimension,
                            label(tgt_el),
                            t.dimension,
                        ))
                        .with_code("UQ001")
                        .with_span(primary_span(element)),
                    )
                } else {
                    None
                }
            }
            // One side dimensioned, the other an untyped attribute: warn — we
            // suspect a missing ISQ type but lack the facts to fail hard.
            (EndpointDim::Dimensioned(d), EndpointDim::Untyped) => {
                Some(uq001_warning(element, src_el, d, tgt_el))
            }
            (EndpointDim::Untyped, EndpointDim::Dimensioned(d)) => {
                Some(uq001_warning(element, tgt_el, d, src_el))
            }
            _ => None,
        };
        if let Some(diag) = diag {
            diagnostics.push(diag);
        }
    }
    // UQ001 also covers the value-binding (`=` / `:=`) form: an attribute whose
    // declared dimension contradicts its default value expression's dimension.
    diagnostics.extend(value_binding_diagnostics(graph));
    diagnostics
}

/// RSC-5.2b — the **value-binding form** of UQ001: an attribute whose declared
/// ISQ dimension contradicts the dimension of its default *value expression*
/// (`attribute x : LengthValue = mass * 2`). This is the static, coded twin of
/// the eval-time add/sub/compare hard error (RSC-5.1b), reported at the
/// declaration site.
///
/// Only the *expression* default form is in scope. A **literal** default
/// (`= 5 [kg]`) is folded by the parser into the attribute's own `unit` prop,
/// which [`infer_m_ref`] (D-5.0.5, path #1) deliberately takes as the slot's
/// authoritative measurement — so there is no separate value dimension to
/// compare. For an expression default there is no `unit` prop, so `infer_m_ref`
/// falls to path #2 (the declared ISQ *type*), which is exactly the declared
/// dimension we want.
///
/// Conservative by construction (mirrors the binding-connector rubric): fires a
/// hard error only when BOTH the declared type dimension and the
/// value-expression dimension are known, non-zero, and unequal. A value that is
/// dimensionless (a bare number assigned to a dimensioned attribute) is
/// scale/boundary territory (RSC-5.3), not a mismatch; an unresolved operand
/// yields `None` from [`expr_dimension`] and stays silent.
fn value_binding_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    let mut attrs: Vec<&Element> = graph
        .elements_by_kind(&ElementKind::AttributeUsage)
        .collect();
    sort_elements_by_source_order(&mut attrs);

    let mut diagnostics = Vec::new();
    for attr in attrs {
        // Declared ISQ dimension. For an expression default this is the ISQ
        // *type* (path #2) — no `[unit]` prop is folded onto an expression.
        let Some(declared) = infer_m_ref(graph, &attr.id) else {
            continue;
        };
        if declared.dimension.is_zero() {
            continue;
        }
        // The default *value* expression. `compile_expression` yields the
        // value's ExprIR only for a non-literal default (the parser emits the
        // subtree as a child); a literal default has no expression child and
        // returns `Err`, so it is skipped here.
        let Ok(ir) = compile_expression(attr, graph) else {
            continue;
        };
        // The value's operand names resolve in the attribute's owner scope
        // (its siblings) — the same nearest-first walk the constraint scan uses.
        let resolver = owner_scoped_resolver(graph, &attr.id);
        let Some(value_dim) = expr_dimension(&ir, &resolver, graph) else {
            continue;
        };
        if value_dim.is_zero() || value_dim == declared.dimension {
            continue;
        }
        diagnostics.push(
            Diagnostic::error(format!(
                "attribute {} declares dimension [{}] but its default value has \
                 dimension [{}]",
                label(attr),
                declared.dimension,
                value_dim,
            ))
            .with_code("UQ001")
            .with_span(primary_span(attr)),
        );
    }
    diagnostics
}

/// The two-tier WARNING: `dimensioned` carries a real dimension, `untyped` is an
/// attribute with no declared ISQ type.
fn uq001_warning(
    binding: &Element,
    dimensioned: &Element,
    d: &MeasurementRef,
    untyped: &Element,
) -> Diagnostic {
    Diagnostic::warning(format!(
        "binding {} binds dimensioned {} [{}] to untyped {} — declare its ISQ \
         type to enable dimensional checking",
        label(binding),
        label(dimensioned),
        d.dimension,
        label(untyped),
    ))
    .with_code("UQ001")
    .with_span(primary_span(binding))
}

/// Constraint element kinds whose body expression we scan (matches
/// `constraints::extract_constraints_filtered`).
const CONSTRAINT_KINDS: &[ElementKind] = &[
    ElementKind::ConstraintUsage,
    ElementKind::AssertConstraintUsage,
    ElementKind::ConstraintDefinition,
];

/// Dimensionless-only (transcendental) functions: their argument must be a pure
/// number. A dimensioned argument is a UQ004 error. Deliberately excludes
/// `sqrt`/`abs`/`floor`/`ceil`/`round`/`min`/`max`, which legitimately carry or
/// preserve a dimension.
const DIMENSIONLESS_ONLY_FNS: &[&str] = &[
    "exp", "ln", "sin", "cos", "tan", "tanh", "asin", "arcsin", "acos", "arccos", "atan", "arctan",
];

fn dimensionless() -> DimensionVector {
    DimensionVector::new(0, 0, 0, 0, 0, 0, 0)
}

/// RSC-5.2b / D-5.0.8 — static cross-dimension diagnostics inside constraint
/// expressions (UQ003 comparison, UQ004 dimensionless-fn argument). See the
/// module docstring for the severity / conservatism rubric.
pub fn quantity_expression_health_diagnostics(graph: &ModelGraph) -> Vec<Diagnostic> {
    let mut constraints: Vec<&Element> = CONSTRAINT_KINDS
        .iter()
        .flat_map(|kind| graph.elements_by_kind(kind))
        .collect();
    sort_elements_by_source_order(&mut constraints);

    let mut diagnostics = Vec::new();
    for element in constraints {
        // Compile the constraint body to ExprIR. A compile failure is CN001's
        // concern (constraint health), never ours — stay silent.
        let Ok(ir) = compile_expression(element, graph) else {
            continue;
        };
        // Owner-scoped operand resolver: the constraint's own feature children
        // (a definition's `in` params) shadow its owner's, which shadow the
        // ancestors' — nearest-first wins (D-3.0.6-B).
        let resolver = owner_scoped_resolver(graph, &element.id);
        scan_expr(&ir, &resolver, graph, element, &mut diagnostics);
    }
    diagnostics
}

/// Nearest-first `name → attribute element id` map over the constraint's
/// owner-tree. First occurrence wins (the shared walk yields nearest scope
/// first), so a name binds to the closest enclosing declaration.
fn owner_scoped_resolver(graph: &ModelGraph, scope_root: &ElementId) -> HashMap<String, ElementId> {
    let mut map = HashMap::new();
    for f in owner_scoped_features(graph, scope_root) {
        map.entry(f.name).or_insert(f.id);
    }
    map
}

/// Recursively scan `expr`, emitting UQ003 (cross-dim ordering comparison) and
/// UQ004 (dimensioned transcendental argument) at the nodes that carry them,
/// then descending into every sub-expression so nested occurrences are caught.
fn scan_expr(
    expr: &ExprIR,
    resolver: &HashMap<String, ElementId>,
    graph: &ModelGraph,
    constraint: &Element,
    out: &mut Vec<Diagnostic>,
) {
    match expr {
        ExprIR::BinaryOp { op, left, right } => {
            if is_ordering_comparison(*op) {
                if let (Some(ld), Some(rd)) = (
                    expr_dimension(left, resolver, graph),
                    expr_dimension(right, resolver, graph),
                ) {
                    // Mirror evaluator.rs `eval_quantity_binary` exactly.
                    if ld != rd && !ld.is_zero() && !rd.is_zero() {
                        out.push(
                            Diagnostic::error(format!(
                                "constraint {} compares incompatible quantity \
                                 dimensions: [{}] {} [{}]",
                                label(constraint),
                                ld,
                                comparison_symbol(*op),
                                rd,
                            ))
                            .with_code("UQ003")
                            .with_span(primary_span(constraint)),
                        );
                    }
                }
            }
            scan_expr(left, resolver, graph, constraint, out);
            scan_expr(right, resolver, graph, constraint, out);
        }
        ExprIR::FunctionCall { name, args } => {
            if args.len() == 1 && DIMENSIONLESS_ONLY_FNS.contains(&name.as_str()) {
                if let Some(d) = expr_dimension(&args[0], resolver, graph) {
                    if !d.is_zero() {
                        out.push(
                            Diagnostic::error(format!(
                                "constraint {} applies dimensionless-only function \
                                 '{}' to a dimensioned argument [{}]",
                                label(constraint),
                                name,
                                d,
                            ))
                            .with_code("UQ004")
                            .with_span(primary_span(constraint)),
                        );
                    }
                }
            }
            for arg in args {
                scan_expr(arg, resolver, graph, constraint, out);
            }
        }
        // Structural recursion into every other expression that carries
        // sub-expressions, so a comparison/fn nested inside is still reached.
        ExprIR::UnaryOp { operand, .. } | ExprIR::MetaAccess { operand, .. } => {
            scan_expr(operand, resolver, graph, constraint, out);
        }
        ExprIR::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            scan_expr(condition, resolver, graph, constraint, out);
            scan_expr(then_expr, resolver, graph, constraint, out);
            scan_expr(else_expr, resolver, graph, constraint, out);
        }
        ExprIR::NullCoalescing { expr, default } => {
            scan_expr(expr, resolver, graph, constraint, out);
            scan_expr(default, resolver, graph, constraint, out);
        }
        ExprIR::Select {
            source, predicate, ..
        }
        | ExprIR::Reject {
            source, predicate, ..
        }
        | ExprIR::ForAll {
            source, predicate, ..
        }
        | ExprIR::Exists {
            source, predicate, ..
        } => {
            scan_expr(source, resolver, graph, constraint, out);
            scan_expr(predicate, resolver, graph, constraint, out);
        }
        ExprIR::Collect {
            source, transform, ..
        } => {
            scan_expr(source, resolver, graph, constraint, out);
            scan_expr(transform, resolver, graph, constraint, out);
        }
        ExprIR::Index { sequence, index } => {
            scan_expr(sequence, resolver, graph, constraint, out);
            scan_expr(index, resolver, graph, constraint, out);
        }
        ExprIR::Range { lower, upper } => {
            scan_expr(lower, resolver, graph, constraint, out);
            scan_expr(upper, resolver, graph, constraint, out);
        }
        ExprIR::Sequence(items) => {
            for item in items {
                scan_expr(item, resolver, graph, constraint, out);
            }
        }
        ExprIR::ConstructorCall { named_args, .. } => {
            for (_, arg) in named_args {
                scan_expr(arg, resolver, graph, constraint, out);
            }
        }
        // Leaves — no sub-expressions.
        ExprIR::LiteralInt(_)
        | ExprIR::LiteralReal(_)
        | ExprIR::LiteralBool(_)
        | ExprIR::LiteralString(_)
        | ExprIR::LiteralNull
        | ExprIR::LiteralQuantity { .. }
        | ExprIR::FeatureRef(_)
        | ExprIR::FeatureChain(_)
        | ExprIR::SlotRef { .. }
        | ExprIR::SlotChainHead { .. } => {}
    }
}

/// The ISQ dimension of `expr`, when it can be determined with confidence.
///
/// Conservative by construction: any operand that does not resolve to a known
/// dimension (an unresolved or untyped reference, a function whose dimensional
/// behaviour we do not model, a chained navigation) yields `None`, and `None`
/// suppresses the diagnostic. A pure numeric literal is dimensionless (the
/// zero vector); a `LiteralQuantity` carries its compile-resolved dimension.
fn expr_dimension(
    expr: &ExprIR,
    resolver: &HashMap<String, ElementId>,
    graph: &ModelGraph,
) -> Option<DimensionVector> {
    match expr {
        ExprIR::LiteralInt(_) | ExprIR::LiteralReal(_) => Some(dimensionless()),
        ExprIR::LiteralQuantity { dimension, .. } => Some(dimension.clone()),
        ExprIR::FeatureRef(name) => resolver
            .get(name)
            .and_then(|id| infer_m_ref(graph, id))
            .map(|m| m.dimension),
        ExprIR::UnaryOp { op, operand } => match op {
            UnaryOp::Negate | UnaryOp::Plus => expr_dimension(operand, resolver, graph),
            _ => None,
        },
        ExprIR::BinaryOp { op, left, right } => {
            let ld = expr_dimension(left, resolver, graph)?;
            let rd = expr_dimension(right, resolver, graph)?;
            match op {
                // Add/subtract require equal dimensions; an arithmetic mismatch
                // is the evaluator's runtime error, not ours — return None so we
                // never double-report it as a comparison code.
                BinOp::Add | BinOp::Subtract => (ld == rd).then_some(ld),
                BinOp::Multiply => Some(ld + rd),
                BinOp::Divide => Some(ld - rd),
                // Power needs a scalar exponent and the rest are non-arithmetic;
                // stay conservative.
                _ => None,
            }
        }
        ExprIR::FunctionCall { name, args } => {
            if DIMENSIONLESS_ONLY_FNS.contains(&name.as_str()) {
                // Transcendental result is always dimensionless.
                Some(dimensionless())
            } else if name == "abs" && args.len() == 1 {
                expr_dimension(&args[0], resolver, graph)
            } else if (name == "min" || name == "max") && args.len() == 2 {
                let a = expr_dimension(&args[0], resolver, graph)?;
                let b = expr_dimension(&args[1], resolver, graph)?;
                (a == b).then_some(a)
            } else {
                None
            }
        }
        _ => None,
    }
}

fn is_ordering_comparison(op: BinOp) -> bool {
    matches!(
        op,
        BinOp::LessThan | BinOp::LessEqual | BinOp::GreaterThan | BinOp::GreaterEqual
    )
}

fn comparison_symbol(op: BinOp) -> &'static str {
    match op {
        BinOp::LessThan => "<",
        BinOp::LessEqual => "<=",
        BinOp::GreaterThan => ">",
        BinOp::GreaterEqual => ">=",
        _ => "?",
    }
}
