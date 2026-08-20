//! `calc def` evaluation — compile and execute SysML v2 CalculationDefinitions.
//!
//! A `CalculationDefinition` is both an `ActionDefinition` (it executes) and a
//! `Function` (it returns a result). The body contains parameters (`in`, `out`,
//! `return`) and a trailing result expression.
//!
//! # Spec reference
//!
//! - `Calculations.sysml`, `Performances.kerml::Evaluation`
//! - `SysML.xtext:1925-1974`
//!
//! # Usage
//!
//! ```ignore
//! let (registry, diags) = CalculationRegistry::compile_all_from_graph(&graph);
//! let result = registry.evaluate("ThermalDerivative", &args, &ctx)?;
//! ```

use std::collections::HashMap;

use sysml_core::{ElementId, ElementKind, ModelGraph, Value};
use sysml_span::Diagnostic;

use crate::expressions::{
    compile_expression, compile_simple_expression, EvalContext, EvaluationError, ExprIR,
    ExpressionEvaluator,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Direction of a calculation parameter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamDirection {
    In,
    Out,
    Return,
}

/// A single parameter of a `calc def`.
#[derive(Debug, Clone)]
pub struct CalcParam {
    /// Parameter name.
    pub name: String,
    /// Direction: in, out, or return.
    pub direction: ParamDirection,
    /// Default value (if specified in the model).
    pub default_value: Option<Value>,
}

/// Compiled representation of a `calc def`.
///
/// Created by [`compile_calculation`] or [`CalculationRegistry::compile_all_from_graph`].
#[derive(Debug, Clone)]
pub struct CalculationIR {
    /// Qualified name (e.g., `"ThermalDerivative"`).
    pub name: String,
    /// Parameters with direction and optional default.
    pub parameters: Vec<CalcParam>,
    /// Compiled result expression.
    pub result_expr: ExprIR,
    /// Owner element ID for diagnostics.
    pub owner_id: Option<ElementId>,
}

// ---------------------------------------------------------------------------
// Compilation
// ---------------------------------------------------------------------------

/// Compile a `CalculationDefinition` element into a `CalculationIR`.
///
/// Extracts parameters from child `AttributeUsage` elements with direction
/// properties, and the result expression from the `return` parameter or
/// trailing expression.
pub fn compile_calculation(
    element: &sysml_core::Element,
    graph: &ModelGraph,
) -> Result<CalculationIR, Vec<Diagnostic>> {
    let name = element
        .name
        .clone()
        .unwrap_or_else(|| "<anonymous>".to_string());
    let mut parameters = Vec::new();
    let mut result_expr: Option<ExprIR> = None;

    // Walk children to find parameters and result expression. For each
    // candidate result-expression child, compile AST-first via
    // `compile_expression` (which already falls back to string props).
    for child in graph.children_of(&element.id) {
        let child_name = child.name.as_deref().unwrap_or("");
        let direction = child
            .get_prop("direction")
            .and_then(|v| match v {
                Value::String(s) => Some(s.as_str()),
                _ => None,
            })
            .or_else(|| {
                // Also check the kind for return parameters
                if matches!(child.kind, ElementKind::ReturnParameterMembership) {
                    Some("return")
                } else {
                    None
                }
            });

        match direction {
            Some(s) if s == "in" => {
                let default = child
                    .get_prop("default")
                    .cloned()
                    .or_else(|| child.get_prop("value").cloned());
                parameters.push(CalcParam {
                    name: child_name.to_string(),
                    direction: ParamDirection::In,
                    default_value: default,
                });
            }
            Some(s) if s == "out" => {
                parameters.push(CalcParam {
                    name: child_name.to_string(),
                    direction: ParamDirection::Out,
                    default_value: None,
                });
            }
            Some(s) if s == "return" => {
                // The return parameter's expression is the result.
                // Try AST-first, then fall back to legacy string props
                // (`default`/`value` which compile_expression doesn't honor).
                if result_expr.is_none() {
                    if let Ok(ir) = compile_expression(child, graph) {
                        result_expr = Some(ir);
                    } else if let Some(text) = child
                        .get_prop("default")
                        .or_else(|| child.get_prop("value"))
                        .and_then(|v| match v {
                            Value::String(s) => Some(s.clone()),
                            _ => None,
                        })
                    {
                        result_expr = Some(compile_simple_expression(&text)?);
                    }
                }
                parameters.push(CalcParam {
                    name: child_name.to_string(),
                    direction: ParamDirection::Return,
                    default_value: None,
                });
            }
            _ => {
                // No direction — could be the trailing result expression
                // child (attribute with an expression subtree).
                if result_expr.is_none() {
                    if let Ok(ir) = compile_expression(child, graph) {
                        result_expr = Some(ir);
                    } else if let Some(text) = child.get_prop("default").and_then(|v| match v {
                        Value::String(s) => Some(s.clone()),
                        _ => None,
                    }) {
                        result_expr = Some(compile_simple_expression(&text)?);
                    }
                }
            }
        }
    }

    // Fall back to a result expression on the element itself.
    if result_expr.is_none() {
        if let Ok(ir) = compile_expression(element, graph) {
            result_expr = Some(ir);
        } else if let Some(text) = element.get_prop("result").and_then(|v| match v {
            Value::String(s) => Some(s.clone()),
            _ => None,
        }) {
            result_expr = Some(compile_simple_expression(&text)?);
        }
    }

    let result_expr = result_expr.ok_or_else(|| {
        vec![Diagnostic::error(format!(
            "calc def '{}': no result expression found",
            name
        ))]
    })?;

    Ok(CalculationIR {
        name,
        parameters,
        result_expr,
        owner_id: Some(element.id.clone()),
    })
}

// ---------------------------------------------------------------------------
// Evaluation
// ---------------------------------------------------------------------------

/// Evaluate a compiled calculation with the given arguments.
///
/// Arguments are bound to parameter names in a scoped `EvalContext`.
/// Missing parameters use their default values if available.
pub fn evaluate_calculation(
    calc: &CalculationIR,
    args: &[(String, Value)],
    ctx: &EvalContext,
) -> Result<Value, EvaluationError> {
    // Create a scoped context inheriting from parent
    let mut eval_ctx = ctx.scratch_snapshot();

    // Bind all named arguments into the context first
    for (name, val) in args {
        eval_ctx.set(name.clone(), val.clone());
    }

    // Apply defaults for declared parameters that weren't provided
    let arg_map: HashMap<&str, &Value> = args.iter().map(|(k, v)| (k.as_str(), v)).collect();
    for param in &calc.parameters {
        if !arg_map.contains_key(param.name.as_str()) {
            if let Some(ref default) = param.default_value {
                eval_ctx.set(param.name.clone(), default.clone());
            }
        }
    }

    let evaluator = ExpressionEvaluator::new();
    evaluator.eval(&calc.result_expr, &eval_ctx)
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Registry of compiled calculation definitions, keyed by name.
///
/// Built from a `ModelGraph` by scanning all `CalculationDefinition` elements.
/// Attached to `EvalContext` for function call dispatch.
#[derive(Debug, Clone, Default)]
pub struct CalculationRegistry {
    calcs: HashMap<String, CalculationIR>,
}

impl CalculationRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a compiled calculation.
    pub fn register(&mut self, calc: CalculationIR) {
        self.calcs.insert(calc.name.clone(), calc);
    }

    /// Look up a calculation by name.
    pub fn get(&self, name: &str) -> Option<&CalculationIR> {
        self.calcs.get(name)
    }

    /// Number of registered calculations.
    pub fn len(&self) -> usize {
        self.calcs.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.calcs.is_empty()
    }

    /// Compile all `CalculationDefinition` elements from a model graph.
    pub fn compile_all_from_graph(graph: &ModelGraph) -> (Self, Vec<Diagnostic>) {
        let mut registry = Self::new();
        let mut diagnostics = Vec::new();

        for element in graph.elements_by_kind(&ElementKind::CalculationDefinition) {
            match compile_calculation(element, graph) {
                Ok(calc) => {
                    registry.register(calc);
                }
                Err(diags) => {
                    diagnostics.extend(diags);
                }
            }
        }

        (registry, diagnostics)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, ElementId};

    /// Build a model with a calc def that has an expression directly on the element.
    fn build_calc_model(name: &str, expr: &str) -> ModelGraph {
        let mut graph = ModelGraph::new();
        let id = ElementId::new_v4();
        let mut elem = Element::new(id, ElementKind::CalculationDefinition);
        elem.name = Some(name.to_string());
        elem.set_prop("result", Value::String(expr.to_string()));
        graph.add_element(elem);
        graph
    }

    /// Build a model with a calc def that has parameters as children.
    fn build_calc_with_params(name: &str, params: &[(&str, &str)], expr: &str) -> ModelGraph {
        let mut graph = ModelGraph::new();
        let calc_id = ElementId::new_v4();
        let mut calc_elem = Element::new(calc_id.clone(), ElementKind::CalculationDefinition);
        calc_elem.name = Some(name.to_string());
        calc_elem.set_prop("result", Value::String(expr.to_string()));
        graph.add_element(calc_elem);

        for (param_name, direction) in params {
            let param_id = ElementId::new_v4();
            let mut param_elem = Element::new(param_id, ElementKind::AttributeUsage);
            param_elem.name = Some(param_name.to_string());
            param_elem.set_prop("direction", Value::String(direction.to_string()));
            param_elem.owner = Some(calc_id.clone());
            graph.add_element(param_elem);
        }

        graph
    }

    #[test]
    fn test_compile_simple_calc() {
        let graph = build_calc_model("Add", "a + b");
        let (registry, diags) = CalculationRegistry::compile_all_from_graph(&graph);
        assert!(diags.is_empty(), "unexpected diags: {:?}", diags);
        assert_eq!(registry.len(), 1);
        let calc = registry.get("Add").unwrap();
        assert_eq!(calc.name, "Add");
    }

    #[test]
    fn test_evaluate_simple_calc() {
        let graph = build_calc_model("Add", "a + b");
        let (registry, _) = CalculationRegistry::compile_all_from_graph(&graph);
        let calc = registry.get("Add").unwrap();

        let ctx = EvalContext::new();
        let result = evaluate_calculation(
            calc,
            &[
                ("a".to_string(), Value::Float(3.0)),
                ("b".to_string(), Value::Float(4.0)),
            ],
            &ctx,
        )
        .unwrap();
        assert_eq!(result, Value::Float(7.0));
    }

    #[test]
    fn test_calc_with_parameters() {
        let graph = build_calc_with_params(
            "ThermalDeriv",
            &[("T", "in"), ("I", "in"), ("R", "in"), ("C", "in")],
            "(I * I * R - T) / C",
        );
        let (registry, diags) = CalculationRegistry::compile_all_from_graph(&graph);
        assert!(diags.is_empty(), "unexpected diags: {:?}", diags);
        let calc = registry.get("ThermalDeriv").unwrap();
        assert_eq!(calc.parameters.len(), 4);

        let ctx = EvalContext::new();
        let result = evaluate_calculation(
            calc,
            &[
                ("T".to_string(), Value::Float(25.0)),
                ("I".to_string(), Value::Float(10.0)),
                ("R".to_string(), Value::Float(1.0)),
                ("C".to_string(), Value::Float(100.0)),
            ],
            &ctx,
        )
        .unwrap();
        // (10*10*1 - 25) / 100 = 75/100 = 0.75
        match result {
            Value::Float(f) => assert!((f - 0.75).abs() < 1e-10, "expected 0.75, got {}", f),
            _ => panic!("expected float, got {:?}", result),
        }
    }

    #[test]
    fn test_calc_default_params() {
        let mut graph = ModelGraph::new();
        let calc_id = ElementId::new_v4();
        let mut calc_elem = Element::new(calc_id.clone(), ElementKind::CalculationDefinition);
        calc_elem.name = Some("Scale".to_string());
        calc_elem.set_prop("result", Value::String("x * factor".to_string()));
        graph.add_element(calc_elem);

        // Add "factor" param with default=2.0
        let param_id = ElementId::new_v4();
        let mut param_elem = Element::new(param_id, ElementKind::AttributeUsage);
        param_elem.name = Some("factor".to_string());
        param_elem.set_prop("direction", Value::String("in".to_string()));
        param_elem.set_prop("default", Value::Float(2.0));
        param_elem.owner = Some(calc_id.clone());
        graph.add_element(param_elem);

        let (registry, _) = CalculationRegistry::compile_all_from_graph(&graph);
        let calc = registry.get("Scale").unwrap();

        let ctx = EvalContext::new();
        // Only pass x, let factor use default
        let result =
            evaluate_calculation(calc, &[("x".to_string(), Value::Float(5.0))], &ctx).unwrap();
        assert_eq!(result, Value::Float(10.0));
    }

    #[test]
    fn test_calc_nested_call() {
        // Calc "Double" doubles x, Calc "Quad" calls Double twice (simulated)
        let double_graph = build_calc_model("Double", "x * 2");
        let (registry, _) = CalculationRegistry::compile_all_from_graph(&double_graph);
        let double = registry.get("Double").unwrap();

        let ctx = EvalContext::new();
        let first =
            evaluate_calculation(double, &[("x".to_string(), Value::Float(3.0))], &ctx).unwrap();
        // Use the result as input for a second call
        let second = evaluate_calculation(double, &[("x".to_string(), first)], &ctx).unwrap();
        assert_eq!(second, Value::Float(12.0));
    }

    #[test]
    fn test_compile_all_multiple_calcs() {
        let mut graph = ModelGraph::new();
        for (name, expr) in [("Add", "a + b"), ("Mul", "a * b"), ("Neg", "0 - x")] {
            let id = ElementId::new_v4();
            let mut elem = Element::new(id, ElementKind::CalculationDefinition);
            elem.name = Some(name.to_string());
            elem.set_prop("result", Value::String(expr.to_string()));
            graph.add_element(elem);
        }
        let (registry, diags) = CalculationRegistry::compile_all_from_graph(&graph);
        assert!(diags.is_empty());
        assert_eq!(registry.len(), 3);
        assert!(registry.get("Add").is_some());
        assert!(registry.get("Mul").is_some());
        assert!(registry.get("Neg").is_some());
    }

    #[test]
    fn test_unknown_calc_still_errors() {
        let registry = CalculationRegistry::new();
        assert!(registry.get("NonExistent").is_none());
    }

    #[test]
    fn test_evaluator_dispatches_to_calc_registry() {
        use std::sync::Arc;

        // Register a calc def "Double" that computes x * 2
        let graph = build_calc_with_params("Double", &[("x", "in")], "x * 2");
        let (registry, _) = CalculationRegistry::compile_all_from_graph(&graph);

        // Create EvalContext with the registry
        let mut ctx = EvalContext::new();
        ctx.calculations = Some(Arc::new(registry));

        // Evaluate a FunctionCall expression that invokes "Double(5)"
        let expr = ExprIR::FunctionCall {
            name: "Double".to_string(),
            args: vec![ExprIR::LiteralReal(5.0)],
        };
        let evaluator = ExpressionEvaluator::new();
        let result = evaluator.eval(&expr, &ctx).unwrap();
        assert_eq!(result, Value::Float(10.0));
    }

    #[test]
    fn test_stdlib_takes_precedence_over_calc_registry() {
        use std::sync::Arc;

        // Register a calc named "abs" (conflicts with stdlib)
        let graph = build_calc_model("abs", "x * 100");
        let (registry, _) = CalculationRegistry::compile_all_from_graph(&graph);

        let mut ctx = EvalContext::new();
        ctx.calculations = Some(Arc::new(registry));

        // "abs" should hit stdlib (returns absolute value), not our calc def
        let expr = ExprIR::FunctionCall {
            name: "abs".to_string(),
            args: vec![ExprIR::LiteralReal(-3.0)],
        };
        let evaluator = ExpressionEvaluator::new();
        let result = evaluator.eval(&expr, &ctx).unwrap();
        // stdlib abs(-3) = 3.0, NOT calc def (-3 * 100 = -300)
        assert_eq!(result, Value::Float(3.0));
    }

    #[test]
    fn test_unknown_function_still_errors_with_registry() {
        use std::sync::Arc;

        let registry = CalculationRegistry::new();
        let mut ctx = EvalContext::new();
        ctx.calculations = Some(Arc::new(registry));

        let expr = ExprIR::FunctionCall {
            name: "noSuchFunction".to_string(),
            args: vec![ExprIR::LiteralReal(1.0)],
        };
        let evaluator = ExpressionEvaluator::new();
        let result = evaluator.eval(&expr, &ctx);
        assert!(result.is_err());
    }
}
