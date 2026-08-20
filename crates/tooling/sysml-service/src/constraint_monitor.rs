//! Live constraint violation diagnostics.
//!
//! After parse+resolve, evaluates constraint elements and surfaces violations
//! as diagnostics with squiggly underlines.
//!
//! - `AssertConstraintUsage` → Warning (C001): invariant assertions that must hold
//! - `ConstraintUsage` → Info (C002): non-assert constraints checked best-effort

use sysml_core::{ElementKind, ModelGraph};
use sysml_runtime::expressions::{compile_expression, ExpressionEvaluator};
use sysml_span::Span;

use crate::evaluation::build_eval_context;
use crate::expression_ast::project_owner;

/// A constraint violation ready for diagnostic conversion.
pub struct ConstraintDiagnostic {
    pub span: Option<Span>,
    pub message: String,
    pub constraint_name: Option<String>,
    pub is_assert: bool,
    /// Structured AST of the violating constraint expression, for clients
    /// that want to render math (KaTeX) in the problem panel. `None` when
    /// the element has no structured expression children (legacy path).
    pub ast: Option<serde_json::Value>,
}

/// Check all constraint elements in the graph for violations.
///
/// CONFORMS-REQUIRED (deferred, perf-gated): this is the LSP-diagnostic path —
/// it runs on the keystroke pipeline (debounced) — and is deliberately kept
/// definition/scope-static (`build_eval_context` per element, one verdict per
/// constraint). It is the only keystroke-path consumer, so it must NOT be
/// routed onto the per-instance primitive until a perf-safe per-occurrence path
/// is designed (the per-instance path builds an instance tree). Single-instance
/// values are resolved correctly via the owner-scoped context. See
/// `evaluate_constraints_impl` for the shared deferral.
pub fn check_constraints(graph: &ModelGraph, source_uri: &str) -> Vec<ConstraintDiagnostic> {
    let mut results = Vec::new();

    for element in graph.elements.values() {
        let is_assert = element.kind == ElementKind::AssertConstraintUsage;
        if !is_assert && element.kind != ElementKind::ConstraintUsage {
            continue;
        }

        let expr_str = sysml_core::expression_pretty::pretty_print_owner(element, graph)
            .or_else(|| {
                element
                    .get_prop("constraint")
                    .and_then(|v| v.as_str())
                    .or_else(|| element.get_prop("expr").and_then(|v| v.as_str()))
                    .map(|s| s.to_owned())
            })
            .unwrap_or_else(|| element.name.clone().unwrap_or_else(|| "<expression>".into()));

        let ir = match compile_expression(element, graph) {
            Ok(ir) => ir,
            Err(diags) => {
                tracing::debug!(
                    constraint = ?element.name,
                    expression = %expr_str,
                    diagnostics = ?diags,
                    "skipping constraint: expression failed to compile"
                );
                continue;
            }
        };

        let ctx = build_eval_context(element, graph);
        let evaluator = ExpressionEvaluator::new();

        match evaluator.eval(&ir, &ctx) {
            Ok(sysml_core::Value::Bool(true)) => {}
            Ok(sysml_core::Value::Bool(false)) => {
                let name = element.name.clone();
                let span = element.spans.iter().find(|s| s.file == source_uri).cloned();
                let kind_label = if is_assert {
                    "Assert constraint"
                } else {
                    "Constraint"
                };
                let message = if let Some(ref n) = name {
                    format!("{} '{}' violated: `{}`", kind_label, n, expr_str)
                } else {
                    format!("{} violated: `{}`", kind_label, expr_str)
                };
                let ast = project_owner(element, graph)
                    .and_then(|r| r.ast)
                    .and_then(|node| serde_json::to_value(node).ok());
                results.push(ConstraintDiagnostic {
                    span,
                    message,
                    constraint_name: name,
                    is_assert,
                    ast,
                });
            }
            Ok(other) => {
                tracing::debug!(
                    constraint = ?element.name,
                    expression = %expr_str,
                    value = ?other,
                    "skipping constraint: expression did not evaluate to boolean"
                );
            }
            Err(e) => {
                tracing::debug!(
                    constraint = ?element.name,
                    expression = %expr_str,
                    error = %e,
                    "skipping constraint: expression evaluation failed"
                );
            }
        }
    }

    results
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind, ModelGraph, Value, VisibilityKind};
    use sysml_id::ElementId;

    fn make_element(kind: ElementKind, name: &str) -> Element {
        Element::new(ElementId::new_v4(), kind).with_name(name)
    }

    #[test]
    fn violation_detected_for_failing_assert() {
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage).with_name("vehicle");
        graph.add_element(owner);

        let speed =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(150));
        graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

        let assert_c = make_element(ElementKind::AssertConstraintUsage, "speedLimit")
            .with_prop("constraint", Value::String("speed < 100".into()));
        graph.add_owned_element(assert_c, owner_id.clone(), VisibilityKind::Public);

        let diags = check_constraints(&graph, "test.sysml");
        assert_eq!(diags.len(), 1, "should detect one violation");
        assert!(diags[0].message.contains("speedLimit"));
        assert!(diags[0].message.contains("violated"));
    }

    #[test]
    fn no_false_positive_for_passing_constraint() {
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage).with_name("vehicle");
        graph.add_element(owner);

        let speed =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(50));
        graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

        let assert_c = make_element(ElementKind::AssertConstraintUsage, "speedLimit")
            .with_prop("constraint", Value::String("speed < 100".into()));
        graph.add_owned_element(assert_c, owner_id.clone(), VisibilityKind::Public);

        let diags = check_constraints(&graph, "test.sysml");
        assert!(diags.is_empty(), "should have no violations");
    }

    #[test]
    fn regular_constraint_also_checked() {
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage).with_name("vehicle");
        graph.add_element(owner);

        let speed =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(150));
        graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

        let regular_c = make_element(ElementKind::ConstraintUsage, "guideline")
            .with_prop("constraint", Value::String("speed < 100".into()));
        graph.add_owned_element(regular_c, owner_id.clone(), VisibilityKind::Public);

        let diags = check_constraints(&graph, "test.sysml");
        assert_eq!(
            diags.len(),
            1,
            "regular ConstraintUsage should produce violation"
        );
        assert!(!diags[0].is_assert, "should not be marked as assert");
        assert!(diags[0].message.contains("Constraint"));
        assert!(!diags[0].message.contains("Assert"));
    }

    #[test]
    fn assert_constraint_is_marked_as_assert() {
        let mut graph = ModelGraph::new();

        let owner_id = ElementId::new_v4();
        let owner = Element::new(owner_id.clone(), ElementKind::PartUsage).with_name("vehicle");
        graph.add_element(owner);

        let speed =
            make_element(ElementKind::AttributeUsage, "speed").with_prop("value", Value::Int(150));
        graph.add_owned_element(speed, owner_id.clone(), VisibilityKind::Public);

        let assert_c = make_element(ElementKind::AssertConstraintUsage, "limit")
            .with_prop("constraint", Value::String("speed < 100".into()));
        graph.add_owned_element(assert_c, owner_id.clone(), VisibilityKind::Public);

        let diags = check_constraints(&graph, "test.sysml");
        assert_eq!(diags.len(), 1);
        assert!(diags[0].is_assert, "should be marked as assert");
        assert!(diags[0].message.contains("Assert constraint"));
    }
}
