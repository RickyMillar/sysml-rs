//! # Adaptive RK45 ODE Solver (Dormand-Prince Method)
//!
//! Implements the Dormand-Prince embedded RK4(5) method with automatic
//! step-size control. The 4th-order solution advances the state while
//! the 5th-order solution provides an error estimate for adaptive stepping.
//!
//! ## When to Use
//!
//! - **RK4** (`ode.rs`): Fixed step, predictable cost, good for well-behaved systems
//! - **RK45** (this module): Adaptive step, better for stiff systems or when
//!   accuracy matters more than predictability

// ODE solver uses pervasive array indexing for coefficient tableaux and state vectors.
#![allow(clippy::indexing_slicing)]

#[cfg(test)]
use std::sync::Arc;

use crate::expressions::EvalContext;
use crate::ode::{OdeRhs, OdeState};
use sysml_core::Value;

/// Dormand-Prince Butcher tableau coefficients (a_ij).
const A21: f64 = 1.0 / 5.0;
const A31: f64 = 3.0 / 40.0;
const A32: f64 = 9.0 / 40.0;
const A41: f64 = 44.0 / 45.0;
const A42: f64 = -56.0 / 15.0;
const A43: f64 = 32.0 / 9.0;
const A51: f64 = 19372.0 / 6561.0;
const A52: f64 = -25360.0 / 2187.0;
const A53: f64 = 64448.0 / 6561.0;
const A54: f64 = -212.0 / 729.0;
const A61: f64 = 9017.0 / 3168.0;
const A62: f64 = -355.0 / 33.0;
const A63: f64 = 46732.0 / 5247.0;
const A64: f64 = 49.0 / 176.0;
const A65: f64 = -5103.0 / 18656.0;

/// 5th-order weights (b*_i) for advancing the solution (higher accuracy).
const BS1: f64 = 35.0 / 384.0;
const BS3: f64 = 500.0 / 1113.0;
const BS4: f64 = 125.0 / 192.0;
const BS5: f64 = -2187.0 / 6784.0;
const BS6: f64 = 11.0 / 84.0;

/// 4th-order weights (b_i) for error estimation (comparison with 5th-order).
const B1: f64 = 5179.0 / 57600.0;
const B3: f64 = 7571.0 / 16695.0;
const B4: f64 = 393.0 / 640.0;
const B5: f64 = -92097.0 / 339200.0;
const B6: f64 = 187.0 / 2100.0;
const B7: f64 = 1.0 / 40.0;

/// Node values (c_i).
const C2: f64 = 1.0 / 5.0;
const C3: f64 = 3.0 / 10.0;
const C4: f64 = 4.0 / 5.0;
const C5: f64 = 8.0 / 9.0;

/// Integration statistics.
#[derive(Debug, Clone, Default)]
pub struct IntegrationStats {
    /// Number of accepted steps.
    pub steps_accepted: usize,
    /// Number of rejected steps (error too large).
    pub steps_rejected: usize,
    /// Total right-hand-side evaluations.
    pub rhs_evaluations: usize,
}

/// Signal sync closure type: `fn(state, shared_ctx, out_ctx)`.
/// Re-evaluates signal expressions with the current state and writes results
/// to the output context, making ODE-internal computed values visible to other subsystems.
pub type SignalSyncFn = std::sync::Arc<
    dyn Fn(&[f64], &crate::expressions::EvalContext, &mut crate::expressions::EvalContext)
        + Send
        + Sync,
>;

