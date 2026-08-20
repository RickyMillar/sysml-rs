//! Additional built-in solver implementations.
//!
//! Provides:
//! - [`BisectionSolver`] — wraps the numeric bisection solver from [`crate::solver`]
//!   to find single-variable solutions to constraints.
//! - [`ConstraintEvaluator`] — evaluates constraints for pass/fail without solving
//!   for unknowns.
//!
//! Both are registered by [`SolverRegistry::with_builtins()`] alongside the
//! default `PropagationSolver`.

#![allow(clippy::indexing_slicing)]
use std::collections::HashMap;

use sysml_core::Value;

use crate::constraints::EvalContext;
use crate::expressions::{compile_simple_expression, ExpressionEvaluator};
use crate::solver::solve_constraint;
use crate::solver_plugin::{
    ParamDirection, SolverCapabilities, SolverError, SolverParam, SolverPlugin, SolverResult,
};
use crate::ConstraintIR;

// ---------------------------------------------------------------------------
// BisectionSolver
// ---------------------------------------------------------------------------

/// Built-in solver using bisection search for single-variable constraints.
///
/// For each constraint, this solver:
/// 1. Builds a set of known values from input parameters.
/// 2. Compiles the constraint expression.
/// 3. Calls [`solve_constraint()`] which identifies the lone free variable
///    and uses bisection to find its value.
/// 4. Folds newly solved values into the known set for subsequent constraints.
pub struct BisectionSolver {
    /// The (low, high) search range for bisection.
    pub search_range: (f64, f64),
}

impl Default for BisectionSolver {
    fn default() -> Self {
        Self {
            search_range: (-1e6, 1e6),
        }
    }
}

impl SolverPlugin for BisectionSolver {
    fn name(&self) -> &str {
        "builtin:bisection"
    }

    fn solve(
        &self,
        inputs: &[SolverParam],
        constraints: &[ConstraintIR],
        _context: &EvalContext,
    ) -> Result<SolverResult, SolverError> {
        // Build known values from input parameters
        let mut known: HashMap<String, Value> = HashMap::new();
        for param in inputs {
            if param.direction == ParamDirection::In || param.direction == ParamDirection::InOut {
                if let Some(ref val) = param.value {
                    let key = param.tool_name.as_deref().unwrap_or(&param.sysml_name);
                    known.insert(key.to_owned(), val.clone());
                }
            }
        }

        let mut total_iterations: usize = 0;
        let mut all_converged = true;
        let mut diagnostics = Vec::new();

        // For each constraint, attempt to solve for the single free variable
        for constraint in constraints {
            let compiled = match compile_simple_expression(&constraint.expr) {
                Ok(expr) => expr,
                Err(diags) => {
                    diagnostics.extend(diags);
                    all_converged = false;
                    continue;
                }
            };

            match solve_constraint(&compiled, &known, self.search_range) {
                crate::solver::SolveResult::Solved { variable, value } => {
                    known.insert(variable, Value::Float(value));
                    total_iterations += 1;
                }
                crate::solver::SolveResult::AlreadyDetermined => {
                    // Constraint is already satisfied by known values
                }
                crate::solver::SolveResult::NoSolution { variable, reason } => {
                    all_converged = false;
                    diagnostics.push(sysml_span::Diagnostic::warning(format!(
                        "bisection could not solve for '{}': {}",
                        variable, reason
                    )));
                }
                crate::solver::SolveResult::UnderDetermined { free_vars } => {
                    all_converged = false;
                    diagnostics.push(sysml_span::Diagnostic::warning(format!(
                        "constraint has {} free variables ({}), need more equations",
                        free_vars.len(),
                        free_vars.join(", ")
                    )));
                }
                crate::solver::SolveResult::MultiSolved { solutions } => {
                    for (var, val) in solutions {
                        known.insert(var, Value::Float(val));
                    }
                    total_iterations += 1;
                }
            }
        }

        // Collect outputs: values that were solved (exclude original inputs)
        let input_keys: std::collections::HashSet<String> = inputs
            .iter()
            .filter(|p| p.direction == ParamDirection::In)
            .map(|p| p.tool_name.as_deref().unwrap_or(&p.sysml_name).to_owned())
            .collect();

        let outputs: HashMap<String, Value> = known
            .into_iter()
            .filter(|(k, _)| !input_keys.contains(k))
            .collect();

        Ok(SolverResult {
            outputs,
            diagnostics,
            iterations: Some(total_iterations),
            converged: all_converged,
        })
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            max_variables: Some(1), // One variable per constraint
            supports_constraints: true,
            supports_optimization: false,
            supports_sensitivity: false,
        }
    }
}

