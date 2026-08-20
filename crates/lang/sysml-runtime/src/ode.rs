//! # ODE Solver for Continuous-Time Simulation (Phase 15)
//!
//! Provides a classic 4th-order Runge-Kutta (RK4) integrator for solving
//! systems of ordinary differential equations arising from SysML v2
//! continuous-time dynamics (e.g., thermal models, mechanical systems).
//!
//! The solver operates on a flat state vector and writes results back to
//! an [`EvalContext`] so that constraint evaluation and state machine
//! guards can observe continuously-evolving quantities.

#![allow(clippy::indexing_slicing)]
use std::sync::Arc;

use crate::expressions::EvalContext;
use sysml_core::Value;

/// State vector with named variables.
#[derive(Debug, Clone)]
pub struct OdeState {
    /// Variable names (e.g., `["temperature", "pressure"]`).
    pub names: Vec<String>,
    /// Current values of the state variables.
    pub values: Vec<f64>,
}

// ---------------------------------------------------------------------------
// RSC-2.4a — precomputed slot write-set (ODE executor cutover)
// ---------------------------------------------------------------------------

use crate::slots::WriteRoute;

/// RSC-4.2: which context an ODE executor evaluates its signal expressions
/// against during a slot-routed writeback. Replaces the implicit
/// `OdeWriteSet::prefixed()` context-position signal so the two execution
/// paths can share one slot-routed writeback.
///
/// - [`FreshState`](Self::FreshState) — the main per-phase loop. Signals are
///   evaluated pre-coupling, off the just-integrated state only: a prefixed
///   executor sees a states-only temp snapshot, an unprefixed one the full
///   shared context (the legacy `sync_context_out_slots` shape, preserved
///   byte-for-byte).
/// - [`FullAccumulated`](Self::FullAccumulated) — the convergence loop. Signals
///   are evaluated against the FULL master context, which in the convergence
///   position already carries this iteration's computed-expr + peer writes
///   (that coupling IS the fixpoint). Reproduces legacy `sync_context_out`'s
///   `shared.clone()` semantics while still routing the results by `SlotId`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalEvalMode {
    /// Pre-coupling: signals off just-integrated state (main per-phase loop).
    FreshState,
    /// Post-coupling fixpoint: signals off the full accumulated master context
    /// (convergence loop).
    FullAccumulated,
}

/// Precomputed write-set of one ODE executor: state-index → route, plus
/// signal-target → route (RSC-2.4a).
#[derive(Debug, Clone)]
pub(crate) struct OdeWriteSet {
    /// Parallel to the solver's state vector.
    states: Vec<WriteRoute>,
    /// `(bare signal name, route)` for every compile-known signal target.
    signals: Vec<(String, WriteRoute)>,
    /// Whether the owning subsystem is instance-prefixed. Preserves the
    /// legacy signal-eval snapshot shape: prefixed executors evaluated
    /// signals against a states-only temp context; unprefixed ones against
    /// the full shared context.
    prefixed: bool,
}

impl OdeWriteSet {
    /// Write the state vector through the precomputed routes.
    pub(crate) fn write_states(&self, shared: &mut EvalContext, values: &[f64]) {
        debug_assert_eq!(
            self.states.len(),
            values.len(),
            "ODE write-set is parallel to the state vector"
        );
        for (route, &v) in self.states.iter().zip(values.iter()) {
            route.apply(shared, Value::Float(v));
        }
    }

    /// Route every signal value present in `evaluated` (signals whose
    /// evaluation failed are absent — exactly the keys the legacy temp loop
    /// would have written).
    ///
    /// Signals are `GetOutput` derived quantities (e.g. `kinetic_energy`,
    /// `i_drive`). RSC-3.0 category 2: the compiler now mints a signal-output
    /// slot for each (`mint_ode_signal_slot`, owner = this ODE executor), so
    /// these routes resolve to claimed slots and write by `SlotId` through the
    /// strict [`apply`](crate::slots::WriteRoute::apply) — identically to state
    /// vars in `write_states`. An unrouted signal reaching here is now a mint
    /// gap that hard-errors in `apply` (principle 1), not a silent name-key.
    pub(crate) fn write_signals(&self, shared: &mut EvalContext, evaluated: &EvalContext) {
        for (name, route) in &self.signals {
            if let Some(v) = evaluated.get(name) {
                route.apply(shared, v.clone());
            }
        }
    }

