//! # Variable-order Backward Differentiation Formula (BDF) solver
//!
//! Implicit multi-step integrator for **stiff** ODE systems (R7.3).
//!
//! BDF methods approximate `dy/dt = f(t, y)` by fitting a polynomial through
//! the last `k` state values + the new (unknown) value, then requiring the
//! polynomial's derivative to match `f` at the new point. The resulting
//! implicit equation is solved by Newton iteration using a dense Jacobian
//! obtained by finite differences.
//!
//! This implementation covers BDF1 (implicit Euler) through BDF5. The order
//! is ramped up as the integration progresses (you need `k` past points to
//! run a `k`-th order BDF). Step size is chosen by a PI controller following
//! Hairer & Wanner, *Solving Ordinary Differential Equations II*.
//!
//! ## When to use
//!
//! - **BDF** (this module) — implicit, A-stable for BDF1/BDF2, L-stable BDF1.
//!   Works on **stiff** systems where explicit methods (RK4/RK45) either
//!   fail or require a prohibitively small step size.
//! - **RK4** / **RK45** — explicit, much cheaper per step but restricted by
//!   the stability region. Use for non-stiff systems.
//!
//! The [`detect_stiffness`] helper gives callers a cheap stiffness heuristic
//! based on step-rejection rate.

#![allow(clippy::indexing_slicing)]

use std::sync::Arc;

use crate::expressions::EvalContext;
use crate::ode::{OdeRhs, OdeState};
use sysml_core::Value;

// ---------------------------------------------------------------------------
// Dense linear algebra (hand-rolled, pure Rust)
// ---------------------------------------------------------------------------
//
// BDF Newton iterations solve  `G(y) = y - sum(alpha_i * y_{n-i}) - h*beta*f(t,y) = 0`,
// linearised as `(I - h*beta*J) * dy = -G(y)`. For moderate state dimensions
// (< 100) a dense row-major matrix with partial-pivoting LU is ample — keeps
// the crate dependency-free.

/// Row-major dense matrix with `n` rows and `n` columns.
#[derive(Debug, Clone)]
struct DenseMatrix {
    n: usize,
    data: Vec<f64>,
}

impl DenseMatrix {
    fn zeros(n: usize) -> Self {
        Self {
            n,
            data: vec![0.0; n * n],
        }
    }

    fn identity(n: usize) -> Self {
        let mut m = Self::zeros(n);
        for i in 0..n {
            m.data[i * n + i] = 1.0;
        }
        m
    }

    #[inline]
    fn set(&mut self, i: usize, j: usize, v: f64) {
        self.data[i * self.n + j] = v;
    }
}

/// LU decomposition with partial pivoting. Stores L/U in-place (L is unit-
/// diagonal, not stored). `piv[i]` is the row swapped into position `i`.
#[derive(Debug, Clone)]
struct LuFactor {
    n: usize,
    lu: Vec<f64>,
    piv: Vec<usize>,
    /// `+1.0` or `-1.0` depending on permutation sign (not used today but
    /// keeps open the door to a determinant-based stiffness test).
    #[allow(dead_code)]
    sign: f64,
    /// `true` if the factorisation ran into a (near-)zero pivot and was
    /// patched. Callers treat this as a Jacobian / step-size problem and
    /// respond by re-computing the Jacobian or shrinking `h`.
    singular: bool,
}

impl LuFactor {
    /// Decompose `m` in-place. Partial pivoting; rejects singular matrices.
    fn decompose(mut m: DenseMatrix) -> Self {
        let n = m.n;
        let mut piv: Vec<usize> = (0..n).collect();
        let mut sign = 1.0_f64;
        let mut singular = false;

        for k in 0..n {
            // Find pivot: largest |m[i,k]| for i >= k
            let mut max_val = 0.0_f64;
            let mut max_row = k;
            for i in k..n {
                let v = m.data[i * n + k].abs();
                if v > max_val {
                    max_val = v;
                    max_row = i;
                }
            }

            if max_val < 1e-14 {
                singular = true;
                // Patch with a tiny diagonal to make the backsolve finite.
                m.data[k * n + k] = 1e-14;
            } else if max_row != k {
                // Swap rows k and max_row
                for j in 0..n {
                    m.data.swap(k * n + j, max_row * n + j);
                }
                piv.swap(k, max_row);
                sign = -sign;
            }

            let pivot = m.data[k * n + k];
            for i in (k + 1)..n {
                let factor = m.data[i * n + k] / pivot;
                m.data[i * n + k] = factor;
                for j in (k + 1)..n {
                    let sub = factor * m.data[k * n + j];
                    m.data[i * n + j] -= sub;
                }
            }
        }

        Self {
            n,
            lu: m.data,
            piv,
            sign,
            singular,
        }
    }

    /// Solve `A * x = b` using the LU factorisation. `b` is consumed and
    /// replaced by the solution.
    fn solve(&self, mut b: Vec<f64>) -> Vec<f64> {
        let n = self.n;
        // Apply row permutation: b'[i] = b[piv[i]]
        let mut rhs = vec![0.0; n];
        for i in 0..n {
            rhs[i] = b[self.piv[i]];
        }
        b = rhs;

        // Forward substitution (L has unit diagonal)
        for i in 0..n {
            let mut sum = b[i];
            for j in 0..i {
                sum -= self.lu[i * n + j] * b[j];
            }
            b[i] = sum;
        }

        // Backward substitution (U)
        for i in (0..n).rev() {
            let mut sum = b[i];
            for j in (i + 1)..n {
                sum -= self.lu[i * n + j] * b[j];
            }
            b[i] = sum / self.lu[i * n + i];
        }

        b
    }
}

// ---------------------------------------------------------------------------
// BDF coefficients
// ---------------------------------------------------------------------------
//
// BDF-k on a uniform grid reads:
//     sum_{i=0..k} alpha_i * y_{n+1-i} = h * beta * f(t_{n+1}, y_{n+1})
//
// With the conventional normalisation `alpha_0 = 1`, the leading coefficient
// on y_{n+1} is `1`. The canonical fixed-step tables (k=1..5) are:
//
//     k=1:  y_{n+1} - y_n                      = h * f_{n+1}          (beta = 1)
//     k=2:  y_{n+1} - 4/3 y_n + 1/3 y_{n-1}    = 2/3  * h * f_{n+1}
//     k=3:  y_{n+1} - 18/11 y_n + 9/11 y_{n-1} - 2/11 y_{n-2}
//                                               = 6/11 * h * f_{n+1}
//     k=4:  y_{n+1} - 48/25 y_n + 36/25 y_{n-1} - 16/25 y_{n-2} + 3/25 y_{n-3}
//                                               = 12/25 * h * f_{n+1}
//     k=5:  y_{n+1} - 300/137 y_n + 300/137 y_{n-1} - 200/137 y_{n-2}
//                       + 75/137 y_{n-3} - 12/137 y_{n-4}
//                                               = 60/137 * h * f_{n+1}
//
// These are the coefficients of **past** states (alpha_i for i >= 1) and the
// RHS scale `beta`. `alpha` stores them negated so the prediction step is
// simply `y_pred = sum(alpha[i] * y_history[i])`.

/// Coefficients for a BDF-k step on a uniform grid.
struct BdfCoeffs {
    /// Coefficients of the *past* history points, most-recent first
    /// (length `k`). The BDF equation reads
    ///     y_{n+1} = sum_i alpha_i * y_{n+1-i-1} + h * beta * f_{n+1}.
    alpha: &'static [f64],
    /// RHS scale: `y_{n+1} - alpha . y_hist = h * beta * f_{n+1}`.
    beta: f64,
    /// Coefficients of the local-error *reference* polynomial — the
    /// degree-`k` polynomial interpolating the last `k+1` points. Most
    /// recent first (length `k+1`). For uniform step size these are the
    /// binomial coefficients `(-1)^i * C(k+1, i+1)` starting with `k+1`:
    /// the `(k+1)`-th backward difference `Δ^(k+1) y_{n+1}` evaluates as
    /// `y_{n+1} - sum_i pred[i] * y_{n-i}` and is ≈ `h^(k+1) * y^(k+1)`.
    pred: &'static [f64],
    /// Error estimate constant: the LTE of BDF-k is
    /// `LTE ≈ -(1/(k+1)) * h^(k+1) * y^(k+1)`, so we take
    /// `err_const = 1/(k+1)` and estimate `|LTE| ≈ err_const * |Δ^(k+1) y|`.
    err_const: f64,
}

