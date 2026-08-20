//! DAE (Differential-Algebraic Equation) system assembly from bond graph
//! constitutive relations and conservation constraints.
//!
//! Maps the physics engine's `ConstitutiveRelation`, `ConservationConstraint`,
//! and equality types into the semi-explicit DAE form `M * dx/dt = f(t, x)`
//! where M is a (possibly singular) mass matrix. Solves using diffsol's BDF
//! solver — a variable-order backward differentiation formula method suitable
//! for stiff systems and index-1 DAEs.
//!
//! # Architecture
//!
//! ```text
//! ConstitutiveRelation + Constraints
//!       │
//!       ▼
//! BondGraphDae::from_constraints()
//!       │
//!       ├── StateVectorMap (variable ↔ index mapping)
//!       ├── Mass matrix M (diagonal: 1=differential, 0=algebraic)
//!       ├── RHS function f(t, x) → dx/dt
//!       └── Jacobian-vector product J(x)*v
//!       │
//!       ▼
//! diffsol OdeBuilder → BDF solver → DaeSolution
//! ```

use std::collections::HashMap;

use diffsol::{
    DiffsolError, NalgebraLU, NalgebraMat, OdeBuilder, OdeSolverMethod, OdeSolverStopReason,
};

use super::constraints::{
    ConservationConstraint, ConstitutiveRelation, EffortEquality, FlowEquality,
    GeneratedConstraints, UserConstraintExpression,
};

// ---------------------------------------------------------------------------
// StateVectorMap
// ---------------------------------------------------------------------------

/// Bidirectional mapping between named physics variables and flat state vector indices.
///
/// Variables are partitioned into **differential** (C/I state variables whose
/// time derivatives appear in the equations) and **algebraic** (efforts, flows
/// at junctions, R-element variables). Differential variables always come first
/// in the state vector — this is required for the mass matrix structure.
#[derive(Debug, Clone)]
pub struct StateVectorMap {
    /// Variable name → index in the state vector.
    name_to_idx: HashMap<String, usize>,
    /// Index → variable name (for writing results back to EvalContext).
    idx_to_name: Vec<String>,
    /// Whether each index is differential (true) or algebraic (false).
    is_differential: Vec<bool>,
    /// Number of differential state variables.
    pub n_diff: usize,
    /// Number of algebraic variables.
    pub n_alg: usize,
}

impl StateVectorMap {
    /// Total number of variables in the state vector.
    pub fn len(&self) -> usize {
        self.idx_to_name.len()
    }

    /// Whether the state vector is empty.
    pub fn is_empty(&self) -> bool {
        self.idx_to_name.is_empty()
    }

    /// Look up the index for a variable name.
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.name_to_idx.get(name).copied()
    }

    /// Get the variable name for an index.
    pub fn name_of(&self, idx: usize) -> Option<&str> {
        self.idx_to_name.get(idx).map(String::as_str)
    }

    /// Whether the variable at the given index is differential.
    pub fn is_differential(&self, idx: usize) -> bool {
        self.is_differential.get(idx).copied().unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// BondGraphDae
// ---------------------------------------------------------------------------

/// A bond graph system assembled into DAE form for numerical integration.
///
/// Holds the state vector layout, parameter values, and initial conditions.
/// Use [`from_constraints`](Self::from_constraints) to build from generated constraints,
/// then [`solve`](Self::solve) to integrate.
#[derive(Debug, Clone)]
pub struct BondGraphDae {
    /// Variable ↔ index mapping with differential/algebraic partitioning.
    pub map: StateVectorMap,
    /// Constitutive relations (cloned from GeneratedConstraints).
    relations: Vec<ConstitutiveRelation>,
    /// Conservation constraints (cloned from GeneratedConstraints).
    conservation: Vec<ConservationConstraint>,
    /// Effort equalities (cloned from GeneratedConstraints).
    effort_eq: Vec<EffortEquality>,
    /// Flow equalities (cloned from GeneratedConstraints).
    flow_eq: Vec<FlowEquality>,
    /// User-written constraint expressions (algebraic residuals).
    user_constraints: Vec<UserConstraintExpression>,
    /// Initial state vector (default: all zeros).
    pub initial_state: Vec<f64>,
}

/// Solution trajectories from DAE integration.
#[derive(Debug, Clone)]
pub struct DaeSolution {
    /// Time points.
    pub t: Vec<f64>,
    /// State values at each time point. Outer index = variable, inner = time.
    pub x: Vec<Vec<f64>>,
    /// Variable names (same order as state vector).
    pub var_names: Vec<String>,
}

/// Linear state-space representation: `dx/dt = A*x + B*u`, `y = C*x + D*u`.
///
/// Extracted from a linear bond graph DAE via [`BondGraphDae::extract_state_space`].
/// The A matrix captures system dynamics (eigenvalues = natural frequencies/decay rates),
/// B captures source forcing, C is identity (all states observable), D is zero
/// (no direct feedthrough in standard linear bond graphs).
#[derive(Debug, Clone)]
pub struct StateSpaceMatrices {
    /// System matrix (n_diff x n_diff). Eigenvalues determine stability.
    pub a: Vec<Vec<f64>>,
    /// Input matrix (n_diff x n_inputs). Maps source inputs to state derivatives.
    pub b: Vec<Vec<f64>>,
    /// Output matrix (n_outputs x n_diff). Identity for full-state output.
    pub c: Vec<Vec<f64>>,
    /// Feedthrough matrix (n_outputs x n_inputs). Zero for standard bond graphs.
    pub d: Vec<Vec<f64>>,
    /// Names of differential state variables (length n_diff).
    pub state_names: Vec<String>,
    /// Names of input sources (length n_inputs).
    pub input_names: Vec<String>,
    /// Names of output variables (length n_outputs).
    pub output_names: Vec<String>,
}

/// Instantaneous energy balance for a bond graph system.
///
/// Computed by [`BondGraphDae::compute_energy_balance`] at a given state vector.
/// For a lossless system, `total_source` should equal `total_stored + total_dissipated`
/// (first law of thermodynamics).
#[derive(Debug, Clone)]
pub struct EnergyBalance {
    /// Stored energy per C-element: `(var_name, 0.5 * C * e^2)`.
    pub stored_c: Vec<(String, f64)>,
    /// Stored energy per I-element: `(var_name, 0.5 * L * f^2)`.
    pub stored_i: Vec<(String, f64)>,
    /// Instantaneous dissipation power per R-element: `(var_name, R * f^2)`.
    pub dissipated_r: Vec<(String, f64)>,
    /// Instantaneous source power per Se/Sf element: `(var_name, e * f)`.
    pub source_power: Vec<(String, f64)>,
    /// Total stored energy across all C and I elements.
    pub total_stored: f64,
    /// Total instantaneous dissipation power across all R elements.
    pub total_dissipated: f64,
    /// Total instantaneous source power across all Se/Sf elements.
    pub total_source: f64,
}

/// Error during DAE assembly or solving.
#[derive(Debug)]
pub enum DaeError {
    /// Failed to assemble the DAE system.
    Assembly(String),
    /// The DAE solver failed.
    Solver(DiffsolError),
    /// The system has no equations.
    EmptySystem,
}

impl std::fmt::Display for DaeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DaeError::Assembly(msg) => write!(f, "DAE assembly failed: {}", msg),
            DaeError::Solver(e) => write!(f, "DAE solver failed: {}", e),
            DaeError::EmptySystem => write!(f, "DAE system has no equations"),
        }
    }
}

impl std::error::Error for DaeError {}

// ---------------------------------------------------------------------------
// Dense linear algebra helpers (small matrices only)
// ---------------------------------------------------------------------------

