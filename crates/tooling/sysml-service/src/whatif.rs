//! What-If parameter analysis (F4).
//!
//! Provides parameter-sweep analysis across a value range, surfacing the
//! threshold where any constraint changes satisfaction. The single-shot
//! "override one variable and report flips" path lives inline in
//! `SysmlService::whatif` (whole-graph constraint evaluation via
//! `sysml_runtime::constraints::extract_and_precompile`).

use sysml_core::{Element, ElementKind, ModelGraph, Value};
use sysml_runtime::expressions::{compile_expression, ExpressionEvaluator};

use crate::evaluation::build_eval_context;

// ---------------------------------------------------------------------------
// Result types
// ---------------------------------------------------------------------------

/// Result of sweeping a parameter across a range.
#[derive(Debug)]
pub struct SweepResult {
    /// The variable being swept.
    pub variable_name: String,
    /// Per-step results: (value, Vec<(constraint_name, satisfied)>).
    pub steps: Vec<SweepStep>,
    /// The threshold value where any constraint flips (if found).
    pub threshold: Option<Value>,
}

/// One step in a parameter sweep.
#[derive(Debug)]
pub struct SweepStep {
    pub value: Value,
    pub constraint_results: Vec<(String, bool)>,
}

// ---------------------------------------------------------------------------
// Core functions
// ---------------------------------------------------------------------------

/// Sweep a parameter across a range and evaluate constraints at each step.
///
/// Delegates to `sysml_runtime::solver::sweep_parameter()` for the core
/// computation, then converts the result to service-level types.
///
/// This avoids duplicating the sweep logic that also exists in the runtime
/// solver module (used by `sysml solve --sweep`).
pub fn sweep_parameter(
    element: &Element,
    graph: &ModelGraph,
    variable_name: &str,
    start: f64,
    end: f64,
    steps: usize,
) -> SweepResult {
    let Some(owner_id) = element.owner.as_ref() else {
        return SweepResult {
            variable_name: variable_name.to_owned(),
            steps: Vec::new(),
            threshold: None,
        };
    };

    // Build baseline context
    let baseline_ctx = build_eval_context(element, graph);

    // Collect and pre-compile constraints into a PrecompiledConstraintSet
    // (the format that the runtime solver expects)
    let constraints: Vec<_> = graph
        .children_of(owner_id)
        .filter(|e| {
            matches!(
                e.kind,
                ElementKind::ConstraintUsage | ElementKind::AssertConstraintUsage
            )
        })
        .filter_map(|e| {
            let name = e.name.clone().unwrap_or_default();
            // AST-native: compile structured expression tree; skip if the
            // element has no expression body or compilation fails.
            let ir = compile_expression(e, graph).ok()?;
            Some(sysml_runtime::constraints::TypedConstraint {
                constraint: sysml_runtime::ConstraintIR {
                    // `expr` is a human-readable label for the solver's
                    // per-constraint diagnostic output; use the name when
                    // present, else a neutral placeholder.
                    expr: if name.is_empty() {
                        format!("constraint#{}", e.id)
                    } else {
                        name.clone()
                    },
                    description: Some(name),
                    owner_id: None,
                    is_negated: false,
                },
                expr_ir: ir,
            })
        })
        .collect();

    let precompiled = sysml_runtime::constraints::PrecompiledConstraintSet {
        compiled: constraints,
        failed: Vec::new(),
    };

    // Delegate to the runtime solver
    let runtime_result = sysml_runtime::solver::sweep_parameter(
        variable_name,
        (start, end),
        steps,
        &precompiled,
        &baseline_ctx,
    );

    // Convert runtime result to service types
    let mut sweep_steps = Vec::new();
    let mut threshold = None;
    let mut prev_satisfied: Option<Vec<bool>> = None;

    for (_i, &sample_value) in runtime_result.samples.iter().enumerate() {
        let results: Vec<(String, bool)> = runtime_result
            .constraint_effects
            .iter()
            .map(|effect| {
                // Reconstruct per-step satisfaction from the sweep data
                // The runtime doesn't store per-step results, so we re-evaluate
                let ctx = baseline_ctx.child_with(variable_name, Value::Float(sample_value));
                let satisfied = precompiled.compiled.iter()
                    .find(|tc| tc.constraint.description.as_deref() == Some(&effect.constraint_name))
                    .map(|tc| {
                        let evaluator = ExpressionEvaluator::new();
                        matches!(evaluator.eval(&tc.expr_ir, &ctx), Ok(Value::Bool(true)))
                    })
                    .unwrap_or(false);
                (effect.constraint_name.clone(), satisfied)
            })
            .collect();

        // Check for threshold (first flip)
        if let Some(prev) = &prev_satisfied {
            if threshold.is_none() {
                let current: Vec<bool> = results.iter().map(|(_, s)| *s).collect();
                if prev != &current {
                    threshold = Some(Value::Float(sample_value));
                }
            }
        }
        prev_satisfied = Some(results.iter().map(|(_, s)| *s).collect());

        sweep_steps.push(SweepStep {
            value: Value::Float(sample_value),
            constraint_results: results,
        });
    }

    SweepResult {
        variable_name: variable_name.to_owned(),
        steps: sweep_steps,
        threshold,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementKind, ModelGraph};

    fn make_element(kind: ElementKind, name: &str) -> Element {
        Element::new_with_kind(kind).with_name(name)
    }

    #[test]
    fn test_sweep_finds_threshold() {
        let mut graph = ModelGraph::new();

        let owner = make_element(ElementKind::PartUsage, "TestPart");
        let owner_id = graph.add_element(owner);

        let var_elem = Element::new_with_kind(ElementKind::AttributeUsage)
            .with_name("value")
            .with_owner(owner_id.clone())
            .with_prop("value", Value::Float(0.0));
        let var_id = graph.add_element(var_elem);

        // Constraint: value < 50 (flips at 50)
        let constraint = Element::new_with_kind(ElementKind::ConstraintUsage)
            .with_name("limit")
            .with_owner(owner_id.clone())
            .with_prop("constraint", "value < 50");
        graph.add_element(constraint);

        let var_elem = graph.get_element(&var_id).unwrap();
        let result = sweep_parameter(var_elem, &graph, "value", 0.0, 100.0, 11);

        // Should have 11 steps
        assert_eq!(result.steps.len(), 11);

        // First step (value=0) should PASS
        assert_eq!(result.steps[0].constraint_results[0].1, true);

        // Last step (value=100) should FAIL
        assert_eq!(result.steps[10].constraint_results[0].1, false);

        // Should find a threshold
        assert!(result.threshold.is_some());

        // Threshold should be somewhere between 0 and 100
        if let Some(Value::Float(t)) = result.threshold {
            assert!(t > 0.0 && t <= 100.0);
        } else {
            panic!("Expected Float threshold");
        }
    }
}