const BDF1: BdfCoeffs = BdfCoeffs {
    alpha: &[1.0],
    beta: 1.0,
    // Δ^2 y_{n+1} = y_{n+1} - 2 y_n + y_{n-1}
    pred: &[2.0, -1.0],
    err_const: 1.0 / 2.0,
};
const BDF2: BdfCoeffs = BdfCoeffs {
    alpha: &[4.0 / 3.0, -1.0 / 3.0],
    beta: 2.0 / 3.0,
    // Δ^3 y_{n+1} = y_{n+1} - 3 y_n + 3 y_{n-1} - y_{n-2}
    pred: &[3.0, -3.0, 1.0],
    err_const: 1.0 / 3.0,
};
const BDF3: BdfCoeffs = BdfCoeffs {
    alpha: &[18.0 / 11.0, -9.0 / 11.0, 2.0 / 11.0],
    beta: 6.0 / 11.0,
    // Δ^4
    pred: &[4.0, -6.0, 4.0, -1.0],
    err_const: 1.0 / 4.0,
};
const BDF4: BdfCoeffs = BdfCoeffs {
    alpha: &[48.0 / 25.0, -36.0 / 25.0, 16.0 / 25.0, -3.0 / 25.0],
    beta: 12.0 / 25.0,
    // Δ^5
    pred: &[5.0, -10.0, 10.0, -5.0, 1.0],
    err_const: 1.0 / 5.0,
};
const BDF5: BdfCoeffs = BdfCoeffs {
    alpha: &[
        300.0 / 137.0,
        -300.0 / 137.0,
        200.0 / 137.0,
        -75.0 / 137.0,
        12.0 / 137.0,
    ],
    beta: 60.0 / 137.0,
    // Δ^6
    pred: &[6.0, -15.0, 20.0, -15.0, 6.0, -1.0],
    err_const: 1.0 / 6.0,
};

/// Return the coefficients for BDF of order `k` (1..=5).
fn coeffs(k: usize) -> &'static BdfCoeffs {
    match k {
        1 => &BDF1,
        2 => &BDF2,
        3 => &BDF3,
        4 => &BDF4,
        5 => &BDF5,
        _ => &BDF1,
    }
}

// ---------------------------------------------------------------------------
// Newton defaults + PI controller tuning
// ---------------------------------------------------------------------------

/// Default Newton convergence tolerance (relative component).
const DEFAULT_NEWTON_RTOL: f64 = 1e-6;
/// Default Newton convergence tolerance (absolute component).
const DEFAULT_NEWTON_ATOL: f64 = 1e-9;
/// Default maximum Newton iterations per step.
const DEFAULT_NEWTON_MAX_ITER: usize = 15;

/// PI step-size controller defaults (Hairer & Wanner).
const CONTROL_SAFETY: f64 = 0.9;
const CONTROL_MIN_FACTOR: f64 = 0.2;
const CONTROL_MAX_FACTOR: f64 = 5.0;
// PI gains: k_I (integral) -0.075, k_P (proportional) 0.175 for order 2.
// (Scaled by 1/order at runtime.)
const PI_KI: f64 = 0.075;
const PI_KP: f64 = 0.175;

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

/// Integration statistics produced by [`BdfSolver`].
#[derive(Debug, Clone, Default)]
pub struct BdfStats {
    /// Number of successfully accepted steps.
    pub steps_taken: usize,
    /// Number of rejected steps (error too large or Newton failure).
    pub steps_rejected: usize,
    /// Total Newton iterations across all attempted steps.
    pub newton_iterations: usize,
    /// Number of Jacobian evaluations (finite-difference probes counted as one).
    pub jacobian_evaluations: usize,
    /// Number of LU decompositions of `(I - h*beta*J)`.
    pub lu_decompositions: usize,
    /// Number of RHS evaluations (Newton residual + Jacobian probes + error checks).
    pub rhs_evaluations: usize,
    /// Number of Newton-failure step rejections (subset of `steps_rejected`).
    pub newton_failures: usize,
}

// ---------------------------------------------------------------------------
// Core BDF solver
// ---------------------------------------------------------------------------

/// Variable-order (1..5) BDF implicit ODE solver.
///
/// Construct with [`BdfSolver::new`] and advance with [`BdfSolver::step`] or
/// [`BdfSolver::step_to`]. The solver manages its own step size and order
/// adaption.
#[derive(Clone)]
pub struct BdfSolver {
    /// Human-readable name.
    pub name: String,
    /// Current state (names + values). The `values` vector is `y_n`.
    pub state: OdeState,
    /// Saved initial values for [`reset`](Self::reset).
    initial_values: Vec<f64>,
    /// The RHS function `f(t, y, ctx)`.
    rhs: OdeRhs,

    /// Current simulation time (updated during `step_to`).
    pub t: f64,
    /// Current step size.
    pub h: f64,
    /// Current BDF order (1..=5). Starts at 1 and ramps up as history fills.
    pub order: usize,

    /// History of past states, most recent first: `[y_n, y_{n-1}, ...]`.
    /// Length equals current order.
    history: Vec<Vec<f64>>,
    /// Number of consecutive accepted steps taken at the *current* step
    /// size. The polynomial error estimator is only valid on a uniform
    /// grid — so we need `steps_at_current_h >= order + 1` to trust it.
    steps_at_current_h: usize,
    /// Step size associated with the current uniform-grid segment.
    h_current_segment: f64,

    /// Error norm from the previous accepted step (used by PI controller).
    /// `None` on the first accepted step.
    prev_err_norm: Option<f64>,

    /// Solver tolerances.
    pub rtol: f64,
    pub atol: f64,
    pub newton_max_iter: usize,
    pub dt_min: f64,
    pub dt_max: f64,

    /// Integration statistics.
    pub stats: BdfStats,

    /// Optional signal sync — writes signal expression results to shared context.
    signal_sync: Option<crate::ode45::SignalSyncFn>,

    /// Retained `OdeSpec` so the orchestrator's slot-binding pass (RSC-2.3) can
    /// rebind the derivative/signal expressions to slots and rebuild the RHS —
    /// parity with `Rk4Solver`/`Rk45Solver`. `None` for raw closure-only callers.
    spec: Option<crate::ode_builder::OdeSpec>,

    /// Rolling window of step-outcome flags (`true` = rejected) used by the
    /// stiffness heuristic. Bounded to 32 entries.
    recent_outcomes: Vec<bool>,

    /// RSC-4.x: precomputed slot write-set (state-index → slot + signal
    /// targets), installed by `Executor::prepare_slot_writeback`. Parity with
    /// `Rk4Solver`/`Rk45Solver` — the string-identity cull deleted BDF's legacy
    /// `sync_context_out`, so this is now BDF's only writeback. `None` for
    /// hand-built solvers without a retained spec (their write-set is not
    /// compile-enumerable) — such solvers publish nothing, as the trait default
    /// did before migration.
    write_set: Option<crate::ode::OdeWriteSet>,

    /// RSC-4.x: every read in the retained spec is servable without a scoped
    /// context clone (`OdeSpec::scoped_bypass_eligible`, latched at prepare
    /// time). Mirrors `Rk4Solver`/`Rk45Solver`.
    bypass_scoped: bool,
}