// ---------------------------------------------------------------------------
// ConstraintEvaluator
// ---------------------------------------------------------------------------

/// Built-in solver that evaluates constraints for pass/fail.
///
/// Unlike the bisection solver, this does not solve for unknowns. Instead
/// it checks whether each constraint is satisfied given the current input
/// values, returning `Value::Bool` results for each constraint.
pub struct ConstraintEvaluator;

impl SolverPlugin for ConstraintEvaluator {
    fn name(&self) -> &str {
        "builtin:evaluate"
    }

    fn solve(
        &self,
        inputs: &[SolverParam],
        constraints: &[ConstraintIR],
        _context: &EvalContext,
    ) -> Result<SolverResult, SolverError> {
        // Build evaluation context from input parameters
        let mut eval_ctx = EvalContext::new();
        for param in inputs {
            if let Some(ref val) = param.value {
                let key = param.tool_name.as_deref().unwrap_or(&param.sysml_name);
                eval_ctx.set(key.to_owned(), val.clone());
            }
        }

        let evaluator = ExpressionEvaluator::new();
        let mut outputs = HashMap::new();
        let mut diagnostics = Vec::new();
        let mut all_passed = true;

        for (i, constraint) in constraints.iter().enumerate() {
            let key = constraint
                .description
                .clone()
                .unwrap_or_else(|| format!("constraint_{}", i));

            match compile_simple_expression(&constraint.expr) {
                Ok(expr) => match evaluator.eval(&expr, &eval_ctx) {
                    Ok(Value::Bool(b)) => {
                        if !b {
                            all_passed = false;
                        }
                        outputs.insert(key, Value::Bool(b));
                    }
                    Ok(other) => {
                        // Non-boolean result: treat as error
                        diagnostics.push(sysml_span::Diagnostic::warning(format!(
                            "constraint '{}' evaluated to non-boolean: {:?}",
                            constraint.expr, other
                        )));
                        outputs.insert(key, Value::Bool(false));
                        all_passed = false;
                    }
                    Err(e) => {
                        diagnostics.push(sysml_span::Diagnostic::warning(format!(
                            "constraint '{}' evaluation error: {}",
                            constraint.expr, e
                        )));
                        outputs.insert(key, Value::Bool(false));
                        all_passed = false;
                    }
                },
                Err(diags) => {
                    diagnostics.extend(diags);
                    outputs.insert(key, Value::Bool(false));
                    all_passed = false;
                }
            }
        }

        // Add an overall pass/fail summary
        outputs.insert("all_passed".to_owned(), Value::Bool(all_passed));

        Ok(SolverResult {
            outputs,
            diagnostics,
            iterations: Some(1),
            converged: true, // Evaluation always "converges"
        })
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            max_variables: None,
            supports_constraints: true,
            supports_optimization: false,
            supports_sensitivity: false,
        }
    }
}

// ---------------------------------------------------------------------------
// OdeRk4Plugin
// ---------------------------------------------------------------------------

/// Built-in ODE solver plugin driven by user-supplied derivative expressions.
///
/// Required parameters:
/// - `derivative_exprs`: list of expression strings, one per state variable
/// - `state_vars`: list of state variable names in matching order
///
/// Optional:
/// - `dt` (default 0.1): time step
/// - `steps` (default 100): number of integration steps
/// - `<state_var>`: initial value for each named state variable (default 0.0)
/// - any additional numeric param binds as a constant in the derivative scope
///
/// Returns final state + `time` in outputs.
pub struct OdeRk4Plugin;

