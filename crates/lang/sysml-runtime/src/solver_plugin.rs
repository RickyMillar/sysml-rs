//! Solver plugin trait and types for external solver integration.
//!
//! The `SolverPlugin` trait defines the interface for solver implementations.
//! Users annotate SysML elements with `ToolExecution` metadata specifying
//! a `toolName`. The `SolverRegistry` dispatches to the matching plugin.

use std::collections::HashMap;
use std::fmt;

use sysml_core::Value;
use sysml_span::Diagnostic;

use crate::constraints::EvalContext;
use crate::ConstraintIR;

/// Direction of a solver parameter: input, output, or bidirectional.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamDirection {
    /// Input parameter (value is provided to the solver).
    In,
    /// Output parameter (value is computed by the solver).
    Out,
    /// Bidirectional parameter (may be read and written by the solver).
    InOut,
}

/// Input/output parameter for solver invocation.
#[derive(Debug, Clone)]
pub struct SolverParam {
    /// SysML parameter name.
    pub sysml_name: String,
    /// ToolVariable mapped name, or `None` if the tool uses the same name as SysML.
    pub tool_name: Option<String>,
    /// Current value (`None` = unknown / to-be-solved).
    pub value: Option<Value>,
    /// Whether this parameter is an input, output, or both.
    pub direction: ParamDirection,
}

/// Result of a solver invocation.
#[derive(Debug, Clone)]
pub struct SolverResult {
    /// Solved output values keyed by parameter name.
    pub outputs: HashMap<String, Value>,
    /// Warnings or informational diagnostics from the solver.
    pub diagnostics: Vec<Diagnostic>,
    /// Number of solver iterations (if applicable).
    pub iterations: Option<usize>,
    /// Whether the solver converged to a solution.
    pub converged: bool,
}

/// Error returned when a solver invocation fails.
#[derive(Debug)]
pub enum SolverError {
    /// The solver did not converge within the iteration limit.
    NotConverged {
        /// Number of iterations attempted.
        iterations: usize,
        /// Final residual magnitude.
        residual: f64,
    },
    /// One or more input parameters are invalid.
    InvalidInput(String),
    /// The solver does not support the requested operation.
    Unsupported(String),
    /// An unexpected runtime error occurred.
    Runtime(String),
}

impl fmt::Display for SolverError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SolverError::NotConverged {
                iterations,
                residual,
            } => write!(
                f,
                "solver did not converge after {iterations} iterations (residual: {residual})"
            ),
            SolverError::InvalidInput(msg) => write!(f, "invalid solver input: {msg}"),
            SolverError::Unsupported(msg) => write!(f, "unsupported solver operation: {msg}"),
            SolverError::Runtime(msg) => write!(f, "solver runtime error: {msg}"),
        }
    }
}

impl std::error::Error for SolverError {}

/// Describes the capabilities of a solver plugin (used for DOF analysis and dispatch).
#[derive(Debug, Clone, Default)]
pub struct SolverCapabilities {
    /// Maximum number of variables the solver can handle (`None` = unlimited).
    pub max_variables: Option<usize>,
    /// Whether the solver can handle inequality constraints.
    pub supports_constraints: bool,
    /// Whether the solver can minimize/maximize an objective.
    pub supports_optimization: bool,
    /// Whether the solver can compute sensitivity gradients.
    pub supports_sensitivity: bool,
}

/// The plugin trait -- implement this for custom solvers.
///
/// Each plugin is identified by a name that matches the `toolName` field in
/// `ToolExecution` metadata annotations. The `SolverRegistry` dispatches to
/// the plugin whose name matches.
pub trait SolverPlugin: Send + Sync {
    /// Human-readable solver name (matches `ToolExecution.toolName`).
    fn name(&self) -> &str;

    /// Solve with given inputs and constraints, returning outputs.
    fn solve(
        &self,
        inputs: &[SolverParam],
        constraints: &[ConstraintIR],
        context: &EvalContext,
    ) -> Result<SolverResult, SolverError>;