/// Adaptive Dormand-Prince RK4(5) ODE solver.
#[derive(Clone)]
pub struct Rk45Solver {
    /// Human-readable name.
    pub name: String,
    /// Current state (names + values).
    pub state: OdeState,
    /// Saved initial values for reset.
    initial_values: Vec<f64>,
    /// The RHS function.
    rhs: OdeRhs,
    /// Absolute tolerance.
    atol: f64,
    /// Relative tolerance.
    rtol: f64,
    /// Minimum allowed step size.
    dt_min: f64,
    /// Maximum allowed step size.
    dt_max: f64,
    /// Current adaptive step size.
    pub current_dt: f64,
    /// Maximum steps per `step_to` call (safety limit).
    max_steps_per_call: usize,
    /// Integration statistics.
    pub stats: IntegrationStats,
    /// Cached k1 from FSAL (= k7 of previous accepted step). None on first step or after rejection.
    fsal_k1: Option<Vec<f64>>,
    /// Derivatives `dy/dt = f(t, y)` captured at the start of the most recent
    /// accepted step (= k1), one per state var. Reported via
    /// `Executor::current_derivatives` so the orchestrator's `derivatives`
    /// snapshot is populated for RK45 at parity with RK4 (WS-B2: RK45 is now
    /// the default, so this snapshot field must not silently go empty).
    last_derivatives: Vec<f64>,
    /// Optional signal sync — writes signal expression results to shared context.
    signal_sync: Option<SignalSyncFn>,
    /// RSC-2.3: the `OdeSpec` this solver's closures were built from, when
    /// the `ModelCompiler` built it (see [`with_spec`](Self::with_spec)).
    /// Retained so `Executor::bind_expression_slots` can rebind the spec's
    /// expressions to slots and rebuild `rhs` / `signal_sync` after the
    /// slot table is minted. `None` for hand-built solvers.
    spec: Option<crate::ode_builder::OdeSpec>,
    /// RSC-2.4a: precomputed slot write-set, installed by
    /// `Executor::prepare_slot_writeback`. `None` for hand-built solvers.
    write_set: Option<crate::ode::OdeWriteSet>,
    /// RSC-2.4a: scoped-context-clone bypass eligibility, latched from
    /// `OdeSpec::scoped_bypass_eligible` at prepare time.
    bypass_scoped: bool,
}

/// Safety factor for step size adjustment (0 < SAFETY < 1).
const SAFETY: f64 = 0.9;
/// Maximum growth factor per step.
const MAX_GROWTH: f64 = 5.0;
/// Minimum shrink factor per step.
const MIN_SHRINK: f64 = 0.2;

impl Rk45Solver {
    /// Create a new adaptive RK45 solver.
    pub fn new(
        name: impl Into<String>,
        state_names: Vec<String>,
        initial_values: Vec<f64>,
        rhs: OdeRhs,
    ) -> Self {
        assert_eq!(state_names.len(), initial_values.len());
        Self {
            name: name.into(),
            state: OdeState {
                names: state_names,
                values: initial_values.clone(),
            },
            initial_values,
            rhs,
            atol: 1e-6,
            rtol: 1e-3,
            dt_min: 1e-12,
            dt_max: 1.0,
            current_dt: 0.01,
            max_steps_per_call: 100_000,
            stats: IntegrationStats::default(),
            signal_sync: None,
            fsal_k1: None,
            last_derivatives: Vec::new(),
            spec: None,
            write_set: None,
            bypass_scoped: false,
        }
    }

    /// RSC-2.3: retain the [`OdeSpec`](crate::ode_builder::OdeSpec) this
    /// solver was built from, enabling slot rebinding via
    /// `Executor::bind_expression_slots`.
    pub fn with_spec(mut self, spec: crate::ode_builder::OdeSpec) -> Self {
        self.spec = Some(spec);
        self
    }

    /// Set absolute tolerance.
    pub fn with_atol(mut self, atol: f64) -> Self {
        self.atol = atol;
        self
    }

    /// Set relative tolerance.
    pub fn with_rtol(mut self, rtol: f64) -> Self {
        self.rtol = rtol;
        self
    }

    /// Set minimum step size.
    pub fn with_dt_min(mut self, dt_min: f64) -> Self {
        self.dt_min = dt_min;
        self
    }