impl BdfSolver {
    /// Create a new BDF solver.
    ///
    /// # Arguments
    /// * `name`           — identifier for logging / diagnostics.
    /// * `state_names`    — names for each state variable.
    /// * `initial_values` — starting values (must match `state_names` length).
    /// * `rhs`            — derivative function `f(t, y, ctx) -> dy/dt`.
    pub fn new(
        name: impl Into<String>,
        state_names: Vec<String>,
        initial_values: Vec<f64>,
        rhs: OdeRhs,
    ) -> Self {
        assert_eq!(
            state_names.len(),
            initial_values.len(),
            "state_names and initial_values must have the same length"
        );

        let history = vec![initial_values.clone()];

        Self {
            name: name.into(),
            state: OdeState {
                names: state_names,
                values: initial_values.clone(),
            },
            initial_values,
            rhs,
            t: 0.0,
            h: 1e-3,
            order: 1,
            history,
            steps_at_current_h: 0,
            h_current_segment: 1e-3,
            prev_err_norm: None,
            rtol: DEFAULT_NEWTON_RTOL,
            atol: DEFAULT_NEWTON_ATOL,
            newton_max_iter: DEFAULT_NEWTON_MAX_ITER,
            dt_min: 1e-14,
            dt_max: 1.0,
            stats: BdfStats::default(),
            signal_sync: None,
            spec: None,
            recent_outcomes: Vec::with_capacity(32),
            write_set: None,
            bypass_scoped: false,
        }
    }

    /// Builder: retain the `OdeSpec` so slot-binding can rebind expressions
    /// (RSC-2.3). Mirrors `Rk45Solver::with_spec`.
    pub fn with_spec(mut self, spec: crate::ode_builder::OdeSpec) -> Self {
        self.spec = Some(spec);
        self
    }

    /// Builder: set absolute tolerance.
    pub fn with_atol(mut self, atol: f64) -> Self {
        self.atol = atol;
        self
    }

    /// Builder: set relative tolerance.
    pub fn with_rtol(mut self, rtol: f64) -> Self {
        self.rtol = rtol;
        self
    }

    /// Builder: set the initial step size.
    pub fn with_initial_dt(mut self, dt: f64) -> Self {
        self.h = dt;
        self
    }

    /// Builder: set the maximum step size.
    pub fn with_dt_max(mut self, dt: f64) -> Self {
        self.dt_max = dt;
        self
    }

    /// Builder: set the minimum step size.
    pub fn with_dt_min(mut self, dt: f64) -> Self {
        self.dt_min = dt;
        self
    }

    /// Builder: set the Newton iteration cap.
    pub fn with_newton_max_iter(mut self, max_iter: usize) -> Self {
        self.newton_max_iter = max_iter;
        self
    }

    /// Builder: attach a signal sync closure that writes signal expressions
    /// back into the shared [`EvalContext`] during `sync_context_out`.
    pub fn with_signal_sync(mut self, sync_fn: crate::ode45::SignalSyncFn) -> Self {
        self.signal_sync = Some(sync_fn);
        self
    }

    /// Reset solver state to initial values.
    pub fn reset(&mut self) {
        self.state.values = self.initial_values.clone();
        self.history = vec![self.initial_values.clone()];
        self.t = 0.0;
        self.h = 1e-3;
        self.order = 1;
        self.steps_at_current_h = 0;
        self.h_current_segment = 1e-3;
        self.prev_err_norm = None;
        self.stats = BdfStats::default();
        self.recent_outcomes.clear();
    }

    /// Current state variable names.
    pub fn state_names(&self) -> &[String] {
        &self.state.names
    }

    /// Current state vector.
    pub fn get_state(&self) -> &[f64] {
        &self.state.values
    }

    /// Write current state into `ctx` as [`Value::Float`] entries.
    pub fn sync_to_context(&self, ctx: &mut EvalContext) {
        for (name, &v) in self.state.names.iter().zip(self.state.values.iter()) {
            ctx.set(name.clone(), Value::Float(v));
        }
    }

    /// Record a step outcome (true = rejected) for the rolling stiffness window.
    fn record_outcome(&mut self, rejected: bool) {
        if self.recent_outcomes.len() >= 32 {
            self.recent_outcomes.remove(0);
        }
        self.recent_outcomes.push(rejected);
    }

    /// Observation-based step rejection rate over the last 32 attempts.
    pub fn recent_rejection_rate(&self) -> f64 {
        if self.recent_outcomes.is_empty() {
            0.0
        } else {
            let rejected = self.recent_outcomes.iter().filter(|r| **r).count();
            rejected as f64 / self.recent_outcomes.len() as f64
        }
    }

    /// Compute a finite-difference Jacobian `J_ij = df_i/dy_j` around `(t, y)`.
    ///
    /// Uses one-sided differences with per-component perturbation sized to
    /// the relative/absolute tolerance pair. Each column costs one RHS call.
    fn finite_diff_jacobian(
        &mut self,
        t: f64,
        y: &[f64],
        f0: &[f64],
        ctx: &EvalContext,
    ) -> DenseMatrix {
        let n = y.len();
        let mut j = DenseMatrix::zeros(n);

        for col in 0..n {
            let yj = y[col];
            let eps = (self.atol + self.rtol * yj.abs()).max(1e-10).sqrt();
            let mut y_pert = y.to_vec();
            y_pert[col] = yj + eps;
            let f_pert = (self.rhs)(t, &y_pert, ctx);
            self.stats.rhs_evaluations += 1;
            let inv_eps = 1.0 / eps;
            for row in 0..n {
                j.set(row, col, (f_pert[row] - f0[row]) * inv_eps);
            }
        }
        self.stats.jacobian_evaluations += 1;
        j
    }

    /// Component-wise error-scale vector `sc_i = atol + rtol * |y_i|` used for
    /// both Newton convergence and local error norms.
    fn error_scale(&self, y: &[f64]) -> Vec<f64> {
        y.iter()
            .map(|yi| self.atol + self.rtol * yi.abs())
            .collect()
    }

    /// Weighted RMS norm used by the integrator (Hairer & Wanner).
    fn wrms_norm(v: &[f64], scale: &[f64]) -> f64 {
        let mut s = 0.0_f64;
        for (vi, si) in v.iter().zip(scale.iter()) {
            let r = vi / si;
            s += r * r;
        }
        (s / v.len() as f64).sqrt()
    }

    /// Predict `y_{n+1}` by applying the BDF history combination.
    ///
    /// `y_pred = sum_i alpha_i * y_{n-i}`.
    /// This is the "frozen" part of the Newton residual — NOT the local-error
    /// predictor polynomial (use [`Self::polynomial_predict`] for that).
    fn predict(&self, k: usize) -> Vec<f64> {
        let c = coeffs(k);
        let n = self.history[0].len();
        let mut y = vec![0.0; n];
        for (i, a) in c.alpha.iter().enumerate() {
            let hist = &self.history[i];
            for d in 0..n {
                y[d] += a * hist[d];
            }
        }
        y
    }

    /// Reference polynomial value at `t_{n+1}` for the local error
    /// estimate.
    ///
    /// Returns `sum_i pred[i] * y_{n-i}` so that
    /// `y_{n+1} - polynomial_predict(k) == Δ^(k+1) y_{n+1}`, the
    /// `(k+1)`-th backward difference ≈ `h^(k+1) y^(k+1)`. Requires
    /// `k+1` history points; returns `None` when we don't have enough
    /// yet.
    fn polynomial_predict(&self, k: usize) -> Option<Vec<f64>> {
        let c = coeffs(k);
        if self.history.len() < c.pred.len() {
            return None;
        }
        let n = self.history[0].len();
        let mut y = vec![0.0; n];
        for (i, p) in c.pred.iter().enumerate() {
            let hist = &self.history[i];
            for d in 0..n {
                y[d] += p * hist[d];
            }
        }
        Some(y)
    }