    /// Whether the owning subsystem is instance-prefixed.
    pub(crate) fn prefixed(&self) -> bool {
        self.prefixed
    }

    /// A2 / RS005: the runtime keys of strict-`apply` routes (state vectors and
    /// signal outputs) that failed to mint a slot. Non-empty means a mint gap
    /// the compiler must hard-fail — `write_states` / `write_signals` would
    /// otherwise silently drop these in release. Empty on the corpus (every
    /// state var and category-2 signal mints a claimed slot).
    pub(crate) fn unrouted_writes(&self) -> Vec<String> {
        let mut out = Vec::new();
        for route in &self.states {
            if !route.is_routed() {
                out.push(route.runtime_key().to_owned());
            }
        }
        for (_, route) in &self.signals {
            if !route.is_routed() {
                out.push(route.runtime_key().to_owned());
            }
        }
        out
    }
}

/// RSC-4.2: evaluate an ODE executor's signal expressions and route the
/// results through its slot write-set, choosing the eval context per
/// [`SignalEvalMode`]. Shared by the RK4 and RK45 executors so the two
/// slot-routed writebacks cannot drift (CLAUDE.md principle 4/5).
///
/// `write_states_into` populates a fresh states-only context (the executor's
/// `sync_to_context`) — used verbatim in [`SignalEvalMode::FreshState`] for a
/// prefixed executor, and as the write target `out` in all cases (the routed
/// `write_signals` reads only the keys present in `out`, exactly the keys the
/// legacy temp loop wrote).
pub(crate) fn eval_signals_slot_routed(
    ws: &OdeWriteSet,
    sync_fn: &crate::ode45::SignalSyncFn,
    state: &OdeState,
    shared: &mut EvalContext,
    mode: SignalEvalMode,
    write_states_into: impl FnOnce(&mut EvalContext),
) {
    let mut out = EvalContext::new();
    write_states_into(&mut out);
    // Which context the signal expressions read from:
    //  - FreshState + prefixed: the states-only `out` snapshot (pre-coupling,
    //    the legacy `sync_context_out_slots` prefixed-branch shape).
    //  - FreshState + unprefixed: the full shared context (the legacy
    //    unprefixed shape).
    //  - FullAccumulated: the full shared context regardless of prefix — the
    //    convergence-loop coupling that legacy `sync_context_out` used via
    //    `shared.clone()`.
    let use_states_only = matches!(mode, SignalEvalMode::FreshState) && ws.prefixed();
    if use_states_only {
        // Cull-arc W3: alias_live — the snapshot is a read-only input to
        // `sync_fn` (never written), so this preserves the exact pre-cull
        // `.clone()` with zero write-plane effect.
        let snapshot = out.alias_live();
        sync_fn(&state.values, &snapshot, &mut out);
    } else {
        let snapshot = shared.alias_live();
        sync_fn(&state.values, &snapshot, &mut out);
    }
    ws.write_signals(shared, &out);
}

/// Build an ODE executor's write-set against the compile-minted slot table
/// (RSC-2.4a). Called by `Executor::prepare_slot_writeback` after the
/// RSC-2.3 bind pass.
///
/// A key gets a slot route only when ALL of:
/// - the legacy runtime key (`{prefix}.{key}` / bare) resolves to a slot,
/// - the slot's `runtime_name`/`canonical_name` spellings are byte-identical
///   to the keys the legacy string loop would write (so the legacy map ends
///   the tick with exactly the same key set), and
/// - the slot's minted writer is `expected_writer` — the single-writer
///   promise (design doc D-2.0.3) live for the first migrated kind. A slot
///   owned by a DIFFERENT executor trips the debug assertion; orchestrator
///   placeholder writers (mint-time name-lookup misses) quietly keep the
///   name-keyed fallback.
pub(crate) fn build_ode_write_set(
    store: &crate::slots::SlotStore,
    var_prefix: Option<&str>,
    canonical_prefix: Option<&str>,
    expected_writer: crate::slots::WriterId,
    state_names: &[String],
    signal_names: impl IntoIterator<Item = String>,
) -> OdeWriteSet {
    // Name/writer matching + the single-writer debug assertion live in
    // `WriteRoute::resolve` (shared with the RSC-2.4b SM write-set).
    let route_for = |key: &str| -> WriteRoute {
        WriteRoute::resolve(store, var_prefix, canonical_prefix, expected_writer, key)
    };

    OdeWriteSet {
        states: state_names.iter().map(|n| route_for(n)).collect(),
        signals: signal_names
            .into_iter()
            .map(|n| {
                let route = route_for(&n);
                (n, route)
            })
            .collect(),
        prefixed: var_prefix.is_some(),
    }
}