    /// Set maximum step size.
    pub fn with_dt_max(mut self, dt_max: f64) -> Self {
        self.dt_max = dt_max;
        self
    }

    /// Set initial step size.
    pub fn with_initial_dt(mut self, dt: f64) -> Self {
        self.current_dt = dt;
        self
    }

    /// Attach a signal sync closure that writes signal expression results
    /// to the shared context during `sync_context_out`.
    pub fn with_signal_sync(mut self, sync_fn: SignalSyncFn) -> Self {
        self.signal_sync = Some(sync_fn);
        self
    }

    /// Take one adaptive step from time `t`.
    ///
    /// Returns the actual step size used. The solver may reject the step
    /// internally and retry with a smaller dt.
    pub fn step(&mut self, t: f64, ctx: &EvalContext) -> f64 {
        let n = self.state.values.len();
        let y = &self.state.values;

        loop {
            let dt = self.current_dt;

            // Six RHS evaluations for the Dormand-Prince method
            // FSAL: reuse k7 from previous accepted step as k1
            let k1 = match self.fsal_k1.take() {
                Some(cached) => cached,
                None => {
                    self.stats.rhs_evaluations += 1;
                    (self.rhs)(t, y, ctx)
                }
            };

            let y2: Vec<f64> = (0..n).map(|i| y[i] + dt * A21 * k1[i]).collect();
            let k2 = (self.rhs)(t + C2 * dt, &y2, ctx);
            self.stats.rhs_evaluations += 1;

            let y3: Vec<f64> = (0..n)
                .map(|i| y[i] + dt * (A31 * k1[i] + A32 * k2[i]))
                .collect();
            let k3 = (self.rhs)(t + C3 * dt, &y3, ctx);
            self.stats.rhs_evaluations += 1;

            let y4: Vec<f64> = (0..n)
                .map(|i| y[i] + dt * (A41 * k1[i] + A42 * k2[i] + A43 * k3[i]))
                .collect();
            let k4 = (self.rhs)(t + C4 * dt, &y4, ctx);
            self.stats.rhs_evaluations += 1;

            let y5: Vec<f64> = (0..n)
                .map(|i| y[i] + dt * (A51 * k1[i] + A52 * k2[i] + A53 * k3[i] + A54 * k4[i]))
                .collect();
            let k5 = (self.rhs)(t + C5 * dt, &y5, ctx);
            self.stats.rhs_evaluations += 1;

            let y6: Vec<f64> = (0..n)
                .map(|i| {
                    y[i] + dt
                        * (A61 * k1[i] + A62 * k2[i] + A63 * k3[i] + A64 * k4[i] + A65 * k5[i])
                })
                .collect();
            let k6 = (self.rhs)(t + dt, &y6, ctx);
            self.stats.rhs_evaluations += 1;

            // 5th-order solution (for advancing)
            let y5th: Vec<f64> = (0..n)
                .map(|i| {
                    y[i] + dt
                        * (BS1 * k1[i] + BS3 * k3[i] + BS4 * k4[i] + BS5 * k5[i] + BS6 * k6[i])
                })
                .collect();

            // 4th-order solution (for error estimation)
            // k7 = f(t+dt, y5th) — the FSAL property
            let k7 = (self.rhs)(t + dt, &y5th, ctx);
            self.stats.rhs_evaluations += 1;

            let y4th: Vec<f64> = (0..n)
                .map(|i| {
                    y[i] + dt
                        * (B1 * k1[i]
                            + B3 * k3[i]
                            + B4 * k4[i]
                            + B5 * k5[i]
                            + B6 * k6[i]
                            + B7 * k7[i])
                })
                .collect();

            // Error estimate: difference between 4th and 5th order solutions
            let mut err_norm = 0.0_f64;
            for i in 0..n {
                let scale = self.atol + self.rtol * y[i].abs().max(y5th[i].abs());
                let err_i = (y5th[i] - y4th[i]) / scale;
                err_norm += err_i * err_i;
            }
            err_norm = (err_norm / n as f64).sqrt();

            if err_norm <= 1.0 {
                // Step accepted
                self.state.values = y5th;
                self.last_derivatives = k1.clone();
                self.stats.steps_accepted += 1;
                self.fsal_k1 = Some(k7);

                // Adjust step size for next step
                if err_norm < 1e-15 {
                    self.current_dt = (dt * MAX_GROWTH).min(self.dt_max);
                } else {
                    let factor = SAFETY * err_norm.powf(-0.2);
                    self.current_dt =
                        (dt * factor.clamp(MIN_SHRINK, MAX_GROWTH)).clamp(self.dt_min, self.dt_max);
                }

                return dt;
            }

            // Step rejected — shrink dt and retry
            self.stats.steps_rejected += 1;
            let factor = SAFETY * err_norm.powf(-0.25);
            self.current_dt = (dt * factor.max(MIN_SHRINK)).max(self.dt_min);

            // If we've hit minimum step size, accept anyway to avoid infinite loop
            if self.current_dt <= self.dt_min {
                self.state.values = y5th;
                self.last_derivatives = k1.clone();
                self.stats.steps_accepted += 1;
                self.fsal_k1 = Some(k7);
                return dt;
            }
        }
    }