impl OdeRk4Plugin {
    /// Extract a named f64 parameter from the input slice, falling back to `default`.
    pub(crate) fn param_f64(inputs: &[SolverParam], name: &str, default: f64) -> f64 {
        for p in inputs {
            let key = p.tool_name.as_deref().unwrap_or(&p.sysml_name);
            if key == name {
                if let Some(ref v) = p.value {
                    match v {
                        sysml_core::Value::Float(f) => return *f,
                        sysml_core::Value::Int(i) => return *i as f64,
                        _ => {}
                    }
                }
            }
        }
        default
    }

    /// Extract a named parameter that is a list of strings (e.g., state variable names
    /// or derivative expressions). Returns empty vec if not found.
    pub(crate) fn param_strings(inputs: &[SolverParam], name: &str) -> Vec<String> {
        for p in inputs {
            let key = p.tool_name.as_deref().unwrap_or(&p.sysml_name);
            if key == name {
                if let Some(ref v) = p.value {
                    match v {
                        Value::List(items) => {
                            return items
                                .iter()
                                .filter_map(|item| {
                                    if let Value::String(s) = item {
                                        Some(s.clone())
                                    } else {
                                        None
                                    }
                                })
                                .collect();
                        }
                        Value::String(s) => {
                            // Single string — split by comma for convenience
                            return s.split(',').map(|s| s.trim().to_owned()).collect();
                        }
                        _ => {}
                    }
                }
            }
        }
        Vec::new()
    }
}

impl SolverPlugin for OdeRk4Plugin {
    fn name(&self) -> &str {
        "builtin:ode-rk4"
    }

