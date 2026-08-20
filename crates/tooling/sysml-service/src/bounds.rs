//! Constraint-bound extraction (R3.3 of the backend-first cleansing audit).
//!
//! Walks every constraint expression in a `ModelGraph`, looks for
//! comparisons that reference an `AttributeUsage`, and emits the
//! resulting [`BoundMarker`] grouped by the attribute's `ElementId`.
//!
//! Replaces the FE's `boundExtractor.ts` AST walker. The backend is
//! authoritative for which constraints attribute to which attribute —
//! we resolve names to `AttributeUsage` ids via `find_by_name`, so two
//! circuits sharing a `temperature` short name end up with separate
//! per-instance bound lists.
//!
//! Comparison shapes recognised (mirroring the original FE walker):
//!
//! ```text
//!   attr   <  literal     -> upper bound  (operator "<")
//!   attr  <=  literal     -> upper bound  (operator "<=")
//!   attr   >  literal     -> lower bound  (operator ">")
//!   attr  >=  literal     -> lower bound  (operator ">=")
//!   attr  ==  literal     -> target bound (operator "==")
//! ```
//!
//! And the reversed forms (literal on the left side), where the
//! operator is canonicalised so the marker's kind reflects the
//! attribute's point of view.

use std::collections::HashMap;

use sysml_core::{ElementId, ElementKind, ModelGraph};
use sysml_runtime::constraints::TypedConstraint;
use sysml_runtime::expressions::{BinOp, ExprIR};

use crate::types::BoundMarker;

/// Walk every constraint in `graph`, scan its compiled expression for
/// comparisons referencing an `AttributeUsage`, and return the bounds
/// grouped by attribute id.
///
/// Returns a `HashMap<ElementId, Vec<BoundMarker>>` so the
/// `model_tree` projection can do a single bulk extract per request
/// and look up per-node in O(1).
pub fn extract_bounds_by_attribute(
    graph: &ModelGraph,
) -> HashMap<ElementId, Vec<BoundMarker>> {
    let mut out: HashMap<ElementId, Vec<BoundMarker>> = HashMap::new();

    for element in graph.elements.values() {
        if !is_constraint_kind(&element.kind) {
            continue;
        }

        // Compile the constraint's expression to typed IR. If
        // compilation fails (parser errors, unresolved refs in the
        // legacy fallback path, …) skip silently — bounds are a
        // display nicety, never a correctness signal.
        let typed = match TypedConstraint::from_element(element, graph) {
            Ok(t) => t,
            Err(_diags) => {
                tracing::trace!(
                    element_id = %element.id,
                    "skipping constraint whose expression failed to compile"
                );
                continue;
            }
        };

        let constraint_name = element
            .name
            .clone()
            .unwrap_or_else(|| "(constraint)".to_string());

        scan_expr_for_bounds(&typed.expr_ir, graph, &constraint_name, &mut out);
    }

    out
}

/// Reflexive equivalent of `kind.is_subtype_of(ConstraintUsage)` /
/// `kind.is_subtype_of(ConstraintDefinition)` — `is_subtype_of` is
/// strict (R2.4 finding), so we anchor with explicit `matches!` for
/// the base kinds and let subtypes (RequirementUsage,
/// AssertConstraintUsage, …) flow through `is_subtype_of`.
fn is_constraint_kind(kind: &ElementKind) -> bool {
    if kind.is_subtype_of(ElementKind::ConstraintUsage)
        || kind.is_subtype_of(ElementKind::ConstraintDefinition)
    {
        return true;
    }
    matches!(
        kind,
        ElementKind::ConstraintUsage | ElementKind::ConstraintDefinition
    )
}