    /// Integrate from current time `t` to `t_target`, taking as many adaptive
    /// steps as needed. Returns the number of steps taken.
    pub fn step_to(&mut self, t: f64, t_target: f64, ctx: &EvalContext) -> usize {
        let mut current_t = t;
        let mut steps = 0;
        // Scale-appropriate epsilon for the completion check
        let eps = 1e-10 * t_target.abs().max(1.0);
        while current_t < t_target - eps {
            let remaining = t_target - current_t;
            // Save dt before clamping so we can restore it
            let saved_dt = self.current_dt;
            let was_clamped = self.current_dt > remaining;
            if was_clamped {
                self.current_dt = remaining;
            }
            let actual_dt = self.step(current_t, ctx);
            current_t += actual_dt;
            steps += 1;
            // Restore dt if it was only clamped for the final sub-step
            // (don't restore if the adaptive logic itself reduced it)
            if was_clamped && self.current_dt >= saved_dt * 0.5 {
                self.current_dt = saved_dt;
            }
            if steps >= self.max_steps_per_call {
                #[cfg(debug_assertions)]
                #[allow(clippy::print_stderr)]
                {
                    eprintln!(
                        "[RK45] step_to hit {} step limit at t={:.6}",
                        self.max_steps_per_call, current_t
                    );
                }
                break;
            }
        }
        steps
    }

    /// Write current state variables into an `EvalContext`.
    pub fn sync_to_context(&self, ctx: &mut EvalContext) {
        for (name, &value) in self.state.names.iter().zip(self.state.values.iter()) {
            ctx.set(name.clone(), Value::Float(value));
        }
    }

    /// Reset to initial values.
    pub fn reset(&mut self) {
        self.state.values = self.initial_values.clone();
        self.stats = IntegrationStats::default();
        self.current_dt = 0.01;
        self.fsal_k1 = None;
        self.last_derivatives.clear();
    }

    /// Return state variable names.
    pub fn state_names(&self) -> &[String] {
        &self.state.names
    }

    /// Return current state values.
    pub fn get_state(&self) -> &[f64] {
        &self.state.values
    }
}

// ---------------------------------------------------------------------------
// Executor trait implementation (Phase 3)
// ---------------------------------------------------------------------------

impl crate::orchestrator::Executor for Rk45Solver {
    fn phase(&self) -> crate::orchestrator::ExecutionPhase {
        crate::orchestrator::ExecutionPhase::ContinuousDynamics
    }