    /// Attempt one implicit BDF step of size `self.h` at current order.
    ///
    /// Returns `true` on success (state advanced), `false` on rejection. On
    /// rejection the solver halves `h` and the caller is expected to retry.
    fn try_step(&mut self, ctx: &EvalContext) -> bool {
        // BDF-k on a uniform grid requires `k` history points AT the
        // current step size. `steps_at_current_h` counts accepted steps
        // taken at `h_current_segment`; if the caller has just changed
        // `self.h` (e.g. `step_to` clamped to the remaining interval),
        // the history is NOT at the current `h`, so we must fall back
        // to BDF1.
        let segment_eps = self.h_current_segment.abs() * 1e-2 + 1e-300;
        let h_matches_segment = (self.h - self.h_current_segment).abs() <= segment_eps;
        let uni_usable = if h_matches_segment {
            self.steps_at_current_h
        } else {
            0
        };
        let achievable = self.order.min(uni_usable.max(1));
        let k = achievable.max(1);
        let c = coeffs(k);
        let y_pred = self.predict(k);
        let t_new = self.t + self.h;

        // Initial guess: the predictor polynomial value.
        let mut y_new = y_pred.clone();

        // Compute Jacobian at the predicted point (fresh each step keeps
        // BDF robust for mildly nonlinear problems without chasing perf).
        let f0 = (self.rhs)(t_new, &y_new, ctx);
        self.stats.rhs_evaluations += 1;
        let j = self.finite_diff_jacobian(t_new, &y_new, &f0, ctx);

        // Build (I - h*beta*J) and factor it.
        let n = y_new.len();
        let hbeta = self.h * c.beta;
        let mut system = DenseMatrix::identity(n);
        for row in 0..n {
            for col in 0..n {
                system.data[row * n + col] -= hbeta * j.data[row * n + col];
            }
        }
        let lu = LuFactor::decompose(system);
        self.stats.lu_decompositions += 1;

        if lu.singular {
            // Jacobian ended up rank-deficient (e.g. tiny `h`). Reject.
            self.stats.steps_rejected += 1;
            self.stats.newton_failures += 1;
            self.record_outcome(true);
            self.h = (self.h * 0.5).max(self.dt_min);
            self.steps_at_current_h = 0;
            self.h_current_segment = self.h;
            return false;
        }

        // Newton iteration: solve `G(y) = y - y_pred - h*beta*f(t,y) = 0`.
        let mut converged = false;
        let mut last_delta_norm = f64::INFINITY;
        // Newton convergence is judged against the magnitude of the
        // current solution guess (scale with |y_new|), not `y_pred`.
        for iter in 0..self.newton_max_iter {
            let f_new = (self.rhs)(t_new, &y_new, ctx);
            self.stats.rhs_evaluations += 1;

            let mut g = vec![0.0; n];
            for d in 0..n {
                g[d] = y_new[d] - y_pred[d] - hbeta * f_new[d];
            }
            let delta = lu.solve(g);
            for d in 0..n {
                y_new[d] -= delta[d];
            }

            self.stats.newton_iterations += 1;

            let scale_now = self.error_scale(&y_new);
            let delta_norm = Self::wrms_norm(&delta, &scale_now);
            if delta_norm <= 0.33 {
                // Cheap early-out when increments become tiny — standard
                // Newton convergence test used by SUNDIALS/CVODE.
                if delta_norm <= 1e-3 || iter > 0 {
                    converged = true;
                    break;
                }
            }
            if iter > 0 && delta_norm > 2.0 * last_delta_norm {
                // Diverging; bail out.
                break;
            }
            last_delta_norm = delta_norm;
        }

        if !converged {
            self.stats.steps_rejected += 1;
            self.stats.newton_failures += 1;
            self.record_outcome(true);
            // Shrink aggressively on Newton failure.
            self.h = (self.h * 0.25).max(self.dt_min);
            self.steps_at_current_h = 0;
            self.h_current_segment = self.h;
            return false;
        }

        // Local error estimate.
        //
        // BDF-k has LTE ≈ err_const * h^(k+1) * y^(k+1). The (k+1)-th
        // backward difference `Δ^(k+1) y_{n+1}` is ≈ `h^(k+1) * y^(k+1)`
        // on a uniform grid, so we compute
        //     err = err_const * || Δ^(k+1) y_{n+1} || / scale.
        //
        // When we don't yet have `k+2` history points (start-up), we
        // cannot form the `(k+1)`-th difference. In that case we fall
        // back to a conservative "accept with moderate err norm" so the
        // PI controller still grows `h` gently. BDF1 is L-stable, so
        // this is safe with a small start step.
        let scale_new: Vec<f64> = (0..n)
            .map(|d| {
                let yold = self.history[0][d].abs();
                let ynew = y_new[d].abs();
                self.atol + self.rtol * yold.max(ynew)
            })
            .collect();

        // The polynomial error estimator assumes a *uniform* step-size
        // segment. Only trust it after we've taken at least `k+1`
        // accepted steps at the current `h`.
        let uniform_ok = self.steps_at_current_h >= k + 1;
        let err_norm = match (uniform_ok, self.polynomial_predict(k)) {
            (true, Some(y_ref)) => {
                let mut diff = vec![0.0; n];
                for d in 0..n {
                    diff[d] = y_new[d] - y_ref[d];
                }
                c.err_const * Self::wrms_norm(&diff, &scale_new)
            }
            _ => {
                // Warm-up / non-uniform step: conservative accept. We
                // deliberately pick a value noticeably below 1 so the
                // PI controller still grows `h` when Newton is happy.
                0.25
            }
        };

        if err_norm <= 1.0 {
            // Accept.
            self.stats.steps_taken += 1;
            self.record_outcome(false);

            // Push new state onto history (most-recent first), trim.
            // We need `k+1` points for the BDF-k equation itself plus
            // one more for the (k+1)-th backward-difference error
            // estimator, so cap at `k+2`. BDF5 maxes out at 7 entries.
            self.history.insert(0, y_new.clone());
            let max_hist = self.order + 2;
            if self.history.len() > max_hist {
                self.history.truncate(max_hist);
            }
            self.state.values = y_new;
            self.t = t_new;

            // Decide the next `h`.
            //
            // BDF-k at fixed `h` needs `k+1` history points AT THIS `h`
            // to be formally correct and have a trustworthy error
            // estimate. `steps_at_current_h` counts accepted steps at
            // the current `h`. While it is smaller than the target
            // order + 1 we HOLD `h` fixed so the order can ramp up and
            // a real uniform segment can form. Once we've built a full
            // segment we hand over to the PI controller.
            //
            // Reconcile segment tracking with the step we just took.
            // If `self.h` no longer matches the segment reference (e.g.
            // `step_to` clamped to the remaining interval), start a new
            // segment at the current `self.h`.
            let segment_eps = self.h_current_segment.abs() * 1e-2 + 1e-300;
            if (self.h - self.h_current_segment).abs() > segment_eps {
                self.h_current_segment = self.h;
                self.steps_at_current_h = 1;
            } else {
                self.steps_at_current_h += 1;
            }

            let order_f = self.order as f64;

            // Adapt `h` as soon as the polynomial error estimator is
            // usable (`uniform_ok` == `k+1` uniform-history points). We
            // deliberately key this on the *effective* order `k`, not
            // `self.order`, so that stiff problems — which spend most
            // time at BDF1/BDF2 during transient phases — can shrink
            // and grow `h` freely.
            let can_adapt = uniform_ok;

            let new_h = if can_adapt {
                let factor = if let Some(prev) = self.prev_err_norm {
                    let a = err_norm.max(1e-12);
                    let b = prev.max(1e-12);
                    CONTROL_SAFETY * a.powf(-(PI_KI + PI_KP) / order_f) * b.powf(PI_KP / order_f)
                } else {
                    CONTROL_SAFETY * err_norm.max(1e-12).powf(-1.0 / order_f)
                };
                let factor = factor.clamp(CONTROL_MIN_FACTOR, CONTROL_MAX_FACTOR);
                (self.h * factor).clamp(self.dt_min, self.dt_max)
            } else {
                // Hold `h` fixed during warm-up / partial segment.
                self.h
            };

            // If `h` changed appreciably, restart the uniform segment.
            let old_eps = self.h.abs() * 1e-2 + 1e-300;
            if (new_h - self.h).abs() > old_eps {
                self.steps_at_current_h = 0;
                self.h_current_segment = new_h;
            }
            self.h = new_h;
            self.prev_err_norm = Some(err_norm);

            // Order ramp-up: bump order as history grows, capped at 5.
            if self.order < 5 && self.history.len() > self.order {
                self.order += 1;
            }

            true
        } else {
            // Reject.
            self.stats.steps_rejected += 1;
            self.record_outcome(true);
            let factor =
                (CONTROL_SAFETY * err_norm.powf(-1.0 / self.order as f64)).max(CONTROL_MIN_FACTOR);
            self.h = (self.h * factor).max(self.dt_min);
            self.steps_at_current_h = 0;
            self.h_current_segment = self.h;
            false
        }
    }