/// Right-hand side function: `f(t, y, ctx) -> dy/dt`.
///
/// The closure receives the current time `t`, state vector `y`, and an
/// [`EvalContext`] for reading external parameters (e.g., ambient temperature,
/// applied power). It returns a vector of derivatives, one per state variable.
///
/// Wrapped in `Arc` so solvers can be cloned (required by `Orchestrator::fork`).
pub type OdeRhs = Arc<dyn Fn(f64, &[f64], &EvalContext) -> Vec<f64> + Send + Sync>;

/// A 4th-order Runge-Kutta ODE solver.
#[derive(Clone)]
pub struct Rk4Solver {
    /// Human-readable name for this solver instance.
    pub name: String,
    /// Current state (names + values).
    pub state: OdeState,
    /// Saved initial values for [`reset`](Self::reset).
    initial_values: Vec<f64>,
    /// The RHS function `dy/dt = f(t, y, ctx)`.
    rhs: OdeRhs,
    /// Optional signal sync — writes signal expression results to shared context.
    signal_sync: Option<crate::ode45::SignalSyncFn>,
    /// Derivatives sampled at the START of the most recent `step()` call
    /// (i.e. `k1` in the RK4 stencil — the instantaneous `dy/dt` at the
    /// tick boundary). Kept separately from `state.values` so the
    /// snapshot can surface both `y` (state) and `dy/dt` (derivative)
    /// for OdeDetail without re-evaluating the RHS. Empty before the
    /// first step. Closes GAP-ODE-002.
    last_derivatives: Vec<f64>,
    /// RSC-2.3: the `OdeSpec` this solver's closures were built from, when
    /// the `ModelCompiler` built it (see [`with_spec`](Self::with_spec)).
    /// Retained so `Executor::bind_expression_slots` can rebind the spec's
    /// expressions to slots and rebuild `rhs` / `signal_sync` after the
    /// slot table is minted. `None` for hand-built solvers (tests, physics
    /// closures) — those have no compiler-known expressions to bind.
    spec: Option<crate::ode_builder::OdeSpec>,
    /// RSC-2.4a: precomputed slot write-set (state-index → slot + signal
    /// targets), installed by `Executor::prepare_slot_writeback`. `None`
    /// for hand-built solvers — those keep the legacy writeback.
    write_set: Option<OdeWriteSet>,
    /// RSC-2.4a: every read in the retained spec is servable without the
    /// orchestrator's scoped-context clone (`OdeSpec::scoped_bypass_eligible`,
    /// latched at prepare time).
    bypass_scoped: bool,
}

/// Element-wise `base + scale * delta`.
fn add_scaled(base: &[f64], delta: &[f64], scale: f64) -> Vec<f64> {
    base.iter()
        .zip(delta.iter())
        .map(|(b, d)| b + scale * d)
        .collect()
}

