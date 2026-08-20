//! Registry mapping tool names to solver plugin implementations.
//!
//! The registry is the central dispatch point: when an analysis case
//! specifies `toolName = "builtin:propagation"`, the registry finds
//! the matching [`SolverPlugin`] implementation and delegates to it.

use std::collections::HashMap;
use std::sync::Arc;

use crate::constraints::EvalContext;
use crate::solver::ConstraintNetwork;
use crate::solver_external::ExternalSolverPlugin;
use crate::solver_plugin::{
    ParamDirection, SolverCapabilities, SolverError, SolverParam, SolverPlugin, SolverResult,
};
use crate::ConstraintIR;

/// Registry mapping tool names to solver plugin implementations.
///
/// The registry is the central dispatch point: when an analysis case
/// specifies `toolName = "builtin:propagation"`, the registry finds
/// the matching `SolverPlugin` implementation and delegates to it.
pub struct SolverRegistry {
    plugins: HashMap<String, Arc<dyn SolverPlugin>>,
    default_name: String,
}

impl SolverRegistry {
    /// Create an empty registry with no default solver.
    pub fn new() -> Self {
        Self {
            plugins: HashMap::new(),
            default_name: String::new(),
        }
    }

    /// Register a solver plugin. Replaces any existing plugin with the same name.
    ///
    /// If this is the first plugin registered and no default has been set,
    /// it becomes the default.
    pub fn register(&mut self, plugin: impl SolverPlugin + 'static) {
        let name = plugin.name().to_owned();
        let is_first = self.plugins.is_empty() && self.default_name.is_empty();
        self.plugins.insert(name.clone(), Arc::new(plugin));
        if is_first {
            self.default_name = name;
        }
    }

    /// Get a solver by tool name.
    pub fn get(&self, tool_name: &str) -> Option<Arc<dyn SolverPlugin>> {
        self.plugins.get(tool_name).cloned()
    }

    /// Get the default solver.
    ///
    /// Returns the solver set via [`set_default`](Self::set_default), or the
    /// first registered solver if no explicit default was set.
    pub fn default_solver(&self) -> Option<Arc<dyn SolverPlugin>> {
        self.plugins.get(&self.default_name).cloned()
    }

    /// Set which registered solver is the default.
    ///
    /// Returns `true` if the named solver exists and was set as default,
    /// `false` if no solver with that name is registered.
    pub fn set_default(&mut self, name: &str) -> bool {
        if self.plugins.contains_key(name) {
            self.default_name = name.to_owned();
            true
        } else {
            false
        }
    }

    /// List all registered solver names.
    pub fn registered_names(&self) -> Vec<&str> {
        self.plugins.keys().map(|s| s.as_str()).collect()
    }

    /// Register an external solver plugin by command path.
    ///
    /// Creates an [`ExternalSolverPlugin`] that communicates with a subprocess
    /// via JSON over stdio. Tool names conventionally start with `"external:"`.
    ///
    /// # Arguments
    /// * `tool_name` - Plugin name matching `ToolExecution.toolName`.
    /// * `command` - Path or name of the executable to spawn.
    /// * `args` - Arguments to pass to the command.
    pub fn register_external(&mut self, tool_name: &str, command: &str, args: Vec<String>) {
        let plugin = ExternalSolverPlugin::new(tool_name, command).with_args(args);
        self.register(plugin);
    }

    /// Create a registry with built-in solvers pre-registered.
    ///
    /// Registers `"builtin:propagation"` as the default solver, plus
    /// `"builtin:bisection"`, `"builtin:evaluate"`, `"builtin:ode-rk4"`,
    /// `"builtin:ode-rk45"`, `"builtin:ode-bdf"` (implicit stiff solver,
    /// also aliased as `"bdf"`).
    pub fn with_builtins() -> Self {
        let mut registry = Self::new();
        registry.register(PropagationSolver);
        registry.register(crate::solver_builtins::BisectionSolver::default());
        registry.register(crate::solver_builtins::ConstraintEvaluator);
        registry.register(crate::solver_builtins::OdeRk4Plugin);
        registry.register(crate::solver_builtins::OdeRk45Plugin);
        // BDF implicit solver (R7.3). Registered under both the
        // `builtin:ode-bdf` conventional id and the short alias `bdf`
        // requested by the extensibility plan.
        registry.register(crate::solvers::bdf::BdfPlugin::new());
        registry.register(crate::solvers::bdf::BdfPlugin::aliased("bdf"));
        registry
    }
}

impl Default for SolverRegistry {
    fn default() -> Self {
        Self::with_builtins()
    }
}

