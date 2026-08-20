//! Parametric constraint solving for SysML v2.
//!
//! Implements binding connector propagation and hierarchical rollup patterns
//! for computing derived property values from constraint networks.
//!
//! ## Architecture
//!
//! ```text
//! ModelGraph → build_constraint_network() → ConstraintNetwork
//!                                               │
//!                                     propagate() (equality chains)
//!                                     rollup()    (ownership aggregation)
//!                                               │
//!                                               ▼
//!                                     HashMap<String, Value> (solved values)
//! ```
//!
//! ## Solving Strategy (ADR-3)
//!
//! 1. Binding propagation first (always converges, covers most MBSE use cases)
//! 2. Rollup patterns (recursive aggregation over part hierarchies)
//! 3. Numeric solver later (Newton-Raphson for nonlinear systems)
//! 4. Monte Carlo as fallback (already in montecarlo.rs)

// Numeric solver code uses pervasive array indexing with invariant-checked bounds.
#![allow(clippy::indexing_slicing, clippy::needless_range_loop)]

use std::collections::{HashMap, HashSet};

use sysml_core::{ElementKind, ModelGraph, RelationshipKind, Value};

use crate::constraints::PrecompiledConstraintSet;
use crate::expressions::{EvalContext, ExprIR, ExpressionEvaluator};

/// Aggregation function for rollup computations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AggregationKind {
    Sum,
    Max,
    Min,
    Avg,
    Count,
}

/// Result of a propagation pass.
#[derive(Debug, Clone)]
pub struct PropagationResult {
    /// Values that were successfully propagated
    pub solved: HashMap<String, Value>,
    /// Number of propagation iterations until fixpoint
    pub iterations: usize,
    /// Variables that remain unsolved (no known value in any connected equality)
    pub unsolved: Vec<String>,
}

/// A constraint network extracted from a ModelGraph.
///
/// Contains equality constraints (from binding connectors) and known values
/// (from attribute defaults, user overrides, or prior computation).
#[derive(Debug, Clone)]
pub struct ConstraintNetwork {
    /// Equality pairs: (variable_a, variable_b) meaning a == b.
    /// Extracted from `RelationshipKind::Binding` relationships.
    pub equalities: Vec<(String, String)>,
    /// Known values (from attribute defaults, user --set overrides, etc.)
    pub known: HashMap<String, Value>,
}

impl ConstraintNetwork {
    pub fn new() -> Self {
        Self {
            equalities: Vec::new(),
            known: HashMap::new(),
        }
    }

    /// Set a known value (e.g., from user --set override).
    pub fn set(&mut self, name: impl Into<String>, value: Value) {
        self.known.insert(name.into(), value);
    }

    /// Add an equality constraint: a == b.
    pub fn add_equality(&mut self, a: impl Into<String>, b: impl Into<String>) {
        self.equalities.push((a.into(), b.into()));
    }

    /// Propagate known values through equality chains until fixpoint.
    ///
    /// For each equality (a, b):
    /// - If a is known and b is not → b = a
    /// - If b is known and a is not → a = b
    /// - If both are known and differ → conflict (logged but not error)
    ///
    /// Repeats until no new values are propagated (guaranteed to terminate
    /// since each variable can only be set once).
    pub fn propagate(&mut self) -> PropagationResult {
        let mut iterations = 0;
        let max_iterations = self.equalities.len() * 2 + 1; // worst case: chain of N equalities

        loop {
            iterations += 1;
            let mut changed = false;

            for (a, b) in &self.equalities {
                let a_known = self.known.get(a).cloned();
                let b_known = self.known.get(b).cloned();

                match (a_known, b_known) {
                    (Some(val), None) => {
                        self.known.insert(b.clone(), val);
                        changed = true;
                    }
                    (None, Some(val)) => {
                        self.known.insert(a.clone(), val);
                        changed = true;
                    }
                    _ => {} // both known (consistent or conflict) or both unknown
                }
            }

            if !changed || iterations >= max_iterations {
                break;
            }
        }

        // Collect unsolved variables
        let all_vars: std::collections::HashSet<&str> = self
            .equalities
            .iter()
            .flat_map(|(a, b)| vec![a.as_str(), b.as_str()])
            .collect();

        let unsolved: Vec<String> = all_vars
            .into_iter()
            .filter(|v| !self.known.contains_key(*v))
            .map(String::from)
            .collect();

        PropagationResult {
            solved: self.known.clone(),
            iterations,
            unsolved,
        }
    }
}