impl Rk4Solver {
    /// Create a new RK4 solver.
    ///
    /// # Arguments
    ///
    /// * `name` — identifier for this solver (used in diagnostics / logging)
    /// * `state_names` — names for each state variable
    /// * `initial_values` — starting values (must match `state_names` length)
    /// * `rhs` — the derivative function `f(t, y, ctx) -> dy/dt`
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
        Self {
            name: name.into(),
            state: OdeState {
                names: state_names,
                values: initial_values.clone(),
            },
            initial_values,
            rhs,
            signal_sync: None,
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

    /// Perform one RK4 integration step.
    ///
    /// Advances the state from time `t` to `t + dt` using the classic
    /// four-stage Runge-Kutta method:
    ///
    /// ```text
    /// k1 = f(t,          y)
    /// k2 = f(t + dt/2,   y + dt/2 * k1)
    /// k3 = f(t + dt/2,   y + dt/2 * k2)
    /// k4 = f(t + dt,     y + dt   * k3)
    /// y_new = y + (dt/6)(k1 + 2*k2 + 2*k3 + k4)
    /// ```
    pub fn step(&mut self, t: f64, dt: f64, ctx: &EvalContext) {
        let y = &self.state.values;

        let k1 = (self.rhs)(t, y, ctx);
        // Cache the instantaneous dy/dt at the tick boundary so the
        // snapshot layer can expose it without another RHS eval.
        self.last_derivatives = k1.clone();
        let y2 = add_scaled(y, &k1, dt / 2.0);
        let k2 = (self.rhs)(t + dt / 2.0, &y2, ctx);
        let y3 = add_scaled(y, &k2, dt / 2.0);
        let k3 = (self.rhs)(t + dt / 2.0, &y3, ctx);
        let y4 = add_scaled(y, &k3, dt);
        let k4 = (self.rhs)(t + dt, &y4, ctx);

        let n = y.len();
        let mut new_values = Vec::with_capacity(n);
        for i in 0..n {
            new_values.push(y[i] + (dt / 6.0) * (k1[i] + 2.0 * k2[i] + 2.0 * k3[i] + k4[i]));
        }
        self.state.values = new_values;
    }

    /// Write current state variables into an [`EvalContext`] as [`Value::Float`].
    pub fn sync_to_context(&self, ctx: &mut EvalContext) {
        for (name, &value) in self.state.names.iter().zip(self.state.values.iter()) {
            ctx.set(name.clone(), Value::Float(value));
        }
    }

    /// Reset state to initial values.
    pub fn reset(&mut self) {
        self.state.values = self.initial_values.clone();
        self.last_derivatives.clear();
    }

    /// Return the derivatives (`dy/dt`) captured at the start of the most
    /// recent `step()` call as `(state_name, value)` pairs. Empty before
    /// the first step.
    pub fn last_derivatives(&self) -> Vec<(String, f64)> {
        self.state
            .names
            .iter()
            .zip(self.last_derivatives.iter())
            .map(|(n, d)| (n.clone(), *d))
            .collect()
    }

    /// Return the state variable names.
    pub fn state_names(&self) -> &[String] {
        &self.state.names
    }

    /// Return the current state vector.
    pub fn get_state(&self) -> &[f64] {
        &self.state.values
    }

    /// Attach a signal sync closure that writes signal expression results
    /// to the shared context during `sync_context_out`.
    pub fn with_signal_sync(mut self, sync_fn: crate::ode45::SignalSyncFn) -> Self {
        self.signal_sync = Some(sync_fn);
        self
    }
}

// ---------------------------------------------------------------------------
// Discrete State-Space Solver (Feature 6.6)
// ---------------------------------------------------------------------------

/// Update function for discrete-time systems: `f(k, x, u, ctx) -> x_next`.
///
/// Takes the current step index `k`, state vector `x`, input vector `u`,
/// and an [`EvalContext`] for reading parameters. Returns the next state vector.
pub type DiscreteUpdateFn =
    Arc<dyn Fn(usize, &[f64], &[f64], &EvalContext) -> Vec<f64> + Send + Sync>;

/// A discrete-time state-space solver.
///
/// Implements the difference equation `x[k+1] = f(k, x[k], u[k], ctx)`.
/// This handles systems that are inherently discrete (sampled controllers,
/// digital filters, difference equations) rather than continuous ODEs.
#[derive(Clone)]
pub struct DiscreteStateSolver {
    /// Human-readable name.
    pub name: String,
    /// Current state (names + values).
    pub state: OdeState,
    /// Input variable names (read from context each step).
    pub input_names: Vec<String>,
    /// Saved initial values for reset.
    initial_values: Vec<f64>,
    /// The update function `x[k+1] = f(k, x[k], u[k], ctx)`.
    update_fn: DiscreteUpdateFn,
    /// Current step index.
    pub step_index: usize,
    /// RSC-4.x: precomputed slot write-set (state-index → slot). Installed by
    /// `Executor::prepare_slot_writeback`. The string-identity cull deleted the
    /// discrete solver's legacy `sync_context_out`, so this is now its only
    /// writeback (state-only — a discrete solver has no signal expressions).
    /// `None` for a hand-built solver whose state vars were never minted as
    /// slots — it then publishes nothing, matching the pre-migration trait
    /// default (the orchestrator no longer runs a legacy fallback).
    write_set: Option<OdeWriteSet>,
}

impl DiscreteStateSolver {
    /// Create a new discrete state-space solver.
    ///
    /// # Arguments
    ///
    /// * `name` — identifier for this solver
    /// * `state_names` — names for each state variable
    /// * `initial_values` — starting state values
    /// * `input_names` — names of input variables (read from EvalContext each step)
    /// * `update_fn` — the state transition function
    pub fn new(
        name: impl Into<String>,
        state_names: Vec<String>,
        initial_values: Vec<f64>,
        input_names: Vec<String>,
        update_fn: DiscreteUpdateFn,
    ) -> Self {
        assert_eq!(
            state_names.len(),
            initial_values.len(),
            "state_names and initial_values must have the same length"
        );
        Self {
            name: name.into(),
            state: OdeState {
                names: state_names,
                values: initial_values.clone(),
            },
            input_names,
            initial_values,
            update_fn,
            step_index: 0,
            write_set: None,
        }
    }