    /// Take one accepted step (retrying internally on rejection).
    ///
    /// Returns the actual `h` used when the step was accepted. Gives up if
    /// `dt_min` is reached to avoid infinite loops — in that case the state
    /// is still advanced with the tiny step (best effort).
    pub fn step(&mut self, ctx: &EvalContext) -> f64 {
        loop {
            let h_try = self.h;
            if self.try_step(ctx) {
                return h_try;
            }
            if self.h <= self.dt_min * 1.0001 {
                // Floored — force-accept the next attempt to avoid spinning.
                let h_force = self.h;
                // Clear Newton history so the next try uses a tiny step.
                self.order = 1;
                self.history.truncate(1);
                // One more desperate attempt; accept whatever comes out.
                if self.try_step(ctx) {
                    return h_force;
                }
                // Still failing — advance with a forward Euler step to make
                // progress. This is an escape hatch so the integrator can
                // ALWAYS return.
                let y = self.state.values.clone();
                let f = (self.rhs)(self.t, &y, ctx);
                self.stats.rhs_evaluations += 1;
                let new_values: Vec<f64> = y
                    .iter()
                    .zip(f.iter())
                    .map(|(yi, fi)| yi + h_force * fi)
                    .collect();
                self.state.values = new_values.clone();
                self.t += h_force;
                self.history.insert(0, new_values);
                self.history.truncate(1);
                self.stats.steps_taken += 1;
                return h_force;
            }
        }
    }

    /// Integrate from the current `self.t` up to `t_target`.
    ///
    /// Returns the number of accepted steps taken.
    pub fn step_to(&mut self, t_target: f64, ctx: &EvalContext) -> usize {
        // Auto-pick a sensible initial step if we're still on the default
        // (very conservative) value and have plenty of ground to cover.
        let interval = (t_target - self.t).abs();
        if self.stats.steps_taken == 0 && self.h <= 1e-3 && interval > 1.0 {
            self.h = (interval * 1e-3).clamp(1e-6, self.dt_max);
            self.h_current_segment = self.h;
        }

        let mut steps = 0usize;
        let safety_limit: usize = 1_000_000;
        let eps = 1e-12 * t_target.abs().max(1.0);
        while self.t < t_target - eps {
            let remaining = t_target - self.t;
            let saved_h = self.h;
            let clamped = self.h > remaining;
            if clamped {
                self.h = remaining;
            }
            self.step(ctx);
            // If we clamped and the controller did not itself ratchet down,
            // restore the previously-chosen `h` so the next iteration isn't
            // permanently stuck at the final-fragment size.
            if clamped && self.h >= remaining * 0.9 {
                self.h = saved_h.min(self.dt_max);
            }
            steps += 1;
            if steps >= safety_limit {
                break;
            }
        }
        steps
    }
}

// ---------------------------------------------------------------------------
// Stiffness detection
// ---------------------------------------------------------------------------

/// Heuristic stiffness classifier.
///
/// Two inputs:
/// - `jacobian_spectrum`: optional pair `(max_eigenvalue_magnitude,
///   min_eigenvalue_magnitude)`. If the ratio is large the system is stiff.
/// - `rejection_rate`: observed step-rejection rate in `[0, 1]` for an
///   explicit integrator.
///
/// Returns `true` when either signal indicates stiffness.
///
/// Intended use: the runtime runs a few steps with a cheap explicit solver
/// (RK45) and calls this to decide whether to switch to BDF.
pub fn detect_stiffness(jacobian_spectrum: Option<(f64, f64)>, rejection_rate: f64) -> bool {
    if let Some((max_abs, min_abs)) = jacobian_spectrum {
        if min_abs > 0.0 {
            let ratio = max_abs / min_abs;
            if ratio > 1e3 {
                return true;
            }
        }
    }
    rejection_rate > 0.5
}

// ---------------------------------------------------------------------------
// WS-B4: build-time stiffness classification for automatic solver selection
// ---------------------------------------------------------------------------

/// Explicit-method stability margin for the step-relative stiffness test.
/// The RK45 (Dormand-Prince) real-axis stability boundary is ≈ 3.3; a
/// conservative `2.0` flags genuinely stiff systems while leaving mildly-fast
/// non-stiff models on the explicit default (steward ruling, WS-B4).
const STIFFNESS_STABILITY_MARGIN: f64 = 2.0;

/// Eigenvalue-spread threshold for the secondary multi-scale stiffness arm
/// (mirrors the ratio test in [`detect_stiffness`]).
const STIFFNESS_RATIO_THRESHOLD: f64 = 1e3;

/// Outcome of build-time stiffness classification (WS-B4).
#[derive(Debug, Clone)]
pub struct StiffnessVerdict {
    /// Route to an implicit (BDF) solver when true; explicit (RK45) otherwise.
    pub is_stiff: bool,
    /// Estimated dominant eigenvalue magnitude `|λ_max|` of the Jacobian (1/s).
    pub spectral_radius: f64,
    /// Step-relative stiffness index `|λ_max|·dt` (dimensionless). The PRIMARY
    /// criterion: greater than [`STIFFNESS_STABILITY_MARGIN`] means an explicit
    /// step of size `dt` cannot resolve the fastest mode.
    pub stiffness_index: f64,
    /// Estimated eigenvalue spread `|λ_max|/|λ_min|` (secondary, multi-scale).
    /// `None` when the Jacobian is singular (smallest eigenvalue ≈ 0).
    pub eigenvalue_ratio: Option<f64>,
    /// True when the Jacobian could not be evaluated (non-finite RHS at `t0`).
    /// Callers must diagnose this and bias to the robust implicit solver.
    pub jacobian_failed: bool,
}