impl Default for ConstraintNetwork {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Numeric constraint solving
// ---------------------------------------------------------------------------

/// Result of attempting to solve a constraint for a single unknown variable.
#[derive(Debug, Clone)]
pub enum SolveResult {
    /// Found a value that satisfies the constraint
    Solved { variable: String, value: f64 },
    /// Could not find a solution (bisection didn't converge)
    NoSolution { variable: String, reason: String },
    /// Constraint has no free variables (already determined)
    AlreadyDetermined,
    /// Constraint has multiple free variables (need more equations)
    UnderDetermined { free_vars: Vec<String> },
    /// Multiple variables solved simultaneously (Newton-Raphson).
    MultiSolved { solutions: HashMap<String, f64> },
}

/// Attempt to solve a single constraint for its lone free variable.
///
/// Strategy:
/// 1. Compile the constraint expression
/// 2. Identify free variables not already in `known`
/// 3. If exactly 1 free variable, use bisection to find the value that makes
///    the constraint evaluate to `true` (for boolean constraints) or `0` (for numeric)
/// 4. If 0 free variables → AlreadyDetermined
/// 5. If >1 free variables → UnderDetermined
pub fn solve_constraint(
    expr: &ExprIR,
    known: &HashMap<String, Value>,
    search_range: (f64, f64),
) -> SolveResult {
    let all_vars = expr.free_variables();
    let free: Vec<String> = all_vars
        .into_iter()
        .filter(|v| !known.contains_key(v))
        .collect();

    match free.len() {
        0 => SolveResult::AlreadyDetermined,
        1 => {
            let var_name = &free[0];
            match bisection_solve(expr, var_name, known, search_range.0, search_range.1) {
                Some(value) => SolveResult::Solved {
                    variable: var_name.clone(),
                    value,
                },
                None => SolveResult::NoSolution {
                    variable: var_name.clone(),
                    reason: "bisection did not converge".into(),
                },
            }
        }
        _ => SolveResult::UnderDetermined { free_vars: free },
    }
}

/// Solve a system of constraints simultaneously.
///
/// When the number of free variables equals the number of constraints,
/// uses Newton-Raphson to solve the system. Falls back to single-variable
/// bisection when only one constraint has one free variable, and returns
/// `UnderDetermined` when the system cannot be matched.
pub fn solve_constraints(
    exprs: &[ExprIR],
    known: &HashMap<String, Value>,
    search_range: (f64, f64),
) -> SolveResult {
    // Collect all free variables across all constraints
    let mut all_free: HashSet<String> = HashSet::new();
    for expr in exprs {
        for v in expr.free_variables() {
            if !known.contains_key(&v) {
                all_free.insert(v);
            }
        }
    }

    let free_vars: Vec<String> = {
        let mut v: Vec<String> = all_free.into_iter().collect();
        v.sort(); // deterministic ordering
        v
    };

    match (exprs.len(), free_vars.len()) {
        (_, 0) => SolveResult::AlreadyDetermined,
        (1, 1) => solve_constraint(&exprs[0], known, search_range),
        (n_eq, n_var) if n_eq == n_var => {
            // Square system: try Newton-Raphson
            let ctx = EvalContext::new();
            match newton_raphson_solve(exprs, &free_vars, known, &ctx) {
                Ok(solutions) => SolveResult::MultiSolved { solutions },
                Err(reason) => SolveResult::NoSolution {
                    variable: free_vars.join(", "),
                    reason,
                },
            }
        }
        _ => SolveResult::UnderDetermined { free_vars },
    }
}

/// Bisection method: find value of `var_name` in [lo, hi] that makes
/// the constraint expression evaluate to true (boolean) or zero (numeric).
///
/// For boolean constraints (e.g., `x > 5`), finds the boundary where
/// the result flips from false to true.
/// For numeric constraints (e.g., `x - 5`), finds the zero crossing.
fn bisection_solve(
    expr: &ExprIR,
    var_name: &str,
    known: &HashMap<String, Value>,
    lo: f64,
    hi: f64,
) -> Option<f64> {
    let evaluator = ExpressionEvaluator::new();
    let max_iterations = 64; // 64 iterations of bisection gives ~1e-19 precision
    let tolerance = 1e-10;

    let eval_at = |x: f64| -> Option<f64> {
        let mut ctx = EvalContext::new();
        for (k, v) in known {
            ctx.set(k.clone(), v.clone());
        }
        ctx.set(var_name.to_owned(), Value::Float(x));

        match evaluator.eval(expr, &ctx) {
            Ok(Value::Bool(true)) => Some(1.0),
            Ok(Value::Bool(false)) => Some(-1.0),
            Ok(Value::Float(f)) => Some(f),
            Ok(Value::Int(i)) => Some(i as f64),
            _ => None,
        }
    };

    let f_lo = eval_at(lo)?;
    let f_hi = eval_at(hi)?;

    // For boolean constraints, find where it flips
    // For numeric, find zero crossing
    if f_lo.signum() == f_hi.signum() {
        // No sign change — try to find the boundary by sampling
        // If both are positive (both true), constraint is already satisfied everywhere
        if f_lo > 0.0 && f_hi > 0.0 {
            return Some(lo); // any value works
        }
        return None; // no solution in range
    }

    let mut a = lo;
    let mut b = hi;
    let mut fa = f_lo;

    for _ in 0..max_iterations {
        let mid = (a + b) / 2.0;
        if (b - a).abs() < tolerance {
            return Some(mid);
        }

        let f_mid = eval_at(mid)?;
        if f_mid.abs() < tolerance {
            return Some(mid);
        }

        if fa.signum() == f_mid.signum() {
            a = mid;
            fa = f_mid;
        } else {
            b = mid;
        }
    }

    Some((a + b) / 2.0) // best approximation
}

// ---------------------------------------------------------------------------
// Multi-variable numeric solving (Newton-Raphson)
// ---------------------------------------------------------------------------

/// Solve Ax = b via Gaussian elimination with partial pivoting.
/// Returns None if the system is singular.
pub(crate) fn solve_linear_system(a: &[Vec<f64>], b: &[f64]) -> Option<Vec<f64>> {
    let n = b.len();
    // Augmented matrix [A|b]
    let mut aug: Vec<Vec<f64>> = a
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let mut r = row.clone();
            r.push(b[i]);
            r
        })
        .collect();

    // Forward elimination with partial pivoting
    for col in 0..n {
        // Find pivot
        let mut max_row = col;
        let mut max_val = aug[col][col].abs();
        for row in (col + 1)..n {
            if aug[row][col].abs() > max_val {
                max_val = aug[row][col].abs();
                max_row = row;
            }
        }
        if max_val < 1e-14 {
            return None;
        } // Singular
        aug.swap(col, max_row);

        // Eliminate below
        for row in (col + 1)..n {
            let factor = aug[row][col] / aug[col][col];
            for j in col..=n {
                aug[row][j] -= factor * aug[col][j];
            }
        }
    }

    // Back substitution
    let mut x = vec![0.0; n];
    for i in (0..n).rev() {
        x[i] = aug[i][n];
        for j in (i + 1)..n {
            x[i] -= aug[i][j] * x[j];
        }
        x[i] /= aug[i][i];
    }
    Some(x)
}