    /// Perform one discrete step: `x[k+1] = f(k, x[k], u[k])`.
    ///
    /// Reads input values from the context, applies the update function,
    /// and advances the step index.
    pub fn step(&mut self, ctx: &EvalContext) {
        // Read input vector from context
        let u: Vec<f64> = self
            .input_names
            .iter()
            .map(|name| {
                ctx.get(name)
                    .and_then(|v| match v {
                        Value::Float(f) => Some(*f),
                        Value::Int(i) => Some(*i as f64),
                        _ => None,
                    })
                    .unwrap_or(0.0)
            })
            .collect();

        let x_next = (self.update_fn)(self.step_index, &self.state.values, &u, ctx);
        self.state.values = x_next;
        self.step_index += 1;
    }

    /// Write current state variables into an [`EvalContext`].
    pub fn sync_to_context(&self, ctx: &mut EvalContext) {
        for (name, &value) in self.state.names.iter().zip(self.state.values.iter()) {
            ctx.set(name.clone(), Value::Float(value));
        }
    }

    /// Reset state to initial values and step index to 0.
    pub fn reset(&mut self) {
        self.state.values = self.initial_values.clone();
        self.step_index = 0;
    }

    /// Return the state variable names.
    pub fn state_names(&self) -> &[String] {
        &self.state.names
    }

    /// Return the current state vector.
    pub fn get_state(&self) -> &[f64] {
        &self.state.values
    }
}

// ---------------------------------------------------------------------------
// Executor trait implementations (Phase 3)
// ---------------------------------------------------------------------------

impl crate::orchestrator::Executor for Rk4Solver {
    fn phase(&self) -> crate::orchestrator::ExecutionPhase {
        crate::orchestrator::ExecutionPhase::ContinuousDynamics
    }

    fn kind_label(&self) -> &'static str {
        "ode"
    }