/// Recursively scan an `ExprIR` for comparison nodes. Compound
/// expressions joined by `&&` / `||` are walked into so a constraint
/// like `temperature < 80 and temperature > 0` produces two bounds.
fn scan_expr_for_bounds(
    expr: &ExprIR,
    graph: &ModelGraph,
    constraint_name: &str,
    out: &mut HashMap<ElementId, Vec<BoundMarker>>,
) {
    if let ExprIR::BinaryOp { op, left, right } = expr {
        // Comparison op? Try to extract a bound at this node.
        if let Some(canonical) = comparison_op_str(*op) {
            try_extract_bound(canonical, left, right, graph, constraint_name, out);
            // Don't return — a comparison can't have a comparison
            // child (no chained `a < b < c` in IR), but harmless to
            // walk anyway.
        }

        // Logical compound? Walk both sides.
        if matches!(op, BinOp::And | BinOp::Or) {
            scan_expr_for_bounds(left, graph, constraint_name, out);
            scan_expr_for_bounds(right, graph, constraint_name, out);
        }
    }
}

/// Map a `BinOp` to its canonical string form, or `None` for
/// non-comparison ops.
fn comparison_op_str(op: BinOp) -> Option<&'static str> {
    Some(match op {
        BinOp::LessThan => "<",
        BinOp::LessEqual => "<=",
        BinOp::GreaterThan => ">",
        BinOp::GreaterEqual => ">=",
        BinOp::Equal => "==",
        _ => return None,
    })
}

/// Flip a comparison operator so the bound's marker kind still
/// reflects the attribute's point of view (used when the literal is
/// on the LEFT of the comparison: `80 < temperature` → after flip,
/// `temperature > 80`).
fn flip_op(op: &str) -> &'static str {
    match op {
        "<" => ">",
        "<=" => ">=",
        ">" => "<",
        ">=" => "<=",
        "==" => "==",
        _ => "<",
    }
}

/// Classify an operator into the marker's `kind` string.
fn classify_kind(op: &str) -> &'static str {
    match op {
        "<" | "<=" => "upper",
        ">" | ">=" => "lower",
        "==" => "target",
        _ => "upper",
    }
}

/// Resolve a `FeatureRef(name)` or `FeatureChain(chain)` operand to
/// the matching `AttributeUsage` element ids. Multiple matches are
/// expected (and the *point* of id-keyed attribution): two circuits
/// each owning their own `temperature` AttributeUsage with the same
/// short name. Returns an empty Vec for non-attribute operands or
/// names that don't resolve.
fn resolve_attribute_ids(expr: &ExprIR, graph: &ModelGraph) -> Vec<ElementId> {
    let name: &str = match expr {
        ExprIR::FeatureRef(n) => n.as_str(),
        // For chains, take the last segment — the FE walker did the
        // same (matches the tail of qualified names like
        // `Panel.Breaker.temperature` against the bare attribute
        // name). If the chain has zero segments, nothing to do.
        ExprIR::FeatureChain(chain) => match chain.last() {
            Some(s) => s.as_str(),
            None => return Vec::new(),
        },
        _ => return Vec::new(),
    };
    sysml_core::query::find_by_name(graph, Some(&ElementKind::AttributeUsage), name)
        .map(|e| e.id.clone())
        .collect()
}

/// Try to extract a numeric literal value from an `ExprIR` operand.
fn literal_as_f64(expr: &ExprIR) -> Option<f64> {
    match expr {
        ExprIR::LiteralInt(i) => Some(*i as f64),
        ExprIR::LiteralReal(r) if r.is_finite() => Some(*r),
        // Quantity literal bound: use the as-written magnitude (bounds reasoning
        // is unit-naive; the unit is dropped here, matching the bare-literal case).
        ExprIR::LiteralQuantity { value, .. } if value.is_finite() => Some(*value),
        _ => None,
    }
}