/// Evaluate constraint residuals: for each constraint expr, compute its value
/// with the given variable assignments. For constraints like `a < b`, the
/// residual is `a - b` (negative when satisfied).
pub(crate) fn evaluate_residuals(
    constraints: &[ExprIR],
    vars: &[String],
    x: &[f64],
    base_ctx: &EvalContext,
    evaluator: &ExpressionEvaluator,
) -> Vec<f64> {
    let mut ctx = base_ctx.scratch_snapshot();
    for (var, val) in vars.iter().zip(x.iter()) {
        ctx.set(var.clone(), Value::Float(*val));
    }
    constraints
        .iter()
        .map(|expr| match evaluator.eval(expr, &ctx) {
            Ok(Value::Float(f)) => f,
            Ok(Value::Int(i)) => i as f64,
            Ok(Value::Bool(true)) => 0.0,  // satisfied
            Ok(Value::Bool(false)) => 1.0, // violated
            _ => f64::NAN,
        })
        .collect()
}

/// Compute the Jacobian matrix via forward finite differences.
/// J[i][j] = d(constraint_i) / d(var_j) ≈ (f(x + h*e_j) - f(x)) / h
pub(crate) fn compute_jacobian(
    constraints: &[ExprIR],
    vars: &[String],
    x: &[f64],
    ctx: &EvalContext,
    evaluator: &ExpressionEvaluator,
    h: f64,
) -> Vec<Vec<f64>> {
    let n_constraints = constraints.len();
    let n_vars = vars.len();

    // Evaluate f(x)
    let f_x = evaluate_residuals(constraints, vars, x, ctx, evaluator);

    let mut jac = vec![vec![0.0; n_vars]; n_constraints];
    for j in 0..n_vars {
        let mut x_plus = x.to_vec();
        x_plus[j] += h;
        let f_plus = evaluate_residuals(constraints, vars, &x_plus, ctx, evaluator);
        for i in 0..n_constraints {
            jac[i][j] = (f_plus[i] - f_x[i]) / h;
        }
    }
    jac
}

/// Multi-variable Newton-Raphson solver.
///
/// Solves a system of N constraints with N free variables by iteratively
/// computing the Jacobian and solving the linear system J*dx = -F(x).
///
/// Returns a mapping of variable names to solved values, or an error
/// if convergence fails.
pub fn newton_raphson_solve(
    constraints: &[ExprIR],
    free_vars: &[String],
    known: &HashMap<String, Value>,
    ctx: &EvalContext,
) -> Result<HashMap<String, f64>, String> {
    if constraints.len() != free_vars.len() {
        return Err(format!(
            "Newton-Raphson requires equal constraints ({}) and variables ({})",
            constraints.len(),
            free_vars.len()
        ));
    }

    let evaluator = ExpressionEvaluator::new();
    let n = free_vars.len();
    let h = 1e-8; // finite difference step
    let tol = 1e-10;
    let max_iter = 100;

    // Build context with known values
    let mut solve_ctx = ctx.scratch_snapshot();
    for (k, v) in known {
        solve_ctx.set(k.clone(), v.clone());
    }

    // Initial guess: use existing context values if available, else 1.0
    let mut x: Vec<f64> = free_vars
        .iter()
        .map(|v| {
            solve_ctx
                .get(v)
                .and_then(|val| match val {
                    Value::Float(f) => Some(*f),
                    Value::Int(i) => Some(*i as f64),
                    _ => None,
                })
                .unwrap_or(1.0)
        })
        .collect();

    for _iter in 0..max_iter {
        let f = evaluate_residuals(constraints, free_vars, &x, &solve_ctx, &evaluator);

        // Check convergence: ||F(x)|| < tol
        let norm: f64 = f.iter().map(|fi| fi * fi).sum::<f64>().sqrt();
        if norm < tol {
            let mut result = HashMap::new();
            for (var, val) in free_vars.iter().zip(x.iter()) {
                result.insert(var.clone(), *val);
            }
            return Ok(result);
        }

        // Compute Jacobian
        let jac = compute_jacobian(constraints, free_vars, &x, &solve_ctx, &evaluator, h);

        // Solve J * dx = -F
        let neg_f: Vec<f64> = f.iter().map(|fi| -fi).collect();
        let dx = solve_linear_system(&jac, &neg_f)
            .ok_or_else(|| "Jacobian is singular — system may be ill-conditioned".to_owned())?;

        // Update: x = x + dx (with damping if step is too large)
        let step_norm: f64 = dx.iter().map(|di| di * di).sum::<f64>().sqrt();
        let damping = if step_norm > 10.0 {
            10.0 / step_norm
        } else {
            1.0
        };
        for i in 0..n {
            x[i] += damping * dx[i];
        }
    }

    Err(format!(
        "Newton-Raphson did not converge after {} iterations",
        max_iter
    ))
}

// ---------------------------------------------------------------------------
// DOF (Degrees of Freedom) Analysis
// ---------------------------------------------------------------------------