/// Build-time stiffness classifier for automatic solver selection (WS-B4).
///
/// SPEC-SILENT sanctioned extension (steward ruling): the SysML v2 Analysis
/// library leaves solver selection to the tool — `StateSpaceRepresentation.sysml`
/// declares `Integrate` `abstract` ("its actual implementation should be given
/// by a solver"). This routes an UN-annotated ODE to BDF when its fastest mode
/// is unresolved by an explicit step `dt_seconds`, and to RK45 otherwise.
/// Explicit `@ToolExecution` choices never reach this path.
///
/// Method: finite-difference Jacobian `J = df/dy` at `(t0, y0)`, dominant
/// eigenvalue magnitude `ρ = |λ_max|` via power iteration, smallest magnitude
/// `|λ_min|` via inverse iteration through the existing LU. Classifies stiff
/// when the textbook step-relative criterion `ρ·dt > margin` holds — this is
/// the Dahlquist/Hairer definition of stiffness, and unlike the eigenvalue-
/// RATIO proxy in [`detect_stiffness`] it correctly flags SCALAR stiff systems
/// (a 1×1 Jacobian has ratio 1.0) — OR when the eigenvalue spread is large
/// (multi-scale).
///
/// On Jacobian failure (non-finite RHS) returns `is_stiff = true` with
/// `jacobian_failed = true`: bias to the robust implicit solver.
pub fn classify_stiffness_at_state<F>(
    rhs: &F,
    t0: f64,
    y0: &[f64],
    ctx: &EvalContext,
    dt_seconds: f64,
) -> StiffnessVerdict
where
    F: Fn(f64, &[f64], &EvalContext) -> Vec<f64> + ?Sized,
{
    let n = y0.len();
    let not_stiff = StiffnessVerdict {
        is_stiff: false,
        spectral_radius: 0.0,
        stiffness_index: 0.0,
        eigenvalue_ratio: None,
        jacobian_failed: false,
    };
    if n == 0 {
        return not_stiff;
    }
    let failed = StiffnessVerdict {
        is_stiff: true,
        spectral_radius: f64::INFINITY,
        stiffness_index: f64::INFINITY,
        eigenvalue_ratio: None,
        jacobian_failed: true,
    };

    let f0 = rhs(t0, y0, ctx);
    if f0.len() != n || f0.iter().any(|v| !v.is_finite()) {
        return failed;
    }

    // Finite-difference Jacobian (one-sided), matching `finite_diff_jacobian`.
    let atol = 1e-6_f64;
    let rtol = 1e-6_f64;
    let mut jac = DenseMatrix::zeros(n);
    for col in 0..n {
        let yj = y0[col];
        let eps = (atol + rtol * yj.abs()).max(1e-10).sqrt();
        let mut y_pert = y0.to_vec();
        y_pert[col] = yj + eps;
        let f_pert = rhs(t0, &y_pert, ctx);
        if f_pert.len() != n {
            return failed;
        }
        let inv_eps = 1.0 / eps;
        for row in 0..n {
            let entry = (f_pert[row] - f0[row]) * inv_eps;
            if !entry.is_finite() {
                return failed;
            }
            jac.set(row, col, entry);
        }
    }

    let rho = spectral_radius_power(&jac);
    let min_eig = smallest_eigenvalue_inverse(&jac);
    let stiffness_index = rho * dt_seconds.abs();
    let eigenvalue_ratio = min_eig.and_then(|m| if m > 0.0 { Some(rho / m) } else { None });

    let is_stiff = stiffness_index > STIFFNESS_STABILITY_MARGIN
        || eigenvalue_ratio.is_some_and(|r| r > STIFFNESS_RATIO_THRESHOLD);

    StiffnessVerdict {
        is_stiff,
        spectral_radius: rho,
        stiffness_index,
        eigenvalue_ratio,
        jacobian_failed: false,
    }
}

/// Matrix-vector product `J·v` for the row-major dense matrix.
fn matvec(m: &DenseMatrix, v: &[f64]) -> Vec<f64> {
    let n = m.n;
    let mut out = vec![0.0; n];
    for i in 0..n {
        let mut s = 0.0;
        for j in 0..n {
            s += m.data[i * n + j] * v[j];
        }
        out[i] = s;
    }
    out
}

fn norm2(v: &[f64]) -> f64 {
    v.iter().map(|x| x * x).sum::<f64>().sqrt()
}

/// Deterministic, non-degenerate start vector for the eigenvalue iterations
/// (no RNG — WS-C build determinism). Slight per-component variation avoids
/// exact orthogonality to the dominant eigenvector for typical Jacobians.
fn start_vector(n: usize) -> Option<Vec<f64>> {
    let mut v: Vec<f64> = (0..n).map(|i| 1.0 + 0.1 * i as f64).collect();
    let norm = norm2(&v);
    if norm == 0.0 {
        return None;
    }
    for x in &mut v {
        *x /= norm;
    }
    Some(v)
}

/// Dominant eigenvalue magnitude `|λ_max|` via power iteration.
fn spectral_radius_power(m: &DenseMatrix) -> f64 {
    let Some(mut v) = start_vector(m.n) else {
        return 0.0;
    };
    let mut rho = 0.0;
    for _ in 0..200 {
        let w = matvec(m, &v);
        let wn = norm2(&w);
        if wn == 0.0 || !wn.is_finite() {
            return rho;
        }
        rho = wn; // ||J v|| with ||v|| = 1 estimates |λ_max|
        for (vi, wi) in v.iter_mut().zip(w.iter()) {
            *vi = wi / wn;
        }
    }
    rho
}

/// Smallest eigenvalue magnitude `|λ_min|` via inverse power iteration through
/// the existing LU factorisation. Returns `None` when the Jacobian is
/// (near-)singular — the smallest eigenvalue is ≈ 0 and the ratio test is
/// unreliable.
fn smallest_eigenvalue_inverse(m: &DenseMatrix) -> Option<f64> {
    let lu = LuFactor::decompose(m.clone());
    if lu.singular {
        return None;
    }
    let mut v = start_vector(m.n)?;
    let mut min_eig = 0.0;
    for _ in 0..200 {
        // Solve J w = v ; ||w|| ≈ 1/|λ_min|.
        let w = lu.solve(v.clone());
        let wn = norm2(&w);
        if wn == 0.0 || !wn.is_finite() {
            return None;
        }
        min_eig = 1.0 / wn;
        for (vi, wi) in v.iter_mut().zip(w.iter()) {
            *vi = wi / wn;
        }
    }
    Some(min_eig)
}

// ---------------------------------------------------------------------------
// Executor trait implementation
// ---------------------------------------------------------------------------

impl crate::orchestrator::Executor for BdfSolver {
    fn phase(&self) -> crate::orchestrator::ExecutionPhase {
        crate::orchestrator::ExecutionPhase::ContinuousDynamics
    }

    fn kind_label(&self) -> &'static str {
        "bdf"
    }

    fn tick(
        &mut self,
        ctx: &crate::orchestrator::TickContext<'_>,
    ) -> crate::orchestrator::TickOutput {
        // Orchestrator supplies `dt_ms`-sized windows; integrate up to
        // `ctx.t + ctx.dt` using adaptive sub-steps.
        self.t = ctx.t;
        self.step_to(ctx.t + ctx.dt, ctx.context);
        let primary = self.state.values.first().copied().unwrap_or(0.0);
        crate::orchestrator::TickOutput::solver(
            format!("{:.4}", primary),
            self.state
                .names
                .iter()
                .zip(&self.state.values)
                .map(|(n, v)| format!("{}={:.4}", n, v))
                .collect(),
        )
    }

    fn reset_executor(&mut self) {
        self.reset();
    }

    fn is_completed(&self) -> bool {
        true
    }

    fn clone_boxed(&self) -> Box<dyn crate::orchestrator::Executor> {
        Box::new(self.clone())
    }

    /// RSC-4.x: slot-routed writeback (see `Rk4Solver::sync_context_out_slots`).
    /// The legacy `sync_context_out` (`sync_to_context` + `signal_sync`
    /// recompute) was deleted with the string-identity cull; this routed path
    /// reproduces it (`write_states` + `eval_signals_slot_routed`) and is now
    /// BDF's only writeback. `false` (publish nothing) for a hand-built solver
    /// with no retained spec — matching the pre-migration trait default.
    fn sync_context_out_slots(
        &self,
        shared: &mut EvalContext,
        mode: crate::ode::SignalEvalMode,
    ) -> bool {
        let Some(ws) = &self.write_set else {
            return false;
        };
        ws.write_states(shared, &self.state.values);
        if let Some(sync_fn) = &self.signal_sync {
            crate::ode::eval_signals_slot_routed(
                ws,
                sync_fn,
                &self.state,
                shared,
                mode,
                |out| self.sync_to_context(out),
            );
        }
        true
    }

    /// RSC-4.x: build the precomputed slot write-set and latch the scoped-clone
    /// bypass eligibility. State write-set always built from `self.state.names`;
    /// signal targets come from the spec (empty for a hand-built solver).
    /// Mirrors `Rk45Solver::prepare_slot_writeback`.
    fn prepare_slot_writeback(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
        canonical_prefix: Option<&str>,
        writer: crate::slots::WriterId,
    ) {
        let signal_names: Vec<String> = match &self.spec {
            Some(spec) => {
                let mut n: Vec<String> = spec.signal_exprs.keys().cloned().collect();
                n.sort();
                n
            }
            None => Vec::new(),
        };
        self.write_set = Some(crate::ode::build_ode_write_set(
            store,
            var_prefix,
            canonical_prefix,
            writer,
            &self.state.names,
            signal_names,
        ));
        self.bypass_scoped = self
            .spec
            .as_ref()
            .is_some_and(|s| s.scoped_bypass_eligible());
    }

    fn scoped_view_bypass(&self) -> bool {
        self.bypass_scoped
    }

    fn unrouted_slot_writes(&self) -> Vec<String> {
        self.write_set
            .as_ref()
            .map(|ws| ws.unrouted_writes())
            .unwrap_or_default()
    }

    fn get_state_snapshot(&self) -> Option<Vec<f64>> {
        Some(self.state.values.clone())
    }

    /// RSC-4.1: read-set = slots read by the bound derivative + signal
    /// expressions of the retained `OdeSpec` (see `Rk4Solver::read_slots`).
    fn read_slots(&self) -> Vec<crate::slots::SlotId> {
        let mut v = self.spec.as_ref().map(|s| s.slot_reads()).unwrap_or_default();
        v.sort();
        v.dedup();
        v
    }

    fn bind_expression_slots(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
    ) -> crate::expressions::BindReport {
        // RSC-2.3: rebind derivative/signal expressions to slots and rebuild the
        // captured RHS closure. Mirrors `Rk45Solver::bind_expression_slots`.
        let Some(spec) = self.spec.as_mut() else {
            return crate::expressions::BindReport::default();
        };
        let report = spec.bind_slots(store, var_prefix);
        self.rhs = spec.build_rhs();
        if self.signal_sync.is_some() {
            self.signal_sync = spec.build_signal_sync();
        }
        report
    }
}