// ---------------------------------------------------------------------------
// Built-in solver: binding propagation
// ---------------------------------------------------------------------------

/// Built-in solver using binding propagation from the constraint network.
///
/// This solver handles simple equality-chain propagation: given a set of
/// input parameters with known values, it propagates those values through
/// binding connectors to compute outputs. It does not solve nonlinear
/// constraints or perform optimization.
struct PropagationSolver;

impl SolverPlugin for PropagationSolver {
    fn name(&self) -> &str {
        "builtin:propagation"
    }

    fn solve(
        &self,
        inputs: &[SolverParam],
        _constraints: &[ConstraintIR],
        _context: &EvalContext,
    ) -> Result<SolverResult, SolverError> {
        // Build a ConstraintNetwork from inputs
        let mut network = ConstraintNetwork::new();

        // Add known values from input parameters
        for param in inputs {
            if param.direction == ParamDirection::In {
                if let Some(ref val) = param.value {
                    let key = param.tool_name.as_deref().unwrap_or(&param.sysml_name);
                    network.set(key.to_owned(), val.clone());
                }
            }
        }

        // Propagate equality chains
        let result = network.propagate();

        Ok(SolverResult {
            outputs: result.solved,
            diagnostics: vec![],
            iterations: Some(result.iterations),
            converged: result.unsolved.is_empty(),
        })
    }