    fn tick(
        &mut self,
        ctx: &crate::orchestrator::TickContext<'_>,
    ) -> crate::orchestrator::TickOutput {
        self.step(ctx.t, ctx.dt, ctx.context);
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

    /// RSC-2.4a: slot-routed writeback. The legacy `sync_context_out`
    /// (whole-context `sync_to_context` + `signal_sync` recompute) was deleted
    /// with the string-identity cull; the routed path below reproduces it
    /// (`write_states` + `eval_signals_slot_routed`) and is now the only
    /// writeback. The `set_slot` dual-spelling mirror keeps the legacy map
    /// coherent.
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

    /// RSC-2.4a: build the precomputed slot write-set and latch the
    /// scoped-clone bypass eligibility.
    ///
    /// The **state** write-set is always built from `self.state.names` — a
    /// solver's own state vars are routable whenever their slots are minted,
    /// with or without a retained spec. Only the **signal** targets come from
    /// the spec (`signal_exprs`); a hand-built solver (no spec) contributes an
    /// empty signal set. This is what lets a bare `add_ode` executor route once
    /// its state slots are minted (the compiler always supplies a spec, so this
    /// path is production-neutral there).
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
        self.write_set = Some(build_ode_write_set(
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

    /// RSC-4.1: read-set = the slots the bound derivative + signal expressions
    /// of the retained `OdeSpec` read. Empty for hand-built solvers without a
    /// retained spec.
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

    // NOTE: `invalidate_step_cache` uses the trait default no-op — the
    // fixed-step RK4 integrator keeps no FSAL cache to drop.

    /// RSC-4.3: fixed-step sub-interval integration for the re-step. Wave-1 scope
    /// is RK45 (the default solver, WS-B2); this is present so a fixed-step ODE
    /// carrying a located crossing detector does not silently no-op the re-step
    /// (which would be a spec violation, not a degraded-but-OK path). Integrates
    /// the exact `[t_start, t_target]` sub-interval as a single RK4 step.
    fn integrate_interval(&mut self, t_start: f64, t_target: f64, ctx: &EvalContext) -> bool {
        let dt = t_target - t_start;
        if dt > 0.0 {
            self.step(t_start, dt, ctx);
        }
        true
    }

    fn current_derivatives(&self) -> Vec<(String, f64)> {
        self.last_derivatives()
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

impl crate::orchestrator::Executor for DiscreteStateSolver {
    fn phase(&self) -> crate::orchestrator::ExecutionPhase {
        crate::orchestrator::ExecutionPhase::DiscreteDynamics
    }

    fn kind_label(&self) -> &'static str {
        "discrete"
    }

    fn tick(
        &mut self,
        ctx: &crate::orchestrator::TickContext<'_>,
    ) -> crate::orchestrator::TickOutput {
        self.step(ctx.context);
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

    /// RSC-4.x: slot-routed writeback. The legacy `sync_context_out`
    /// (`sync_to_context`) was deleted with the string-identity cull; this
    /// routes the state vector by `SlotId` (`set_slot` mirrors both name
    /// spellings). State-only — a discrete solver has no signal expressions.
    /// `false` (publish nothing) when the state vars were never minted as
    /// slots, matching the pre-migration trait default.
    fn sync_context_out_slots(
        &self,
        shared: &mut EvalContext,
        _mode: crate::ode::SignalEvalMode,
    ) -> bool {
        let Some(ws) = &self.write_set else {
            return false;
        };
        ws.write_states(shared, &self.state.values);
        true
    }

    /// RSC-4.x: build the state-only slot write-set. No-op when the executor's
    /// state vars resolve to no owned slot (hand-built solver) — `write_set`
    /// stays `None` and the writeback publishes nothing.
    fn prepare_slot_writeback(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
        canonical_prefix: Option<&str>,
        writer: crate::slots::WriterId,
    ) {
        self.write_set = Some(build_ode_write_set(
            store,
            var_prefix,
            canonical_prefix,
            writer,
            &self.state.names,
            std::iter::empty::<String>(),
        ));
    }

    /// A2 / RS005: report unrouted strict writes so the compile-time gate can
    /// see them.
    ///
    /// This override was missing while `prepare_slot_writeback` above was not,
    /// and the two are a pair: building strict routes without reporting the
    /// unrouted ones moves the failure from a build error the compiler
    /// explains to a `WriteRoute::apply` panic on the first tick, whose own
    /// message asserts this is "statically impossible after the RS005 gate".
    /// It was — because the gate could not see this executor.
    fn unrouted_slot_writes(&self) -> Vec<String> {
        self.write_set
            .as_ref()
            .map(|ws| ws.unrouted_writes())
            .unwrap_or_default()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    // A2 / RS005 census primitive: a state var with no minted slot yields an
    // unrouted strict route, which `unrouted_writes` surfaces (fed to the RS005
    // build gate). Against an empty store, every route is unrouted.
    #[test]
    fn ode_write_set_reports_unrouted_strict_writes() {
        let store = crate::slots::SlotStore::new();
        let ws = build_ode_write_set(
            &store,
            None,
            None,
            crate::slots::WriterId::Executor(0),
            &["x".to_string(), "y".to_string()],
            std::iter::empty(),
        );
        let unrouted = ws.unrouted_writes();
        assert_eq!(
            unrouted,
            vec!["x".to_string(), "y".to_string()],
            "an empty store mints no slots, so both state routes are unrouted mint gaps"
        );
    }

    #[test]
    fn test_rk4_constant_derivative() {
        // dy/dt = 1.0  =>  y(t) = t + y0
        let mut solver = Rk4Solver::new(
            "constant",
            vec!["y".to_string()],
            vec![0.0],
            Arc::new(|_t, _y, _ctx| vec![1.0]),
        );
        let ctx = EvalContext::new();
        solver.step(0.0, 1.0, &ctx);
        assert!((solver.get_state()[0] - 1.0).abs() < 1e-12);
    }

    #[test]
    fn test_rk4_exponential_decay() {
        // dy/dt = -y  =>  y(t) = y0 * e^(-t)
        let mut solver = Rk4Solver::new(
            "decay",
            vec!["y".to_string()],
            vec![1.0],
            Arc::new(|_t, y, _ctx| vec![-y[0]]),
        );
        let ctx = EvalContext::new();
        let dt = 0.01;
        for i in 0..100 {
            solver.step(i as f64 * dt, dt, &ctx);
        }
        let expected = (-1.0_f64).exp(); // e^(-1)
        let actual = solver.get_state()[0];
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn test_rk4_newton_cooling() {
        // dT/dt = (power - loss*(T - ambient)) / mass
        // Equilibrium: T_eq = ambient + power/loss
        let power = 100.0;
        let loss = 10.0;
        let ambient = 20.0;
        let mass = 5.0;

        let mut solver = Rk4Solver::new(
            "cooling",
            vec!["temperature".to_string()],
            vec![ambient], // start at ambient
            Arc::new(move |_t, y, _ctx| vec![(power - loss * (y[0] - ambient)) / mass]),
        );

        let ctx = EvalContext::new();
        let dt = 0.01;
        // Run for 20 simulated seconds — should converge to equilibrium
        for i in 0..2000 {
            solver.step(i as f64 * dt, dt, &ctx);
        }

        let t_eq = ambient + power / loss; // 30.0
        let actual = solver.get_state()[0];
        assert!(
            (actual - t_eq).abs() < 0.01,
            "expected convergence to {t_eq}, got {actual}"
        );
    }

    #[test]
    fn test_sync_to_context() {
        let mut solver = Rk4Solver::new(
            "sync_test",
            vec!["x".to_string(), "v".to_string()],
            vec![1.0, 2.0],
            Arc::new(|_t, _y, _ctx| vec![0.0, 0.0]),
        );
        let ctx = EvalContext::new();
        // Take a step so state is not just initial
        solver.step(0.0, 0.1, &ctx);

        let mut out_ctx = EvalContext::new();
        solver.sync_to_context(&mut out_ctx);

        assert_eq!(out_ctx.get("x"), Some(&Value::Float(1.0)));
        assert_eq!(out_ctx.get("v"), Some(&Value::Float(2.0)));
    }

    #[test]
    fn test_reset() {
        let mut solver = Rk4Solver::new(
            "reset_test",
            vec!["y".to_string()],
            vec![0.0],
            Arc::new(|_t, _y, _ctx| vec![1.0]),
        );
        let ctx = EvalContext::new();

        // Step a few times — state should change
        solver.step(0.0, 1.0, &ctx);
        solver.step(1.0, 1.0, &ctx);
        assert!((solver.get_state()[0] - 2.0).abs() < 1e-12);

        // Reset — should be back to initial
        solver.reset();
        assert!((solver.get_state()[0] - 0.0).abs() < 1e-12);
    }

    // ── Discrete state-space solver tests ──

    #[test]
    fn test_discrete_counter() {
        // x[k+1] = x[k] + 1 (simple counter)
        let mut solver = DiscreteStateSolver::new(
            "counter",
            vec!["count".to_string()],
            vec![0.0],
            vec![],
            Arc::new(|_k, x, _u, _ctx| vec![x[0] + 1.0]),
        );
        let ctx = EvalContext::new();
        solver.step(&ctx);
        assert!((solver.get_state()[0] - 1.0).abs() < 1e-12);
        solver.step(&ctx);
        assert!((solver.get_state()[0] - 2.0).abs() < 1e-12);
        solver.step(&ctx);
        assert!((solver.get_state()[0] - 3.0).abs() < 1e-12);
        assert_eq!(solver.step_index, 3);
    }

    #[test]
    fn test_discrete_with_input() {
        // x[k+1] = x[k] + u[0]  (accumulator with input)
        let mut solver = DiscreteStateSolver::new(
            "accumulator",
            vec!["total".to_string()],
            vec![0.0],
            vec!["increment".to_string()],
            Arc::new(|_k, x, u, _ctx| vec![x[0] + u[0]]),
        );
        let mut ctx = EvalContext::new();
        ctx.set("increment".to_string(), Value::Float(5.0));
        solver.step(&ctx);
        assert!((solver.get_state()[0] - 5.0).abs() < 1e-12);
        solver.step(&ctx);
        assert!((solver.get_state()[0] - 10.0).abs() < 1e-12);

        // Change input mid-run
        ctx.set("increment".to_string(), Value::Float(3.0));
        solver.step(&ctx);
        assert!((solver.get_state()[0] - 13.0).abs() < 1e-12);
    }

    #[test]
    fn test_discrete_exponential_decay() {
        // x[k+1] = 0.9 * x[k] (discrete exponential decay, alpha=0.9)
        let mut solver = DiscreteStateSolver::new(
            "decay",
            vec!["y".to_string()],
            vec![100.0],
            vec![],
            Arc::new(|_k, x, _u, _ctx| vec![0.9 * x[0]]),
        );
        let ctx = EvalContext::new();
        for _ in 0..10 {
            solver.step(&ctx);
        }
        let expected = 100.0 * 0.9_f64.powi(10); // ~34.87
        let actual = solver.get_state()[0];
        assert!(
            (actual - expected).abs() < 1e-8,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn test_discrete_two_state() {
        // 2D system: position + velocity with discrete damping
        // x1[k+1] = x1[k] + dt * x2[k]
        // x2[k+1] = x2[k] - dt * 0.5 * x2[k]  (damped)
        let dt = 0.1;
        let mut solver = DiscreteStateSolver::new(
            "mass_spring",
            vec!["position".to_string(), "velocity".to_string()],
            vec![0.0, 10.0],
            vec![],
            Arc::new(move |_k, x, _u, _ctx| vec![x[0] + dt * x[1], x[1] - dt * 0.5 * x[1]]),
        );
        let ctx = EvalContext::new();
        for _ in 0..100 {
            solver.step(&ctx);
        }
        // Velocity should decay toward 0
        assert!(
            solver.get_state()[1].abs() < 1.0,
            "velocity should decay, got {}",
            solver.get_state()[1]
        );
        // Position should have increased
        assert!(
            solver.get_state()[0] > 5.0,
            "position should increase, got {}",
            solver.get_state()[0]
        );
    }

    #[test]
    fn test_discrete_sync_to_context() {
        let mut solver = DiscreteStateSolver::new(
            "sync_test",
            vec!["a".to_string(), "b".to_string()],
            vec![1.0, 2.0],
            vec![],
            Arc::new(|_k, x, _u, _ctx| vec![x[0] + 1.0, x[1] + 1.0]),
        );
        let ctx = EvalContext::new();
        solver.step(&ctx);

        let mut out_ctx = EvalContext::new();
        solver.sync_to_context(&mut out_ctx);
        assert_eq!(out_ctx.get("a"), Some(&Value::Float(2.0)));
        assert_eq!(out_ctx.get("b"), Some(&Value::Float(3.0)));
    }

    #[test]
    fn test_discrete_reset() {
        let mut solver = DiscreteStateSolver::new(
            "reset_test",
            vec!["y".to_string()],
            vec![0.0],
            vec![],
            Arc::new(|_k, x, _u, _ctx| vec![x[0] + 1.0]),
        );
        let ctx = EvalContext::new();
        solver.step(&ctx);
        solver.step(&ctx);
        assert!((solver.get_state()[0] - 2.0).abs() < 1e-12);
        assert_eq!(solver.step_index, 2);

        solver.reset();
        assert!((solver.get_state()[0] - 0.0).abs() < 1e-12);
        assert_eq!(solver.step_index, 0);
    }

    #[test]
    fn test_discrete_step_dependent() {
        // x[k+1] = k (state tracks step index)
        let mut solver = DiscreteStateSolver::new(
            "step_dep",
            vec!["y".to_string()],
            vec![0.0],
            vec![],
            Arc::new(|k, _x, _u, _ctx| vec![k as f64]),
        );
        let ctx = EvalContext::new();
        solver.step(&ctx); // k=0 -> y=0
        assert!((solver.get_state()[0] - 0.0).abs() < 1e-12);
        solver.step(&ctx); // k=1 -> y=1
        assert!((solver.get_state()[0] - 1.0).abs() < 1e-12);
        solver.step(&ctx); // k=2 -> y=2
        assert!((solver.get_state()[0] - 2.0).abs() < 1e-12);
    }

    #[test]
    fn test_discrete_digital_filter() {
        // First-order IIR low-pass: y[k] = alpha*u[k] + (1-alpha)*y[k-1]
        let alpha = 0.1;
        let mut solver = DiscreteStateSolver::new(
            "lowpass",
            vec!["filtered".to_string()],
            vec![0.0],
            vec!["input".to_string()],
            Arc::new(move |_k, x, u, _ctx| vec![alpha * u[0] + (1.0 - alpha) * x[0]]),
        );
        let mut ctx = EvalContext::new();
        ctx.set("input".to_string(), Value::Float(100.0));

        // Run 50 steps — output should approach input
        for _ in 0..50 {
            solver.step(&ctx);
        }
        let actual = solver.get_state()[0];
        assert!(
            (actual - 100.0).abs() < 1.0,
            "filter should converge to 100.0, got {actual}"
        );
    }
}