// ---------------------------------------------------------------------------
// Plugin wrapper (so BDF shows up in the `SolverRegistry`)
// ---------------------------------------------------------------------------

/// Solver-plugin wrapper that exposes BDF through the
/// [`crate::solver_plugin::SolverPlugin`] API, matching the pattern used by
/// [`crate::solver_builtins::OdeRk45Plugin`].
///
/// Accepts the same parameter set as the RK plugins:
/// - `derivative_exprs` : list of expression strings (one per state variable)
/// - `state_vars`       : list of state-variable names
/// - `dt`   (default 0.1)   : nominal macro step
/// - `steps` (default 100)  : number of macro steps
/// - `atol` (default 1e-9)  : Newton absolute tolerance
/// - `rtol` (default 1e-6)  : Newton relative tolerance
/// - `<state_var>` : initial value for each state variable (default 0.0)
/// - any other numeric param : bound as a constant in the derivative scope
pub struct BdfPlugin {
    /// Tool name this plugin is registered under. Default is
    /// `"builtin:ode-bdf"`; a second registration aliases plain `"bdf"`.
    name: String,
}

impl BdfPlugin {
    /// Default plugin registered as `"builtin:ode-bdf"`.
    pub fn new() -> Self {
        Self {
            name: "builtin:ode-bdf".to_string(),
        }
    }