/// Inspect a single comparison `(canonical_op, left, right)` and
/// append a [`BoundMarker`] to every matching attribute's vec.
fn try_extract_bound(
    canonical_op: &'static str,
    left: &ExprIR,
    right: &ExprIR,
    graph: &ModelGraph,
    constraint_name: &str,
    out: &mut HashMap<ElementId, Vec<BoundMarker>>,
) {
    // Canonical: (attribute, literal). Use the operator as-is.
    let lhs_attr_ids = resolve_attribute_ids(left, graph);
    let rhs_lit = literal_as_f64(right);
    if !lhs_attr_ids.is_empty() {
        if let Some(value) = rhs_lit {
            for id in lhs_attr_ids {
                push_marker(out, id, value, canonical_op, constraint_name);
            }
            return;
        }
    }

    // Reversed: (literal, attribute). Flip the operator so the bound's
    // kind reflects the attribute's point of view.
    let lhs_lit = literal_as_f64(left);
    let rhs_attr_ids = resolve_attribute_ids(right, graph);
    if !rhs_attr_ids.is_empty() {
        if let Some(value) = lhs_lit {
            let flipped = flip_op(canonical_op);
            for id in rhs_attr_ids {
                push_marker(out, id, value, flipped, constraint_name);
            }
        }
    }
}