/// Result of DOF analysis on a constraint system.
#[derive(Debug, Clone)]
pub struct DofAnalysis {
    /// Total number of equations (constraints + equalities)
    pub equations: usize,
    /// Total number of unique variables
    pub variables: usize,
    /// Number of known (fixed) variables
    pub known_count: usize,
    /// Number of free (unknown) variables
    pub free_count: usize,
    /// Degrees of freedom = free_count - equations_over_free
    /// Positive = under-determined, Zero = determined, Negative = over-determined
    pub dof: i64,
    /// Human-readable status
    pub status: DofStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DofStatus {
    /// Exactly determined (DOF = 0)
    Determined,
    /// Under-determined (DOF > 0, more unknowns than equations)
    UnderDetermined,
    /// Over-determined (DOF < 0, more equations than unknowns)
    OverDetermined,
}

/// Analyze degrees of freedom for a constraint network + constraint set.
///
/// Counts:
/// - Equations: equality constraints (binding connectors) + boolean/numeric constraints
/// - Variables: all unique variable names across all constraints and equalities
/// - Known: variables with values in the network's known map
/// - Free: variables - known
/// - DOF: free - (equations that involve only free variables)
pub fn analyze_dof(
    network: &ConstraintNetwork,
    constraints: &PrecompiledConstraintSet,
) -> DofAnalysis {
    let mut all_vars: HashSet<String> = HashSet::new();

    // Variables from equalities
    for (a, b) in &network.equalities {
        all_vars.insert(a.clone());
        all_vars.insert(b.clone());
    }

    // Variables from constraints
    let mut constraint_equations = 0;
    for tc in constraints.compiled.as_slice() {
        let free = tc.expr_ir.free_variables();
        all_vars.extend(free);
        constraint_equations += 1;
    }

    let total_equations = network.equalities.len() + constraint_equations;
    let known_count = network
        .known
        .keys()
        .filter(|k| all_vars.contains(k.as_str()))
        .count();
    let free_count = all_vars.len().saturating_sub(known_count);

    // DOF = free variables - equations
    // Positive = under-determined, Zero = just-determined, Negative = over-determined
    // When all variables are known but constraints exist, the system is over-determined
    // (constraints are redundant checks, not equations to solve)
    let dof = free_count as i64 - total_equations as i64;

    let status = if dof == 0 {
        DofStatus::Determined
    } else if dof > 0 {
        DofStatus::UnderDetermined
    } else {
        DofStatus::OverDetermined
    };

    DofAnalysis {
        equations: total_equations,
        variables: all_vars.len(),
        known_count,
        free_count,
        dof,
        status,
    }
}

// ---------------------------------------------------------------------------
// Sensitivity Analysis
// ---------------------------------------------------------------------------

/// Result of sweeping a single parameter across a range.
#[derive(Debug, Clone)]
pub struct SensitivityResult {
    /// The parameter being swept
    pub parameter: String,
    /// Values sampled
    pub samples: Vec<f64>,
    /// For each constraint: (name, flipped_at_index, flip_direction)
    pub constraint_effects: Vec<ConstraintEffect>,
}

/// How a constraint responds to a parameter sweep.
#[derive(Debug, Clone)]
pub struct ConstraintEffect {
    /// Constraint description or expression
    pub constraint_name: String,
    /// Index in samples where the constraint first flips (None if stable)
    pub flip_index: Option<usize>,
    /// Value at which the constraint flips
    pub flip_value: Option<f64>,
    /// Whether the constraint goes from pass→fail or fail→pass
    pub flip_direction: Option<FlipDirection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlipDirection {
    PassToFail,
    FailToPass,
}

/// Sweep a parameter across a range and observe which constraints flip.
///
/// Returns sensitivity data showing the threshold value where each
/// constraint changes satisfaction status.
pub fn sweep_parameter(
    parameter: &str,
    range: (f64, f64),
    steps: usize,
    constraints: &PrecompiledConstraintSet,
    base_context: &EvalContext,
) -> SensitivityResult {
    let evaluator = ExpressionEvaluator::new();
    let step_size = if steps > 1 {
        (range.1 - range.0) / (steps - 1) as f64
    } else {
        0.0
    };

    let samples: Vec<f64> = (0..steps).map(|i| range.0 + i as f64 * step_size).collect();

    // Evaluate all constraints at each sample point
    let compiled = constraints.compiled.as_slice();
    let mut per_constraint: Vec<Vec<bool>> = vec![Vec::with_capacity(steps); compiled.len()];

    for &sample in &samples {
        let mut ctx = base_context.scratch_snapshot();
        ctx.set(parameter.to_owned(), Value::Float(sample));

        for (ci, tc) in compiled.iter().enumerate() {
            let satisfied = evaluator
                .eval(&tc.expr_ir, &ctx)
                .ok()
                .and_then(|v| match v {
                    Value::Bool(b) => Some(b),
                    _ => None,
                })
                .unwrap_or(false);
            per_constraint[ci].push(satisfied);
        }
    }

    // Find flip points
    let constraint_effects: Vec<ConstraintEffect> = compiled
        .iter()
        .enumerate()
        .map(|(ci, tc)| {
            let results = &per_constraint[ci];
            let name = tc
                .constraint
                .description
                .clone()
                .unwrap_or_else(|| tc.constraint.expr.clone());

            let mut flip_index = None;
            let mut flip_direction = None;

            for i in 1..results.len() {
                if results[i] != results[i - 1] {
                    flip_index = Some(i);
                    flip_direction = Some(if results[i] {
                        FlipDirection::FailToPass
                    } else {
                        FlipDirection::PassToFail
                    });
                    break;
                }
            }

            let flip_value = flip_index.map(|idx| samples[idx]);

            ConstraintEffect {
                constraint_name: name,
                flip_index,
                flip_value,
                flip_direction,
            }
        })
        .collect();

    SensitivityResult {
        parameter: parameter.to_owned(),
        samples,
        constraint_effects,
    }
}

// ---------------------------------------------------------------------------
// Model graph integration
// ---------------------------------------------------------------------------

/// Build a constraint network from a ModelGraph.
///
/// Extracts:
/// - Binding connector equalities (RelationshipKind::Binding)
/// - Known values from AttributeUsage elements with default/value properties
pub fn build_constraint_network(graph: &ModelGraph) -> ConstraintNetwork {
    let mut network = ConstraintNetwork::new();

    // Extract equalities from binding connectors.
    //
    // RSC-5.3 (D-5.0.4) KNOWN GAP — binding-connector magnitude propagation is
    // currently scale-naive: it copies bare values between feature names without
    // unit conversion. Boundary auto-conversion shipped for the flow/signal-link
    // seam (`compile_signal_propagation` → `SignalPropPair::convert`), where both
    // endpoints' `SlotMeta.m_ref` are reachable at compile time. This solver path
    // is string-keyed (bare names + bare `Value`, no `SlotStore`), so converting
    // here needs the store threaded in (or a post-propagation pass). Cross-DIMENSION
    // binding endpoints are still diagnosed (UQ001); cross-SCALE same-dimension
    // bindings are the open gap tracked as RSC-5.3-B (ledger). The spec is
    // SPEC-SILENT on conversion at bindings (a sanctioned extension, design doc §3),
    // so this is a deliberate, documented deferral — not a silent soft fallback.
    for rel in graph.relationships_by_kind(&RelationshipKind::Binding) {
        let source_name = graph.get_element(&rel.source).and_then(|e| e.name.clone());
        let target_name = graph.get_element(&rel.target).and_then(|e| e.name.clone());

        if let (Some(src), Some(tgt)) = (source_name, target_name) {
            network.add_equality(src, tgt);
        }
    }

    // Extract known values from attribute usages. Only typed-literal props
    // (`default` / `value`) participate — the constraint network is for
    // solving over concrete values, not symbolic expressions. Post-Phase-6D
    // the parser no longer writes `unresolved_value`; anything non-literal
    // lives in the AST subtree and is out of scope for this extractor.
    for elem in graph.elements_by_kind(&ElementKind::AttributeUsage) {
        if let Some(name) = &elem.name {
            let value = elem.get_prop("default").or_else(|| elem.get_prop("value"));

            if let Some(val) = value {
                match val {
                    Value::Int(_) | Value::Float(_) | Value::Bool(_) | Value::String(_) => {
                        network.set(name.clone(), val.clone());
                    }
                    _ => {}
                }
            }
        }
    }

    network
}

/// Compute a rollup aggregation over the ownership hierarchy.
///
/// Starting from `root_id`, recursively collects the named property from
/// all owned parts (and their sub-parts), then aggregates using the
/// specified function.
///
/// Example: `compute_rollup(graph, vehicle_id, "mass", Sum)` sums mass
/// from all sub-parts recursively.
pub fn compute_rollup(
    graph: &ModelGraph,
    root_id: &sysml_core::ElementId,
    property_name: &str,
    aggregation: AggregationKind,
) -> Option<Value> {
    let mut values = Vec::new();
    collect_property_recursive(graph, root_id, property_name, &mut values);

    if values.is_empty() {
        return None;
    }

    Some(aggregate(&values, aggregation))
}

/// Recursively collect a named property from all owned elements.
fn collect_property_recursive(
    graph: &ModelGraph,
    owner_id: &sysml_core::ElementId,
    property_name: &str,
    values: &mut Vec<f64>,
) {
    for child in graph.children_of(owner_id) {
        // Check if this child has the property
        if child.name.as_deref() == Some(property_name) {
            if let Some(val) = extract_numeric(&child.props) {
                values.push(val);
            }
        }

        // Also check child's properties map
        if let Some(val) = child.get_prop(property_name) {
            if let Some(num) = value_to_f64(val) {
                values.push(num);
            }
        }

        // Recurse into parts (structural children)
        if child.kind.is_usage() || child.kind.is_definition() {
            collect_property_recursive(graph, &child.id, property_name, values);
        }
    }
}

/// Extract a numeric value from element properties (`default` or `value`).
/// Rollups operate on typed literal props only — AST expression subtrees
/// are not evaluated here.
fn extract_numeric(
    props: &std::collections::BTreeMap<std::borrow::Cow<'static, str>, Value>,
) -> Option<f64> {
    props
        .get("default")
        .or_else(|| props.get("value"))
        .and_then(value_to_f64)
}

/// Convert a Value to f64.
fn value_to_f64(v: &Value) -> Option<f64> {
    match v {
        Value::Float(f) => Some(*f),
        Value::Int(i) => Some(*i as f64),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Apply an aggregation function to a list of values.
fn aggregate(values: &[f64], kind: AggregationKind) -> Value {
    match kind {
        AggregationKind::Sum => Value::Float(values.iter().sum()),
        AggregationKind::Max => {
            Value::Float(values.iter().cloned().fold(f64::NEG_INFINITY, f64::max))
        }
        AggregationKind::Min => Value::Float(values.iter().cloned().fold(f64::INFINITY, f64::min)),
        AggregationKind::Avg => {
            let sum: f64 = values.iter().sum();
            Value::Float(sum / values.len() as f64)
        }
        AggregationKind::Count => Value::Int(values.len() as i64),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn propagate_simple_chain() {
        let mut net = ConstraintNetwork::new();
        net.add_equality("a", "b");
        net.add_equality("b", "c");
        net.set("a", Value::Float(42.0));

        let result = net.propagate();
        assert_eq!(result.solved.get("a"), Some(&Value::Float(42.0)));
        assert_eq!(result.solved.get("b"), Some(&Value::Float(42.0)));
        assert_eq!(result.solved.get("c"), Some(&Value::Float(42.0)));
        assert!(result.unsolved.is_empty());
    }

    #[test]
    fn propagate_reverse_direction() {
        let mut net = ConstraintNetwork::new();
        net.add_equality("x", "y");
        net.set("y", Value::Int(7));

        let result = net.propagate();
        assert_eq!(result.solved.get("x"), Some(&Value::Int(7)));
        assert_eq!(result.solved.get("y"), Some(&Value::Int(7)));
    }

    #[test]
    fn propagate_disconnected_unsolved() {
        let mut net = ConstraintNetwork::new();
        net.add_equality("a", "b");
        net.add_equality("c", "d");
        net.set("a", Value::Float(1.0));
        // c and d have no known values

        let result = net.propagate();
        assert_eq!(result.solved.get("b"), Some(&Value::Float(1.0)));
        assert!(!result.unsolved.is_empty());
        assert!(
            result.unsolved.contains(&"c".to_string())
                || result.unsolved.contains(&"d".to_string())
        );
    }

    #[test]
    fn propagate_empty_network() {
        let mut net = ConstraintNetwork::new();
        let result = net.propagate();
        assert!(result.solved.is_empty());
        assert!(result.unsolved.is_empty());
        assert_eq!(result.iterations, 1);
    }

    #[test]
    fn propagate_long_chain() {
        let mut net = ConstraintNetwork::new();
        // Chain: a = b = c = d = e
        net.add_equality("a", "b");
        net.add_equality("b", "c");
        net.add_equality("c", "d");
        net.add_equality("d", "e");
        net.set("e", Value::Float(99.0));

        let result = net.propagate();
        for var in &["a", "b", "c", "d", "e"] {
            assert_eq!(
                result.solved.get(*var),
                Some(&Value::Float(99.0)),
                "variable {var} should be 99.0"
            );
        }
    }

    #[test]
    fn propagate_branching() {
        let mut net = ConstraintNetwork::new();
        // a = b, a = c (fan-out)
        net.add_equality("a", "b");
        net.add_equality("a", "c");
        net.set("a", Value::Float(10.0));

        let result = net.propagate();
        assert_eq!(result.solved.get("b"), Some(&Value::Float(10.0)));
        assert_eq!(result.solved.get("c"), Some(&Value::Float(10.0)));
    }

    #[test]
    fn aggregate_sum() {
        assert_eq!(
            aggregate(&[1.0, 2.0, 3.0], AggregationKind::Sum),
            Value::Float(6.0)
        );
    }

    #[test]
    fn aggregate_max() {
        assert_eq!(
            aggregate(&[1.0, 5.0, 3.0], AggregationKind::Max),
            Value::Float(5.0)
        );
    }

    #[test]
    fn aggregate_min() {
        assert_eq!(
            aggregate(&[1.0, 5.0, 3.0], AggregationKind::Min),
            Value::Float(1.0)
        );
    }

    #[test]
    fn aggregate_avg() {
        assert_eq!(
            aggregate(&[2.0, 4.0, 6.0], AggregationKind::Avg),
            Value::Float(4.0)
        );
    }

    #[test]
    fn aggregate_count() {
        assert_eq!(
            aggregate(&[1.0, 2.0, 3.0], AggregationKind::Count),
            Value::Int(3)
        );
    }

    #[test]
    fn build_network_from_graph() {
        use sysml_core::{Element, ModelGraph, Relationship};

        let mut graph = ModelGraph::new();

        // Create two attributes with binding
        let mut attr_a = Element::new_with_kind(ElementKind::AttributeUsage);
        attr_a.name = Some("totalMass".into());
        let id_a = graph.add_element(attr_a);

        let mut attr_b = Element::new_with_kind(ElementKind::AttributeUsage);
        attr_b.name = Some("computedMass".into());
        attr_b.set_prop("default", Value::Float(2500.0));
        let id_b = graph.add_element(attr_b);

        // Create binding connector relationship
        let binding = Relationship::new(RelationshipKind::Binding, id_a.clone(), id_b.clone());
        graph.add_relationship(binding);

        let mut network = build_constraint_network(&graph);
        assert_eq!(network.equalities.len(), 1);
        assert_eq!(
            network.known.get("computedMass"),
            Some(&Value::Float(2500.0))
        );

        let result = network.propagate();
        assert_eq!(result.solved.get("totalMass"), Some(&Value::Float(2500.0)));
    }

    // -----------------------------------------------------------------------
    // Numeric solver tests
    // -----------------------------------------------------------------------

    #[test]
    fn solve_single_variable_gt() {
        // Constraint: x > 5 — find the boundary
        use crate::expressions::compile_simple_expression;
        let expr = compile_simple_expression("x > 5").unwrap();
        let known = HashMap::new();

        let result = solve_constraint(&expr, &known, (0.0, 10.0));
        match result {
            SolveResult::Solved { variable, value } => {
                assert_eq!(variable, "x");
                assert!((value - 5.0).abs() < 0.01, "expected ~5.0, got {value}");
            }
            other => panic!("expected Solved, got {other:?}"),
        }
    }

    #[test]
    fn solve_single_variable_equality() {
        // Constraint: x - 10 (numeric, find zero)
        use crate::expressions::compile_simple_expression;
        let expr = compile_simple_expression("x - 10").unwrap();
        let known = HashMap::new();

        let result = solve_constraint(&expr, &known, (-100.0, 100.0));
        match result {
            SolveResult::Solved { value, .. } => {
                assert!((value - 10.0).abs() < 0.01, "expected ~10.0, got {value}");
            }
            other => panic!("expected Solved, got {other:?}"),
        }
    }

    #[test]
    fn solve_with_known_variables() {
        // Constraint: x + y > 10, with y=3 known → solve for x
        use crate::expressions::compile_simple_expression;
        let expr = compile_simple_expression("x + y > 10").unwrap();
        let mut known = HashMap::new();
        known.insert("y".into(), Value::Float(3.0));

        let result = solve_constraint(&expr, &known, (0.0, 20.0));
        match result {
            SolveResult::Solved { variable, value } => {
                assert_eq!(variable, "x");
                assert!((value - 7.0).abs() < 0.1, "expected ~7.0, got {value}");
            }
            other => panic!("expected Solved, got {other:?}"),
        }
    }

    #[test]
    fn solve_already_determined() {
        use crate::expressions::compile_simple_expression;
        let expr = compile_simple_expression("3 > 2").unwrap();
        let known = HashMap::new();

        let result = solve_constraint(&expr, &known, (0.0, 10.0));
        assert!(matches!(result, SolveResult::AlreadyDetermined));
    }

    #[test]
    fn solve_under_determined() {
        use crate::expressions::compile_simple_expression;
        let expr = compile_simple_expression("x + y > 10").unwrap();
        let known = HashMap::new(); // neither x nor y known

        let result = solve_constraint(&expr, &known, (0.0, 10.0));
        match result {
            SolveResult::UnderDetermined { free_vars } => {
                assert_eq!(free_vars.len(), 2);
            }
            other => panic!("expected UnderDetermined, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // DOF analysis tests
    // -----------------------------------------------------------------------

    #[test]
    fn dof_determined_system() {
        use crate::constraints::TypedConstraint;
        use crate::expressions::compile_simple_expression;
        use crate::ConstraintIR;

        let mut network = ConstraintNetwork::new();
        network.set("y", Value::Float(3.0));

        // 1 constraint with 1 free variable = determined
        let expr = compile_simple_expression("x + y > 10").unwrap();
        let constraint_set = PrecompiledConstraintSet {
            compiled: vec![TypedConstraint {
                constraint: ConstraintIR {
                    expr: "x + y > 10".into(),
                    description: Some("test".into()),
                    owner_id: None,
                    is_negated: false,
                },
                expr_ir: expr,
            }],
            failed: vec![],
        };

        let dof = analyze_dof(&network, &constraint_set);
        assert_eq!(dof.free_count, 1); // x is free
        assert_eq!(dof.known_count, 1); // y is known
        assert_eq!(dof.equations, 1);
        assert_eq!(dof.status, DofStatus::Determined);
    }

    #[test]
    fn dof_under_determined() {
        let network = ConstraintNetwork::new();
        // 1 constraint with 2 free variables
        use crate::constraints::TypedConstraint;
        use crate::expressions::compile_simple_expression;
        use crate::ConstraintIR;

        let expr = compile_simple_expression("x + y > 10").unwrap();
        let constraint_set = PrecompiledConstraintSet {
            compiled: vec![TypedConstraint {
                constraint: ConstraintIR {
                    expr: "x + y > 10".into(),
                    description: None,
                    owner_id: None,
                    is_negated: false,
                },
                expr_ir: expr,
            }],
            failed: vec![],
        };

        let dof = analyze_dof(&network, &constraint_set);
        assert_eq!(dof.free_count, 2);
        assert_eq!(dof.equations, 1);
        assert_eq!(dof.status, DofStatus::UnderDetermined);
    }

    #[test]
    fn dof_over_determined() {
        let mut network = ConstraintNetwork::new();
        network.set("x", Value::Float(5.0));

        use crate::constraints::TypedConstraint;
        use crate::expressions::compile_simple_expression;
        use crate::ConstraintIR;

        // 2 constraints but 0 free variables (x already known)
        let expr1 = compile_simple_expression("x > 3").unwrap();
        let expr2 = compile_simple_expression("x < 10").unwrap();
        let constraint_set = PrecompiledConstraintSet {
            compiled: vec![
                TypedConstraint {
                    constraint: ConstraintIR {
                        expr: "x > 3".into(),
                        description: None,
                        owner_id: None,
                        is_negated: false,
                    },
                    expr_ir: expr1,
                },
                TypedConstraint {
                    constraint: ConstraintIR {
                        expr: "x < 10".into(),
                        description: None,
                        owner_id: None,
                        is_negated: false,
                    },
                    expr_ir: expr2,
                },
            ],
            failed: vec![],
        };

        let dof = analyze_dof(&network, &constraint_set);
        // x is known, so free_count = 0, but we have 2 equations → over-determined
        assert_eq!(dof.status, DofStatus::OverDetermined);
    }

    // -----------------------------------------------------------------------
    // Sensitivity analysis tests
    // -----------------------------------------------------------------------

    #[test]
    fn sensitivity_finds_flip_point() {
        use crate::constraints::TypedConstraint;
        use crate::expressions::compile_simple_expression;
        use crate::ConstraintIR;

        // speed < 100 — sweep speed from 50 to 150
        let expr = compile_simple_expression("speed < 100").unwrap();
        let constraints = PrecompiledConstraintSet {
            compiled: vec![TypedConstraint {
                constraint: ConstraintIR {
                    expr: "speed < 100".into(),
                    description: Some("speedLimit".into()),
                    owner_id: None,
                    is_negated: false,
                },
                expr_ir: expr,
            }],
            failed: vec![],
        };

        let ctx = EvalContext::new();
        let result = sweep_parameter("speed", (50.0, 150.0), 101, &constraints, &ctx);

        assert_eq!(result.parameter, "speed");
        assert_eq!(result.samples.len(), 101);
        assert_eq!(result.constraint_effects.len(), 1);

        let effect = &result.constraint_effects[0];
        assert_eq!(effect.constraint_name, "speedLimit");
        assert!(effect.flip_index.is_some());
        let flip_val = effect.flip_value.unwrap();
        assert!(
            (flip_val - 100.0).abs() < 2.0,
            "expected flip near 100, got {flip_val}"
        );
        assert_eq!(effect.flip_direction, Some(FlipDirection::PassToFail));
    }

    #[test]
    fn sensitivity_no_flip() {
        use crate::constraints::TypedConstraint;
        use crate::expressions::compile_simple_expression;
        use crate::ConstraintIR;

        // speed < 100 — sweep from 10 to 50 (always passes)
        let expr = compile_simple_expression("speed < 100").unwrap();
        let constraints = PrecompiledConstraintSet {
            compiled: vec![TypedConstraint {
                constraint: ConstraintIR {
                    expr: "speed < 100".into(),
                    description: Some("speedLimit".into()),
                    owner_id: None,
                    is_negated: false,
                },
                expr_ir: expr,
            }],
            failed: vec![],
        };

        let ctx = EvalContext::new();
        let result = sweep_parameter("speed", (10.0, 50.0), 41, &constraints, &ctx);

        let effect = &result.constraint_effects[0];
        assert!(effect.flip_index.is_none(), "should not flip in safe range");
    }

    // -----------------------------------------------------------------------
    // Gaussian elimination tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_solve_linear_system_2x2() {
        // 2x + y = 5, x + 3y = 10 -> x = 1, y = 3
        let a = vec![vec![2.0, 1.0], vec![1.0, 3.0]];
        let b = vec![5.0, 10.0];
        let x = solve_linear_system(&a, &b).unwrap();
        assert!((x[0] - 1.0).abs() < 1e-10);
        assert!((x[1] - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_solve_linear_system_singular() {
        let a = vec![vec![1.0, 2.0], vec![2.0, 4.0]]; // rank 1
        let b = vec![3.0, 6.0];
        assert!(solve_linear_system(&a, &b).is_none());
    }

    #[test]
    fn test_solve_linear_system_3x3() {
        // x + y + z = 6, 2x + y - z = 1, x - y + z = 2
        // Solution: x = 1, y = 2, z = 3
        let a = vec![
            vec![1.0, 1.0, 1.0],
            vec![2.0, 1.0, -1.0],
            vec![1.0, -1.0, 1.0],
        ];
        let b = vec![6.0, 1.0, 2.0];
        let x = solve_linear_system(&a, &b).unwrap();
        assert!((x[0] - 1.0).abs() < 1e-10);
        assert!((x[1] - 2.0).abs() < 1e-10);
        assert!((x[2] - 3.0).abs() < 1e-10);
    }

    // -----------------------------------------------------------------------
    // Newton-Raphson tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_newton_raphson_two_vars() {
        use crate::expressions::compile_simple_expression;
        // x^2 + y^2 = 25 (circle) and x - y = 1 (line)
        // Note: use (x - y) - 1 to avoid right-associative parsing of x - y - 1
        let c1 = compile_simple_expression("x * x + y * y - 25").unwrap();
        let c2 = compile_simple_expression("(x - y) - 1").unwrap();
        let vars = vec!["x".to_string(), "y".to_string()];
        let known = HashMap::new();
        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Float(3.0)); // initial guess near solution
        ctx.set("y", Value::Float(2.0));

        let result = newton_raphson_solve(&[c1, c2], &vars, &known, &ctx).unwrap();
        let x = result["x"];
        let y = result["y"];
        // Verify: x^2 + y^2 ~= 25 and x - y ~= 1
        assert!(
            (x * x + y * y - 25.0).abs() < 1e-6,
            "circle: {}",
            x * x + y * y
        );
        assert!((x - y - 1.0).abs() < 1e-6, "line: {}", x - y);
    }

    #[test]
    fn test_newton_raphson_three_vars() {
        use crate::expressions::compile_simple_expression;
        // x + y + z = 6, x - y = 0, x + z = 4
        // Solution: x = 2, y = 2, z = 2
        let c1 = compile_simple_expression("x + y + z - 6").unwrap();
        let c2 = compile_simple_expression("x - y").unwrap();
        let c3 = compile_simple_expression("x + z - 4").unwrap();
        let vars = vec!["x".to_string(), "y".to_string(), "z".to_string()];
        let known = HashMap::new();
        let ctx = EvalContext::new();

        let result = newton_raphson_solve(&[c1, c2, c3], &vars, &known, &ctx).unwrap();
        assert!((result["x"] - 2.0).abs() < 1e-6);
        assert!((result["y"] - 2.0).abs() < 1e-6);
        assert!((result["z"] - 2.0).abs() < 1e-6);
    }

    #[test]
    fn test_newton_raphson_with_known_values() {
        use crate::expressions::compile_simple_expression;
        // x + y = a, where a is known = 10
        // x - y = 2
        // Note: use (x - y) - 2 to avoid right-associative parsing of x - y - 2
        let c1 = compile_simple_expression("x + y - a").unwrap();
        let c2 = compile_simple_expression("(x - y) - 2").unwrap();
        let vars = vec!["x".to_string(), "y".to_string()];
        let mut known = HashMap::new();
        known.insert("a".into(), Value::Float(10.0));
        let ctx = EvalContext::new();

        let result = newton_raphson_solve(&[c1, c2], &vars, &known, &ctx).unwrap();
        assert!((result["x"] - 6.0).abs() < 1e-6);
        assert!((result["y"] - 4.0).abs() < 1e-6);
    }

    #[test]
    fn test_newton_raphson_nonlinear() {
        use crate::expressions::compile_simple_expression;
        // x * y = 12, x + y = 7 -> x=3,y=4 or x=4,y=3
        let c1 = compile_simple_expression("x * y - 12").unwrap();
        let c2 = compile_simple_expression("x + y - 7").unwrap();
        let vars = vec!["x".to_string(), "y".to_string()];
        let known = HashMap::new();
        let mut ctx = EvalContext::new();
        ctx.set("x", Value::Float(2.0));
        ctx.set("y", Value::Float(5.0));

        let result = newton_raphson_solve(&[c1, c2], &vars, &known, &ctx).unwrap();
        let x = result["x"];
        let y = result["y"];
        assert!((x * y - 12.0).abs() < 1e-6);
        assert!((x + y - 7.0).abs() < 1e-6);
    }

    #[test]
    fn test_newton_raphson_dimension_mismatch() {
        use crate::expressions::compile_simple_expression;
        // 1 constraint but 2 variables -> error
        let c1 = compile_simple_expression("x + y - 5").unwrap();
        let vars = vec!["x".to_string(), "y".to_string()];
        let known = HashMap::new();
        let ctx = EvalContext::new();

        let result = newton_raphson_solve(&[c1], &vars, &known, &ctx);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("equal constraints"));
    }

    // -----------------------------------------------------------------------
    // solve_constraints (multi-constraint entry point) tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_solve_constraints_multi_solved() {
        use crate::expressions::compile_simple_expression;
        // x + y = 10, x - y = 2 -> x = 6, y = 4
        // Note: use (x - y) - 2 to avoid right-associative parsing of x - y - 2
        let c1 = compile_simple_expression("x + y - 10").unwrap();
        let c2 = compile_simple_expression("(x - y) - 2").unwrap();
        let known = HashMap::new();

        let result = solve_constraints(&[c1, c2], &known, (-100.0, 100.0));
        match result {
            SolveResult::MultiSolved { solutions } => {
                assert!((solutions["x"] - 6.0).abs() < 1e-6);
                assert!((solutions["y"] - 4.0).abs() < 1e-6);
            }
            other => panic!("expected MultiSolved, got {other:?}"),
        }
    }

    #[test]
    fn test_solve_constraints_single_falls_back_to_bisection() {
        use crate::expressions::compile_simple_expression;
        // Single constraint with single variable -> uses bisection
        let c1 = compile_simple_expression("x - 7").unwrap();
        let known = HashMap::new();

        let result = solve_constraints(&[c1], &known, (-100.0, 100.0));
        match result {
            SolveResult::Solved { variable, value } => {
                assert_eq!(variable, "x");
                assert!((value - 7.0).abs() < 0.01);
            }
            other => panic!("expected Solved, got {other:?}"),
        }
    }

    #[test]
    fn test_solve_constraints_already_determined() {
        use crate::expressions::compile_simple_expression;
        let c1 = compile_simple_expression("x + y - 10").unwrap();
        let mut known = HashMap::new();
        known.insert("x".into(), Value::Float(6.0));
        known.insert("y".into(), Value::Float(4.0));

        let result = solve_constraints(&[c1], &known, (-100.0, 100.0));
        assert!(matches!(result, SolveResult::AlreadyDetermined));
    }
}