    fn solve(
        &self,
        inputs: &[SolverParam],
        _constraints: &[ConstraintIR],
        _context: &EvalContext,
    ) -> Result<SolverResult, SolverError> {
        // Read integration parameters
        let dt = Self::param_f64(inputs, "dt", 0.1);
        let steps = Self::param_f64(inputs, "steps", 100.0) as usize;

        // Check for general ODE mode: derivative expressions provided as params
        let derivative_exprs = Self::param_strings(inputs, "derivative_exprs");
        let state_var_names = Self::param_strings(inputs, "state_vars");

        if derivative_exprs.is_empty() || state_var_names.is_empty() {
            return Err(SolverError::InvalidInput(
                "ode-rk4: `derivative_exprs` and `state_vars` parameters are required".to_owned(),
            ));
        }

        let mut spec = crate::ode_builder::OdeSpec::new();
        for (i, var_name) in state_var_names.iter().enumerate() {
            let initial = Self::param_f64(inputs, var_name, 0.0);
            let expr_str = derivative_exprs.get(i).map(|s| s.as_str()).unwrap_or("0.0");
            let expr = crate::ode_builder::parse_derivative(expr_str)
                .map_err(SolverError::InvalidInput)?;
            spec = spec.with_state_var(var_name.clone(), initial, expr);
        }
        // Bind all remaining params as constants
        for p in inputs {
            let key = p.tool_name.as_deref().unwrap_or(&p.sysml_name);
            if key == "dt" || key == "steps" || key == "derivative_exprs" || key == "state_vars" {
                continue;
            }
            if let Some(ref v) = p.value {
                if let Some(f) = match v {
                    Value::Float(f) => Some(*f),
                    Value::Int(i) => Some(*i as f64),
                    _ => None,
                } {
                    spec = spec.with_param(key.to_owned(), f);
                }
            }
        }
        let mut solver = spec.build_solver("ode-rk4-plugin");

        // Run the integration
        let ctx = EvalContext::new();
        for i in 0..steps {
            solver.step(i as f64 * dt, dt, &ctx);
        }

        // Build output: all state variables + time
        let mut outputs = HashMap::new();
        for (name, &val) in solver.state_names().iter().zip(solver.get_state().iter()) {
            outputs.insert(name.clone(), Value::Float(val));
        }
        outputs.insert("time".to_owned(), Value::Float(steps as f64 * dt));

        Ok(SolverResult {
            outputs,
            diagnostics: Vec::new(),
            iterations: Some(steps),
            converged: true,
        })
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            supports_constraints: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// OdeRk45Plugin
// ---------------------------------------------------------------------------

/// Built-in adaptive ODE solver plugin using Dormand-Prince RK4(5).
///
/// Same interface as `OdeRk4Plugin` but uses adaptive step-size control
/// for better accuracy on stiff systems. Accepts additional tolerance parameters.
pub struct OdeRk45Plugin;

impl SolverPlugin for OdeRk45Plugin {
    fn name(&self) -> &str {
        "builtin:ode-rk45"
    }

    fn solve(
        &self,
        inputs: &[SolverParam],
        _constraints: &[ConstraintIR],
        _context: &EvalContext,
    ) -> Result<SolverResult, SolverError> {
        let dt = OdeRk4Plugin::param_f64(inputs, "dt", 0.1);
        let steps = OdeRk4Plugin::param_f64(inputs, "steps", 100.0) as usize;
        let atol = OdeRk4Plugin::param_f64(inputs, "atol", 1e-6);
        let rtol = OdeRk4Plugin::param_f64(inputs, "rtol", 1e-3);

        // Check for general mode (derivative expressions)
        let derivative_exprs = OdeRk4Plugin::param_strings(inputs, "derivative_exprs");
        let state_var_names = OdeRk4Plugin::param_strings(inputs, "state_vars");

        if derivative_exprs.is_empty() || state_var_names.is_empty() {
            return Err(SolverError::InvalidInput(
                "ode-rk45: `derivative_exprs` and `state_vars` parameters are required".to_owned(),
            ));
        }

        let mut spec = crate::ode_builder::OdeSpec::new();
        for (i, var_name) in state_var_names.iter().enumerate() {
            let initial = OdeRk4Plugin::param_f64(inputs, var_name, 0.0);
            let expr_str = derivative_exprs.get(i).map(|s| s.as_str()).unwrap_or("0.0");
            let expr = crate::ode_builder::parse_derivative(expr_str)
                .map_err(SolverError::InvalidInput)?;
            spec = spec.with_state_var(var_name.clone(), initial, expr);
        }
        for p in inputs {
            let key = p.tool_name.as_deref().unwrap_or(&p.sysml_name);
            if [
                "dt",
                "steps",
                "atol",
                "rtol",
                "derivative_exprs",
                "state_vars",
            ]
            .contains(&key)
            {
                continue;
            }
            if let Some(ref v) = p.value {
                if let Some(f) = match v {
                    Value::Float(f) => Some(*f),
                    Value::Int(i) => Some(*i as f64),
                    _ => None,
                } {
                    spec = spec.with_param(key.to_owned(), f);
                }
            }
        }
        let rhs = spec.build_rhs();
        let mut solver = crate::ode45::Rk45Solver::new(
            "ode-rk45-plugin",
            spec.state_vars.clone(),
            spec.initial_values.clone(),
            rhs,
        )
        .with_atol(atol)
        .with_rtol(rtol);

        let total_time = steps as f64 * dt;
        let ctx = EvalContext::new();
        let num_steps = solver.step_to(0.0, total_time, &ctx);

        let mut outputs = HashMap::new();
        for (name, &val) in solver.state_names().iter().zip(solver.get_state().iter()) {
            outputs.insert(name.clone(), Value::Float(val));
        }
        outputs.insert("time".to_owned(), Value::Float(total_time));

        Ok(SolverResult {
            outputs,
            diagnostics: Vec::new(),
            iterations: Some(num_steps),
            converged: true,
        })
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            supports_constraints: true,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    // =======================================================================
    // BisectionSolver tests
    // =======================================================================

    #[test]
    fn bisection_solver_name() {
        let solver = BisectionSolver::default();
        assert_eq!(solver.name(), "builtin:bisection");
    }

    #[test]
    fn bisection_solver_capabilities() {
        let solver = BisectionSolver::default();
        let caps = solver.capabilities();
        assert_eq!(caps.max_variables, Some(1));
        assert!(caps.supports_constraints);
        assert!(!caps.supports_optimization);
    }

    #[test]
    fn bisection_solver_single_variable_constraint() {
        // Solve: x > 5 (with bisection, finds the boundary x ~ 5.0)
        let solver = BisectionSolver {
            search_range: (0.0, 100.0),
        };
        let ctx = EvalContext::new();

        let inputs = vec![]; // no known values; x is the free variable

        // Constraint: x - 10 == 0 → solve for x = 10
        // Use a numeric constraint that has a zero crossing
        let constraints = vec![ConstraintIR::new("x - 10")];

        let result = solver.solve(&inputs, &constraints, &ctx).unwrap();
        assert!(result.converged);

        let x_val = result.outputs.get("x").expect("should solve for x");
        if let Value::Float(x) = x_val {
            assert!((*x - 10.0).abs() < 0.001, "expected x ~ 10.0, got {}", x);
        } else {
            panic!("expected Float for x, got {:?}", x_val);
        }
    }

    #[test]
    fn bisection_solver_with_known_inputs() {
        // Solve: x + y - 15 == 0, where y = 5 (known). Expect x = 10.
        let solver = BisectionSolver {
            search_range: (-100.0, 100.0),
        };
        let ctx = EvalContext::new();

        let inputs = vec![SolverParam {
            sysml_name: "y".to_string(),
            tool_name: None,
            value: Some(Value::Float(5.0)),
            direction: ParamDirection::In,
        }];

        let constraints = vec![ConstraintIR::new("x + y - 15")];

        let result = solver.solve(&inputs, &constraints, &ctx).unwrap();
        assert!(result.converged);

        let x_val = result.outputs.get("x").expect("should solve for x");
        if let Value::Float(x) = x_val {
            assert!((*x - 10.0).abs() < 0.001, "expected x ~ 10.0, got {}", x);
        } else {
            panic!("expected Float for x, got {:?}", x_val);
        }
    }

    #[test]
    fn bisection_solver_empty_constraints() {
        let solver = BisectionSolver::default();
        let ctx = EvalContext::new();

        let result = solver.solve(&[], &[], &ctx).unwrap();
        assert!(result.converged);
        assert!(result.outputs.is_empty());
    }

    #[test]
    fn bisection_solver_no_solution_reports_not_converged() {
        // Constraint with two free variables: under-determined
        let solver = BisectionSolver::default();
        let ctx = EvalContext::new();

        let constraints = vec![ConstraintIR::new("x + y - 10")];

        let result = solver.solve(&[], &constraints, &ctx).unwrap();
        // Under-determined: should not converge
        assert!(!result.converged);
        assert!(!result.diagnostics.is_empty());
    }

    // =======================================================================
    // ConstraintEvaluator tests
    // =======================================================================

    #[test]
    fn evaluator_name() {
        let solver = ConstraintEvaluator;
        assert_eq!(solver.name(), "builtin:evaluate");
    }

    #[test]
    fn evaluator_capabilities() {
        let solver = ConstraintEvaluator;
        let caps = solver.capabilities();
        assert!(caps.supports_constraints);
        assert!(!caps.supports_optimization);
    }

    #[test]
    fn evaluator_pass() {
        let solver = ConstraintEvaluator;
        let ctx = EvalContext::new();

        let inputs = vec![
            SolverParam {
                sysml_name: "speed".to_string(),
                tool_name: None,
                value: Some(Value::Float(80.0)),
                direction: ParamDirection::In,
            },
            SolverParam {
                sysml_name: "limit".to_string(),
                tool_name: None,
                value: Some(Value::Float(100.0)),
                direction: ParamDirection::In,
            },
        ];

        let constraints = vec![ConstraintIR::new("speed < limit")];

        let result = solver.solve(&inputs, &constraints, &ctx).unwrap();
        assert!(result.converged);
        assert_eq!(result.outputs.get("all_passed"), Some(&Value::Bool(true)));
    }

    #[test]
    fn evaluator_fail() {
        let solver = ConstraintEvaluator;
        let ctx = EvalContext::new();

        let inputs = vec![
            SolverParam {
                sysml_name: "speed".to_string(),
                tool_name: None,
                value: Some(Value::Float(120.0)),
                direction: ParamDirection::In,
            },
            SolverParam {
                sysml_name: "limit".to_string(),
                tool_name: None,
                value: Some(Value::Float(100.0)),
                direction: ParamDirection::In,
            },
        ];

        let constraints = vec![ConstraintIR::new("speed < limit")];

        let result = solver.solve(&inputs, &constraints, &ctx).unwrap();
        assert!(result.converged); // Evaluation always converges
        assert_eq!(result.outputs.get("all_passed"), Some(&Value::Bool(false)));
    }

    #[test]
    fn evaluator_multiple_constraints() {
        let solver = ConstraintEvaluator;
        let ctx = EvalContext::new();

        let inputs = vec![
            SolverParam {
                sysml_name: "x".to_string(),
                tool_name: None,
                value: Some(Value::Float(5.0)),
                direction: ParamDirection::In,
            },
            SolverParam {
                sysml_name: "y".to_string(),
                tool_name: None,
                value: Some(Value::Float(10.0)),
                direction: ParamDirection::In,
            },
        ];

        let constraints = vec![
            ConstraintIR::new("x > 0").with_description("positive_x"),
            ConstraintIR::new("y > 0").with_description("positive_y"),
            ConstraintIR::new("x < y").with_description("x_less_than_y"),
        ];

        let result = solver.solve(&inputs, &constraints, &ctx).unwrap();
        assert_eq!(result.outputs.get("all_passed"), Some(&Value::Bool(true)));
        assert_eq!(result.outputs.get("positive_x"), Some(&Value::Bool(true)));
        assert_eq!(result.outputs.get("positive_y"), Some(&Value::Bool(true)));
        assert_eq!(
            result.outputs.get("x_less_than_y"),
            Some(&Value::Bool(true))
        );
    }

    #[test]
    fn evaluator_empty_constraints() {
        let solver = ConstraintEvaluator;
        let ctx = EvalContext::new();

        let result = solver.solve(&[], &[], &ctx).unwrap();
        assert!(result.converged);
        assert_eq!(result.outputs.get("all_passed"), Some(&Value::Bool(true)));
    }

    #[test]
    fn evaluator_uses_tool_name() {
        let solver = ConstraintEvaluator;
        let ctx = EvalContext::new();

        let inputs = vec![SolverParam {
            sysml_name: "temperature".to_string(),
            tool_name: Some("T".to_string()),
            value: Some(Value::Float(300.0)),
            direction: ParamDirection::In,
        }];

        // The constraint references "T" (the tool name)
        let constraints = vec![ConstraintIR::new("T > 200")];

        let result = solver.solve(&inputs, &constraints, &ctx).unwrap();
        assert_eq!(result.outputs.get("all_passed"), Some(&Value::Bool(true)));
    }

    // =======================================================================
    // OdeRk4Plugin tests
    // =======================================================================

    #[test]
    fn ode_rk4_plugin_name() {
        let solver = OdeRk4Plugin;
        assert_eq!(solver.name(), "builtin:ode-rk4");
    }

    #[test]
    fn ode_rk4_plugin_capabilities() {
        let solver = OdeRk4Plugin;
        let caps = solver.capabilities();
        assert!(caps.supports_constraints);
        assert!(!caps.supports_optimization);
    }

    /// Build a solver-input list driving the canonical thermal-body ODE
    /// `dT/dt = (heaterPower - lossCoefficient * (T - ambientTemp)) / thermalMass`
    /// through the general derivative-expression pipeline. Used by the RK4
    /// and RK45 plugin tests as a representative non-trivial integration.
    fn thermal_body_inputs(steps: f64, dt: f64) -> Vec<SolverParam> {
        let p = |name: &str, val: f64, dir| SolverParam {
            sysml_name: name.to_string(),
            tool_name: None,
            value: Some(Value::Float(val)),
            direction: dir,
        };
        let s = |name: &str, val: &str, dir| SolverParam {
            sysml_name: name.to_string(),
            tool_name: None,
            value: Some(Value::String(val.to_string())),
            direction: dir,
        };
        vec![
            p("dt", dt, ParamDirection::In),
            p("steps", steps, ParamDirection::In),
            s("state_vars", "temperature", ParamDirection::In),
            s(
                "derivative_exprs",
                "(heaterPower - lossCoefficient * (temperature - ambientTemp)) / thermalMass",
                ParamDirection::In,
            ),
            p("temperature", 20.0, ParamDirection::InOut),
            p("heaterPower", 100.0, ParamDirection::In),
            p("ambientTemp", 20.0, ParamDirection::In),
            p("thermalMass", 5.0, ParamDirection::In),
            p("lossCoefficient", 10.0, ParamDirection::In),
        ]
    }

    #[test]
    fn ode_rk4_plugin_thermal_body_heats_up() {
        // Drive the plugin via the production path (derivative_exprs +
        // state_vars). Heater power above ambient should bring the body to
        // its analytic equilibrium T_eq = ambient + power/loss.
        let solver = OdeRk4Plugin;
        let ctx = EvalContext::new();
        let inputs = thermal_body_inputs(2000.0, 0.01);

        let result = solver.solve(&inputs, &[], &ctx).unwrap();
        assert!(result.converged);

        let temp = result
            .outputs
            .get("temperature")
            .expect("should have temperature");
        if let Value::Float(t) = temp {
            assert!(
                (*t - 30.0).abs() < 0.1,
                "expected temperature near 30.0, got {}",
                t
            );
        } else {
            panic!("expected Float for temperature, got {:?}", temp);
        }

        let time = result.outputs.get("time").expect("should have time");
        if let Value::Float(t) = time {
            assert!(
                (*t - 20.0).abs() < 0.01,
                "expected time ~ 20.0s (2000 * 0.01), got {}",
                t
            );
        } else {
            panic!("expected Float for time, got {:?}", time);
        }
    }

    #[test]
    fn ode_rk4_plugin_missing_required_params_errors() {
        // Without `derivative_exprs` / `state_vars` the plugin has no ODE to
        // solve and must return an explicit error rather than silently
        // running a hardcoded model.
        let solver = OdeRk4Plugin;
        let ctx = EvalContext::new();
        let result = solver.solve(&[], &[], &ctx);
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("derivative_exprs") && msg.contains("state_vars"),
            "error should mention both required params, got: {}",
            msg
        );
    }

    #[test]
    fn ode_rk4_registered_in_builtins() {
        let registry = crate::solver_registry::SolverRegistry::with_builtins();
        let solver = registry.get("builtin:ode-rk4");
        assert!(solver.is_some(), "OdeRk4Plugin should be registered");
        assert_eq!(solver.unwrap().name(), "builtin:ode-rk4");
    }

    // =======================================================================
    // OdeRk45Plugin tests
    // =======================================================================

    #[test]
    fn ode_rk45_plugin_name() {
        let p = OdeRk45Plugin;
        assert_eq!(p.name(), "builtin:ode-rk45");
    }

    #[test]
    fn ode_rk45_plugin_capabilities() {
        let p = OdeRk45Plugin;
        assert!(p.capabilities().supports_constraints);
    }

    #[test]
    fn ode_rk45_plugin_thermal_body() {
        let p = OdeRk45Plugin;
        let inputs = thermal_body_inputs(2000.0, 0.01);
        let result = p.solve(&inputs, &[], &EvalContext::new()).unwrap();
        let temp = match result.outputs.get("temperature") {
            Some(Value::Float(f)) => *f,
            _ => panic!("expected temperature"),
        };
        // Equilibrium: T_eq = 20 + 100/10 = 30
        assert!((temp - 30.0).abs() < 0.1, "expected ~30, got {}", temp);
    }

    #[test]
    fn ode_rk45_plugin_missing_required_params_errors() {
        let p = OdeRk45Plugin;
        let result = p.solve(&[], &[], &EvalContext::new());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("derivative_exprs") && msg.contains("state_vars"),
            "error should mention both required params, got: {}",
            msg
        );
    }

    #[test]
    fn ode_rk45_registered_in_builtins() {
        let registry = crate::SolverRegistry::with_builtins();
        assert!(registry.get("builtin:ode-rk45").is_some());
    }
}