    /// Alias plugin registered under a custom name (e.g. `"bdf"`).
    pub fn aliased(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

impl Default for BdfPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl crate::solver_plugin::SolverPlugin for BdfPlugin {
    fn name(&self) -> &str {
        &self.name
    }

    fn solve(
        &self,
        inputs: &[crate::solver_plugin::SolverParam],
        _constraints: &[crate::ConstraintIR],
        _context: &EvalContext,
    ) -> Result<crate::solver_plugin::SolverResult, crate::solver_plugin::SolverError> {
        use crate::solver_builtins::OdeRk4Plugin;
        use crate::solver_plugin::{SolverError, SolverResult};
        use std::collections::HashMap;

        let dt = OdeRk4Plugin::param_f64(inputs, "dt", 0.1);
        let steps = OdeRk4Plugin::param_f64(inputs, "steps", 100.0) as usize;
        let atol = OdeRk4Plugin::param_f64(inputs, "atol", DEFAULT_NEWTON_ATOL);
        let rtol = OdeRk4Plugin::param_f64(inputs, "rtol", DEFAULT_NEWTON_RTOL);

        let derivative_exprs = OdeRk4Plugin::param_strings(inputs, "derivative_exprs");
        let state_var_names = OdeRk4Plugin::param_strings(inputs, "state_vars");

        if derivative_exprs.is_empty() || state_var_names.is_empty() {
            return Err(SolverError::InvalidInput(
                "ode-bdf: `derivative_exprs` and `state_vars` parameters are required".into(),
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
        let mut solver = BdfSolver::new(
            "ode-bdf-plugin",
            spec.state_vars.clone(),
            spec.initial_values.clone(),
            rhs,
        )
        .with_atol(atol)
        .with_rtol(rtol)
        .with_initial_dt(dt);

        let total_time = steps as f64 * dt;
        let ctx = EvalContext::new();
        let num_steps = solver.step_to(total_time, &ctx);

        let mut outputs = HashMap::new();
        for (name, &val) in solver.state_names().iter().zip(solver.get_state().iter()) {
            outputs.insert(name.clone(), Value::Float(val));
        }
        outputs.insert("time".to_string(), Value::Float(total_time));
        outputs.insert(
            "bdf_steps_taken".to_string(),
            Value::Int(solver.stats.steps_taken as i64),
        );
        outputs.insert(
            "bdf_steps_rejected".to_string(),
            Value::Int(solver.stats.steps_rejected as i64),
        );
        outputs.insert(
            "bdf_newton_iterations".to_string(),
            Value::Int(solver.stats.newton_iterations as i64),
        );

        Ok(SolverResult {
            outputs,
            diagnostics: Vec::new(),
            iterations: Some(num_steps),
            converged: true,
        })
    }

    fn capabilities(&self) -> crate::solver_plugin::SolverCapabilities {
        crate::solver_plugin::SolverCapabilities {
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

    // ---- Dense LU unit tests ----------------------------------------------

    #[test]
    fn lu_solves_small_identity() {
        let n = 3;
        let m = DenseMatrix::identity(n);
        let lu = LuFactor::decompose(m);
        let x = lu.solve(vec![1.0, 2.0, 3.0]);
        assert!((x[0] - 1.0).abs() < 1e-12);
        assert!((x[1] - 2.0).abs() < 1e-12);
        assert!((x[2] - 3.0).abs() < 1e-12);
    }

    #[test]
    fn lu_solves_permuted_system() {
        // Requires pivoting: [[0, 1], [1, 0]] * x = [2, 1]  =>  x = [1, 2]
        let mut m = DenseMatrix::zeros(2);
        m.set(0, 1, 1.0);
        m.set(1, 0, 1.0);
        let lu = LuFactor::decompose(m);
        let x = lu.solve(vec![2.0, 1.0]);
        assert!((x[0] - 1.0).abs() < 1e-12);
        assert!((x[1] - 2.0).abs() < 1e-12);
    }

    #[test]
    fn lu_solves_diagonal_system() {
        let mut m = DenseMatrix::zeros(3);
        m.set(0, 0, 2.0);
        m.set(1, 1, 5.0);
        m.set(2, 2, 10.0);
        let lu = LuFactor::decompose(m);
        let x = lu.solve(vec![4.0, 10.0, 20.0]);
        assert!((x[0] - 2.0).abs() < 1e-12);
        assert!((x[1] - 2.0).abs() < 1e-12);
        assert!((x[2] - 2.0).abs() < 1e-12);
    }

    // ---- Smoke: non-stiff linear system ------------------------------------

    #[test]
    fn bdf_exponential_decay() {
        // dy/dt = -y,  y(0) = 1  =>  y(1) = exp(-1)
        let mut solver = BdfSolver::new(
            "decay",
            vec!["y".into()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-y[0]]),
        )
        .with_atol(1e-9)
        .with_rtol(1e-7)
        .with_initial_dt(0.05);

        let ctx = EvalContext::new();
        solver.step_to(1.0, &ctx);
        let expected = (-1.0f64).exp();
        // BDF5 with fresh-Jacobian Newton in f64 accumulates roughly
        // `steps * err_const * tol` error over the integration interval;
        // 3e-3 is comfortable for ~660 steps at this tolerance.
        assert!(
            (solver.get_state()[0] - expected).abs() < 3e-3,
            "expected {expected}, got {} (steps={}, rej={})",
            solver.get_state()[0],
            solver.stats.steps_taken,
            solver.stats.steps_rejected,
        );
    }

    #[test]
    fn bdf_handles_constant_derivative() {
        // dy/dt = 1 => y(t) = t + y0
        let mut solver = BdfSolver::new(
            "const",
            vec!["y".into()],
            vec![0.0],
            Arc::new(|_t, _y, _ctx| vec![1.0]),
        );
        let ctx = EvalContext::new();
        solver.step_to(1.0, &ctx);
        assert!(
            (solver.get_state()[0] - 1.0).abs() < 1e-6,
            "got {}",
            solver.get_state()[0]
        );
    }

    #[test]
    fn bdf_stiff_linear_scalar_stable() {
        // dy/dt = -1000 y, y(0)=1. RK4 with dt=0.01 would blow up; BDF stays
        // stable and converges to zero.
        let mut solver = BdfSolver::new(
            "stiff",
            vec!["y".into()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-1000.0 * y[0]]),
        )
        .with_initial_dt(0.1)
        .with_dt_max(0.2);

        let ctx = EvalContext::new();
        solver.step_to(1.0, &ctx);
        assert!(
            solver.get_state()[0].abs() < 1e-3,
            "stiff system should decay to 0, got {}",
            solver.get_state()[0],
        );
    }

    // ---- Stiffness detection heuristic -------------------------------------

    #[test]
    fn detect_stiffness_from_spectrum() {
        assert!(detect_stiffness(Some((1e6, 1.0)), 0.0));
        assert!(!detect_stiffness(Some((10.0, 1.0)), 0.0));
    }

    #[test]
    fn detect_stiffness_from_rejections() {
        assert!(detect_stiffness(None, 0.8));
        assert!(!detect_stiffness(None, 0.2));
    }

    // ---- WS-B4 build-time stiffness classifier -----------------------------

    #[test]
    fn classify_scalar_stiff_via_step_relative_criterion() {
        // dx/dt = -1000 x. The 1×1 Jacobian is [-1000] → eigenvalue RATIO is
        // 1.0 (the ratio test misses it), but |λ|·dt = 1000·0.01 = 10 > margin
        // at a coarse 10 ms step → stiff. This is the canonical scalar case the
        // ratio-only `detect_stiffness` cannot catch.
        let rhs = |_t: f64, y: &[f64], _c: &EvalContext| vec![-1000.0 * y[0]];
        let ctx = EvalContext::new();
        let v = classify_stiffness_at_state(&rhs, 0.0, &[1.0], &ctx, 0.01);
        assert!(v.is_stiff, "scalar stiff dx/dt=-1000x must classify stiff");
        assert!(!v.jacobian_failed);
        assert!(
            (v.spectral_radius - 1000.0).abs() < 1.0,
            "ρ≈1000, got {}",
            v.spectral_radius
        );
        assert!(
            (v.stiffness_index - 10.0).abs() < 0.1,
            "|λ|·dt≈10, got {}",
            v.stiffness_index
        );
    }

    #[test]
    fn classify_nonstiff_stays_explicit() {
        // dx/dt = -1 x at a fine step: |λ|·dt = 1·0.01 = 0.01 ≪ margin → RK45.
        let rhs = |_t: f64, y: &[f64], _c: &EvalContext| vec![-1.0 * y[0]];
        let ctx = EvalContext::new();
        let v = classify_stiffness_at_state(&rhs, 0.0, &[1.0], &ctx, 0.01);
        assert!(
            !v.is_stiff,
            "mild decay must stay non-stiff, index={}",
            v.stiffness_index
        );
    }

    #[test]
    fn classify_multiscale_stiff_via_ratio() {
        // Decoupled fast+slow modes: dy0=-1000 y0, dy1=-0.1 y1. At a step where
        // even the fast mode is resolved (|λ_max|·dt = 1000·1e-4 = 0.1 < margin)
        // the secondary eigenvalue-spread arm (1000/0.1 = 1e4 > 1e3) flags it.
        let rhs = |_t: f64, y: &[f64], _c: &EvalContext| vec![-1000.0 * y[0], -0.1 * y[1]];
        let ctx = EvalContext::new();
        let v = classify_stiffness_at_state(&rhs, 0.0, &[1.0, 1.0], &ctx, 1e-4);
        assert!(
            v.stiffness_index < STIFFNESS_STABILITY_MARGIN,
            "step resolves fast mode"
        );
        assert!(
            v.eigenvalue_ratio.is_some_and(|r| r > 1e3),
            "spread should be large"
        );
        assert!(v.is_stiff, "multi-scale system flagged stiff via ratio arm");
    }

    #[test]
    fn classify_jacobian_failure_biases_to_implicit() {
        // RHS returns NaN → cannot classify → bias to robust implicit (stiff).
        let rhs = |_t: f64, _y: &[f64], _c: &EvalContext| vec![f64::NAN];
        let ctx = EvalContext::new();
        let v = classify_stiffness_at_state(&rhs, 0.0, &[1.0], &ctx, 0.01);
        assert!(v.jacobian_failed && v.is_stiff);
    }

    // ---- Reset / snapshot --------------------------------------------------

    #[test]
    fn bdf_reset_restores_initial_state() {
        let mut solver = BdfSolver::new(
            "reset",
            vec!["y".into()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-y[0]]),
        );
        let ctx = EvalContext::new();
        solver.step_to(0.5, &ctx);
        assert!(solver.get_state()[0] < 1.0);
        solver.reset();
        assert_eq!(solver.get_state()[0], 1.0);
        assert_eq!(solver.stats.steps_taken, 0);
        assert_eq!(solver.t, 0.0);
    }

    #[test]
    fn bdf_sync_to_context() {
        let solver = BdfSolver::new(
            "sync",
            vec!["x".into(), "v".into()],
            vec![3.0, 4.0],
            Arc::new(|_t, _y, _ctx| vec![0.0, 0.0]),
        );
        let mut ctx = EvalContext::new();
        solver.sync_to_context(&mut ctx);
        assert_eq!(ctx.get("x"), Some(&Value::Float(3.0)));
        assert_eq!(ctx.get("v"), Some(&Value::Float(4.0)));
    }

    // ---- Plugin round-trip -------------------------------------------------

    #[test]
    fn bdf_plugin_name_default() {
        use crate::solver_plugin::SolverPlugin;
        let p = BdfPlugin::new();
        assert_eq!(p.name(), "builtin:ode-bdf");
    }

    #[test]
    fn bdf_plugin_name_alias() {
        use crate::solver_plugin::SolverPlugin;
        let p = BdfPlugin::aliased("bdf");
        assert_eq!(p.name(), "bdf");
    }

    #[test]
    fn bdf_plugin_missing_required_params_errors() {
        use crate::solver_plugin::SolverPlugin;
        let p = BdfPlugin::new();
        let result = p.solve(&[], &[], &EvalContext::new());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("derivative_exprs"));
    }

    #[test]
    fn bdf_plugin_registered_in_builtins() {
        use crate::solver_registry::SolverRegistry;
        let reg = SolverRegistry::with_builtins();
        assert!(
            reg.get("builtin:ode-bdf").is_some(),
            "BdfPlugin must be registered under the conventional id"
        );
        assert!(
            reg.get("bdf").is_some(),
            "BdfPlugin must also be aliased under the short id"
        );
    }

    #[test]
    fn bdf_recent_rejection_rate_bounded() {
        let mut s = BdfSolver::new(
            "rr",
            vec!["y".into()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-y[0]]),
        );
        assert_eq!(s.recent_rejection_rate(), 0.0);
        for _ in 0..40 {
            s.record_outcome(true);
        }
        assert_eq!(s.recent_outcomes.len(), 32);
        assert!(s.recent_rejection_rate() > 0.99);
    }
}