/// Invert an n×n dense matrix using Gaussian elimination with partial pivoting.
///
/// For the small matrices typical of bond graph systems (n < 20), this is
/// efficient enough. Panics on singular matrices (which indicate a malformed
/// constraint system).
fn invert_dense(n: usize, mat: &[Vec<f64>]) -> Vec<Vec<f64>> {
    // Augmented matrix [A | I]
    let mut aug: Vec<Vec<f64>> = (0..n)
        .map(|i| {
            let mut row = Vec::with_capacity(2 * n);
            row.extend_from_slice(&mat[i]);
            for j in 0..n {
                row.push(if i == j { 1.0 } else { 0.0 });
            }
            row
        })
        .collect();

    for col in 0..n {
        // Partial pivoting: find row with largest absolute value in this column.
        let mut max_row = col;
        let mut max_val = aug[col][col].abs();
        for row in (col + 1)..n {
            let val = aug[row][col].abs();
            if val > max_val {
                max_val = val;
                max_row = row;
            }
        }
        aug.swap(col, max_row);

        let pivot = aug[col][col];
        assert!(
            pivot.abs() > 1e-15,
            "singular algebraic constraint matrix (pivot {col} = {pivot})"
        );

        // Scale pivot row
        let inv_pivot = 1.0 / pivot;
        for j in 0..(2 * n) {
            aug[col][j] *= inv_pivot;
        }

        // Eliminate column in all other rows
        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = aug[row][col];
            for j in 0..(2 * n) {
                aug[row][j] -= factor * aug[col][j];
            }
        }
    }

    // Extract the inverse from the right half.
    (0..n).map(|i| aug[i][n..].to_vec()).collect()
}