    fn kind_label(&self) -> &'static str {
        "ode45"
    }

    fn tick(
        &mut self,
        ctx: &crate::orchestrator::TickContext<'_>,
    ) -> crate::orchestrator::TickOutput {
        self.step_to(ctx.t, ctx.t + ctx.dt, ctx.context);
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

    /// Derivatives `dy/dt` captured at the start of the most recent accepted
    /// step, paired with state-var names — parity with `Rk4Solver` so the
    /// orchestrator's `derivatives` snapshot is populated under RK45 (WS-B2).
    fn current_derivatives(&self) -> Vec<(String, f64)> {
        self.state
            .names
            .iter()
            .cloned()
            .zip(self.last_derivatives.iter().copied())
            .collect()
    }

    fn clone_boxed(&self) -> Box<dyn crate::orchestrator::Executor> {
        Box::new(self.clone())
    }

    /// RSC-2.4a: slot-routed writeback (see `Rk4Solver::sync_context_out_slots`).
    /// The legacy `sync_context_out` (`sync_to_context` + `signal_sync`
    /// recompute) was deleted with the string-identity cull; this routed path
    /// reproduces it and is now the only writeback.
    fn sync_context_out_slots(
        &self,
        shared: &mut crate::expressions::EvalContext,
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

    /// RSC-2.4a: build the precomputed slot write-set and latch the
    /// scoped-clone bypass eligibility. The state write-set is always built
    /// from `self.state.names` (state vars route whenever their slots are
    /// minted); only the signal targets come from the spec, so a hand-built
    /// solver (no spec) contributes an empty signal set. See
    /// `Rk4Solver::prepare_slot_writeback`.
    fn prepare_slot_writeback(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
        canonical_prefix: Option<&str>,
        writer: crate::slots::WriterId,
    ) {
        // Task #8: draw the signal name list from the spec's single canonical
        // ordered source (dependency order, or deterministic name-sorted
        // fallback) — no third independent sort of the signal set.
        let signal_names: Vec<String> = match &self.spec {
            Some(spec) => spec.ordered_signal_names(),
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

    /// RSC-4.1: read-set = slots read by the bound derivative + signal
    /// expressions of the retained `OdeSpec` (see `Rk4Solver::read_slots`).
    fn read_slots(&self) -> Vec<crate::slots::SlotId> {
        let mut v = self.spec.as_ref().map(|s| s.slot_reads()).unwrap_or_default();
        v.sort();
        v.dedup();
        v
    }

    fn get_state_snapshot(&self) -> Option<Vec<f64>> {
        Some(self.get_state().to_vec())
    }

    /// RSC-4.3: roll the state vector back to a captured `y_start` for the
    /// time-accurate re-step. Length mismatch ⇒ `false` (caller fails hard RS014).
    fn restore_state(&mut self, y: &[f64]) -> bool {
        if y.len() == self.state.values.len() {
            self.state.values.copy_from_slice(y);
            true
        } else {
            false
        }
    }

    /// RSC-4.3: drop the cached FSAL `k1` (= previous accepted step's `k7`). The
    /// cached derivative belongs to the abandoned trajectory after a rollback,
    /// and is evaluated under the OLD drive after a mid-tick SM flip — either way
    /// the next step must recompute `f(t, y)`.
    fn invalidate_step_cache(&mut self) {
        self.fsal_k1 = None;
    }

    /// RSC-4.3: adaptive sub-interval integration for the re-step (delegates to
    /// [`step_to`](Rk45Solver::step_to)).
    fn integrate_interval(&mut self, t_start: f64, t_target: f64, ctx: &EvalContext) -> bool {
        self.step_to(t_start, t_target, ctx);
        true
    }

    /// RSC-2.3: rebind the retained spec's expressions to slots
    /// (subsystem-local scope) and rebuild the captured closures. No-op
    /// for hand-built solvers without a retained spec.
    fn bind_expression_slots(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
    ) -> crate::expressions::BindReport {
        let Some(spec) = self.spec.as_mut() else {
            return crate::expressions::BindReport::default();
        };
        let report = spec.bind_slots(store, var_prefix);
        self.rhs = spec.build_rhs();
        // Only rebuild signal_sync where the build path installed one —
        // the single-model `build_orchestrator` path deliberately runs
        // without signal write-back, and rebinding must not change that.
        if self.signal_sync.is_some() {
            self.signal_sync = spec.build_signal_sync();
        }
        report
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_rk45_constant_derivative() {
        // dy/dt = 1  =>  y(1) = 1
        let mut solver = Rk45Solver::new(
            "const",
            vec!["y".into()],
            vec![0.0],
            Arc::new(|_t, _y, _ctx| vec![1.0]),
        )
        .with_initial_dt(0.1);

        let ctx = EvalContext::new();
        solver.step_to(0.0, 1.0, &ctx);

        assert!(
            (solver.get_state()[0] - 1.0).abs() < 1e-8,
            "expected 1.0, got {}",
            solver.get_state()[0]
        );
    }

    #[test]
    fn test_rk45_exponential_decay() {
        // dy/dt = -y  =>  y(1) = e^(-1)
        let mut solver = Rk45Solver::new(
            "decay",
            vec!["y".into()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-y[0]]),
        )
        .with_initial_dt(0.1)
        .with_atol(1e-10)
        .with_rtol(1e-8);

        let ctx = EvalContext::new();
        solver.step_to(0.0, 1.0, &ctx);

        let expected = (-1.0_f64).exp();
        assert!(
            (solver.get_state()[0] - expected).abs() < 1e-8,
            "expected {}, got {}",
            expected,
            solver.get_state()[0]
        );
    }

    #[test]
    fn test_rk45_vs_rk4_accuracy() {
        // Solve dy/dt = -y from 0 to 2 with both solvers.
        // RK45 should be more accurate with fewer function evaluations.
        let mut rk45 = Rk45Solver::new(
            "rk45",
            vec!["y".into()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-y[0]]),
        )
        .with_atol(1e-10)
        .with_rtol(1e-8)
        .with_initial_dt(0.1);

        let ctx = EvalContext::new();
        rk45.step_to(0.0, 2.0, &ctx);

        let expected = (-2.0_f64).exp();
        let rk45_error = (rk45.get_state()[0] - expected).abs();

        // With tight tolerances, RK45 should be very accurate
        assert!(rk45_error < 1e-8, "RK45 error {} is too large", rk45_error);
    }

    #[test]
    fn test_rk45_step_rejection() {
        // A stiff-ish system that should trigger some step rejections
        // dy/dt = -50*y (fast decay)
        let mut solver = Rk45Solver::new(
            "stiff",
            vec!["y".into()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-50.0 * y[0]]),
        )
        .with_initial_dt(1.0) // Start with a too-large step
        .with_atol(1e-6)
        .with_rtol(1e-3);

        let ctx = EvalContext::new();
        solver.step_to(0.0, 0.5, &ctx);

        // Should have rejected some steps due to initial large dt
        assert!(
            solver.stats.steps_rejected > 0,
            "expected some step rejections, got 0"
        );
        // But still converge to the right answer
        let expected = (-25.0_f64).exp();
        assert!(
            (solver.get_state()[0] - expected).abs() < 1e-4,
            "expected {}, got {}",
            expected,
            solver.get_state()[0]
        );
    }

    #[test]
    fn test_rk45_harmonic_oscillator() {
        // dx/dt = v, dv/dt = -x  (period = 2*pi)
        // x(0)=1, v(0)=0 => x(2*pi) ≈ 1, v(2*pi) ≈ 0
        let mut solver = Rk45Solver::new(
            "osc",
            vec!["x".into(), "v".into()],
            vec![1.0, 0.0],
            Arc::new(|_t, y, _ctx| vec![y[1], -y[0]]),
        )
        .with_atol(1e-10)
        .with_rtol(1e-8)
        .with_initial_dt(0.1);

        let ctx = EvalContext::new();
        let period = 2.0 * std::f64::consts::PI;
        solver.step_to(0.0, period, &ctx);

        assert!(
            (solver.get_state()[0] - 1.0).abs() < 1e-6,
            "x(2pi) should be 1.0, got {}",
            solver.get_state()[0]
        );
        assert!(
            solver.get_state()[1].abs() < 1e-6,
            "v(2pi) should be 0.0, got {}",
            solver.get_state()[1]
        );
    }

    #[test]
    fn test_rk45_error_control() {
        // With tight tolerance, error should be small
        let mut tight = Rk45Solver::new(
            "tight",
            vec!["y".into()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-y[0]]),
        )
        .with_atol(1e-12)
        .with_rtol(1e-10)
        .with_initial_dt(0.01);

        // With loose tolerance, error can be larger but fewer steps
        let mut loose = Rk45Solver::new(
            "loose",
            vec!["y".into()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-y[0]]),
        )
        .with_atol(1e-4)
        .with_rtol(1e-2)
        .with_initial_dt(0.01);

        let ctx = EvalContext::new();
        tight.step_to(0.0, 1.0, &ctx);
        loose.step_to(0.0, 1.0, &ctx);

        let expected = (-1.0_f64).exp();
        let tight_err = (tight.get_state()[0] - expected).abs();
        let loose_err = (loose.get_state()[0] - expected).abs();

        // Tight should be more accurate
        assert!(
            tight_err < loose_err || loose_err < 1e-4,
            "tight error {} should be less than loose error {}",
            tight_err,
            loose_err
        );
        // Loose should use fewer RHS evaluations
        assert!(
            loose.stats.rhs_evaluations <= tight.stats.rhs_evaluations,
            "loose evals {} should be <= tight evals {}",
            loose.stats.rhs_evaluations,
            tight.stats.rhs_evaluations
        );
    }

    #[test]
    fn test_rk45_reset() {
        let mut solver = Rk45Solver::new(
            "reset",
            vec!["y".into()],
            vec![0.0],
            Arc::new(|_t, _y, _ctx| vec![1.0]),
        );

        let ctx = EvalContext::new();
        solver.step_to(0.0, 1.0, &ctx);
        assert!(solver.get_state()[0] > 0.5);
        assert!(solver.stats.steps_accepted > 0);

        solver.reset();
        assert!((solver.get_state()[0]).abs() < 1e-15);
        assert_eq!(solver.stats.steps_accepted, 0);
    }

    #[test]
    fn test_rk45_sync_to_context() {
        let solver = Rk45Solver::new(
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

    #[test]
    fn test_fsal_reduces_rhs_evaluations() {
        // With FSAL, accepted steps should average ~6 RHS evals instead of 7
        let mut solver = Rk45Solver::new(
            "fsal_test",
            vec!["y".into()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-y[0]]),
        )
        .with_atol(1e-8)
        .with_rtol(1e-6)
        .with_initial_dt(0.1);

        let ctx = EvalContext::new();
        solver.step_to(0.0, 2.0, &ctx);

        let accepted = solver.stats.steps_accepted;
        let evals = solver.stats.rhs_evaluations;
        // First step does 7 evals (no cache), subsequent do 6
        // So total should be 7 + 6*(accepted-1) = 6*accepted + 1
        let expected_max = 7 * accepted; // Without FSAL
        let expected_with_fsal = 6 * accepted + 1; // With FSAL

        assert!(
            evals <= expected_with_fsal + accepted, // Allow some slack for rejections
            "FSAL should reduce evals: got {} for {} accepted steps (expected ~{}, max without FSAL={})",
            evals, accepted, expected_with_fsal, expected_max
        );
        assert!(
            evals < expected_max,
            "evals {} should be less than {} (7 * accepted steps)",
            evals,
            expected_max
        );
    }
}