fn push_marker(
    out: &mut HashMap<ElementId, Vec<BoundMarker>>,
    id: ElementId,
    y: f64,
    operator: &str,
    constraint_name: &str,
) {
    out.entry(id).or_default().push(BoundMarker {
        y,
        kind: classify_kind(operator).to_string(),
        constraint_name: constraint_name.to_string(),
        operator: operator.to_string(),
    });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind, ModelGraph, Value};
    use sysml_runtime::expressions::{BinOp, ExprIR};

    /// Build a minimal graph with a single AttributeUsage and a
    /// ConstraintUsage whose expression is the supplied `ExprIR`.
    fn graph_with_attr_and_constraint(
        attr_name: &str,
        constraint_name: &str,
        expr: ExprIR,
    ) -> (ModelGraph, ElementId) {
        let mut graph = ModelGraph::new();

        let attr = Element::new_with_kind(ElementKind::AttributeUsage).with_name(attr_name);
        let attr_id = graph.add_element(attr);

        let constraint =
            Element::new_with_kind(ElementKind::ConstraintUsage).with_name(constraint_name);
        let cid = graph.add_element(constraint);

        // Materialise the ExprIR into the parser-style child element
        // tree the AST compiler walks. `ir_builder` does this for us.
        materialise_expr(&mut graph, &cid, &expr);

        (graph, attr_id)
    }

    /// Recursively turn an `ExprIR` into parser-shaped child elements
    /// owned by `parent`. We only need the variants the tests below
    /// actually exercise (Comparison, And/Or, FeatureRef, LiteralInt).
    fn materialise_expr(graph: &mut ModelGraph, parent: &ElementId, expr: &ExprIR) {
        match expr {
            ExprIR::BinaryOp { op, left, right } => {
                let op_str = match op {
                    BinOp::LessThan => "<",
                    BinOp::LessEqual => "<=",
                    BinOp::GreaterThan => ">",
                    BinOp::GreaterEqual => ">=",
                    BinOp::Equal => "==",
                    BinOp::And => "and",
                    BinOp::Or => "or",
                    _ => panic!("unsupported op in test materialiser: {:?}", op),
                };
                let op_el = Element::new_with_kind(ElementKind::OperatorExpression)
                    .with_owner(parent.clone())
                    .with_prop("operator", Value::String(op_str.into()));
                let op_id = graph.add_element(op_el);
                materialise_arg(graph, &op_id, left, 0);
                materialise_arg(graph, &op_id, right, 1);
            }
            _ => panic!("top-level expr must be a binary op in this test helper"),
        }
    }

    fn materialise_arg(
        graph: &mut ModelGraph,
        parent: &ElementId,
        expr: &ExprIR,
        arg_index: usize,
    ) {
        match expr {
            ExprIR::FeatureRef(name) => {
                let _ = graph.add_element(
                    Element::new_with_kind(ElementKind::FeatureReferenceExpression)
                        .with_name(name.clone())
                        .with_owner(parent.clone())
                        .with_prop("argIndex", Value::Int(arg_index as i64)),
                );
            }
            ExprIR::LiteralInt(i) => {
                let _ = graph.add_element(
                    Element::new_with_kind(ElementKind::LiteralInteger)
                        .with_owner(parent.clone())
                        .with_prop("value", Value::Int(*i))
                        .with_prop("argIndex", Value::Int(arg_index as i64)),
                );
            }
            ExprIR::LiteralReal(r) => {
                let _ = graph.add_element(
                    Element::new_with_kind(ElementKind::LiteralRational)
                        .with_owner(parent.clone())
                        .with_prop("value", Value::Float(*r))
                        .with_prop("argIndex", Value::Int(arg_index as i64)),
                );
            }
            ExprIR::BinaryOp { op, left, right } => {
                let op_str = match op {
                    BinOp::LessThan => "<",
                    BinOp::LessEqual => "<=",
                    BinOp::GreaterThan => ">",
                    BinOp::GreaterEqual => ">=",
                    BinOp::Equal => "==",
                    BinOp::And => "and",
                    BinOp::Or => "or",
                    _ => panic!("unsupported op in test materialiser: {:?}", op),
                };
                let op_el = Element::new_with_kind(ElementKind::OperatorExpression)
                    .with_owner(parent.clone())
                    .with_prop("operator", Value::String(op_str.into()))
                    .with_prop("argIndex", Value::Int(arg_index as i64));
                let op_id = graph.add_element(op_el);
                materialise_arg(graph, &op_id, left, 0);
                materialise_arg(graph, &op_id, right, 1);
            }
            _ => panic!("unsupported operand kind in test materialiser"),
        }
    }

    fn cmp(op: BinOp, left: ExprIR, right: ExprIR) -> ExprIR {
        ExprIR::BinaryOp {
            op,
            left: Box::new(left),
            right: Box::new(right),
        }
    }

    #[test]
    fn upper_bound_canonical_form() {
        let expr = cmp(
            BinOp::LessThan,
            ExprIR::FeatureRef("temperature".into()),
            ExprIR::LiteralInt(80),
        );
        let (graph, attr_id) =
            graph_with_attr_and_constraint("temperature", "thermalSafe", expr);
        let bounds_map = extract_bounds_by_attribute(&graph);

        let bounds = bounds_map.get(&attr_id).expect("temperature should have bounds");
        assert_eq!(bounds.len(), 1);
        let b = &bounds[0];
        assert_eq!(b.y, 80.0);
        assert_eq!(b.kind, "upper");
        assert_eq!(b.operator, "<");
        assert_eq!(b.constraint_name, "thermalSafe");
    }

    #[test]
    fn reversed_form_produces_lower_bound() {
        // 80 < temperature  --canonicalised-->  temperature > 80
        let expr = cmp(
            BinOp::LessThan,
            ExprIR::LiteralInt(80),
            ExprIR::FeatureRef("temperature".into()),
        );
        let (graph, attr_id) = graph_with_attr_and_constraint("temperature", "tempMin", expr);
        let bounds_map = extract_bounds_by_attribute(&graph);

        let bounds = bounds_map.get(&attr_id).expect("temperature should have bounds");
        assert_eq!(bounds.len(), 1);
        assert_eq!(bounds[0].y, 80.0);
        assert_eq!(bounds[0].kind, "lower");
        assert_eq!(bounds[0].operator, ">");
    }

    #[test]
    fn equality_produces_target_bound() {
        let expr = cmp(
            BinOp::Equal,
            ExprIR::FeatureRef("temperature".into()),
            ExprIR::LiteralInt(25),
        );
        let (graph, attr_id) = graph_with_attr_and_constraint("temperature", "set", expr);
        let bounds_map = extract_bounds_by_attribute(&graph);

        let bounds = bounds_map.get(&attr_id).expect("temperature should have bounds");
        assert_eq!(bounds.len(), 1);
        assert_eq!(bounds[0].y, 25.0);
        assert_eq!(bounds[0].kind, "target");
        assert_eq!(bounds[0].operator, "==");
    }

    #[test]
    fn compound_and_produces_two_bounds() {
        // temperature < 80 and temperature > 0
        let lhs = cmp(
            BinOp::LessThan,
            ExprIR::FeatureRef("temperature".into()),
            ExprIR::LiteralInt(80),
        );
        let rhs = cmp(
            BinOp::GreaterThan,
            ExprIR::FeatureRef("temperature".into()),
            ExprIR::LiteralInt(0),
        );
        let expr = cmp(BinOp::And, lhs, rhs);
        let (graph, attr_id) =
            graph_with_attr_and_constraint("temperature", "envelope", expr);
        let bounds_map = extract_bounds_by_attribute(&graph);

        let bounds = bounds_map.get(&attr_id).expect("temperature should have bounds");
        assert_eq!(bounds.len(), 2);
        let kinds: Vec<&str> = bounds.iter().map(|b| b.kind.as_str()).collect();
        assert!(kinds.contains(&"upper"));
        assert!(kinds.contains(&"lower"));
    }

    #[test]
    fn id_keyed_separates_two_attributes_sharing_a_short_name() {
        // Two AttributeUsages, each named `temperature`, each
        // referenced by its own constraint with a different bound.
        // The id-keyed map MUST keep them separate so the FE renders
        // each circuit's chart with its own marker line.
        let mut graph = ModelGraph::new();

        let a1 = graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage).with_name("temperature"),
        );
        let a2 = graph.add_element(
            Element::new_with_kind(ElementKind::AttributeUsage).with_name("temperature"),
        );

        // First constraint: temperature < 80 (constraint c1).
        let c1 = graph.add_element(
            Element::new_with_kind(ElementKind::ConstraintUsage).with_name("c1"),
        );
        materialise_expr(
            &mut graph,
            &c1,
            &cmp(
                BinOp::LessThan,
                ExprIR::FeatureRef("temperature".into()),
                ExprIR::LiteralInt(80),
            ),
        );

        // Second constraint: temperature > 0 (constraint c2).
        let c2 = graph.add_element(
            Element::new_with_kind(ElementKind::ConstraintUsage).with_name("c2"),
        );
        materialise_expr(
            &mut graph,
            &c2,
            &cmp(
                BinOp::GreaterThan,
                ExprIR::FeatureRef("temperature".into()),
                ExprIR::LiteralInt(0),
            ),
        );

        let bounds_map = extract_bounds_by_attribute(&graph);

        // Both attributes should appear in the map (every reference
        // by short name resolves to BOTH ids; that's the id-keyed
        // attribution doing its job — each circuit's chart will
        // pick up the bounds it actually owns by id).
        let b1 = bounds_map.get(&a1).expect("a1 should have bounds");
        let b2 = bounds_map.get(&a2).expect("a2 should have bounds");

        assert_eq!(b1.len(), 2);
        assert_eq!(b2.len(), 2);
        // Each gets one upper from c1 and one lower from c2.
        let kinds_a1: Vec<&str> = b1.iter().map(|b| b.kind.as_str()).collect();
        let kinds_a2: Vec<&str> = b2.iter().map(|b| b.kind.as_str()).collect();
        assert!(kinds_a1.contains(&"upper") && kinds_a1.contains(&"lower"));
        assert!(kinds_a2.contains(&"upper") && kinds_a2.contains(&"lower"));
    }

    #[test]
    fn unknown_attribute_name_is_skipped_silently() {
        // Constraint references a name that isn't an
        // AttributeUsage in the graph — emit nothing, don't panic.
        let mut graph = ModelGraph::new();
        let cid = graph.add_element(
            Element::new_with_kind(ElementKind::ConstraintUsage).with_name("orphan"),
        );
        materialise_expr(
            &mut graph,
            &cid,
            &cmp(
                BinOp::LessThan,
                ExprIR::FeatureRef("nobody".into()),
                ExprIR::LiteralInt(42),
            ),
        );

        let bounds_map = extract_bounds_by_attribute(&graph);
        assert!(bounds_map.is_empty());
    }
}