    fn capabilities(&self) -> SolverCapabilities {
        SolverCapabilities {
            max_variables: None,
            supports_constraints: false, // Only equality propagation
            supports_optimization: false,
            supports_sensitivity: false,
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::Value;

    // -----------------------------------------------------------------------
    // Registry tests
    // -----------------------------------------------------------------------

    /// A trivial test solver for registry tests.
    struct DummySolver {
        solver_name: &'static str,
    }

    impl DummySolver {
        fn new(name: &'static str) -> Self {
            Self { solver_name: name }
        }
    }

    impl SolverPlugin for DummySolver {
        fn name(&self) -> &str {
            self.solver_name
        }

        fn solve(
            &self,
            _inputs: &[SolverParam],
            _constraints: &[ConstraintIR],
            _context: &EvalContext,
        ) -> Result<SolverResult, SolverError> {
            Ok(SolverResult {
                outputs: HashMap::new(),
                diagnostics: vec![],
                iterations: Some(0),
                converged: true,
            })
        }
    }

    #[test]
    fn empty_registry() {
        let registry = SolverRegistry::new();
        assert!(registry.get("anything").is_none());
        assert!(registry.default_solver().is_none());
        assert!(registry.registered_names().is_empty());
    }

    #[test]
    fn register_and_get() {
        let mut registry = SolverRegistry::new();
        registry.register(DummySolver::new("test:alpha"));

        let solver = registry.get("test:alpha");
        assert!(solver.is_some());
        assert_eq!(solver.unwrap().name(), "test:alpha");
    }

    #[test]
    fn get_not_found() {
        let mut registry = SolverRegistry::new();
        registry.register(DummySolver::new("test:alpha"));

        assert!(registry.get("test:beta").is_none());
    }

    #[test]
    fn first_registered_becomes_default() {
        let mut registry = SolverRegistry::new();
        registry.register(DummySolver::new("first"));
        registry.register(DummySolver::new("second"));

        let default = registry.default_solver().unwrap();
        assert_eq!(default.name(), "first");
    }

    #[test]
    fn set_default_existing() {
        let mut registry = SolverRegistry::new();
        registry.register(DummySolver::new("alpha"));
        registry.register(DummySolver::new("beta"));

        assert!(registry.set_default("beta"));
        assert_eq!(registry.default_solver().unwrap().name(), "beta");
    }

    #[test]
    fn set_default_nonexistent() {
        let mut registry = SolverRegistry::new();
        registry.register(DummySolver::new("alpha"));

        assert!(!registry.set_default("nonexistent"));
        // Default unchanged
        assert_eq!(registry.default_solver().unwrap().name(), "alpha");
    }

    #[test]
    fn registered_names_lists_all() {
        let mut registry = SolverRegistry::new();
        registry.register(DummySolver::new("a"));
        registry.register(DummySolver::new("b"));
        registry.register(DummySolver::new("c"));

        let mut names = registry.registered_names();
        names.sort();
        assert_eq!(names, vec!["a", "b", "c"]);
    }

    #[test]
    fn register_replaces_existing() {
        let mut registry = SolverRegistry::new();
        registry.register(DummySolver::new("test:alpha"));
        registry.register(DummySolver::new("test:alpha")); // replace

        assert_eq!(registry.registered_names().len(), 1);
        assert!(registry.get("test:alpha").is_some());
    }

    // -----------------------------------------------------------------------
    // with_builtins tests
    // -----------------------------------------------------------------------

    #[test]
    fn with_builtins_has_propagation() {
        let registry = SolverRegistry::with_builtins();
        let names = registry.registered_names();
        assert!(names.contains(&"builtin:propagation"));
    }

    #[test]
    fn with_builtins_default_is_propagation() {
        let registry = SolverRegistry::with_builtins();
        let default = registry.default_solver().unwrap();
        assert_eq!(default.name(), "builtin:propagation");
    }

    #[test]
    fn default_trait_impl_matches_with_builtins() {
        let registry = SolverRegistry::default();
        assert!(registry.get("builtin:propagation").is_some());
        assert_eq!(
            registry.default_solver().unwrap().name(),
            "builtin:propagation"
        );
    }

    // -----------------------------------------------------------------------
    // PropagationSolver tests
    // -----------------------------------------------------------------------

    #[test]
    fn propagation_solver_name() {
        let solver = PropagationSolver;
        assert_eq!(solver.name(), "builtin:propagation");
    }

    #[test]
    fn propagation_solver_capabilities() {
        let solver = PropagationSolver;
        let caps = solver.capabilities();
        assert!(caps.max_variables.is_none());
        assert!(!caps.supports_constraints);
        assert!(!caps.supports_optimization);
        assert!(!caps.supports_sensitivity);
    }

    #[test]
    fn propagation_solver_basic_solve() {
        let solver = PropagationSolver;
        let ctx = EvalContext::new();

        let inputs = vec![
            SolverParam {
                sysml_name: "temperature".to_string(),
                tool_name: Some("T".to_string()),
                value: Some(Value::Float(300.0)),
                direction: ParamDirection::In,
            },
            SolverParam {
                sysml_name: "pressure".to_string(),
                tool_name: None,
                value: Some(Value::Float(101.325)),
                direction: ParamDirection::In,
            },
            SolverParam {
                sysml_name: "result".to_string(),
                tool_name: None,
                value: None,
                direction: ParamDirection::Out,
            },
        ];

        let result = solver.solve(&inputs, &[], &ctx).unwrap();
        assert!(result.converged);
        assert!(result.iterations.is_some());
        // Input values should be propagated into outputs map
        assert_eq!(result.outputs.get("T"), Some(&Value::Float(300.0)));
        assert_eq!(result.outputs.get("pressure"), Some(&Value::Float(101.325)));
    }

    #[test]
    fn propagation_solver_empty_inputs() {
        let solver = PropagationSolver;
        let ctx = EvalContext::new();

        let result = solver.solve(&[], &[], &ctx).unwrap();
        assert!(result.converged);
        assert!(result.outputs.is_empty());
    }

    #[test]
    fn propagation_solver_skips_non_input_params() {
        let solver = PropagationSolver;
        let ctx = EvalContext::new();

        let inputs = vec![
            SolverParam {
                sysml_name: "x".to_string(),
                tool_name: None,
                value: Some(Value::Float(1.0)),
                direction: ParamDirection::In,
            },
            SolverParam {
                sysml_name: "y".to_string(),
                tool_name: None,
                value: Some(Value::Float(2.0)),
                direction: ParamDirection::Out, // Should be skipped
            },
            SolverParam {
                sysml_name: "z".to_string(),
                tool_name: None,
                value: Some(Value::Float(3.0)),
                direction: ParamDirection::InOut, // Should be skipped
            },
        ];

        let result = solver.solve(&inputs, &[], &ctx).unwrap();
        assert!(result.outputs.contains_key("x"));
        assert!(!result.outputs.contains_key("y"));
        assert!(!result.outputs.contains_key("z"));
    }

    #[test]
    fn propagation_solver_uses_tool_name_over_sysml_name() {
        let solver = PropagationSolver;
        let ctx = EvalContext::new();

        let inputs = vec![SolverParam {
            sysml_name: "temperature".to_string(),
            tool_name: Some("T".to_string()),
            value: Some(Value::Float(450.0)),
            direction: ParamDirection::In,
        }];

        let result = solver.solve(&inputs, &[], &ctx).unwrap();
        // Should use tool_name "T", not sysml_name "temperature"
        assert_eq!(result.outputs.get("T"), Some(&Value::Float(450.0)));
        assert!(!result.outputs.contains_key("temperature"));
    }
}
