//! View condition evaluation — evaluate the Boolean Expression that
//! backs an [`ElementFilterMembership`] against a candidate element.
//!
//! Phase 4.5a stubbed `ViewFilter::expression` evaluation to `true` so
//! views with filters wouldn't silently exclude every element while the
//! evaluator was being built. This module supplies the real evaluator
//! that the diagram crate calls before rendering.
//!
//! ## Calling convention
//!
//! - `expression_id` is normally the `ElementFilterMembership` element
//!   itself. The membership stores the filter expression as a
//!   `filterExpression` string property (Pest path) or as a child
//!   Boolean Expression element (tree-sitter / AST path). Both shapes
//!   are handled.
//! - `element` is the candidate being filtered. The evaluator binds it
//!   under the variable name `self`, so filter expressions like
//!   `self.kind == "PartUsage"` or simply `kind == "PartUsage"` (when
//!   FeatureRef falls back to looking up element props) can be written.
//! - On any error (missing expression, compile failure, runtime error,
//!   non-Boolean result) the evaluator returns `true`. This is the safe
//!   fallthrough — a broken filter must never silently delete elements
//!   from a diagram. Diagnostic surfacing is the IDE's job.

use std::sync::Arc;

use sysml_core::{Element, ElementId, ModelGraph, Value};

use crate::expressions::{
    compile_expression, compile_simple_expression, EvalContext, ExprIR, ExpressionEvaluator,
};

/// Evaluate `expression_id`'s Boolean Expression against `element`.
/// Returns `false` only when the expression compiled, evaluated, and
/// produced a Boolean `false`. Any other outcome returns `true`.
///
/// Compiles `expression_id` on every call. For hot diagram paths that
/// evaluate the same filter against thousands of candidate elements,
/// callers should precompile via [`resolve_filter_expr_ir`] (kept
/// public for this use case) and route through
/// [`evaluate_view_condition_with_compiled`] instead — see ADR-011 §3 /
/// S3.T6b for the cached-filter-expression pattern.
pub fn evaluate_view_condition(
    graph: &ModelGraph,
    expression_id: &ElementId,
    element: &Element,
) -> bool {
    let Some(holder) = graph.get_element(expression_id) else {
        return true;
    };
    let expr = match resolve_filter_expr_ir(graph, holder) {
        Some(ir) => ir,
        None => return true,
    };
    evaluate_view_condition_with_compiled(graph, &expr, element)
}

/// Evaluate a precompiled filter expression against `element`.
///
/// Same fall-through-to-true safety as
/// [`evaluate_view_condition`] — any non-Boolean result or
/// evaluator error returns `true` so a broken filter never silently
/// deletes elements from a diagram. The split exists so callers
/// holding a salsa-cached `ExprIR` (per S3.T6b's
/// `view_filter_exprs` tracked queries) can skip the per-element
/// compile step.
pub fn evaluate_view_condition_with_compiled(
    graph: &ModelGraph,
    expr: &ExprIR,
    element: &Element,
) -> bool {
    // Borrow-API convenience: wraps the graph in a fresh `Arc` per call.
    // Hot paths (per-element filtering in the diagram generator) must use
    // [`evaluate_view_condition_with_compiled_shared`] with a graph `Arc` built
    // ONCE per render — cloning the whole `ModelGraph` per element over the
    // stdlib-merged workspace graph made filtered views effectively never
    // terminate (the Requirement-view render hang, Jun 2026).
    evaluate_view_condition_with_compiled_shared(Arc::new(graph.clone()), expr, element)
}

/// Evaluate a precompiled filter expression against `element`, reusing a shared
/// `Arc<ModelGraph>`.
///
/// Same fall-through-to-true safety as [`evaluate_view_condition`]. This is the
/// per-element hot-path entry: the caller builds the graph `Arc` once and hands
/// a cheap `Arc::clone` in for every element, so a filtered view never
/// deep-clones the graph per candidate.
pub fn evaluate_view_condition_with_compiled_shared(
    graph: Arc<ModelGraph>,
    expr: &ExprIR,
    element: &Element,
) -> bool {
    let mut ctx = EvalContext::new();
    ctx.graph = Some(graph);
    ctx.set("self", Value::Ref(element.id.clone()));
    // Bind Element-intrinsic fields so filter expressions like
    // `kind == "PartUsage"` or `name == "Engine"` resolve directly.
    // These shadow nothing in practice — SysML feature names cannot
    // collide with these reserved keys inside a viewCondition because
    // the candidate is bound only as `self`.
    ctx.set("kind", Value::String(format!("{:?}", element.kind)));
    if let Some(name) = element.name.as_ref() {
        ctx.set("name", Value::String(name.clone()));
    }
    ctx.set("id", Value::String(element.id.to_string()));

    let evaluator = ExpressionEvaluator::new();
    match evaluator.eval(expr, &ctx) {
        Ok(Value::Bool(b)) => b,
        _ => true,
    }
}