    /// Report solver capabilities (for DOF analysis and dispatch decisions).
    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities::default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn param_direction_equality() {
        assert_eq!(ParamDirection::In, ParamDirection::In);
        assert_ne!(ParamDirection::In, ParamDirection::Out);
        assert_ne!(ParamDirection::Out, ParamDirection::InOut);
    }

    #[test]
    fn solver_param_creation() {
        let param = SolverParam {
            sysml_name: "temperature".to_string(),
            tool_name: Some("T".to_string()),
            value: Some(Value::Float(300.0)),
            direction: ParamDirection::In,
        };
        assert_eq!(param.sysml_name, "temperature");
        assert_eq!(param.tool_name.as_deref(), Some("T"));
        assert_eq!(param.value, Some(Value::Float(300.0)));
        assert_eq!(param.direction, ParamDirection::In);
    }

    #[test]
    fn solver_param_no_tool_name() {
        let param = SolverParam {
            sysml_name: "pressure".to_string(),
            tool_name: None,
            value: None,
            direction: ParamDirection::Out,
        };
        assert!(param.tool_name.is_none());
        assert!(param.value.is_none());
    }

    #[test]
    fn solver_result_creation() {
        let mut outputs = HashMap::new();
        outputs.insert("result".to_string(), Value::Float(42.0));
        let result = SolverResult {
            outputs,
            diagnostics: Vec::new(),
            iterations: Some(5),
            converged: true,
        };
        assert_eq!(result.outputs.get("result"), Some(&Value::Float(42.0)));
        assert!(result.converged);
        assert_eq!(result.iterations, Some(5));
    }

    #[test]
    fn solver_error_display() {
        let e = SolverError::NotConverged {
            iterations: 100,
            residual: 0.001,
        };
        assert!(e.to_string().contains("100 iterations"));
        assert!(e.to_string().contains("0.001"));

        let e = SolverError::InvalidInput("missing param x".to_string());
        assert!(e.to_string().contains("missing param x"));

        let e = SolverError::Unsupported("nonlinear".to_string());
        assert!(e.to_string().contains("nonlinear"));

        let e = SolverError::Runtime("segfault".to_string());
        assert!(e.to_string().contains("segfault"));
    }

    #[test]
    fn solver_error_is_error_trait() {
        let e: Box<dyn std::error::Error> = Box::new(SolverError::Runtime("test".to_string()));
        assert!(e.to_string().contains("test"));
    }

    #[test]
    fn solver_capabilities_default() {
        let caps = SolverCapabilities::default();
        assert!(caps.max_variables.is_none());
        assert!(!caps.supports_constraints);
        assert!(!caps.supports_optimization);
        assert!(!caps.supports_sensitivity);
    }

    /// A trivial plugin for testing the trait.
    struct EchoSolver;

    impl SolverPlugin for EchoSolver {
        fn name(&self) -> &str {
            "echo"
        }

        fn solve(
            &self,
            inputs: &[SolverParam],
            _constraints: &[ConstraintIR],
            _context: &EvalContext,
        ) -> Result<SolverResult, SolverError> {
            let mut outputs = HashMap::new();
            for p in inputs {
                if let Some(v) = &p.value {
                    outputs.insert(p.sysml_name.clone(), v.clone());
                }
            }
            Ok(SolverResult {
                outputs,
                diagnostics: Vec::new(),
                iterations: Some(1),
                converged: true,
            })
        }
    }

    #[test]
    fn echo_solver_plugin() {
        let solver = EchoSolver;
        assert_eq!(solver.name(), "echo");

        let caps = solver.capabilities();
        assert!(!caps.supports_optimization);

        let inputs = vec![SolverParam {
            sysml_name: "x".to_string(),
            tool_name: None,
            value: Some(Value::Float(7.0)),
            direction: ParamDirection::In,
        }];
        let ctx = EvalContext::new();
        let result = solver.solve(&inputs, &[], &ctx).unwrap();
        assert!(result.converged);
        assert_eq!(result.outputs.get("x"), Some(&Value::Float(7.0)));
    }
}