/// Multiply two dense matrices: C = A (rows_a × cols_b) = A (rows_a × inner) * B (inner × cols_b).
fn mat_mul(
    rows_a: usize,
    cols_b: usize,
    inner: usize,
    a: &[Vec<f64>],
    b: &[Vec<f64>],
) -> Vec<Vec<f64>> {
    (0..rows_a)
        .map(|i| {
            (0..cols_b)
                .map(|j| (0..inner).map(|k| a[i][k] * b[k][j]).sum())
                .collect()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Assembly
// ---------------------------------------------------------------------------

impl BondGraphDae {
    /// Assemble a DAE system from generated constraints.
    ///
    /// Walks all constitutive relations, conservation constraints, and equality
    /// constraints to collect variables, partition them into differential vs
    /// algebraic, and build the state vector layout.
    pub fn from_constraints(constraints: &GeneratedConstraints) -> Result<Self, DaeError> {
        let mut diff_vars: Vec<String> = Vec::new();
        let mut alg_vars: Vec<String> = Vec::new();

        // Helper: ensure a variable is in the algebraic set (if not already differential).
        let mut ensure_alg = |name: &str, diff: &[String], alg: &mut Vec<String>| {
            if !diff.iter().any(|d| d == name) && !alg.iter().any(|a| a == name) {
                alg.push(name.to_string());
            }
        };

        // Pass 1: Identify differential state variables from C/I elements.
        for rel in &constraints.constitutive {
            match rel {
                ConstitutiveRelation::Capacitance { effort_var, .. } => {
                    if !diff_vars.contains(effort_var) {
                        diff_vars.push(effort_var.clone());
                    }
                }
                ConstitutiveRelation::Inductance { flow_var, .. } => {
                    if !diff_vars.contains(flow_var) {
                        diff_vars.push(flow_var.clone());
                    }
                }
                _ => {}
            }
        }

        // Pass 2: Collect all other variables as algebraic.
        for rel in &constraints.constitutive {
            match rel {
                ConstitutiveRelation::Resistance {
                    effort_in_var,
                    effort_out_var,
                    flow_var,
                    ..
                } => {
                    ensure_alg(effort_in_var, &diff_vars, &mut alg_vars);
                    ensure_alg(effort_out_var, &diff_vars, &mut alg_vars);
                    ensure_alg(flow_var, &diff_vars, &mut alg_vars);
                }
                ConstitutiveRelation::Conductance {
                    effort_var,
                    flow_var,
                    ..
                } => {
                    ensure_alg(effort_var, &diff_vars, &mut alg_vars);
                    ensure_alg(flow_var, &diff_vars, &mut alg_vars);
                }
                ConstitutiveRelation::Capacitance {
                    effort_var,
                    flow_var,
                    ..
                } => {
                    // effort_var is differential (already added)
                    ensure_alg(flow_var, &diff_vars, &mut alg_vars);
                    let _ = effort_var; // already in diff_vars
                }
                ConstitutiveRelation::Inductance {
                    flow_var,
                    effort_var,
                    ..
                } => {
                    // flow_var is differential (already added)
                    ensure_alg(effort_var, &diff_vars, &mut alg_vars);
                    let _ = flow_var; // already in diff_vars
                }
                ConstitutiveRelation::EffortSource { effort_var, .. } => {
                    ensure_alg(effort_var, &diff_vars, &mut alg_vars);
                }
                ConstitutiveRelation::FlowSource { flow_var, .. } => {
                    ensure_alg(flow_var, &diff_vars, &mut alg_vars);
                }
                ConstitutiveRelation::Transformer {
                    effort_in_var,
                    effort_out_var,
                    flow_in_var,
                    flow_out_var,
                    ..
                }
                | ConstitutiveRelation::Gyrator {
                    effort_in_var,
                    effort_out_var,
                    flow_in_var,
                    flow_out_var,
                    ..
                } => {
                    ensure_alg(effort_in_var, &diff_vars, &mut alg_vars);
                    ensure_alg(effort_out_var, &diff_vars, &mut alg_vars);
                    ensure_alg(flow_in_var, &diff_vars, &mut alg_vars);
                    ensure_alg(flow_out_var, &diff_vars, &mut alg_vars);
                }
            }
        }

        // Pass 3: Collect variables from conservation and equality constraints.
        for cc in &constraints.conservation {
            for var in cc.incoming_vars.iter().chain(cc.outgoing_vars.iter()) {
                ensure_alg(var, &diff_vars, &mut alg_vars);
            }
        }
        for eq in &constraints.effort_equalities {
            ensure_alg(&eq.source_var, &diff_vars, &mut alg_vars);
            ensure_alg(&eq.target_var, &diff_vars, &mut alg_vars);
        }
        for eq in &constraints.flow_equalities {
            ensure_alg(&eq.source_var, &diff_vars, &mut alg_vars);
            ensure_alg(&eq.target_var, &diff_vars, &mut alg_vars);
        }

        // Pass 4: Collect variables from user-written constraint expressions.
        for uc in &constraints.user_constraints {
            for var in &uc.referenced_vars {
                ensure_alg(var, &diff_vars, &mut alg_vars);
            }
        }

        let n_diff = diff_vars.len();
        let n_alg = alg_vars.len();
        let n_total = n_diff + n_alg;

        if n_total == 0 {
            return Err(DaeError::EmptySystem);
        }

        // Build index maps: differential first, then algebraic.
        let mut name_to_idx = HashMap::with_capacity(n_total);
        let mut idx_to_name = Vec::with_capacity(n_total);
        let mut is_differential = Vec::with_capacity(n_total);

        for (i, name) in diff_vars.iter().enumerate() {
            name_to_idx.insert(name.clone(), i);
            idx_to_name.push(name.clone());
            is_differential.push(true);
        }
        for (i, name) in alg_vars.iter().enumerate() {
            let idx = n_diff + i;
            name_to_idx.insert(name.clone(), idx);
            idx_to_name.push(name.clone());
            is_differential.push(false);
        }

        let map = StateVectorMap {
            name_to_idx,
            idx_to_name,
            is_differential,
            n_diff,
            n_alg,
        };

        Ok(Self {
            map,
            relations: constraints.constitutive.clone(),
            conservation: constraints.conservation.clone(),
            effort_eq: constraints.effort_equalities.clone(),
            flow_eq: constraints.flow_equalities.clone(),
            user_constraints: constraints.user_constraints.clone(),
            initial_state: vec![0.0; n_total],
        })
    }

    /// Set initial condition for a named variable.
    pub fn set_initial(&mut self, name: &str, value: f64) {
        if let Some(idx) = self.map.index_of(name) {
            self.initial_state[idx] = value;
        }
    }

    /// Count the total number of equations that the RHS will produce.
    ///
    /// Must equal `map.len()` for the system to be well-determined.
    pub fn n_equations(&self) -> usize {
        let mut n = 0;
        for rel in &self.relations {
            n += match rel {
                ConstitutiveRelation::Resistance { .. } => 1,
                ConstitutiveRelation::Conductance { .. } => 1,
                ConstitutiveRelation::Capacitance { .. } => 1,
                ConstitutiveRelation::Inductance { .. } => 1,
                ConstitutiveRelation::EffortSource { .. } => 1,
                ConstitutiveRelation::FlowSource { .. } => 1,
                ConstitutiveRelation::Transformer { .. } => 2,
                ConstitutiveRelation::Gyrator { .. } => 2,
            };
        }
        n += self.conservation.len();
        n += self.effort_eq.len();
        n += self.flow_eq.len();
        n += self.user_constraints.len();
        n
    }

    /// Evaluate the RHS function `f(t, x)` for `M * dx/dt = f(t, x)`.
    ///
    /// For differential equations (C/I), the RHS entry is the time derivative.
    /// For algebraic constraints (R, G, TF, GY, Se, Sf, junctions, equalities),
    /// the RHS entry is a residual that should be zero.
    ///
    /// The output vector `y` must have length `map.len()`. Entries are written
    /// by equation index (0..n_equations), which must equal map.len().
    pub fn eval_rhs(&self, _t: f64, x: &[f64], y: &mut [f64]) {
        let n = self.map.len();
        debug_assert_eq!(x.len(), n);
        debug_assert_eq!(y.len(), n);

        for v in y.iter_mut() {
            *v = 0.0;
        }

        let idx = |name: &str| -> usize { self.map.index_of(name).unwrap_or(0) };

        // Algebraic equations fill rows n_diff..n_total sequentially.
        let mut alg_row = self.map.n_diff;

        for rel in &self.relations {
            match rel {
                // C-element: d(effort)/dt = flow / C
                // Writes to the differential row for this state variable.
                ConstitutiveRelation::Capacitance {
                    effort_var,
                    flow_var,
                    parameter_value,
                    ..
                } => {
                    let c = parameter_value.unwrap_or(1.0);
                    if c != 0.0 {
                        let effort_idx = idx(effort_var);
                        y[effort_idx] = x[idx(flow_var)] / c;
                    }
                }

                // I-element: d(flow)/dt = effort / L
                ConstitutiveRelation::Inductance {
                    flow_var,
                    effort_var,
                    parameter_value,
                    ..
                } => {
                    let l = parameter_value.unwrap_or(1.0);
                    if l != 0.0 {
                        let flow_idx = idx(flow_var);
                        y[flow_idx] = x[idx(effort_var)] / l;
                    }
                }

                // R-element: 0 = e_in - e_out - R*f
                ConstitutiveRelation::Resistance {
                    effort_in_var,
                    effort_out_var,
                    flow_var,
                    parameter_value,
                    ..
                } => {
                    let r = parameter_value.unwrap_or(1.0);
                    y[alg_row] =
                        x[idx(effort_in_var)] - x[idx(effort_out_var)] - r * x[idx(flow_var)];
                    alg_row += 1;
                }

                // G-element: 0 = f - G*e
                ConstitutiveRelation::Conductance {
                    effort_var,
                    flow_var,
                    parameter_value,
                    ..
                } => {
                    let g = parameter_value.unwrap_or(1.0);
                    y[alg_row] = x[idx(flow_var)] - g * x[idx(effort_var)];
                    alg_row += 1;
                }

                // Se: 0 = e - e_source
                ConstitutiveRelation::EffortSource {
                    effort_var,
                    source_value,
                } => {
                    let val = source_value.unwrap_or(0.0);
                    y[alg_row] = x[idx(effort_var)] - val;
                    alg_row += 1;
                }

                // Sf: 0 = f + f_source
                ConstitutiveRelation::FlowSource {
                    flow_var,
                    source_value,
                } => {
                    let val = source_value.unwrap_or(0.0);
                    y[alg_row] = x[idx(flow_var)] + val;
                    alg_row += 1;
                }

                // TF: 0 = e_out - m*e_in, 0 = f_in + m*f_out
                ConstitutiveRelation::Transformer {
                    effort_in_var,
                    effort_out_var,
                    flow_in_var,
                    flow_out_var,
                    modulus,
                } => {
                    let m = *modulus;
                    y[alg_row] = x[idx(effort_out_var)] - m * x[idx(effort_in_var)];
                    alg_row += 1;
                    y[alg_row] = x[idx(flow_in_var)] + m * x[idx(flow_out_var)];
                    alg_row += 1;
                }

                // GY: 0 = e_out + r*f_in, 0 = e_in - r*f_out
                ConstitutiveRelation::Gyrator {
                    effort_in_var,
                    effort_out_var,
                    flow_in_var,
                    flow_out_var,
                    modulus,
                } => {
                    let r = *modulus;
                    y[alg_row] = x[idx(effort_out_var)] + r * x[idx(flow_in_var)];
                    alg_row += 1;
                    y[alg_row] = x[idx(effort_in_var)] - r * x[idx(flow_out_var)];
                    alg_row += 1;
                }
            }
        }

        // Conservation: 0 = sum(incoming) - sum(outgoing)
        for cc in &self.conservation {
            let incoming: f64 = cc.incoming_vars.iter().map(|v| x[idx(v)]).sum();
            let outgoing: f64 = cc.outgoing_vars.iter().map(|v| x[idx(v)]).sum();
            y[alg_row] = incoming - outgoing;
            alg_row += 1;
        }

        // Effort equalities: 0 = source - target
        for eq in &self.effort_eq {
            y[alg_row] = x[idx(&eq.source_var)] - x[idx(&eq.target_var)];
            alg_row += 1;
        }

        // Flow equalities: 0 = source - target
        for eq in &self.flow_eq {
            y[alg_row] = x[idx(&eq.source_var)] - x[idx(&eq.target_var)];
            alg_row += 1;
        }

        // User-written constraint residuals: evaluate ExprIR with state variables bound.
        if !self.user_constraints.is_empty() {
            use crate::expressions::{EvalContext, ExpressionEvaluator};

            let evaluator = ExpressionEvaluator::new();
            let mut eval_ctx = EvalContext::new();
            eval_ctx.set("t", sysml_core::Value::Float(_t));
            for (name, &i) in &self.map.name_to_idx {
                eval_ctx.set(name.clone(), sysml_core::Value::Float(x[i]));
            }

            for uc in &self.user_constraints {
                let residual = match evaluator.eval(&uc.residual_expr, &eval_ctx) {
                    Ok(sysml_core::Value::Float(f)) => f,
                    Ok(sysml_core::Value::Int(i)) => i as f64,
                    _ => 0.0, // Failed eval → zero residual (will produce diagnostic elsewhere)
                };
                y[alg_row] = residual;
                alg_row += 1;
            }
        }
    }

    /// Extract a linear state-space representation from the bond graph DAE.
    ///
    /// For linear bond graph systems (R/C/I/Se/Sf), the semi-explicit DAE is:
    /// ```text
    ///   dx_d/dt = J_dd * x_d + J_da * x_a + b_d   (differential)
    ///         0 = J_ad * x_d + J_aa * x_a + b_a   (algebraic)
    /// ```
    /// Eliminating x_a via the Schur complement gives the reduced ODE:
    /// ```text
    ///   dx_d/dt = A * x_d + b
    ///   A = J_dd - J_da * J_aa^{-1} * J_ad
    ///   b = b_d - J_da * J_aa^{-1} * b_a
    /// ```
    ///
    /// Returns identity C and zero D matrices (all differential states are outputs).
    pub fn extract_state_space(&self) -> StateSpaceMatrices {
        let n = self.map.len();
        let n_diff = self.map.n_diff;
        let n_alg = self.map.n_alg;

        // Build the full Jacobian J (n × n) column by column via eval_jac_v.
        let zeros = vec![0.0; n];
        let mut j_full = vec![vec![0.0; n]; n]; // j_full[row][col]
        for col in 0..n {
            let mut e = vec![0.0; n];
            e[col] = 1.0;
            let mut result = vec![0.0; n];
            self.eval_jac_v(0.0, &zeros, &e, &mut result);
            for row in 0..n {
                j_full[row][col] = result[row];
            }
        }

        // Extract sub-blocks:
        // J_dd (n_diff × n_diff): rows 0..n_diff, cols 0..n_diff
        // J_da (n_diff × n_alg):  rows 0..n_diff, cols n_diff..n
        // J_ad (n_alg × n_diff):  rows n_diff..n, cols 0..n_diff
        // J_aa (n_alg × n_alg):   rows n_diff..n, cols n_diff..n
        let j_dd: Vec<Vec<f64>> = (0..n_diff).map(|i| j_full[i][..n_diff].to_vec()).collect();
        let j_da: Vec<Vec<f64>> = (0..n_diff).map(|i| j_full[i][n_diff..].to_vec()).collect();
        let j_ad: Vec<Vec<f64>> = (n_diff..n).map(|i| j_full[i][..n_diff].to_vec()).collect();
        let j_aa: Vec<Vec<f64>> = (n_diff..n).map(|i| j_full[i][n_diff..].to_vec()).collect();

        // Evaluate RHS at x=0 for the constant forcing vectors.
        let mut rhs_zero = vec![0.0; n];
        self.eval_rhs(0.0, &zeros, &mut rhs_zero);
        let b_d: Vec<f64> = rhs_zero[..n_diff].to_vec();
        let b_a: Vec<f64> = rhs_zero[n_diff..].to_vec();

        // Compute A = J_dd - J_da * J_aa^{-1} * J_ad via solving J_aa * X = J_ad.
        // Also compute forcing: b = b_d - J_da * J_aa^{-1} * b_a.
        let a;
        let forcing;

        if n_alg == 0 {
            // No algebraic variables: A = J_dd, b = b_d
            a = j_dd;
            forcing = b_d;
        } else {
            // Invert J_aa using Gaussian elimination with partial pivoting.
            let j_aa_inv = invert_dense(n_alg, &j_aa);

            // J_aa_inv * J_ad → n_alg × n_diff
            let inv_j_ad = mat_mul(n_alg, n_diff, n_alg, &j_aa_inv, &j_ad);

            // J_da * (J_aa_inv * J_ad) → n_diff × n_diff
            let correction = mat_mul(n_diff, n_diff, n_alg, &j_da, &inv_j_ad);

            // A = J_dd - correction
            a = (0..n_diff)
                .map(|i| (0..n_diff).map(|j| j_dd[i][j] - correction[i][j]).collect())
                .collect();

            // J_aa_inv * b_a → n_alg vector
            let inv_b_a: Vec<f64> = (0..n_alg)
                .map(|i| (0..n_alg).map(|k| j_aa_inv[i][k] * b_a[k]).sum())
                .collect();

            // b = b_d - J_da * inv_b_a
            forcing = (0..n_diff)
                .map(|i| b_d[i] - (0..n_alg).map(|k| j_da[i][k] * inv_b_a[k]).sum::<f64>())
                .collect();
        }

        // --- Collect source input names ---
        let mut input_names = Vec::new();
        for rel in &self.relations {
            match rel {
                ConstitutiveRelation::EffortSource { effort_var, .. } => {
                    input_names.push(effort_var.clone());
                }
                ConstitutiveRelation::FlowSource { flow_var, .. } => {
                    input_names.push(flow_var.clone());
                }
                _ => {}
            }
        }

        // B: n_diff × 1 (MVP: single combined forcing column)
        let b = (0..n_diff).map(|i| vec![forcing[i]]).collect::<Vec<_>>();

        // C: identity (n_diff × n_diff)
        let c = (0..n_diff)
            .map(|i| {
                let mut row = vec![0.0; n_diff];
                row[i] = 1.0;
                row
            })
            .collect();

        // D: zero (n_diff × 1)
        let d = vec![vec![0.0]; n_diff];

        // Names
        let state_names: Vec<String> = (0..n_diff)
            .map(|i| self.map.idx_to_name[i].clone())
            .collect();
        let output_names = state_names.clone();
        let final_input_names = if input_names.is_empty() {
            vec!["u".to_string()]
        } else {
            vec![input_names.join("+")]
        };

        StateSpaceMatrices {
            a,
            b,
            c,
            d,
            state_names,
            input_names: final_input_names,
            output_names,
        }
    }

    /// Compute the instantaneous energy balance for the bond graph system.
    ///
    /// Walks all constitutive relations to compute stored energy (C/I elements),
    /// instantaneous dissipation power (R elements), and source power (Se/Sf).
    pub fn compute_energy_balance(&self, x: &[f64]) -> EnergyBalance {
        let idx = |name: &str| -> usize { self.map.index_of(name).unwrap_or(0) };

        let mut stored_c = Vec::new();
        let mut stored_i = Vec::new();
        let mut dissipated_r = Vec::new();
        let mut source_power = Vec::new();

        let mut total_stored = 0.0;
        let mut total_dissipated = 0.0;
        let mut total_source = 0.0;

        for rel in &self.relations {
            match rel {
                ConstitutiveRelation::Capacitance {
                    effort_var,
                    parameter_value,
                    ..
                } => {
                    let c_val = parameter_value.unwrap_or(1.0);
                    let e = x[idx(effort_var)];
                    let energy = 0.5 * c_val * e * e;
                    total_stored += energy;
                    stored_c.push((effort_var.clone(), energy));
                }
                ConstitutiveRelation::Inductance {
                    flow_var,
                    parameter_value,
                    ..
                } => {
                    let l_val = parameter_value.unwrap_or(1.0);
                    let f = x[idx(flow_var)];
                    let energy = 0.5 * l_val * f * f;
                    total_stored += energy;
                    stored_i.push((flow_var.clone(), energy));
                }
                ConstitutiveRelation::Resistance {
                    effort_in_var,
                    effort_out_var,
                    flow_var,
                    parameter_value,
                    ..
                } => {
                    let r_val = parameter_value.unwrap_or(1.0);
                    let f = x[idx(flow_var)];
                    // Dissipated power: P = R * f^2 = (e_in - e_out) * f
                    let power = r_val * f * f;
                    total_dissipated += power;
                    dissipated_r.push((flow_var.clone(), power));
                    let _ = (effort_in_var, effort_out_var); // used implicitly via R*f^2
                }
                ConstitutiveRelation::EffortSource {
                    effort_var,
                    source_value,
                } => {
                    // Source power = e_source * f_through_source
                    // We approximate f from the RHS: evaluate to find current
                    let e = source_value.unwrap_or(0.0);
                    // Find flow through this source by looking for an R-element
                    // sharing a variable. Heuristic: sum flows connected to this effort.
                    let mut f_total = 0.0;
                    let e_idx = idx(effort_var);
                    for other in &self.relations {
                        if let ConstitutiveRelation::Resistance {
                            effort_in_var,
                            flow_var,
                            ..
                        } = other
                        {
                            if idx(effort_in_var) == e_idx {
                                f_total += x[idx(flow_var)];
                            }
                        }
                    }
                    let power = e * f_total;
                    total_source += power;
                    source_power.push((effort_var.clone(), power));
                }
                ConstitutiveRelation::FlowSource {
                    flow_var,
                    source_value,
                } => {
                    let f = source_value.unwrap_or(0.0);
                    // Find effort at the flow source port. Heuristic: check R-elements.
                    let mut e_total = 0.0;
                    let f_idx = idx(flow_var);
                    for other in &self.relations {
                        if let ConstitutiveRelation::Resistance {
                            effort_in_var,
                            flow_var: r_flow,
                            ..
                        } = other
                        {
                            if idx(r_flow) == f_idx {
                                e_total += x[idx(effort_in_var)];
                            }
                        }
                    }
                    let power = e_total * f;
                    total_source += power;
                    source_power.push((flow_var.clone(), power));
                }
                ConstitutiveRelation::Conductance { .. }
                | ConstitutiveRelation::Transformer { .. }
                | ConstitutiveRelation::Gyrator { .. } => {
                    // Conductance dissipates but is handled similarly to R
                    // TF/GY are power-conserving: no net energy change
                }
            }
        }

        EnergyBalance {
            stored_c,
            stored_i,
            dissipated_r,
            source_power,
            total_stored,
            total_dissipated,
            total_source,
        }
    }

    /// Evaluate the Jacobian-vector product J(x)*v for the RHS.
    ///
    /// Since all constitutive relations are linear, the Jacobian is constant
    /// (independent of x). This computes J*v directly from coefficients.
    pub fn eval_jac_v(&self, _t: f64, _x: &[f64], v: &[f64], y: &mut [f64]) {
        let n = self.map.len();
        debug_assert_eq!(v.len(), n);
        debug_assert_eq!(y.len(), n);

        for val in y.iter_mut() {
            *val = 0.0;
        }

        let idx = |name: &str| -> usize { self.map.index_of(name).unwrap_or(0) };

        let mut alg_row = self.map.n_diff;

        for rel in &self.relations {
            match rel {
                ConstitutiveRelation::Capacitance {
                    effort_var,
                    flow_var,
                    parameter_value,
                    ..
                } => {
                    let c = parameter_value.unwrap_or(1.0);
                    if c != 0.0 {
                        y[idx(effort_var)] = v[idx(flow_var)] / c;
                    }
                }
                ConstitutiveRelation::Inductance {
                    flow_var,
                    effort_var,
                    parameter_value,
                    ..
                } => {
                    let l = parameter_value.unwrap_or(1.0);
                    if l != 0.0 {
                        y[idx(flow_var)] = v[idx(effort_var)] / l;
                    }
                }
                ConstitutiveRelation::Resistance {
                    effort_in_var,
                    effort_out_var,
                    flow_var,
                    parameter_value,
                    ..
                } => {
                    let r = parameter_value.unwrap_or(1.0);
                    y[alg_row] =
                        v[idx(effort_in_var)] - v[idx(effort_out_var)] - r * v[idx(flow_var)];
                    alg_row += 1;
                }
                ConstitutiveRelation::Conductance {
                    effort_var,
                    flow_var,
                    parameter_value,
                    ..
                } => {
                    let g = parameter_value.unwrap_or(1.0);
                    y[alg_row] = v[idx(flow_var)] - g * v[idx(effort_var)];
                    alg_row += 1;
                }
                ConstitutiveRelation::EffortSource { effort_var, .. } => {
                    y[alg_row] = v[idx(effort_var)];
                    alg_row += 1;
                }
                ConstitutiveRelation::FlowSource { flow_var, .. } => {
                    y[alg_row] = v[idx(flow_var)];
                    alg_row += 1;
                }
                ConstitutiveRelation::Transformer {
                    effort_in_var,
                    effort_out_var,
                    flow_in_var,
                    flow_out_var,
                    modulus,
                } => {
                    let m = *modulus;
                    y[alg_row] = v[idx(effort_out_var)] - m * v[idx(effort_in_var)];
                    alg_row += 1;
                    y[alg_row] = v[idx(flow_in_var)] + m * v[idx(flow_out_var)];
                    alg_row += 1;
                }
                ConstitutiveRelation::Gyrator {
                    effort_in_var,
                    effort_out_var,
                    flow_in_var,
                    flow_out_var,
                    modulus,
                } => {
                    let r = *modulus;
                    y[alg_row] = v[idx(effort_out_var)] + r * v[idx(flow_in_var)];
                    alg_row += 1;
                    y[alg_row] = v[idx(effort_in_var)] - r * v[idx(flow_out_var)];
                    alg_row += 1;
                }
            }
        }

        for cc in &self.conservation {
            let incoming: f64 = cc.incoming_vars.iter().map(|name| v[idx(name)]).sum();
            let outgoing: f64 = cc.outgoing_vars.iter().map(|name| v[idx(name)]).sum();
            y[alg_row] = incoming - outgoing;
            alg_row += 1;
        }

        for eq in &self.effort_eq {
            y[alg_row] = v[idx(&eq.source_var)] - v[idx(&eq.target_var)];
            alg_row += 1;
        }

        for eq in &self.flow_eq {
            y[alg_row] = v[idx(&eq.source_var)] - v[idx(&eq.target_var)];
            alg_row += 1;
        }

        // User-written constraint Jacobian rows via numerical differentiation.
        // For each user constraint, compute J_row * v by finite differences:
        //   J_row * v ≈ (f(x + h*v) - f(x)) / h
        // where f is the residual function.
        if !self.user_constraints.is_empty() {
            use crate::expressions::{EvalContext, ExpressionEvaluator};

            let evaluator = ExpressionEvaluator::new();
            let h = 1e-7_f64;

            for uc in &self.user_constraints {
                // Evaluate residual at x
                let mut ctx_base = EvalContext::new();
                for (name, &i) in &self.map.name_to_idx {
                    ctx_base.set(name.clone(), sysml_core::Value::Float(_x[i]));
                }
                let f0 = match evaluator.eval(&uc.residual_expr, &ctx_base) {
                    Ok(sysml_core::Value::Float(f)) => f,
                    Ok(sysml_core::Value::Int(i)) => i as f64,
                    _ => 0.0,
                };

                // Evaluate residual at x + h*v
                let mut ctx_pert = EvalContext::new();
                for (name, &i) in &self.map.name_to_idx {
                    ctx_pert.set(name.clone(), sysml_core::Value::Float(_x[i] + h * v[i]));
                }
                let f1 = match evaluator.eval(&uc.residual_expr, &ctx_pert) {
                    Ok(sysml_core::Value::Float(f)) => f,
                    Ok(sysml_core::Value::Int(i)) => i as f64,
                    _ => 0.0,
                };

                y[alg_row] = (f1 - f0) / h;
                alg_row += 1;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// diffsol integration
// ---------------------------------------------------------------------------

impl BondGraphDae {
    /// Solve the DAE system using diffsol's BDF solver.
    ///
    /// Returns time points and state trajectories for all variables.
    pub fn solve(&self, t_span: (f64, f64), rtol: f64, atol: f64) -> Result<DaeSolution, DaeError> {
        let n = self.map.len();
        if n == 0 {
            return Err(DaeError::EmptySystem);
        }

        // Clone data for move into closures.
        let initial = self.initial_state.clone();
        let n_diff = self.map.n_diff;

        // We need to clone self for the closures since they need 'static lifetime.
        let dae_rhs = self.clone();
        let dae_jac = self.clone();

        // Capture n for closures (avoids needing Vector trait in scope for .len())
        let n_vars = n;

        let problem = OdeBuilder::<NalgebraMat<f64>>::new()
            .t0(t_span.0)
            .h0(1e-6)
            .rtol(rtol)
            .atol([atol])
            .p(std::iter::empty::<f64>())
            .rhs_implicit(
                // RHS: f(x, p, t, y)
                move |x, _p, t, y| {
                    let mut x_slice = vec![0.0; n_vars];
                    let mut y_slice = vec![0.0; n_vars];
                    for i in 0..n_vars {
                        x_slice[i] = x[i];
                    }
                    dae_rhs.eval_rhs(t, &x_slice, &mut y_slice);
                    for i in 0..n_vars {
                        y[i] = y_slice[i];
                    }
                },
                // Jacobian-vector product: J(x)*v
                {
                    let n_jac = n;
                    move |x, _p, t, v, y| {
                        let mut x_slice = vec![0.0; n_jac];
                        let mut v_slice = vec![0.0; n_jac];
                        let mut y_slice = vec![0.0; n_jac];
                        for i in 0..n_jac {
                            x_slice[i] = x[i];
                            v_slice[i] = v[i];
                        }
                        dae_jac.eval_jac_v(t, &x_slice, &v_slice, &mut y_slice);
                        for i in 0..n_jac {
                            y[i] = y_slice[i];
                        }
                    }
                },
            )
            .mass(
                // Mass matrix: y = M*v + beta*y
                // M is diagonal: 1 for differential (0..n_diff), 0 for algebraic
                {
                    let n_mass = n;
                    move |v, _p, _t, beta, y| {
                        for i in 0..n_mass {
                            let m_ii = if i < n_diff { 1.0 } else { 0.0 };
                            y[i] = m_ii * v[i] + beta * y[i];
                        }
                    }
                },
            )
            .init(
                // Initial conditions
                {
                    let init = initial.clone();
                    let n_init = n;
                    move |_p, _t, y| {
                        for i in 0..n_init {
                            y[i] = init[i];
                        }
                    }
                },
                n,
            )
            .build()
            .map_err(DaeError::Solver)?;

        let mut solver = problem.bdf::<NalgebraLU<f64>>().map_err(DaeError::Solver)?;

        let (sol_matrix, times, _stop) = solver.solve(t_span.1).map_err(DaeError::Solver)?;

        // Convert solution matrix (n_vars x n_times) to Vec<Vec<f64>> (per-variable trajectories)
        let n_times = times.len();
        let mut x_trajectories = vec![Vec::with_capacity(n_times); n];
        for t_idx in 0..n_times {
            for var_idx in 0..n {
                x_trajectories[var_idx].push(sol_matrix[(var_idx, t_idx)]);
            }
        }

        let t_vec: Vec<f64> = times;

        Ok(DaeSolution {
            t: t_vec,
            x: x_trajectories,
            var_names: self.map.idx_to_name.clone(),
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::physics::constraints::*;

    /// Helper: build a simple RC circuit constraint set.
    /// Se(10V) → R(5Ω) → C(1F), ground at 0V.
    ///
    /// Variables: source.voltage, r1.vin, r1.vout, r1.current, cap.voltage, cap.current
    /// Differential: cap.voltage (C-element state)
    /// Algebraic: everything else
    fn rc_circuit_constraints() -> GeneratedConstraints {
        let mut gc = GeneratedConstraints::default();

        // Source: 10V
        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "source.voltage".to_string(),
            source_value: Some(10.0),
        });

        // Resistance: 5 ohm
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "source.voltage".to_string(),
            effort_out_var: "cap.voltage".to_string(),
            flow_var: "rc.current".to_string(),
            parameter_var: "r1.resistance".to_string(),
            parameter_value: Some(5.0),
        });

        // Capacitor: 1 Farad
        gc.constitutive.push(ConstitutiveRelation::Capacitance {
            effort_var: "cap.voltage".to_string(),
            flow_var: "rc.current".to_string(),
            parameter_var: "cap.capacitance".to_string(),
            parameter_value: Some(1.0),
        });

        gc
    }

    #[test]
    fn state_vector_map_from_rc_circuit() {
        let gc = rc_circuit_constraints();
        let dae = BondGraphDae::from_constraints(&gc).unwrap();

        // cap.voltage should be differential (C-element state)
        assert_eq!(
            dae.map.n_diff, 1,
            "1 differential state (capacitor voltage)"
        );
        assert!(dae.map.n_alg >= 2, "at least 2 algebraic vars");

        let cap_idx = dae.map.index_of("cap.voltage").unwrap();
        assert!(
            dae.map.is_differential(cap_idx),
            "cap.voltage is differential"
        );

        let src_idx = dae.map.index_of("source.voltage").unwrap();
        assert!(
            !dae.map.is_differential(src_idx),
            "source.voltage is algebraic"
        );

        let cur_idx = dae.map.index_of("rc.current").unwrap();
        assert!(!dae.map.is_differential(cur_idx), "rc.current is algebraic");

        // Differential vars come first
        assert_eq!(cap_idx, 0, "differential vars at start of vector");
        assert!(src_idx >= 1, "algebraic vars after differential");
    }

    #[test]
    fn mass_matrix_diagonal() {
        let gc = rc_circuit_constraints();
        let dae = BondGraphDae::from_constraints(&gc).unwrap();

        // Check mass matrix structure: 1 for differential, 0 for algebraic
        for i in 0..dae.map.len() {
            let expected = if i < dae.map.n_diff { true } else { false };
            assert_eq!(
                dae.map.is_differential(i),
                expected,
                "index {} should be {}",
                i,
                if expected {
                    "differential"
                } else {
                    "algebraic"
                }
            );
        }
    }

    #[test]
    fn rhs_r_element_residual_zero_when_consistent() {
        // V_in=10, V_out=0, R=5, I=2 → residual = 10 - 0 - 5*2 = 0
        let mut gc = GeneratedConstraints::default();
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "r.vin".to_string(),
            effort_out_var: "r.vout".to_string(),
            flow_var: "r.current".to_string(),
            parameter_var: "r.R".to_string(),
            parameter_value: Some(5.0),
        });

        let dae = BondGraphDae::from_constraints(&gc).unwrap();
        let mut x = vec![0.0; dae.map.len()];
        x[dae.map.index_of("r.vin").unwrap()] = 10.0;
        x[dae.map.index_of("r.vout").unwrap()] = 0.0;
        x[dae.map.index_of("r.current").unwrap()] = 2.0;

        let mut y = vec![0.0; dae.map.len()];
        dae.eval_rhs(0.0, &x, &mut y);

        // All variables are algebraic, so the R residual should be in the algebraic portion
        // residual = e_in - e_out - R*f = 10 - 0 - 5*2 = 0
        let residual: f64 = y.iter().map(|v| v.abs()).sum();
        assert!(
            residual < 1e-10,
            "consistent state should have zero residual, got {:?}",
            y
        );
    }

    #[test]
    fn rhs_c_element_derivative() {
        // C=1F, flow=2A → d(effort)/dt = flow/C = 2.0
        let mut gc = GeneratedConstraints::default();
        gc.constitutive.push(ConstitutiveRelation::Capacitance {
            effort_var: "cap.v".to_string(),
            flow_var: "cap.i".to_string(),
            parameter_var: "cap.C".to_string(),
            parameter_value: Some(1.0),
        });

        let dae = BondGraphDae::from_constraints(&gc).unwrap();
        assert_eq!(dae.map.n_diff, 1);

        let mut x = vec![0.0; dae.map.len()];
        x[dae.map.index_of("cap.v").unwrap()] = 5.0; // current voltage (doesn't affect derivative)
        x[dae.map.index_of("cap.i").unwrap()] = 2.0; // current through capacitor

        let mut y = vec![0.0; dae.map.len()];
        dae.eval_rhs(0.0, &x, &mut y);

        // Differential equation: y[cap.v_idx] = flow/C = 2.0/1.0 = 2.0
        let cap_v_idx = dae.map.index_of("cap.v").unwrap();
        assert!(
            (y[cap_v_idx] - 2.0).abs() < 1e-10,
            "d(voltage)/dt should be 2.0 (I/C), got {}",
            y[cap_v_idx]
        );
    }

    #[test]
    fn n_equations_matches_vars_rc() {
        let gc = rc_circuit_constraints();
        let dae = BondGraphDae::from_constraints(&gc).unwrap();

        // For RC: Se(1 eq) + R(1 eq) + C(1 eq) = 3 equations
        // Variables: cap.voltage(diff), source.voltage(alg), rc.current(alg) = 3 vars
        assert_eq!(
            dae.n_equations(),
            dae.map.len(),
            "system must be square: {} equations vs {} variables",
            dae.n_equations(),
            dae.map.len()
        );
    }

    #[test]
    fn jac_v_matches_rhs_for_linear_system() {
        // For a linear system, J*v should equal f(v) - f(0)
        let gc = rc_circuit_constraints();
        let dae = BondGraphDae::from_constraints(&gc).unwrap();

        let n = dae.map.len();
        let x = vec![1.0; n]; // arbitrary state
        let v = vec![0.1; n]; // arbitrary direction

        let mut jv = vec![0.0; n];
        dae.eval_jac_v(0.0, &x, &v, &mut jv);

        // For linear f(x) = Ax + b: J*v = A*v = f(v) - f(0) + correction for constants
        // Actually for f(x) = Ax + b, the Jacobian is A, so J*v = A*v.
        // And f(v) = A*v + b, f(0) = A*0 + b = b.
        // So J*v = f(v) - b = f(v) - f(0).
        let mut f_v = vec![0.0; n];
        dae.eval_rhs(0.0, &v, &mut f_v);
        let mut f_0 = vec![0.0; n];
        let zero = vec![0.0; n];
        dae.eval_rhs(0.0, &zero, &mut f_0);

        for i in 0..n {
            let expected = f_v[i] - f_0[i];
            assert!(
                (jv[i] - expected).abs() < 1e-10,
                "Jv[{}] = {} but f(v)-f(0) = {} (diff={})",
                i,
                jv[i],
                expected,
                jv[i] - expected
            );
        }
    }

    // ── diffsol integration tests ──

    #[test]
    fn solve_rc_charging() {
        // RC circuit: V_s=10V, R=5Ω, C=1F
        // Analytical: V_c(t) = V_s * (1 - e^(-t/RC))
        // τ = RC = 5*1 = 5s
        // At t=τ=5: V_c = 10 * (1 - e^-1) ≈ 6.321
        let gc = rc_circuit_constraints();
        let dae = BondGraphDae::from_constraints(&gc).unwrap();

        let solution = dae.solve((0.0, 5.0), 1e-6, 1e-8).unwrap();

        // Find capacitor voltage trajectory
        let cap_v_idx = solution
            .var_names
            .iter()
            .position(|n| n == "cap.voltage")
            .unwrap();
        let final_v = *solution.x[cap_v_idx].last().unwrap();

        let expected = 10.0 * (1.0 - (-1.0_f64).exp()); // 6.3212...
        assert!(
            (final_v - expected).abs() < 0.01,
            "V_c(τ) should be {:.4}, got {:.4}",
            expected,
            final_v
        );
    }

    // ── State-space extraction tests ──

    #[test]
    fn state_space_rc_circuit() {
        // RC: Se(10V), R(5Ω), C(1F)
        // Reduced ODE: dV_c/dt = (V_s - V_c) / (R*C) = -V_c/(RC) + V_s/(RC)
        // A = [[-1/(RC)]] = [[-0.2]]
        let gc = rc_circuit_constraints();
        let dae = BondGraphDae::from_constraints(&gc).unwrap();
        let ss = dae.extract_state_space();

        assert_eq!(
            ss.state_names.len(),
            1,
            "one differential state (cap.voltage)"
        );
        assert_eq!(ss.a.len(), 1);
        assert_eq!(ss.a[0].len(), 1);

        let eigenvalue = ss.a[0][0];
        let expected = -1.0 / (5.0 * 1.0); // -1/(RC) = -0.2
        assert!(
            (eigenvalue - expected).abs() < 1e-10,
            "A[0][0] should be {}, got {}",
            expected,
            eigenvalue
        );

        // C matrix should be identity
        assert_eq!(ss.c.len(), 1);
        assert_eq!(ss.c[0][0], 1.0);

        // D matrix should be zero
        assert_eq!(ss.d.len(), 1);
        assert!((ss.d[0][0]).abs() < 1e-10);
    }

    #[test]
    fn state_space_rl_circuit() {
        // RL: Se(10V), R(5Ω), L(2H)
        // Reduced ODE: dI/dt = (V_s - R*I) / L = -R/L * I + V_s/L
        // A = [[-R/L]] = [[-2.5]]
        let mut gc = GeneratedConstraints::default();

        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "source.voltage".to_string(),
            source_value: Some(10.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "source.voltage".to_string(),
            effort_out_var: "r.vout".to_string(),
            flow_var: "rl.current".to_string(),
            parameter_var: "r.R".to_string(),
            parameter_value: Some(5.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Inductance {
            flow_var: "rl.current".to_string(),
            effort_var: "r.vout".to_string(),
            parameter_var: "l.L".to_string(),
            parameter_value: Some(2.0),
        });

        let dae = BondGraphDae::from_constraints(&gc).unwrap();
        let ss = dae.extract_state_space();

        assert_eq!(
            ss.state_names.len(),
            1,
            "one differential state (rl.current)"
        );
        assert_eq!(ss.a.len(), 1);
        assert_eq!(ss.a[0].len(), 1);

        let eigenvalue = ss.a[0][0];
        let expected = -5.0 / 2.0; // -R/L = -2.5
        assert!(
            (eigenvalue - expected).abs() < 1e-10,
            "A[0][0] should be {}, got {}",
            expected,
            eigenvalue
        );
    }

    // ── Energy bookkeeping tests ──

    #[test]
    fn energy_balance_rc_at_steady_state() {
        // RC circuit at steady state: cap.voltage = V_s = 10V, current = 0
        // Stored energy = 0.5 * C * V^2 = 0.5 * 1.0 * 100.0 = 50.0 J
        let gc = rc_circuit_constraints();
        let dae = BondGraphDae::from_constraints(&gc).unwrap();

        let mut x = vec![0.0; dae.map.len()];
        x[dae.map.index_of("cap.voltage").unwrap()] = 10.0; // fully charged
        x[dae.map.index_of("source.voltage").unwrap()] = 10.0;
        x[dae.map.index_of("rc.current").unwrap()] = 0.0; // steady state

        let eb = dae.compute_energy_balance(&x);

        assert_eq!(eb.stored_c.len(), 1, "one capacitor");
        assert!(
            (eb.stored_c[0].1 - 50.0).abs() < 1e-10,
            "stored energy should be 50J, got {}",
            eb.stored_c[0].1
        );
        assert!(
            (eb.total_stored - 50.0).abs() < 1e-10,
            "total stored should be 50J, got {}",
            eb.total_stored
        );

        // At steady state, no current flows → dissipation = 0
        assert!(
            eb.total_dissipated.abs() < 1e-10,
            "no dissipation at steady state, got {}",
            eb.total_dissipated
        );
    }

    #[test]
    fn energy_balance_r_only_dissipation() {
        // Pure R circuit: Se(10V), R(5Ω) with current flowing
        // No storage elements → stored = 0
        // Dissipated power = R * I^2 = 5 * 4 = 20W
        let mut gc = GeneratedConstraints::default();
        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "source.voltage".to_string(),
            source_value: Some(10.0),
        });
        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "source.voltage".to_string(),
            effort_out_var: "r.vout".to_string(),
            flow_var: "r.current".to_string(),
            parameter_var: "r.R".to_string(),
            parameter_value: Some(5.0),
        });

        let dae = BondGraphDae::from_constraints(&gc).unwrap();
        let mut x = vec![0.0; dae.map.len()];
        x[dae.map.index_of("source.voltage").unwrap()] = 10.0;
        x[dae.map.index_of("r.vout").unwrap()] = 0.0;
        x[dae.map.index_of("r.current").unwrap()] = 2.0; // I = V/R = 10/5 = 2A

        let eb = dae.compute_energy_balance(&x);

        assert!(
            eb.total_stored.abs() < 1e-10,
            "no storage elements, stored should be 0, got {}",
            eb.total_stored
        );
        assert!(
            (eb.total_dissipated - 20.0).abs() < 1e-10,
            "dissipated should be R*I^2 = 20W, got {}",
            eb.total_dissipated
        );
        assert!(eb.stored_c.is_empty(), "no capacitors");
        assert!(eb.stored_i.is_empty(), "no inductors");
        assert_eq!(eb.dissipated_r.len(), 1, "one resistor");
    }

    #[test]
    fn solve_rl_current_ramp() {
        // RL circuit: V_s=10V, R=5Ω, L=2H
        // Analytical: I(t) = (V/R) * (1 - e^(-Rt/L))
        // τ = L/R = 2/5 = 0.4s
        // At t=τ=0.4: I = 2 * (1 - e^-1) ≈ 1.2642
        let mut gc = GeneratedConstraints::default();

        gc.constitutive.push(ConstitutiveRelation::EffortSource {
            effort_var: "source.voltage".to_string(),
            source_value: Some(10.0),
        });

        gc.constitutive.push(ConstitutiveRelation::Resistance {
            effort_in_var: "source.voltage".to_string(),
            effort_out_var: "r.vout".to_string(),
            flow_var: "rl.current".to_string(),
            parameter_var: "r.R".to_string(),
            parameter_value: Some(5.0),
        });

        gc.constitutive.push(ConstitutiveRelation::Inductance {
            flow_var: "rl.current".to_string(),
            effort_var: "r.vout".to_string(),
            parameter_var: "l.L".to_string(),
            parameter_value: Some(2.0),
        });

        let dae = BondGraphDae::from_constraints(&gc).unwrap();

        let tau = 2.0 / 5.0; // L/R
        let solution = dae.solve((0.0, tau), 1e-6, 1e-8).unwrap();

        let i_idx = solution
            .var_names
            .iter()
            .position(|n| n == "rl.current")
            .unwrap();
        let final_i = *solution.x[i_idx].last().unwrap();

        let expected = (10.0 / 5.0) * (1.0 - (-1.0_f64).exp()); // 1.2642...
        assert!(
            (final_i - expected).abs() < 0.01,
            "I(τ) should be {:.4}, got {:.4}",
            expected,
            final_i
        );
    }

    /// RC circuit using only UserConstraintExpression (no auto-detected constitutive relations).
    ///
    /// Circuit: V_s = 1V source, R = 1Ω, C = 1F
    /// Constraints:
    ///   v_s = 1.0                    (source voltage)
    ///   i = (v_s - v_c) / R          →  rewritten as: i * R == v_s - v_c
    ///   v_c has ODE: dv_c/dt = i/C   →  modeled as capacitance constitutive relation
    ///
    /// But since we want to test purely user constraints, we use:
    ///   v_s == 1.0                   (source)
    ///   i == (v_s - v_c) / R         →  i - (v_s - v_c) / R == 0
    ///
    /// Plus a Capacitance constitutive for the differential equation dv_c/dt = i/C
    ///
    /// Expected: v_c(t) = 1 - e^(-t/RC) = 1 - e^(-t)
    /// At t=1: v_c ≈ 0.6321
    #[test]
    fn test_rc_circuit_with_user_constraints() {
        use crate::expressions::{compile_simple_expression, BinOp, ExprIR};

        // Build user constraints:
        // Constraint 1: v_s == 1.0  →  residual: v_s - 1.0
        let vs_lhs = compile_simple_expression("v_s").unwrap();
        let vs_rhs = compile_simple_expression("1.0").unwrap();
        let vs_residual = ExprIR::BinaryOp {
            op: BinOp::Subtract,
            left: Box::new(vs_lhs.clone()),
            right: Box::new(vs_rhs.clone()),
        };
        let mut vs_vars = vs_lhs.free_variables();
        vs_vars.extend(vs_rhs.free_variables());
        let uc_source = UserConstraintExpression {
            source: "v_s == 1.0".to_string(),
            residual_expr: vs_residual,
            referenced_vars: vs_vars.into_iter().collect(),
            owner_name: Some("VoltageSource".to_string()),
        };

        // Constraint 2: i == (v_s - v_c) / R  →  residual: i - (v_s - v_c) / R
        // With R=1: i - (v_s - v_c)
        let i_lhs = compile_simple_expression("i").unwrap();
        let i_rhs = compile_simple_expression("v_s - v_c").unwrap();
        let i_residual = ExprIR::BinaryOp {
            op: BinOp::Subtract,
            left: Box::new(i_lhs.clone()),
            right: Box::new(i_rhs.clone()),
        };
        let mut i_vars = i_lhs.free_variables();
        i_vars.extend(i_rhs.free_variables());
        let uc_ohms = UserConstraintExpression {
            source: "i == v_s - v_c".to_string(),
            residual_expr: i_residual,
            referenced_vars: i_vars.into_iter().collect(),
            owner_name: Some("OhmsLaw".to_string()),
        };

        // Use a Capacitance constitutive relation for the differential equation
        // dv_c/dt = i / C  (C=1)
        let gc = GeneratedConstraints {
            constitutive: vec![ConstitutiveRelation::Capacitance {
                effort_var: "v_c".to_string(),
                flow_var: "i".to_string(),
                parameter_var: "C".to_string(),
                parameter_value: Some(1.0),
            }],
            user_constraints: vec![uc_source, uc_ohms],
            ..Default::default()
        };

        let dae = BondGraphDae::from_constraints(&gc).unwrap();

        // v_c is differential (from Capacitance), i and v_s are algebraic
        assert_eq!(dae.map.n_diff, 1, "one differential var: v_c");
        assert!(dae.map.index_of("v_c").is_some());
        assert!(dae.map.index_of("i").is_some());
        assert!(dae.map.index_of("v_s").is_some());

        // Solve for t ∈ [0, 1]
        let solution = dae.solve((0.0, 1.0), 1e-6, 1e-8).unwrap();

        let vc_idx = solution.var_names.iter().position(|n| n == "v_c").unwrap();
        let final_vc = *solution.x[vc_idx].last().unwrap();

        let expected = 1.0 - (-1.0_f64).exp(); // ≈ 0.6321
        assert!(
            (final_vc - expected).abs() < 0.02,
            "v_c(1) should be {:.4}, got {:.4}",
            expected,
            final_vc
        );
    }
}