/// Compile the filter expression carried by `holder` to ExprIR.
///
/// Tries (in order):
/// 1. `filterExpression` string property (Pest parser path)
/// 2. `compile_expression`'s AST walk over the holder's children
///    (tree-sitter / typed-AST path)
///
/// Public so the salsa `view_filter_exprs` query in `sysml-ide-db`
/// (S3.T6b) can build the precompiled-ExprIR cache once per graph
/// revision instead of re-deriving it on every passes_filter check.
pub fn resolve_filter_expr_ir(graph: &ModelGraph, holder: &Element) -> Option<ExprIR> {
    if let Some(Value::String(s)) = holder.get_prop("filterExpression") {
        if let Ok(ir) = compile_simple_expression(s) {
            return Some(ir);
        }
    }
    compile_expression(holder, graph).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysml_core::{ElementFactory, ElementKind};

    #[test]
    fn missing_expression_holder_returns_true() {
        let graph = ModelGraph::new();
        let bogus = sysml_core::ElementId::new_v4();
        let elem = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        assert!(evaluate_view_condition(&graph, &bogus, &elem));
    }

    #[test]
    fn literal_true_passes() {
        let mut graph = ModelGraph::new();
        let mut filter = ElementFactory::create(ElementKind::ElementFilterMembership);
        filter.set_prop("filterExpression", "true");
        let filter_id = filter.id.clone();
        graph.add_element(filter);

        let elem = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        assert!(evaluate_view_condition(&graph, &filter_id, &elem));
    }

    #[test]
    fn literal_false_excludes() {
        let mut graph = ModelGraph::new();
        let mut filter = ElementFactory::create(ElementKind::ElementFilterMembership);
        filter.set_prop("filterExpression", "false");
        let filter_id = filter.id.clone();
        graph.add_element(filter);

        let elem = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        assert!(!evaluate_view_condition(&graph, &filter_id, &elem));
    }

    #[test]
    fn non_boolean_result_falls_through_to_true() {
        let mut graph = ModelGraph::new();
        let mut filter = ElementFactory::create(ElementKind::ElementFilterMembership);
        // Numeric literal — the evaluator returns Value::Int(42), not
        // a Bool. Safe fallthrough applies.
        filter.set_prop("filterExpression", "42");
        let filter_id = filter.id.clone();
        graph.add_element(filter);

        let elem = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        assert!(evaluate_view_condition(&graph, &filter_id, &elem));
    }

    #[test]
    fn compile_failure_falls_through_to_true() {
        let mut graph = ModelGraph::new();
        let mut filter = ElementFactory::create(ElementKind::ElementFilterMembership);
        filter.set_prop("filterExpression", "@@@ not parseable @@@");
        let filter_id = filter.id.clone();
        graph.add_element(filter);

        let elem = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        assert!(evaluate_view_condition(&graph, &filter_id, &elem));
    }

    #[test]
    fn bare_kind_field_matches_partusage() {
        let mut graph = ModelGraph::new();
        let mut filter = ElementFactory::create(ElementKind::ElementFilterMembership);
        filter.set_prop("filterExpression", "kind == \"PartUsage\"");
        let filter_id = filter.id.clone();
        graph.add_element(filter);

        let part = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        let action = ElementFactory::create(ElementKind::ActionUsage).with_name("a");
        assert!(evaluate_view_condition(&graph, &filter_id, &part));
        assert!(!evaluate_view_condition(&graph, &filter_id, &action));
    }

    #[test]
    fn self_dot_kind_chain_matches_partusage() {
        // `self.kind` resolution walks via the graph, so the candidate
        // elements must be added before the chain projector can read
        // their intrinsic kind. This mirrors real diagram-time usage —
        // the filter only ever evaluates against elements that already
        // live in the model graph.
        let mut graph = ModelGraph::new();
        let mut filter = ElementFactory::create(ElementKind::ElementFilterMembership);
        filter.set_prop("filterExpression", "self.kind == \"PartUsage\"");
        let filter_id = filter.id.clone();
        graph.add_element(filter);

        let part = ElementFactory::create(ElementKind::PartUsage).with_name("p");
        let part_id = part.id.clone();
        graph.add_element(part);
        let action = ElementFactory::create(ElementKind::ActionUsage).with_name("a");
        let action_id = action.id.clone();
        graph.add_element(action);

        let part_ref = graph.get_element(&part_id).unwrap();
        let action_ref = graph.get_element(&action_id).unwrap();
        assert!(evaluate_view_condition(&graph, &filter_id, part_ref));
        assert!(!evaluate_view_condition(&graph, &filter_id, action_ref));
    }

    #[test]
    fn metaclass_filter_at_operator_selects_kind() {
        // Spec KerML `@` classification operator: `@SysML::RequirementUsage`
        // is true iff the candidate's abstract-syntax metaclass conforms to
        // RequirementUsage. Previously this errored → safe-fallthrough → the
        // filter silently passed everything (the bug Phase 3a fixes).
        let mut graph = ModelGraph::new();
        let mut filter = ElementFactory::create(ElementKind::ElementFilterMembership);
        filter.set_prop(
            "filterExpression",
            "@SysML::RequirementUsage or @SysML::RequirementDefinition",
        );
        let filter_id = filter.id.clone();
        graph.add_element(filter);

        let req = ElementFactory::create(ElementKind::RequirementUsage).with_name("R");
        let req_id = req.id.clone();
        graph.add_element(req);
        let reqdef = ElementFactory::create(ElementKind::RequirementDefinition).with_name("RD");
        let reqdef_id = reqdef.id.clone();
        graph.add_element(reqdef);
        let part = ElementFactory::create(ElementKind::PartUsage).with_name("P");
        let part_id = part.id.clone();
        graph.add_element(part);

        let g = &graph;
        assert!(evaluate_view_condition(g, &filter_id, g.get_element(&req_id).unwrap()));
        assert!(evaluate_view_condition(g, &filter_id, g.get_element(&reqdef_id).unwrap()));
        assert!(!evaluate_view_condition(g, &filter_id, g.get_element(&part_id).unwrap()));
    }

    #[test]
    fn metaclass_filter_at_operator_is_subtype_inclusive() {
        // `@` is conformance-based: a subtype metaclass matches a supertype
        // target. SatisfyRequirementUsage :> AssertConstraintUsage, so
        // `@SysML::AssertConstraintUsage` must match a SatisfyRequirementUsage.
        assert!(
            ElementKind::SatisfyRequirementUsage
                .is_subtype_of(ElementKind::AssertConstraintUsage),
            "precondition: SatisfyRequirementUsage is a subtype of AssertConstraintUsage"
        );
        let mut graph = ModelGraph::new();
        let mut filter = ElementFactory::create(ElementKind::ElementFilterMembership);
        filter.set_prop("filterExpression", "@SysML::AssertConstraintUsage");
        let filter_id = filter.id.clone();
        graph.add_element(filter);

        let satisfy =
            ElementFactory::create(ElementKind::SatisfyRequirementUsage).with_name("s");
        let satisfy_id = satisfy.id.clone();
        graph.add_element(satisfy);

        assert!(evaluate_view_condition(
            &graph,
            &filter_id,
            graph.get_element(&satisfy_id).unwrap()
        ));
    }

    #[test]
    fn bare_name_field_matches_string() {
        let mut graph = ModelGraph::new();
        let mut filter = ElementFactory::create(ElementKind::ElementFilterMembership);
        filter.set_prop("filterExpression", "name == \"Engine\"");
        let filter_id = filter.id.clone();
        graph.add_element(filter);

        let engine = ElementFactory::create(ElementKind::PartUsage).with_name("Engine");
        let gearbox = ElementFactory::create(ElementKind::PartUsage).with_name("Gearbox");
        assert!(evaluate_view_condition(&graph, &filter_id, &engine));
        assert!(!evaluate_view_condition(&graph, &filter_id, &gearbox));
    }
}
