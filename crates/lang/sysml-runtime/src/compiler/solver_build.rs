//! Solver assembly / selection and the simulation entry point.

use std::collections::HashMap;

use sysml_core::Value;

use crate::expressions::{EvalContext, ExprIR};
use crate::ode::Rk4Solver;
use crate::ode_builder::{self, OdeSpec};
use crate::orchestrator::Orchestrator;

use super::*;

impl ModelCompiler {
    /// Assemble the [`OdeSpec`] shared by the three solver builders
    /// (`build_ode_solver_with_config`/`build_ode45_solver`/`build_bdf_solver`)
    /// and the WS-B4 auto-switch classifier. Compiles derivative + signal
    /// expressions, folds in parameters (with overrides) and config Maps. The
    /// only difference between the builders is the concrete solver type wrapped
    /// around this spec — so the assembly lives in one place (no duplicate
    /// paths).
    fn assemble_ode_spec(
        ode: &OdeDetection,
        override_map: &HashMap<&str, f64>,
        config_maps: &[(String, Value)],
    ) -> Result<OdeSpec, CompileError> {
        let compiled_derivs: Vec<ExprIR> = ode
            .derivative_exprs
            .iter()
            .map(|expr_str| {
                ode_builder::parse_derivative(expr_str).map_err(|e| {
                    CompileError::from_message(format!("derivative compile error: {e}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let mut spec = OdeSpec::new();
        for (i, var_name) in ode.state_vars.iter().enumerate() {
            spec = spec.with_state_var(
                var_name.clone(),
                ode.initial_values[i],
                compiled_derivs[i].clone(),
            );
        }
        for (k, v) in &ode.parameters {
            let val = override_map.get(k.as_str()).copied().unwrap_or(*v);
            spec = spec.with_param(k.clone(), val);
        }
        // Compile signal expressions (best-effort — skip failures).
        for (sig_name, sig_expr_str) in &ode.signal_exprs {
            if let Ok(ir) = ode_builder::parse_derivative(sig_expr_str) {
                spec = spec.with_signal(sig_name.clone(), ir);
            }
        }
        // Inject config Maps so FeatureChain expressions resolve correctly.
        for (name, val) in config_maps {
            spec = spec.with_context_value(name.clone(), val.clone());
        }
        // Task #8: fix the canonical dependency evaluation order of the signal
        // expressions ONCE, before any RHS/signal-sync closure captures it — a
        // cyclic definition is a hard CompileError.
        spec.compute_signal_order()?;
        Ok(spec)
    }

    /// WS-B4: decide whether an UN-annotated ODE should integrate with the
    /// implicit (BDF) solver because it is stiff at the integration step, vs the
    /// explicit RK45 default. SPEC-SILENT sanctioned extension (steward ruling).
    /// Emits the mandatory non-silent solver-choice / Jacobian-failure
    /// diagnostic. `ctx` should be the seeded orchestrator context so the RHS
    /// resolves the same free variables / sampled functions it will see at run
    /// time; `dt_seconds` is the orchestrator step in seconds (the ODE's time
    /// unit). Returns `true` to route to BDF.
    pub(crate) fn auto_select_stiff(
        ode_label: &str,
        ode: &OdeDetection,
        override_map: &HashMap<&str, f64>,
        config_maps: &[(String, Value)],
        ctx: &EvalContext,
        dt_seconds: f64,
    ) -> bool {
        let Some(verdict) =
            Self::classify_stiffness_verdict(ode, override_map, config_maps, ctx, dt_seconds)
        else {
            return false;
        };
        Self::log_auto_solver_choice(ode_label, &verdict);
        verdict.is_stiff
    }

    /// RSC-4.3 (L47): the WS-B4 stiffness verdict, without logging — factored
    /// out of [`Self::auto_select_stiff`] so the restep-eligible pin path
    /// (`build_workspace_orchestrator`) can obtain the SAME verdict for its
    /// own, differently-worded diagnostic
    /// ([`Self::log_restep_pin_over_stiffness`]) without also emitting the
    /// "auto-selected implicit BDF" wording — which would be a lie once RK45
    /// is pinned instead.
    pub(crate) fn classify_stiffness_verdict(
        ode: &OdeDetection,
        override_map: &HashMap<&str, f64>,
        config_maps: &[(String, Value)],
        ctx: &EvalContext,
        dt_seconds: f64,
    ) -> Option<crate::solvers::bdf::StiffnessVerdict> {
        // If the spec cannot be assembled, the subsequent build_*_solver call
        // surfaces the compile error; default to "not stiff" here.
        let spec = Self::assemble_ode_spec(ode, override_map, config_maps).ok()?;
        let rhs = spec.build_rhs();
        Some(crate::solvers::bdf::classify_stiffness_at_state(
            rhs.as_ref(),
            0.0,
            &ode.initial_values,
            ctx,
            dt_seconds,
        ))
    }

    /// RSC-4.3 (L47): loud, non-blocking diagnostic when a restep-eligible
    /// ODE would have been classified stiff (⇒ BDF) by WS-B4, but RK45 is
    /// pinned instead — restep-eligibility structurally requires RK45 in
    /// Wave 1 (only `Rk45Solver`/`Rk4Solver` implement `restore_state`/
    /// `integrate_interval`; `BdfSolver`'s multistep history makes true
    /// rollback FORBIDDEN new solver machinery, Q9). This is never a
    /// failure — RK45's `dt_min` floor still converges, just at a
    /// performance cost for genuinely stiff dynamics at coarse dt — so it
    /// must not error or panic, only inform.
    pub(crate) fn log_restep_pin_over_stiffness(ode_label: &str, verdict: &crate::solvers::bdf::StiffnessVerdict) {
        #[cfg(feature = "tracing")]
        {
            if verdict.is_stiff {
                tracing::warn!(
                    ode = %ode_label,
                    stiffness_index = verdict.stiffness_index,
                    spectral_radius = verdict.spectral_radius,
                    "RSC-4.3 (L47): WS-B4 classified this ODE stiff (would auto-select \
                     implicit BDF), but it is restep-eligible (a paired state machine \
                     has a qualifying `accept when` crossing trigger) — pinning RK45 \
                     instead. BDF cannot support the time-accurate mid-tick re-step \
                     (its multistep history cannot be rolled back, out of Wave-1 \
                     scope). RK45 will still converge (dt_min floor) but may take many \
                     small adaptive substeps at this dt."
                );
            }
        }
        #[cfg(not(feature = "tracing"))]
        {
            let _ = (ode_label, verdict);
        }
    }

    /// Emit the WS-B4 solver-choice diagnostic. Auto-selection must never be
    /// silent (steward ruling): the chosen solver is always observable as the
    /// ODE subsystem's `kind` ("bdf"/"ode45") in the execution snapshot, and —
    /// when the optional `tracing` feature is on — additionally logged here:
    /// Jacobian-failure routes to BDF at `warn`, a stiff classification at
    /// `info`, the non-stiff RK45 default at `debug`.
    pub(crate) fn log_auto_solver_choice(ode_label: &str, verdict: &crate::solvers::bdf::StiffnessVerdict) {
        #[cfg(feature = "tracing")]
        {
            if verdict.jacobian_failed {
                tracing::warn!(
                    ode = %ode_label,
                    "WS-B4 auto-stiffness: Jacobian eval failed at t=0; routing to robust implicit \
                     BDF (annotate @ToolExecution {{ toolName }} to override)"
                );
            } else if verdict.is_stiff {
                tracing::info!(
                    ode = %ode_label,
                    stiffness_index = verdict.stiffness_index,
                    spectral_radius = verdict.spectral_radius,
                    "WS-B4 auto-selected implicit BDF: detected stiff dynamics \
                     (|λ|·dt exceeds the explicit-stability margin)"
                );
            } else {
                tracing::debug!(
                    ode = %ode_label,
                    stiffness_index = verdict.stiffness_index,
                    "WS-B4 auto-selected RK45: non-stiff at this step"
                );
            }
        }
        #[cfg(not(feature = "tracing"))]
        {
            let _ = (ode_label, verdict);
        }
    }

    pub(crate) fn build_ode_solver_with_config(
        label: &str,
        ode: &OdeDetection,
        override_map: &HashMap<&str, f64>,
        config_maps: &[(String, Value)],
    ) -> Result<Rk4Solver, CompileError> {
        let spec = Self::assemble_ode_spec(ode, override_map, config_maps)?;

        let signal_sync = spec.build_signal_sync();
        let rhs = spec.build_rhs();
        let mut solver = Rk4Solver::new(
            label,
            ode.state_vars.clone(),
            ode.initial_values.clone(),
            rhs,
        );
        if let Some(sync_fn) = signal_sync {
            solver = solver.with_signal_sync(sync_fn);
        }
        // RSC-2.3: retain the spec so the orchestrator's slot-binding pass
        // can rebind the expressions and rebuild the captured closures.
        Ok(solver.with_spec(spec))
    }

    /// Build an RK45 adaptive solver from an `OdeDetection`.
    pub(crate) fn build_ode45_solver(
        label: &str,
        ode: &OdeDetection,
        override_map: &HashMap<&str, f64>,
        config_maps: &[(String, Value)],
    ) -> Result<crate::ode45::Rk45Solver, CompileError> {
        let spec = Self::assemble_ode_spec(ode, override_map, config_maps)?;

        let signal_sync = spec.build_signal_sync();
        let rhs = spec.build_rhs();
        let mut solver = crate::ode45::Rk45Solver::new(
            label,
            ode.state_vars.clone(),
            ode.initial_values.clone(),
            rhs,
        );
        if let Some(sync_fn) = signal_sync {
            solver = solver.with_signal_sync(sync_fn);
        }
        // RSC-2.3: retain the spec so the orchestrator's slot-binding pass
        // can rebind the expressions and rebuild the captured closures.
        Ok(solver.with_spec(spec))
    }

    /// Build an implicit BDF solver for a stiff ODE detection. Mirrors
    /// [`Self::build_ode45_solver`] — same `OdeSpec` assembly + signal sync +
    /// retained spec (RSC-2.3) — differing only in the concrete solver type.
    /// Reachable via `@ToolExecution { toolName = "builtin:ode-bdf" }`
    /// (`OdeDetection::is_bdf`).
    pub(crate) fn build_bdf_solver(
        label: &str,
        ode: &OdeDetection,
        override_map: &HashMap<&str, f64>,
        config_maps: &[(String, Value)],
    ) -> Result<crate::solvers::bdf::BdfSolver, CompileError> {
        let spec = Self::assemble_ode_spec(ode, override_map, config_maps)?;

        let signal_sync = spec.build_signal_sync();
        let rhs = spec.build_rhs();
        let mut solver = crate::solvers::bdf::BdfSolver::new(
            label,
            ode.state_vars.clone(),
            ode.initial_values.clone(),
            rhs,
        );
        if let Some(sync_fn) = signal_sync {
            solver = solver.with_signal_sync(sync_fn);
        }
        Ok(solver.with_spec(spec))
    }

    /// Build an orchestrator, run it to completion, and return both.
    ///
    /// Convenience method composing [`build_orchestrator`] + [`Orchestrator::run_to_completion`].
    /// Used by analysis patterns (sweep, verify) that just need
    /// the final snapshot.
    pub fn run_simulation(
        &self,
        sm_name: &str,
        overrides: &[(String, String)],
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<(Orchestrator, Option<crate::orchestrator::ExecutionSnapshot>), CompileError> {
        let mut orch = self.build_orchestrator(sm_name, overrides, dt_ms, max_time_ms)?;
        let snap = orch.run_to_completion();
        Ok((orch, snap))
    }

    // `build_orchestrator_explicit` was removed in the execution-entry
    // unification arc (execution-entry-unification-plan.md P5). It existed only
    // to serve `simulate.continuous.start`, which took ODE derivative/signal
    // expressions as raw strings from the caller and carried a legacy hardcoded
    // thermal-body fallback — model-bypassing invented semantics (principle #3).
    // Model-driven ODE orchestration goes through `build_orchestrator`
    // (auto-detected single-SM dynamics) and `build_workspace_orchestrator`.
}
