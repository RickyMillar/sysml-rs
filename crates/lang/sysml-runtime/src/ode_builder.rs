//! # ODE Builder — Compile SysML Constraints to ODE Right-Hand Sides
//!
//! Converts SysML constraint expressions and state variable declarations into
//! `OdeRhs` closures usable by [`Rk4Solver`](crate::ode::Rk4Solver).
//!
//! ## Spec Alignment
//!
//! The SysML v2 `StateSpaceRepresentation.sysml` defines:
//! - `GetDerivative`: `(input, stateSpace) -> StateDerivative` (i.e., `dx/dt = f(u, x)`)
//! - `Integrate`: calls `GetDerivative` over a time interval — "its actual
//!   implementation should be given by a solver"
//!
//! Our `OdeRhs` is `(t, y, ctx) -> dy/dt` — the same concept. This module
//! bridges from SysML expressions to that closure.

#![allow(clippy::indexing_slicing)]
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::expressions::{compile_simple_expression, EvalContext, ExprIR, ExpressionEvaluator};
use crate::ode::{OdeRhs, Rk4Solver};
use sysml_core::Value;

/// Specification of an ODE system compiled from SysML.
///
/// Maps state variable names to their derivative expressions.
#[derive(Debug, Clone, Default)]
pub struct OdeSpec {
    /// State variable names (e.g., `["temperature", "pressure"]`).
    pub state_vars: Vec<String>,
    /// Initial values for each state variable (same order as `state_vars`).
    pub initial_values: Vec<f64>,
    /// Derivative expressions: one per state variable.
    /// Each expression is evaluated with the current state bound into context.
    pub derivative_exprs: Vec<ExprIR>,
    /// Constant parameters bound into the evaluation context.
    pub parameters: HashMap<String, f64>,
    /// Time-varying signal expressions: param name → compiled expression.
    /// Evaluated each tick with current `t` to update the parameter value.
    pub signal_exprs: HashMap<String, ExprIR>,
    /// Additional context values (non-float), e.g., config Maps for FeatureChain resolution.
    pub context_values: Vec<(String, Value)>,
    /// RSC-2.4a: parameters omitted from the RHS template context because
    /// the subsystem-local slot serves them live (set by
    /// [`bind_slots`](Self::bind_slots), non-empty only when
    /// [`scoped_bypass_eligible`](Self::scoped_bypass_eligible) is true).
    /// With the template entry gone, `ExprIR::SlotRef` evaluation falls
    /// through context-name-first lookup to the slot read — which is what
    /// makes runtime overrides visible without the scoped-context clone.
    template_omitted_params: std::collections::HashSet<String>,
    /// RSC-2.4b: the subset of `template_omitted_params` whose subsystem-
    /// local slot is actually written at tick time (`Variability::Continuous`
    /// / `Discrete`), consumed only by
    /// [`build_signal_sync`](Self::build_signal_sync). See the comment block
    /// in [`bind_slots`](Self::bind_slots) for why this must stay separate
    /// from `template_omitted_params` (which governs `build_rhs` and
    /// scoped-clone-bypass eligibility and must not change).
    signal_sync_omitted_params: std::collections::HashSet<String>,
    /// RSC-2.4a: every read in the derivative/signal expressions is provably
    /// servable without the orchestrator's per-prefix scoped-context clone
    /// (see the eligibility walk in [`bind_slots`](Self::bind_slots)).
    scoped_bypass_eligible: bool,
    /// OPT #1 (runtime-hotpath-perf-plan): per-`state_vars` slot the binder
    /// resolved each state variable to (parallel to `state_vars`; `None` when
    /// the state var has no slot, e.g. a synthetic var with no model decl).
    /// Captured during [`bind_slots`](Self::bind_slots) so [`build_rhs`]
    /// (Self::build_rhs) can write each stage's state value into the
    /// `EvalContext` slot-fast lane (`set_slot_fast`) instead of the
    /// string-keyed variable map — the matching `SlotRef` read in the bound
    /// derivative/signal expressions then resolves it by index. Empty until
    /// `bind_slots` runs (hand-built specs without a slot table fall back to
    /// the by-name write).
    state_var_slots: Vec<Option<crate::slots::SlotId>>,
    /// Task #8 (steward-ruled): the canonical DEPENDENCY order of the signal
    /// targets in `signal_exprs` — topologically sorted so a signal that reads
    /// another signal's target (e.g. `I_norm = I_est / I_dn` reads `I_est`) is
    /// evaluated AFTER the signal it reads. Computed ONCE by
    /// [`compute_signal_order`](Self::compute_signal_order) and consumed by BOTH
    /// [`build_rhs`](Self::build_rhs) and
    /// [`build_signal_sync`](Self::build_signal_sync) via
    /// [`ordered_signal_exprs`](Self::ordered_signal_exprs) — the single source
    /// of evaluation order for both closures. Replaces the old per-closure
    /// `signal_exprs.clone().into_iter().collect()` (a `HashMap`→`Vec` whose
    /// per-process-random order let a dependent signal evaluate before its
    /// input, reading a stale value — the nondeterministic faultIntegral-starve
    /// of task #8). Empty for a hand-built spec that never called
    /// `compute_signal_order`; `ordered_signal_exprs` then falls back to a
    /// deterministic name sort (never HashMap order).
    signal_order: Vec<String>,
}

impl OdeSpec {
    /// Create a new empty ODE spec.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a state variable with its initial value and derivative expression.
    pub fn with_state_var(
        mut self,
        name: impl Into<String>,
        initial: f64,
        derivative_expr: ExprIR,
    ) -> Self {
        self.state_vars.push(name.into());
        self.initial_values.push(initial);
        self.derivative_exprs.push(derivative_expr);
        self
    }

    /// Add a constant parameter.
    pub fn with_param(mut self, name: impl Into<String>, value: f64) -> Self {
        self.parameters.insert(name.into(), value);
        self
    }

    /// Add a time-varying signal expression for a parameter.
    ///
    /// The expression is evaluated each ODE tick with the current `t` to produce
    /// the parameter's value. Overrides any constant parameter with the same name.
    pub fn with_signal(mut self, name: impl Into<String>, expr: ExprIR) -> Self {
        self.signal_exprs.insert(name.into(), expr);
        self
    }

    /// Add a non-float context value (e.g., a Map for config attribute resolution).
    pub fn with_context_value(mut self, name: impl Into<String>, value: Value) -> Self {
        self.context_values.push((name.into(), value));
        self
    }

    /// Task #8 (steward-ruled): compute the canonical dependency order of the
    /// signal targets and store it in [`signal_order`](Self::signal_order).
    ///
    /// An edge `A -> B` exists when signal `A`'s expression reads signal `B`'s
    /// target name (e.g. `I_norm = I_est / I_dn` reads `I_est`, so
    /// `I_norm -> I_est`); the emitted order places `B` before `A`. Ties are
    /// broken by name so the order is deterministic regardless of `HashMap`
    /// iteration. A cyclic definition (unsatisfiable evaluation order) is a hard
    /// [`CompileError`](crate::compiler::CompileError) — fail-hard, never a
    /// fixpoint or lexical-luck workaround.
    ///
    /// Must run on the freshly-parsed (pre-`bind_slots`) expressions, where
    /// inter-signal reads are `FeatureRef`s that `free_variables` surfaces;
    /// binding rewrites those to `SlotRef`s but never changes the dependency
    /// structure, and `signal_order` stores NAMES, so it stays valid across the
    /// bind pass without recomputation.
    pub fn compute_signal_order(&mut self) -> Result<(), crate::compiler::CompileError> {
        use std::collections::{BTreeMap, BTreeSet};
        let targets: BTreeSet<String> = self.signal_exprs.keys().cloned().collect();
        let mut deps: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for (name, expr) in &self.signal_exprs {
            let d: BTreeSet<String> = expr
                .free_variables()
                .into_iter()
                .filter(|fv| fv != name && targets.contains(fv))
                .collect();
            deps.insert(name.clone(), d);
        }
        let mut ordered: Vec<String> = Vec::with_capacity(targets.len());
        let mut emitted: BTreeSet<String> = BTreeSet::new();
        while ordered.len() < targets.len() {
            // Smallest-named signal whose dependencies are all already emitted.
            let next = targets
                .iter()
                .find(|n| !emitted.contains(*n) && deps[*n].iter().all(|d| emitted.contains(d)))
                .cloned();
            match next {
                Some(n) => {
                    emitted.insert(n.clone());
                    ordered.push(n);
                }
                None => {
                    let cycle: Vec<String> =
                        targets.iter().filter(|n| !emitted.contains(*n)).cloned().collect();
                    return Err(crate::compiler::CompileError::from_message(format!(
                        "ODE signal expressions have a cyclic dependency \
                         (unsatisfiable evaluation order): {}",
                        cycle.join(", ")
                    )));
                }
            }
        }
        self.signal_order = ordered;
        Ok(())
    }

    /// Task #8: the signal expressions in canonical dependency order — the
    /// single ordering both [`build_rhs`](Self::build_rhs) and
    /// [`build_signal_sync`](Self::build_signal_sync) evaluate in. Uses
    /// [`signal_order`](Self::signal_order) when
    /// [`compute_signal_order`](Self::compute_signal_order) has run (the
    /// compiler path); otherwise falls back to a deterministic name sort so a
    /// hand-built spec (tests) is never at the mercy of `HashMap` order.
    fn ordered_signal_exprs(&self) -> Vec<(String, ExprIR)> {
        self.ordered_names()
            .into_iter()
            .filter_map(|n| self.signal_exprs.get(&n).map(|e| (n.clone(), e.clone())))
            .collect()
    }

    /// The canonical signal-target names in dependency order (or the
    /// deterministic name-sorted fallback for an un-ordered hand-built spec) —
    /// the shared source for both [`ordered_signal_exprs`](Self::ordered_signal_exprs)
    /// and the write-set-routing name list in `prepare_slot_writeback`
    /// (`ode.rs` / `ode45.rs`), so there is no third independent derivation of
    /// the signal set/order (task #8).
    pub fn ordered_signal_names(&self) -> Vec<String> {
        self.ordered_names()
    }

    fn ordered_names(&self) -> Vec<String> {
        if !self.signal_order.is_empty() {
            self.signal_order
                .iter()
                .filter(|n| self.signal_exprs.contains_key(*n))
                .cloned()
                .collect()
        } else {
            let mut v: Vec<String> = self.signal_exprs.keys().cloned().collect();
            v.sort();
            v
        }
    }

    /// RSC-2.3 (design doc D-2.0.4): bind this spec's derivative and signal
    /// expressions against the compile-minted slot table, in the
    /// **subsystem-local** scope: `var_prefix` (e.g. `circuit5`) maps the
    /// instance's local names to the instance's own slots.
    ///
    /// Must run **before** [`build_rhs`](Self::build_rhs) /
    /// [`build_signal_sync`](Self::build_signal_sync) capture the IR (this
    /// is the closure-capture seam the design doc names); the orchestrator
    /// drives this through `Executor::bind_expression_slots`, which rebinds
    /// and rebuilds the closures after the slot table is minted.
    ///
    /// Names the RHS context binds locally without them being slots (the
    /// integration time `t`, signal targets, injected config/context
    /// values) are declared to the binder so they are not reported as
    /// RS003 candidates.
    pub fn bind_slots(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
    ) -> crate::expressions::BindReport {
        use crate::expressions::{bind_slots, BindReport, SlotBinder};

        let locals: Vec<String> = self
            .parameters
            .keys()
            .cloned()
            .chain(self.state_vars.iter().cloned())
            .chain(self.signal_exprs.keys().cloned())
            .chain(self.context_values.iter().map(|(n, _)| n.clone()))
            .chain(std::iter::once("t".to_owned()))
            .collect();
        let binder = SlotBinder::for_subsystem(store, var_prefix).with_locals(locals);
        let mut report = BindReport::default();
        for expr in &mut self.derivative_exprs {
            bind_slots(expr, &binder, &mut report);
        }
        for expr in self.signal_exprs.values_mut() {
            bind_slots(expr, &binder, &mut report);
        }

        // OPT #1 (runtime-hotpath-perf-plan): capture the slot each state var
        // resolved to, using the SAME `binder.resolve` the rewrite above used
        // — so the slot recorded here is exactly the one any `SlotRef` to this
        // state var carries. `build_rhs` writes the per-stage state value to
        // this slot's fast lane; the bound derivative/signal `SlotRef`s read it
        // back by index. `None` (no slot) keeps the legacy by-name write.
        self.state_var_slots = self
            .state_vars
            .iter()
            .map(|name| binder.resolve(name))
            .collect();

        // RSC-2.4a — scoped-clone bypass eligibility. The orchestrator may
        // skip building this subsystem's scoped-context view (the per-prefix
        // master-map clone) only when every read the RHS / signal
        // expressions perform is provably servable without it:
        //
        // - state vars, `t`, signal targets: bound by name into the local
        //   eval context by the closures themselves — safe.
        // - injected context values (config maps): re-inserted from the
        //   template AFTER the merge on both paths — safe for bare refs.
        // - parameters whose *subsystem-local* slot exists: omitted from the
        //   RHS template so `SlotRef` eval falls through to the live slot
        //   (runtime overrides land on the slot via by-name routing) — safe.
        // - everything else slot-bound (`SlotRef`): served by the read-only
        //   slot handle — safe.
        //
        // What genuinely still needs the scoped view (→ ineligible, the
        // orchestrator keeps the legacy clone and reports the fallback):
        // - parameters with no subsystem-local slot (a runtime override of
        //   `{prefix}.{param}` would only be visible through the stripped
        //   scoped view),
        // - any FeatureChain / SlotChainHead (the scoped view's
        //   prefix-stripped flat keys — e.g. instance config overrides
        //   `config.ratedCurrent` — can shadow the per-segment walk),
        // - unresolved names (eval-time dynamic resolution).
        let omitted: std::collections::HashSet<String> = self
            .parameters
            .keys()
            .filter(|p| binder.resolve_subsystem_local(p).is_some())
            .cloned()
            .collect();
        let safe_locals: std::collections::HashSet<&str> = self
            .state_vars
            .iter()
            .map(String::as_str)
            .chain(self.signal_exprs.keys().map(String::as_str))
            .chain(self.context_values.iter().map(|(n, _)| n.as_str()))
            .chain(std::iter::once("t"))
            .collect();
        let eligible = report.unresolved.is_empty()
            && self
                .derivative_exprs
                .iter()
                .chain(self.signal_exprs.values())
                .all(|e| bypass_safe(e, &safe_locals, &self.parameters, &omitted));
        self.scoped_bypass_eligible = eligible;
        self.template_omitted_params = if eligible {
            omitted.clone()
        } else {
            Default::default()
        };
        // RSC-2.4b (duty-seam regression fix, see `build_signal_sync`'s
        // comment block): a NARROWER subset of `template_omitted_params`,
        // restricted to names whose subsystem-local slot is actually
        // *written at tick time* (`Variability::Continuous` / `Discrete` —
        // e.g. `duty`'s dedicated mint at compiler.rs's step 2c, or a
        // promoted `GetOutput` signal slot). `Variability::Parameter`
        // ("written by no executor at tick time", per that variant's own
        // doc comment) and `Constant` slots do NOT qualify even though
        // `resolve_subsystem_local` finds them — those are exactly
        // `bypass_safe`'s target for the scoped-clone-bypass decision above
        // (a runtime OVERRIDE of the raw parameter is what that eligibility
        // walk protects), which is a completely different question from
        // "does this name's *live per-tick value* come from the slot." Kept
        // separate from `template_omitted_params` deliberately: eligibility
        // and `build_rhs` must stay exactly as they were pre-existing
        // (touching them flips other subsystems' RS004 hard-error gate,
        // since the legacy scoped-clone fallback no longer exists per
        // RSC-3.5f.3) — only `build_signal_sync`'s own runtime skip/strip
        // decisions need the narrower test.
        let live_write_omitted: std::collections::HashSet<String> = omitted
            .into_iter()
            .filter(|p| {
                binder.resolve_subsystem_local(p).is_some_and(|slot| {
                    store.meta(slot).is_some_and(|meta| {
                        matches!(
                            meta.variability,
                            crate::slots::Variability::Continuous
                                | crate::slots::Variability::Discrete
                        )
                    })
                })
            })
            .collect();
        self.signal_sync_omitted_params = if eligible {
            live_write_omitted
        } else {
            Default::default()
        };

        report
    }

    /// RSC-2.4a: whether every read in this spec's expressions can be served
    /// without the orchestrator's scoped-context clone (computed by
    /// [`bind_slots`](Self::bind_slots); `false` for unbound specs).
    pub fn scoped_bypass_eligible(&self) -> bool {
        self.scoped_bypass_eligible
    }

    /// Whether `name` is in the RSC-2.4a template-omission set (see
    /// [`template_omitted_params`](Self::template_omitted_params) and the
    /// WS-D Stage 2 duty-seam fix comment in
    /// [`build_signal_sync`](Self::build_signal_sync)). Only ever non-empty
    /// when [`scoped_bypass_eligible`](Self::scoped_bypass_eligible) is
    /// `true` — a single `FeatureChain`/`SlotChainHead` anywhere in
    /// `derivative_exprs` or `signal_exprs` makes the whole spec ineligible
    /// and empties the set, which would silently no-op the duty-seam fix.
    /// Exposed so integration tests can pin the precondition directly
    /// instead of inferring it from black-box test color.
    pub fn template_omits(&self, name: &str) -> bool {
        self.template_omitted_params.contains(name)
    }

    /// RSC-4.1: the compiler-resolved `SlotId`s read by this spec's bound
    /// derivative and signal expressions — every `SlotRef` / `SlotChainHead`
    /// that [`bind_slots`](Self::bind_slots) produced. Empty for an unbound
    /// spec (no `bind_slots` run) or a spec whose reads never resolved to
    /// slots. Raw (unsorted, may duplicate); the `Executor::read_slots` impl
    /// sorts + dedups. This is the ODE half of the RSC-4.1 read-set accessor.
    pub fn slot_reads(&self) -> Vec<crate::slots::SlotId> {
        let mut out = Vec::new();
        for expr in &self.derivative_exprs {
            out.extend(expr.slot_reads());
        }
        for expr in self.signal_exprs.values() {
            out.extend(expr.slot_reads());
        }
        out
    }

    /// Build an `OdeRhs` closure from this spec.
    ///
    /// The closure binds state variables and parameters into an `EvalContext`,
    /// evaluates each derivative expression, and returns the derivatives vector.
    ///
    /// If an expression evaluates to a non-numeric value, the derivative defaults
    /// to 0.0 (with a debug assertion warning in debug builds).
    pub fn build_rhs(&self) -> OdeRhs {
        let state_var_names = self.state_vars.clone();
        // OPT #1: parallel to `state_var_names`. A `Some(slot)` routes the
        // per-stage state value to the scratch context's slot-fast lane
        // (`set_slot_fast`) — no `String` clone, no map hash/insert — and the
        // bound `SlotRef` reads it back by index. `None` keeps the by-name
        // write for state vars the binder could not resolve to a slot.
        let state_var_slots = self.state_var_slots.clone();
        let derivative_exprs: Vec<ExprIR> = self.derivative_exprs.clone();
        // Task #8: single canonical dependency order (see `ordered_signal_exprs`),
        // shared by build_rhs and build_signal_sync — replaces the old per-closure
        // `HashMap`→`Vec` collect whose per-process-random order was the bug.
        let signal_exprs: Vec<(String, ExprIR)> = self.ordered_signal_exprs();
        let evaluator = Arc::new(ExpressionEvaluator::new());

        // Pre-build a template context with parameters and config values already set.
        // Only state vars and time are overwritten per call (avoids re-inserting
        // constant params on every RHS evaluation).
        let mut template_ctx = EvalContext::new();
        for (name, &value) in self.parameters.iter() {
            // RSC-2.4a: parameters the subsystem-local slot serves live are
            // left out of the template — a stale template entry would win
            // the evaluator's context-name-first lookup and hide runtime
            // overrides once the scoped-context clone is bypassed. Empty
            // (no omissions) unless `bind_slots` proved the spec eligible.
            if self.template_omitted_params.contains(name) {
                continue;
            }
            template_ctx.set(name.clone(), Value::Float(value));
        }
        for (name, value) in &self.context_values {
            template_ctx.set(name.clone(), value.clone());
        }
        // Capture config values (Maps) separately so we can re-insert after merge.
        // The shared context may carry a different ODE's config Map under the same
        // key (e.g., both ThermalProtectionModel and FaultProtectionModel define `config`).
        let config_values: Vec<(String, Value)> = self.context_values.clone();

        // PERF: per-orchestrator-step scratch cache. The `ctx` handed to the RHS
        // is identical across every RK45 stage evaluation within a `step_to`
        // call (and indeed the whole orchestrator step) — the solver borrows it
        // immutably for the duration. The expensive part of the old per-eval
        // body was `template_ctx.clone(); merge_from(ctx)` (a full variable-map
        // clone plus an insert of every `ctx` variable) repeated ~7×/step.
        //
        // We build that merged base exactly once per distinct `ctx` (keyed on
        // the identity of `ctx.variables`), then reuse the owned scratch map in
        // place across calls — overwriting only `t`, the state vars, and the
        // signal values, which are the *only* keys the RHS body writes. Because
        // the scratch's `variables` Arc is uniquely owned, `set()`'s
        // `Arc::make_mut` mutates in place rather than cloning the whole map.
        //
        // Semantics are byte-identical to the old path: `signal_base` records
        // each signal name's value in the merged base (usually absent), so a
        // stage where a signal eval fails restores exactly what a fresh
        // `template_ctx.clone(); merge_from` would have presented.
        struct RhsScratch {
            key: Option<Arc<HashMap<String, Value>>>,
            ctx: EvalContext,
            signal_base: Vec<Option<Value>>,
        }
        let cache = Mutex::new(RhsScratch {
            key: None,
            ctx: EvalContext::new(),
            signal_base: Vec::new(),
        });

        Arc::new(move |t: f64, y: &[f64], ctx: &EvalContext| {
            let mut guard = cache.lock().unwrap_or_else(|p| p.into_inner());

            let fresh = !matches!(&guard.key, Some(k) if Arc::ptr_eq(k, &ctx.variables));
            if fresh {
                // Start from template (default params), overlay shared context
                // (runtime overrides), then re-insert this ODE's config Maps —
                // the shared context may have overwritten them with a different
                // ODE type's config.
                let mut base = template_ctx.scratch_snapshot();
                base.merge_from(ctx);
                for (name, value) in &config_values {
                    base.set(name.clone(), value.clone());
                }
                guard.signal_base = signal_exprs
                    .iter()
                    .map(|(name, _)| base.get(name).cloned())
                    .collect();
                guard.ctx = base;
                guard.key = Some(Arc::clone(&ctx.variables));
            }

            let RhsScratch {
                ctx: eval_ctx,
                signal_base,
                ..
            } = &mut *guard;

            // Reset the signal slots to their base value so a stage where a
            // signal eval fails sees exactly what the legacy fresh-from-template
            // path saw (the base value, or absent).
            for ((sig_name, _), base) in signal_exprs.iter().zip(signal_base.iter()) {
                match base {
                    Some(v) => eval_ctx.set(sig_name.clone(), v.clone()),
                    None => {
                        Arc::make_mut(&mut eval_ctx.variables).remove(sig_name);
                    }
                }
            }
            eval_ctx.set("t".to_owned(), Value::Float(t));
            for (i, name) in state_var_names.iter().enumerate() {
                // OPT #1: slotted state vars go to the fast lane, read back by
                // the bound `SlotRef` (fast-lane-first). This must NOT also
                // write the name map: the cached base may carry a stale prior
                // value for this name from `merge_from(ctx)`, and a name-first
                // read of a slotted var would return that stale value.
                match state_var_slots.get(i).copied().flatten() {
                    Some(slot) => eval_ctx.set_slot_fast(slot, y[i]),
                    None => eval_ctx.set(name.clone(), Value::Float(y[i])),
                }
            }

            // Evaluate time-varying signal expressions and bind results into context
            for (sig_name, sig_expr) in &signal_exprs {
                match evaluator.eval(sig_expr, eval_ctx) {
                    Ok(Value::Float(f)) => {
                        eval_ctx.set(sig_name.clone(), Value::Float(f));
                    }
                    Ok(Value::Int(i)) => {
                        eval_ctx.set(sig_name.clone(), Value::Float(i as f64));
                    }
                    _ => {} // keep base value if signal eval fails (restored above)
                }
            }

            // Evaluate each derivative expression
            derivative_exprs
                .iter()
                .map(|expr| {
                    match evaluator.eval(expr, eval_ctx) {
                        // `as_float` unifies Float / Int / Quantity — a
                        // *dimensioned* numeric derivative (e.g. one carrying
                        // units) is still a number, so integrate its scalar
                        // value. Previously only Float/Int were handled and any
                        // `Quantity` fell through to the 0.0 default, silently
                        // ZEROING real derivatives (a correctness bug) and
                        // flooding logs every RHS evaluation.
                        Ok(v) => v.as_float().unwrap_or_else(|| {
                            // Genuinely non-numeric result (Bool/String/Null).
                            // This means the derivative expression is malformed;
                            // surface it in debug and integrate 0.0 as a last
                            // resort (the diffsol RHS closure cannot return an
                            // error). A build-time numeric check on derivative
                            // expressions is the fuller fail-hard fix.
                            #[cfg(debug_assertions)]
                            #[allow(clippy::print_stderr)]
                            {
                                eprintln!(
                                    "[ODE] derivative expression is non-numeric: {v:?}; using 0.0 — check the model's derivative definition"
                                );
                            }
                            0.0
                        }),
                        Err(e) => {
                            #[cfg(debug_assertions)]
                            #[allow(clippy::print_stderr)]
                            {
                                eprintln!(
                                    "[ODE] derivative expression failed to evaluate: {e:?}; using 0.0"
                                );
                            }
                            let _ = &e;
                            0.0
                        }
                    }
                })
                .collect()
        })
    }

    /// Build a signal evaluator closure that re-evaluates signal expressions
    /// with the current state and writes results to a shared context.
    ///
    /// Called by `sync_context_out` so that ODE-internal signal values (like
    /// `i_drive` computed from the BH inverse lookup) become visible to other
    /// subsystems (state machines, constraints).
    pub fn build_signal_sync(
        &self,
    ) -> Option<Arc<dyn Fn(&[f64], &EvalContext, &mut EvalContext) + Send + Sync>> {
        if self.signal_exprs.is_empty() {
            return None;
        }

        let state_var_names = self.state_vars.clone();
        // Task #8: single canonical dependency order (see `ordered_signal_exprs`),
        // shared by build_rhs and build_signal_sync — replaces the old per-closure
        // `HashMap`→`Vec` collect whose per-process-random order was the bug.
        let signal_exprs: Vec<(String, ExprIR)> = self.ordered_signal_exprs();
        let evaluator = Arc::new(ExpressionEvaluator::new());

        // WS-D Stage 2 duty-seam fix: mirror `build_rhs`'s `template_omitted_params`
        // exclusion (task #6's flagged asymmetry — this IS the duty freeze, not a
        // separate cleanup). A parameter with a live-servable slot (e.g. `duty`,
        // mint-time-promoted to Continuous by `mint_slot_store` step 2c) must
        // never have a name-keyed entry win over its slot: `ExprIR::SlotRef`
        // evaluation (evaluator.rs `SlotRef` arm) checks the name map BEFORE the
        // slot — "context names win" is load-bearing for RK4 stage bindings and
        // must stay that way — so a present name entry permanently shadows the
        // live slot; it never gets a chance to be consulted at all.
        //
        // Skipping the bake below (mirroring build_rhs) is necessary but not
        // sufficient on its own: it must be re-enforced post-merge against
        // `shared` (the master/orchestrator context merged in below via
        // `merge_from`).
        //
        // NB the original justifying example — "the tracker's write path only
        // ever updates the qualified `{ode}.duty` key, leaving the bare `duty`
        // map entry frozen at its t=0 default" — described the PRE-Task-#8 bug
        // and is no longer the mechanism: Task #8 routed the tracker write
        // through `set_slot`, whose full-alias mirror now updates EVERY spelling
        // (bare + qualified, via `aliases_of`), so a slot's OWN bare entry stays
        // live. The post-merge strip is retained for a different, still-live
        // reason (cull-arc ADR-018, steward-w2b ruling): `shared`'s bare-name
        // entry for an omitted parameter can carry a DIFFERENT slot instance's
        // aliased value, or a literal model default not yet written this tick,
        // and under "context names win" (`SlotRef` checks the name map before
        // the slot) that foreign/stale bare entry would shadow THIS closure's
        // own slot. `merge_from` copies it in unconditionally, so the omission
        // has to be enforced again post-merge, not just against this closure's
        // own template construction. This is a cross-slot collision, not a
        // within-slot alias-mirror gap — W1 did not retire it.
        //
        // RSC-2.4b (duty-seam regression fix): this closure uses
        // `signal_sync_omitted_params`, NOT the full `template_omitted_params`
        // `build_rhs` uses — deliberately narrower (see `bind_slots`'s comment
        // block). `template_omitted_params` includes any parameter with *a*
        // subsystem-local slot, regardless of whether anything actually writes
        // it at tick time; that's the right test for `build_rhs` (which never
        // strips post-merge, so a `Value::Ref` `shared` might still carry gets
        // discarded harmlessly the moment a real Float lands) and for the
        // scoped-clone-bypass eligibility decision (about runtime overrides,
        // not tick-time freshness) — but it is NOT the right test for what
        // this closure omits-and-strips: a `DerivedBinding` attribute (e.g.
        // `H_dc = N_fault * I_residual / le`) gets an ODE-parameter slot with
        // `Variability::Parameter` (mint_slot_store's per-parameter loop) that
        // nothing ever promotes or writes — its one live source is `shared`'s
        // name map, refreshed once per tick by
        // `Orchestrator::evaluate_computed_expressions`, which runs LATER in
        // the same tick than this closure (`sync_context_out_slots`) does.
        // `context_from_graph` seeds that bare name as an unresolved
        // `Value::Ref` placeholder; omitting it from the bake below removes
        // the only thing that could protect against copying that Ref straight
        // through (`merge_from`'s "don't let a Ref shadow a resolved value"
        // guard only fires when the template already has a concrete entry to
        // protect) — so on tick 1, before `evaluate_computed_expressions` has
        // ever run, evaluating `i_drive` (which reads `H_dc`) fails outright
        // with `UndefinedVariable("H_dc (attribute has no assigned value)")`.
        // `signal_sync_omitted_params` excludes such names by construction
        // (`bind_slots` restricts it to `Variability::Continuous` / `Discrete`
        // — slots an executor genuinely writes every tick, exactly `duty`'s
        // and a promoted `GetOutput` signal's shape).
        let mut template_ctx = EvalContext::new();
        for (name, &value) in self.parameters.iter() {
            if self.signal_sync_omitted_params.contains(name) {
                continue;
            }
            template_ctx.set(name.clone(), Value::Float(value));
        }
        for (name, value) in &self.context_values {
            template_ctx.set(name.clone(), value.clone());
        }
        // PART 1 finding (the actual nondeterminism source, not the mint/merge
        // seam above): a signal's own OWN target name (e.g. `I_est`, `I_norm`,
        // `i_drive`) is ALSO a member of `signal_sync_omitted_params` whenever
        // it carries a literal default (`attribute I_est : Real default 0.0;`
        // — same declaration shape as `duty`) and a mint-time-servable slot
        // (RSC-3.0 cat-2 signal-output slots mint one for every `GetOutput`-
        // derived quantity, not just externally-tracked parameters like
        // `duty`). Cross-signal references within THIS SAME closure
        // (`I_norm = I_est / I_dn`) are intentionally resolved through the
        // name map's intra-tick chaining — each signal's
        // `eval_ctx.set(sig_name, ...)` makes it visible to a LATER signal in
        // the SAME `signal_exprs` iteration — not through the slot, which
        // only reflects the PREVIOUS tick's write (`write_signals` batches
        // all of this tick's slot writes AFTER the whole loop finishes).
        // `signal_exprs` is a `Vec` collected from a `HashMap`
        // (`self.signal_exprs.clone().into_iter().collect()`), so its
        // iteration order is per-process-random (RandomState-seeded) and
        // FIXED only for this closure's lifetime — the ws_determinism disease
        // family. Blanket-stripping every `signal_sync_omitted_params` name
        // after `merge_from` (below) — including a signal's OWN target name
        // carried over from the previous tick — silently drops that carryover
        // in any process where a dependent signal (`I_norm`) happens to be
        // ordered BEFORE the signal it reads (`I_est`): the name is gone, the
        // slot hasn't been written for THIS tick yet, so the read is
        // genuinely missing rather than merely one tick stale, and `I_norm`
        // never recovers because the same bad order repeats every tick. The
        // strip must exclude names this closure itself is going to
        // (re)compute — those are safe to carry over by name, and stripping
        // them is not just unnecessary but actively wrong.
        let signal_target_names: std::collections::HashSet<String> =
            self.signal_exprs.keys().cloned().collect();
        let omitted_params: std::collections::HashSet<String> = self
            .signal_sync_omitted_params
            .difference(&signal_target_names)
            .cloned()
            .collect();

        Some(Arc::new(
            move |state: &[f64], shared: &EvalContext, out: &mut EvalContext| {
                let mut eval_ctx = template_ctx.scratch_snapshot();
                eval_ctx.merge_from(shared);
                if !omitted_params.is_empty() {
                    let vars = Arc::make_mut(&mut eval_ctx.variables);
                    for name in &omitted_params {
                        vars.remove(name);
                    }
                }
                for (i, name) in state_var_names.iter().enumerate() {
                    if i < state.len() {
                        eval_ctx.set(name.clone(), Value::Float(state[i]));
                    }
                }
                for (sig_name, sig_expr) in &signal_exprs {
                    match evaluator.eval(sig_expr, &eval_ctx) {
                        Ok(Value::Float(f)) => {
                            eval_ctx.set(sig_name.clone(), Value::Float(f));
                            out.set(sig_name.clone(), Value::Float(f));
                        }
                        Ok(Value::Int(i)) => {
                            let f = i as f64;
                            eval_ctx.set(sig_name.clone(), Value::Float(f));
                            out.set(sig_name.clone(), Value::Float(f));
                        }
                        _ => {}
                    }
                }
            },
        ))
    }

    /// Build a complete `Rk4Solver` from this spec.
    pub fn build_solver(&self, name: impl Into<String>) -> Rk4Solver {
        Rk4Solver::new(
            name,
            self.state_vars.clone(),
            self.initial_values.clone(),
            self.build_rhs(),
        )
    }
}

/// RSC-2.4a eligibility walk (see the comment block in
/// [`OdeSpec::bind_slots`]): `true` when every reference in `expr` is
/// servable without the orchestrator's scoped-context view. Exhaustive over
/// [`ExprIR`] so new variants force a conscious eligibility decision.
fn bypass_safe(
    expr: &ExprIR,
    safe_locals: &std::collections::HashSet<&str>,
    parameters: &HashMap<String, f64>,
    omitted_params: &std::collections::HashSet<String>,
) -> bool {
    fn walk(
        expr: &ExprIR,
        safe_locals: &std::collections::HashSet<&str>,
        parameters: &HashMap<String, f64>,
        omitted: &std::collections::HashSet<String>,
        lambda_bound: &mut std::collections::HashSet<String>,
    ) -> bool {
        match expr {
            ExprIR::LiteralInt(_)
            | ExprIR::LiteralReal(_)
            | ExprIR::LiteralBool(_)
            | ExprIR::LiteralString(_)
            | ExprIR::LiteralQuantity { .. }
            | ExprIR::LiteralNull => true,

            // Bare name kept name-resolved: safe only when the closures
            // themselves bind it (state var / t / signal target / config
            // value). A name-resolved PARAMETER would read the static
            // template while the scoped view could carry an override.
            ExprIR::FeatureRef(n) => {
                lambda_bound.contains(n.as_str())
                    || (safe_locals.contains(n.as_str()) && !parameters.contains_key(n))
            }

            // Slot-bound ref: safe unless it names a parameter that kept its
            // template entry (template would shadow the live slot).
            ExprIR::SlotRef { name, .. } => {
                !parameters.contains_key(name) || omitted.contains(name)
            }

            // Chains rely on the scoped view's prefix-stripped flat keys
            // (instance config overrides etc.) — never bypass.
            ExprIR::FeatureChain(_) | ExprIR::SlotChainHead { .. } => false,

            ExprIR::BinaryOp { left, right, .. } => {
                walk(left, safe_locals, parameters, omitted, lambda_bound)
                    && walk(right, safe_locals, parameters, omitted, lambda_bound)
            }
            ExprIR::UnaryOp { operand, .. } => {
                walk(operand, safe_locals, parameters, omitted, lambda_bound)
            }
            ExprIR::Conditional {
                condition,
                then_expr,
                else_expr,
            } => {
                walk(condition, safe_locals, parameters, omitted, lambda_bound)
                    && walk(then_expr, safe_locals, parameters, omitted, lambda_bound)
                    && walk(else_expr, safe_locals, parameters, omitted, lambda_bound)
            }
            ExprIR::NullCoalescing { expr, default } => {
                walk(expr, safe_locals, parameters, omitted, lambda_bound)
                    && walk(default, safe_locals, parameters, omitted, lambda_bound)
            }
            ExprIR::Select {
                source,
                binding,
                predicate,
            }
            | ExprIR::Reject {
                source,
                binding,
                predicate,
            }
            | ExprIR::ForAll {
                source,
                binding,
                predicate,
            }
            | ExprIR::Exists {
                source,
                binding,
                predicate,
            } => {
                if !walk(source, safe_locals, parameters, omitted, lambda_bound) {
                    return false;
                }
                let newly = lambda_bound.insert(binding.clone());
                let ok = walk(predicate, safe_locals, parameters, omitted, lambda_bound);
                if newly {
                    lambda_bound.remove(binding.as_str());
                }
                ok
            }
            ExprIR::Collect {
                source,
                binding,
                transform,
            } => {
                if !walk(source, safe_locals, parameters, omitted, lambda_bound) {
                    return false;
                }
                let newly = lambda_bound.insert(binding.clone());
                let ok = walk(transform, safe_locals, parameters, omitted, lambda_bound);
                if newly {
                    lambda_bound.remove(binding.as_str());
                }
                ok
            }
            ExprIR::Index { sequence, index } => {
                walk(sequence, safe_locals, parameters, omitted, lambda_bound)
                    && walk(index, safe_locals, parameters, omitted, lambda_bound)
            }
            ExprIR::FunctionCall { args, .. } => args
                .iter()
                .all(|a| walk(a, safe_locals, parameters, omitted, lambda_bound)),
            ExprIR::Sequence(items) => items
                .iter()
                .all(|i| walk(i, safe_locals, parameters, omitted, lambda_bound)),
            ExprIR::Range { lower, upper } => {
                walk(lower, safe_locals, parameters, omitted, lambda_bound)
                    && walk(upper, safe_locals, parameters, omitted, lambda_bound)
            }
            ExprIR::MetaAccess { operand, .. } => {
                walk(operand, safe_locals, parameters, omitted, lambda_bound)
            }
            ExprIR::ConstructorCall { named_args, .. } => named_args
                .iter()
                .all(|(_, a)| walk(a, safe_locals, parameters, omitted, lambda_bound)),
        }
    }
    let mut lambda_bound = std::collections::HashSet::new();
    walk(
        expr,
        safe_locals,
        parameters,
        omitted_params,
        &mut lambda_bound,
    )
}

/// Parse a derivative expression string into an `ExprIR`.
///
/// This is a convenience wrapper around `compile_simple_expression` that
/// provides better error messages for ODE-specific contexts.
pub fn parse_derivative(expr_str: &str) -> Result<ExprIR, String> {
    compile_simple_expression(expr_str).map_err(|diags| {
        let msgs: Vec<_> = diags.iter().map(|d| d.message.as_str()).collect();
        format!(
            "failed to compile derivative expression '{}': {}",
            expr_str,
            msgs.join("; ")
        )
    })
}

/// Build a thermal body ODE spec (the canonical example).
///
/// `dT/dt = (heaterPower - lossCoefficient * (T - ambientTemp)) / thermalMass`
///
/// This is the spec-aligned replacement for the previously hardcoded thermal
/// model in `OdeRk4Plugin`.
pub fn thermal_body_spec(
    initial_temp: f64,
    heater_power: f64,
    ambient_temp: f64,
    thermal_mass: f64,
    loss_coefficient: f64,
) -> Result<OdeSpec, String> {
    let expr = parse_derivative(
        "(heaterPower - lossCoefficient * (temperature - ambientTemp)) / thermalMass",
    )?;

    Ok(OdeSpec::new()
        .with_state_var("temperature", initial_temp, expr)
        .with_param("heaterPower", heater_power)
        .with_param("ambientTemp", ambient_temp)
        .with_param("thermalMass", thermal_mass)
        .with_param("lossCoefficient", loss_coefficient))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_ode_spec_builder() {
        let expr = parse_derivative("1.0").unwrap();
        let spec = OdeSpec::new()
            .with_state_var("x", 0.0, expr)
            .with_param("k", 2.0);

        assert_eq!(spec.state_vars, vec!["x"]);
        assert_eq!(spec.initial_values, vec![0.0]);
        assert_eq!(spec.parameters.get("k"), Some(&2.0));
    }

    /// Task #8: a signal that reads another signal's target must be evaluated
    /// AFTER it — `compute_signal_order` puts `I_est` before `I_norm` (which
    /// reads `I_est`) regardless of how many times the spec is rebuilt (the
    /// old `HashMap`→`Vec` collect was per-process-random, which let the
    /// dependent signal read a stale value and starved the fault-integral
    /// signal chain nondeterministically).
    #[test]
    fn test_signal_order_is_dependency_topological() {
        // I_norm = I_est / I_dn  (reads the I_est signal)
        // I_est  = interpolateSaturating(lut, duty)  (reads no other signal)
        // i_drive depends on neither signal.
        let mut spec = OdeSpec::new()
            .with_state_var("B", 0.0, parse_derivative("1.0").unwrap())
            .with_signal("I_norm", parse_derivative("I_est / I_dn").unwrap())
            .with_signal("I_est", parse_derivative("duty * 2.0").unwrap())
            .with_signal("i_drive", parse_derivative("B * 3.0").unwrap());
        spec.compute_signal_order().expect("acyclic signal graph must order");
        let order: Vec<String> = spec.ordered_signal_names();
        let pos = |n: &str| order.iter().position(|x| x == n).unwrap();
        assert!(
            pos("I_est") < pos("I_norm"),
            "I_est must be evaluated before its consumer I_norm; got {order:?}"
        );
        // Order is deterministic (not HashMap-random): recomputing yields the same.
        let mut spec2 = spec.clone();
        spec2.compute_signal_order().unwrap();
        assert_eq!(spec.ordered_signal_names(), spec2.ordered_signal_names());
    }

    /// Task #8: a cyclic signal definition is a hard `CompileError`, never a
    /// silent fixpoint or lexical-luck workaround.
    #[test]
    fn test_cyclic_signal_order_is_compile_error() {
        let mut spec = OdeSpec::new()
            .with_signal("A", parse_derivative("B + 1.0").unwrap())
            .with_signal("B", parse_derivative("A + 1.0").unwrap());
        let err = spec.compute_signal_order().unwrap_err();
        assert!(
            err.to_string().contains("cyclic"),
            "expected a cyclic-dependency CompileError, got: {err}"
        );
    }

    #[test]
    fn test_constant_derivative_from_expr() {
        // dx/dt = 5.0  =>  x(1) = 5.0
        let expr = parse_derivative("5.0").unwrap();
        let spec = OdeSpec::new().with_state_var("x", 0.0, expr);
        let mut solver = spec.build_solver("test");
        let ctx = EvalContext::new();

        solver.step(0.0, 1.0, &ctx);
        assert!((solver.get_state()[0] - 5.0).abs() < 1e-10);
    }

    #[test]
    fn test_exponential_decay_from_expr() {
        // dx/dt = -x  =>  x(1) = e^(-1) ≈ 0.3679
        let expr = parse_derivative("0.0 - x").unwrap();
        let spec = OdeSpec::new().with_state_var("x", 1.0, expr);
        let mut solver = spec.build_solver("decay");
        let ctx = EvalContext::new();

        let dt = 0.01;
        for i in 0..100 {
            solver.step(i as f64 * dt, dt, &ctx);
        }
        let expected = (-1.0_f64).exp();
        assert!(
            (solver.get_state()[0] - expected).abs() < 1e-6,
            "expected {}, got {}",
            expected,
            solver.get_state()[0]
        );
    }

    #[test]
    fn test_parameterized_derivative() {
        // dx/dt = k  where k=3.0  =>  x(1) = 3.0
        let expr = parse_derivative("k").unwrap();
        let spec = OdeSpec::new()
            .with_state_var("x", 0.0, expr)
            .with_param("k", 3.0);
        let mut solver = spec.build_solver("param");
        let ctx = EvalContext::new();

        solver.step(0.0, 1.0, &ctx);
        assert!((solver.get_state()[0] - 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_context_override() {
        // dx/dt = power  where power comes from the shared context
        let expr = parse_derivative("power").unwrap();
        let spec = OdeSpec::new().with_state_var("x", 0.0, expr);
        let mut solver = spec.build_solver("ctx_test");

        let mut ctx = EvalContext::new();
        ctx.set("power".to_string(), Value::Float(7.0));

        solver.step(0.0, 1.0, &ctx);
        assert!((solver.get_state()[0] - 7.0).abs() < 1e-10);
    }

    #[test]
    fn test_quantity_derivative_integrates_scalar_not_zero() {
        // dx/dt = rate, where `rate` is a *dimensioned* Quantity (value 4.0).
        // Regression: a Quantity-valued derivative must integrate its scalar
        // (4.0), not fall through to the old 0.0 default that silently zeroed
        // every unit-carrying derivative.
        let expr = parse_derivative("rate").unwrap();
        let spec = OdeSpec::new().with_state_var("x", 0.0, expr);
        let mut solver = spec.build_solver("quantity_deriv");

        let mut ctx = EvalContext::new();
        ctx.set(
            "rate".to_string(),
            Value::Quantity {
                value: 4.0,
                dimension: sysml_core::physics::DimensionVector::default(),
                unit: None,
            },
        );

        solver.step(0.0, 1.0, &ctx);
        assert!(
            (solver.get_state()[0] - 4.0).abs() < 1e-10,
            "Quantity derivative should integrate its scalar value (4.0), got {}",
            solver.get_state()[0]
        );
    }

    #[test]
    fn test_thermal_body_spec() {
        let spec = thermal_body_spec(20.0, 100.0, 20.0, 5.0, 10.0).unwrap();
        assert_eq!(spec.state_vars, vec!["temperature"]);
        assert_eq!(spec.initial_values, vec![20.0]);

        let mut solver = spec.build_solver("thermal");
        let ctx = EvalContext::new();

        // Run to equilibrium: T_eq = ambient + power/loss = 20 + 100/10 = 30
        let dt = 0.01;
        for i in 0..2000 {
            solver.step(i as f64 * dt, dt, &ctx);
        }
        let t_eq = 30.0;
        assert!(
            (solver.get_state()[0] - t_eq).abs() < 0.01,
            "expected convergence to {}, got {}",
            t_eq,
            solver.get_state()[0]
        );
    }

    #[test]
    fn test_multi_state_system() {
        // Coupled system:
        //   dx/dt = v
        //   dv/dt = -x  (simple harmonic oscillator)
        // With x(0)=1, v(0)=0: x(pi) ≈ -1, v(pi) ≈ 0
        let dx_expr = parse_derivative("v").unwrap();
        let dv_expr = parse_derivative("0.0 - x").unwrap();
        let spec = OdeSpec::new()
            .with_state_var("x", 1.0, dx_expr)
            .with_state_var("v", 0.0, dv_expr);

        let mut solver = spec.build_solver("oscillator");
        let ctx = EvalContext::new();

        let dt = 0.001;
        let steps = (std::f64::consts::PI / dt) as usize;
        for i in 0..steps {
            solver.step(i as f64 * dt, dt, &ctx);
        }

        let x_final = solver.get_state()[0];
        let v_final = solver.get_state()[1];
        assert!(
            (x_final - (-1.0)).abs() < 0.001,
            "x(pi) should be -1, got {}",
            x_final
        );
        assert!(v_final.abs() < 0.001, "v(pi) should be 0, got {}", v_final);
    }

    #[test]
    fn test_time_dependent_derivative() {
        // dx/dt = t  =>  x(1) = 0.5 (integral of t from 0 to 1)
        let expr = parse_derivative("t").unwrap();
        let spec = OdeSpec::new().with_state_var("x", 0.0, expr);
        let mut solver = spec.build_solver("time_dep");
        let ctx = EvalContext::new();

        let dt = 0.001;
        for i in 0..1000 {
            solver.step(i as f64 * dt, dt, &ctx);
        }
        assert!(
            (solver.get_state()[0] - 0.5).abs() < 0.001,
            "expected 0.5, got {}",
            solver.get_state()[0]
        );
    }

    #[test]
    fn test_parse_derivative_error() {
        let result = parse_derivative("+++invalid");
        assert!(result.is_err());
    }

    #[test]
    fn test_sampled_function_as_ode_signal() {
        // A SampledFunction defines a step input: 0 for t<0.5, 1 for t>=0.5
        // ODE: dx/dt = input - x  (first-order lag towards the input)
        // After t=1.0 (0.5s of unit step), x should approach 1 - e^(-0.5) ≈ 0.3935

        // Build the SampledFunction as a Value::Map (same representation as stdlib)
        let sf = {
            let mut map = std::collections::BTreeMap::new();
            map.insert(
                "__type".to_string(),
                Value::String("SampledFunction".to_string()),
            );
            map.insert(
                "domain".to_string(),
                Value::List(vec![
                    Value::Float(0.0),
                    Value::Float(0.49),
                    Value::Float(0.50),
                    Value::Float(10.0),
                ]),
            );
            map.insert(
                "range".to_string(),
                Value::List(vec![
                    Value::Float(0.0),
                    Value::Float(0.0),
                    Value::Float(1.0),
                    Value::Float(1.0),
                ]),
            );
            Value::Map(map)
        };

        // ODE: dx/dt = input - x
        let deriv = parse_derivative("input - x").unwrap();

        // Signal: input = interpolateLinear(__sf_waveform, t)
        let signal_expr = ExprIR::FunctionCall {
            name: "interpolateLinear".to_string(),
            args: vec![
                ExprIR::FeatureRef("__sf_waveform".to_string()),
                ExprIR::FeatureRef("t".to_string()),
            ],
        };

        let spec = OdeSpec::new()
            .with_state_var("x", 0.0, deriv)
            .with_signal("input", signal_expr)
            .with_context_value("__sf_waveform", sf);

        let mut solver = spec.build_solver("sf_signal_test");
        let ctx = EvalContext::new();

        let dt = 0.001;
        for i in 0..1000 {
            solver.step(i as f64 * dt, dt, &ctx);
        }

        // At t=1.0: step happened at t=0.5, so 0.5s of exponential approach to 1.0
        // x(1.0) ≈ 1 - e^(-0.5) ≈ 0.3935
        let expected = 1.0 - (-0.5_f64).exp();
        let actual = solver.get_state()[0];
        assert!(
            (actual - expected).abs() < 0.01,
            "expected ~{:.4}, got {:.4}",
            expected,
            actual
        );
    }

    /// Phase 3.3: Piecewise ODE test — free-fall with ground bounce.
    ///
    /// Models a bouncing ball:
    ///   dy/dt = v
    ///   dv/dt = if y > 0.01 ? -9.81 else 0
    ///
    /// The conditional derivative applies gravity only when above ground.
    /// When the ball is at or below ground, velocity is zeroed (simplified
    /// inelastic bounce for test purposes).
    ///
    /// Tests that ExprIR::Conditional works inside ODE derivative expressions.
    #[test]
    fn test_piecewise_ode_bouncing_ball() {
        // dy/dt = v
        let dy = parse_derivative("v").unwrap();
        // dv/dt = if y > 0.01 ? -9.81 else 0
        let dv = parse_derivative("if y > 0.01 ? -9.81 else 0").unwrap();

        let spec = OdeSpec::new()
            .with_state_var("y", 10.0, dy) // start 10m high
            .with_state_var("v", 0.0, dv); // start at rest

        let mut solver = spec.build_solver("bounce");
        let ctx = EvalContext::new();

        // Free fall from 10m: t_hit = sqrt(2*10/9.81) ≈ 1.428s
        // Simulate 1.4s at 1ms steps
        let dt = 0.001;
        for i in 0..1400 {
            solver.step(i as f64 * dt, dt, &ctx);
        }

        let y = solver.get_state()[0];
        let v = solver.get_state()[1];

        // At t=1.4s, ball should be near ground (y ≈ 0.38m) and moving down
        assert!(
            y > 0.0 && y < 1.0,
            "y should be near ground at t=1.4s, got {y}"
        );
        assert!(
            v < -10.0,
            "v should be negative (falling) at t=1.4s, got {v}"
        );

        // Continue to t=1.5s — should hit ground
        for i in 1400..1500 {
            solver.step(i as f64 * dt, dt, &ctx);
        }

        let y2 = solver.get_state()[0];
        // Ball should be at or below ground, with gravity disabled
        assert!(
            y2 < 0.5,
            "y should be at or near ground at t=1.5s, got {y2}"
        );
    }

    /// Phase 3.3: Piecewise ODE with multiple conditions.
    ///
    /// Simple thermostat: dT/dt = if T < setpoint ? heatingRate else -coolingRate
    #[test]
    fn test_piecewise_ode_thermostat() {
        let dt_expr = parse_derivative("if temperature < 22.0 ? 2.0 else -1.0").unwrap();

        let spec = OdeSpec::new().with_state_var("temperature", 18.0, dt_expr);

        let mut solver = spec.build_solver("thermostat");
        let ctx = EvalContext::new();

        // Heating phase: T starts at 18, rises at 2°/s
        let dt = 0.01;
        for i in 0..200 {
            solver.step(i as f64 * dt, dt, &ctx);
        }
        let t_at_2s = solver.get_state()[0];
        assert!(
            t_at_2s > 21.5,
            "should be near setpoint at t=2s, got {t_at_2s}"
        );

        // Continue — thermostat oscillates around setpoint (bang-bang control)
        for i in 200..500 {
            solver.step(i as f64 * dt, dt, &ctx);
        }
        let t_at_5s = solver.get_state()[0];
        // Bang-bang control oscillates near setpoint: should be within ±0.5°
        assert!(
            (t_at_5s - 22.0).abs() < 0.5,
            "should be near setpoint at t=5s, got {t_at_5s}"
        );
    }

    // -----------------------------------------------------------------------
    // Model scenario tests: radiation T^4, Coulomb friction, 3-phase AC
    // -----------------------------------------------------------------------

    /// Radiation cooling: dT/dt = -(ε·σ·A·(T⁴ - T_amb⁴)) / C
    ///
    /// Hot steel billet (1000K) radiating in vacuum. Should cool toward
    /// ambient (300K) with the nonlinear T⁴ law.
    #[test]
    fn test_radiation_t4_cooling() {
        // dT/dt = -(ε·σ·A·(T^4 - T_amb^4)) / thermalCapacity
        let deriv = parse_derivative(
            "-(emissivity * stefanBoltzmann * surfaceArea * (temperature ** 4 - ambientTemp ** 4)) / thermalCapacity"
        ).unwrap();

        let spec = OdeSpec::new()
            .with_state_var("temperature", 1000.0, deriv)
            .with_param("emissivity", 0.9)
            .with_param("stefanBoltzmann", 5.67e-8)
            .with_param("surfaceArea", 0.1)
            .with_param("thermalCapacity", 500.0)
            .with_param("ambientTemp", 300.0);

        let mut solver = spec.build_solver("radiation");
        let ctx = EvalContext::new();

        // Simulate 2000s at 0.1s steps
        let dt = 0.1;
        let n_steps = 20_000;
        for i in 0..n_steps {
            solver.step(i as f64 * dt, dt, &ctx);
        }

        let t_final = solver.get_state()[0];

        // After 2000s of radiation from 1000K:
        // Cooling rate at 1000K: -(0.9 * 5.67e-8 * 0.1 * (1e12 - 8.1e9)) / 500
        //                      ≈ -(5.103e-9 * 9.919e11) / 500 ≈ -10.12 K/s initially
        // Temperature should drop significantly but stay above ambient
        assert!(
            t_final > 300.0 && t_final < 800.0,
            "T should cool from 1000K toward ambient, got {t_final:.1}K"
        );

        // Verify it's actually cooling (not diverging)
        let mut solver2 = spec.build_solver("radiation_short");
        for i in 0..100 {
            solver2.step(i as f64 * dt, dt, &ctx);
        }
        let t_10s = solver2.get_state()[0];
        assert!(
            t_10s < 1000.0 && t_10s > 800.0,
            "T should drop modestly in first 10s, got {t_10s:.1}K"
        );
    }

    /// Coulomb friction: sliding block with smoothed friction.
    ///
    /// dx/dt = v
    /// dv/dt = (F_applied - μ·m·g·v/√(v²+ε²)) / m
    ///
    /// Block starts at v=1 m/s with F_applied=5N and friction μ=0.3.
    /// F_friction_max = μ·m·g = 0.3·1·9.81 = 2.943N < 5N → accelerates.
    #[test]
    fn test_coulomb_friction_sliding_block() {
        let dx = parse_derivative("velocity").unwrap();
        // Smoothed Coulomb: F_fric = μ·m·g·v/√(v²+ε²)
        // Use sqrt() stdlib function instead of ** 0.5 to avoid precedence issues
        let dv = parse_derivative(
            "(appliedForce - frictionCoeff * mass * gravity * velocity / sqrt(velocity * velocity + eps * eps)) / mass"
        ).unwrap();

        let spec = OdeSpec::new()
            .with_state_var("position", 0.0, dx)
            .with_state_var("velocity", 1.0, dv)
            .with_param("mass", 1.0)
            .with_param("frictionCoeff", 0.3)
            .with_param("gravity", 9.81)
            .with_param("eps", 0.01) // smoothing threshold
            .with_param("appliedForce", 5.0);

        let mut solver = spec.build_solver("friction");
        let ctx = EvalContext::new();

        // Simulate 5s at 1ms steps
        let dt = 0.001;
        for i in 0..5000 {
            solver.step(i as f64 * dt, dt, &ctx);
        }

        let x = solver.get_state()[0];
        let v = solver.get_state()[1];

        // Net force = 5.0 - 2.943 ≈ 2.057N → a ≈ 2.057 m/s²
        // After 5s: v ≈ 1 + 2.057*5 ≈ 11.3 m/s (approximate, friction varies with v)
        assert!(
            v > 5.0,
            "velocity should increase under net positive force, got {v:.2}"
        );
        assert!(
            x > 10.0,
            "block should travel significant distance, got {x:.2}m"
        );

        // Test friction-dominated case: no applied force, should decelerate
        let dv_no_push = parse_derivative(
            "(0 - frictionCoeff * mass * gravity * velocity / sqrt(velocity * velocity + eps * eps)) / mass"
        ).unwrap();

        let spec2 = OdeSpec::new()
            .with_state_var("position", 0.0, parse_derivative("velocity").unwrap())
            .with_state_var("velocity", 10.0, dv_no_push)
            .with_param("mass", 1.0)
            .with_param("frictionCoeff", 0.3)
            .with_param("gravity", 9.81)
            .with_param("eps", 0.01)
            .with_param("appliedForce", 0.0);

        let mut solver2 = spec2.build_solver("friction_decel");
        for i in 0..5000 {
            solver2.step(i as f64 * dt, dt, &ctx);
        }

        let v2 = solver2.get_state()[1];
        // With F_friction = 2.943N, a = -2.943 m/s² → v should hit 0 in ~3.4s
        assert!(
            v2.abs() < 0.1,
            "velocity should be near zero after deceleration, got {v2:.4}"
        );
    }

    /// Three-phase AC: balanced sinusoidal voltages with resistive load.
    ///
    /// V_a = Vpeak·sin(ωt), V_b = Vpeak·sin(ωt - 2π/3), V_c = Vpeak·sin(ωt + 2π/3)
    /// I = V/R, P = V²/R
    /// Total P = 3·Vpeak²/(2R) = constant for balanced load.
    #[test]
    fn test_three_phase_ac_balanced() {
        // Energy accumulator: dE/dt = P_total
        // P_total = (vA*vA + vB*vB + vC*vC) / resistance
        // Where vA/vB/vC are signal expressions evaluated each tick.
        let de = parse_derivative("(vA * vA + vB * vB + vC * vC) / resistance").unwrap();

        // Signal expressions for 3-phase voltages
        let v_a = parse_derivative("vPeak * sin(omega * t)").unwrap();
        let v_b = parse_derivative("vPeak * sin(omega * t - 2.0944)").unwrap(); // 2π/3
        let v_c = parse_derivative("vPeak * sin(omega * t + 2.0944)").unwrap();

        let spec = OdeSpec::new()
            .with_state_var("energy", 0.0, de)
            .with_param("vPeak", 325.27) // 230V RMS × √2
            .with_param("omega", 314.159) // 2π × 50Hz
            .with_param("resistance", 10.0)
            .with_signal("vA", v_a)
            .with_signal("vB", v_b)
            .with_signal("vC", v_c);

        let mut solver = spec.build_solver("three_phase");
        let ctx = EvalContext::new();

        // Simulate exactly 1 full cycle: T = 1/50Hz = 0.02s
        let dt = 0.0001; // 100μs for accuracy
        let one_cycle = 200; // 0.02s / 0.0001s
        for i in 0..one_cycle {
            solver.step(i as f64 * dt, dt, &ctx);
        }

        let energy_one_cycle = solver.get_state()[0];

        // Expected: P_avg = 3·Vpeak²/(2·R) = 3·325.27²/(2·10) = 15,869.7W
        // Energy in one cycle: P_avg × T = 15869.7 × 0.02 = 317.4 J
        let expected_energy = 3.0 * 325.27_f64.powi(2) / (2.0 * 10.0) * 0.02;
        assert!(
            (energy_one_cycle - expected_energy).abs() / expected_energy < 0.02,
            "energy in one cycle should be ~{expected_energy:.1}J, got {energy_one_cycle:.1}J"
        );

        // Verify neutral current balance: I_a + I_b + I_c ≈ 0
        // At any instant, sin(ωt) + sin(ωt-2π/3) + sin(ωt+2π/3) = 0
        // Test at t = 0.005s (quarter cycle)
        let t_test: f64 = 0.005;
        let omega: f64 = 314.159;
        let v_peak: f64 = 325.27;
        let i_a = v_peak * (omega * t_test).sin() / 10.0;
        let i_b = v_peak * (omega * t_test - 2.0944_f64).sin() / 10.0;
        let i_c = v_peak * (omega * t_test + 2.0944_f64).sin() / 10.0;
        let i_neutral = i_a + i_b + i_c;
        assert!(
            i_neutral.abs() < 0.01,
            "neutral current should be ~0 for balanced load, got {i_neutral:.6}A"
        );
    }

    // -----------------------------------------------------------------------
    // Nanocrystalline-core oscillator physics tests
    // -----------------------------------------------------------------------

    /// Test the two-stage tanh BH model produces correct hysteresis shape.
    ///
    /// At H=0: B should equal Br (remanent) on ascending branch
    /// At H=Hc: B should be near 0 (coercive point)
    /// At H>>Hc: B should approach Bs (saturation)
    #[test]
    fn test_rcd_bh_model_shape() {
        // Material: VITROPERM 500F defaults
        let bs: f64 = 0.863;
        let br: f64 = 0.597;
        let hc: f64 = 7.95;

        // Shape params from regression
        let br_bs = br / bs; // 0.692
        let k1 = (0.857 + 1.89 * (br_bs - 0.692)).clamp(0.5, 1.5);
        let k2 = (2.53 + 0.42 * (hc - 7.95) + 0.3 * (br_bs - 0.692)).clamp(2.0, 3.5);
        let beta = (2.75 + 0.68 * (k2 - 2.53) + 0.27 * (br_bs - 0.692)).clamp(2.3, 3.2);

        // B_ascending(H) using two-stage tanh
        let b_ascending = |h: f64| -> f64 {
            let x = h / hc - 1.0;
            let stage1 = (k1 * x).tanh();
            let stage2 = (k2 * x).tanh();
            let w = 0.5 * (1.0 + (beta * x).tanh());
            bs * ((1.0 - w) * stage1 + w * stage2)
        };

        // At H=0: ascending branch should give approximately -Br
        // (starting from negative saturation, H=0 is at negative remanence)
        let b_at_0 = b_ascending(0.0);
        assert!(
            (b_at_0 - (-br)).abs() < 0.1,
            "B at H=0 should be near -Br={br:.3}, got {b_at_0:.3}"
        );

        // At H=Hc: should cross through ~0
        let b_at_hc = b_ascending(hc);
        assert!(
            b_at_hc.abs() < 0.1,
            "B at H=Hc should be near 0, got {b_at_hc:.3}"
        );

        // At H=3*Hc: should approach saturation
        let b_at_3hc = b_ascending(3.0 * hc);
        assert!(
            b_at_3hc > 0.7 * bs,
            "B at H=3Hc should approach Bs, got {b_at_3hc:.3}"
        );

        // B_descending(H) = -B_ascending(-H) (point symmetry)
        let b_desc_at_0 = -b_ascending(0.0);
        assert!(
            (b_desc_at_0 - br).abs() < 0.1,
            "B_descending at H=0 should be near +Br, got {b_desc_at_0:.3}"
        );
    }

    /// Test the core ODE: single ascending half-cycle.
    ///
    /// Starting at B = -Br, apply +V_supply. The core should sweep
    /// from -Br toward +Bs. When current exceeds I_threshold, the
    /// comparator should flip (in the real system).
    ///
    /// Expected half-period: T_half ≈ 2*N*Ae*(Bs+Br) / V_supply
    #[test]
    fn test_protection_core_ode_ascending_halfcycle() {
        // Core parameters
        let bs: f64 = 0.863;
        let br: f64 = 0.597;
        let hc: f64 = 7.95;
        let ae: f64 = 3.2e-6;
        let le: f64 = 54e-3;
        let n_drive: f64 = 17.0;
        let v_supply: f64 = 3.47;
        let r_circuit: f64 = 36.0;

        // Shape params
        let br_bs = br / bs;
        let k1 = (0.857 + 1.89 * (br_bs - 0.692)).clamp(0.5, 1.5);
        let k2 = (2.53 + 0.42 * (hc - 7.95) + 0.3 * (br_bs - 0.692)).clamp(2.0, 3.5);
        let beta = (2.75 + 0.68 * (k2 - 2.53) + 0.27 * (br_bs - 0.692)).clamp(2.3, 3.2);

        // BH inverse: build lookup table (B → H for ascending branch)
        let b_ascending = |h: f64| -> f64 {
            let x = h / hc - 1.0;
            let stage1 = (k1 * x).tanh();
            let stage2 = (k2 * x).tanh();
            let w = 0.5 * (1.0 + (beta * x).tanh());
            bs * ((1.0 - w) * stage1 + w * stage2)
        };

        // Build inverse lookup: 200 points from H=-3Hc to H=+3Hc
        let n_points = 200;
        let mut bh_b: Vec<f64> = Vec::with_capacity(n_points);
        let mut bh_h: Vec<f64> = Vec::with_capacity(n_points);
        for i in 0..n_points {
            let h = -3.0 * hc + (6.0 * hc * i as f64) / (n_points as f64 - 1.0);
            let b = b_ascending(h);
            bh_b.push(b);
            bh_h.push(h);
        }

        // ODE: dB/dt = (V_applied - R * i_drive) / (N * Ae)
        // With V_applied = +V_supply (ascending mode), no DC bias
        // Clone lookup tables into the closure
        let bh_b_clone = bh_b.clone();
        let bh_h_clone = bh_h.clone();
        let rhs = std::sync::Arc::new(move |_t: f64, y: &[f64], _ctx: &EvalContext| -> Vec<f64> {
            let b = y[0];
            // Inline inverse lookup
            let h_total = {
                let bb = &bh_b_clone;
                let bh = &bh_h_clone;
                if b <= bb[0] {
                    bh[0]
                } else if b >= bb[bb.len() - 1] {
                    bh[bh.len() - 1]
                } else {
                    let mut h_val = bh[bh.len() - 1];
                    for i in 0..bb.len() - 1 {
                        if bb[i] <= b && b <= bb[i + 1] {
                            let frac = (b - bb[i]) / (bb[i + 1] - bb[i]);
                            h_val = bh[i] + frac * (bh[i + 1] - bh[i]);
                            break;
                        }
                    }
                    h_val
                }
            };
            let i_drive = le * h_total / n_drive;
            let v_net = v_supply - r_circuit * i_drive;
            let db_dt = v_net / (n_drive * ae);
            vec![db_dt]
        });

        // Start at B = -Br (negative remanence)
        let mut solver =
            crate::ode::Rk4Solver::new("protection_core", vec!["B".to_string()], vec![-br], rhs);

        let ctx = EvalContext::new();
        let dt = 1e-7; // 100ns step (oscillator runs at ~10kHz)

        // Simulate ascending half-cycle (B should sweep from -Br to near +Bs)
        let mut peak_b: f64 = -br;
        let mut half_period_us: f64 = 0.0;
        for i in 0..100_000 {
            solver.step(i as f64 * dt, dt, &ctx);
            let b = solver.get_state()[0];
            if b > peak_b {
                peak_b = b;
            }
            // Detect when B crosses zero (midpoint of sweep)
            if b > 0.0 && half_period_us == 0.0 {
                half_period_us = i as f64 * dt * 1e6;
            }
            // Stop if approaching saturation
            if b > 0.95 * bs {
                break;
            }
        }

        // B should have swept from -Br past 0 toward +Bs
        assert!(
            peak_b > 0.5 * bs,
            "B should reach at least 50% of Bs during ascending sweep, got {peak_b:.3}T"
        );

        // Analytical estimate: T_half ≈ 2*N*Ae*(Bs+Br) / V
        // (simplified, ignoring R*i drop)
        let t_half_ideal = 2.0 * n_drive * ae * (bs + br) / v_supply;
        let f_ideal = 1.0 / (2.0 * t_half_ideal);

        println!("ascending half-cycle:");
        println!("  peak B = {peak_b:.3} T (Bs = {bs})");
        println!("  zero-crossing at {half_period_us:.1} us");
        println!(
            "  analytical T_half = {:.1} us, f_ideal = {:.0} Hz",
            t_half_ideal * 1e6,
            f_ideal
        );

        // Frequency should be in the right ballpark (5-20 kHz for these params)
        assert!(
            f_ideal > 5000.0 && f_ideal < 25000.0,
            "oscillation frequency should be 5-25 kHz, got {f_ideal:.0} Hz"
        );
    }

    /// Test DC bias effect: fault current shifts the core's operating point.
    ///
    /// H_dc = N_fault * I_residual / le. This biases the BH curve so the
    /// ascending half-cycle is shorter than the descending half-cycle.
    /// The duty cycle distortion is the detection mechanism.
    ///
    /// Key metric: r = H_dc / Hc (ratio of DC bias to coercive field).
    /// When r approaches 1.0, the core saturates on one side and the
    /// oscillator stalls — definite fault detection.
    #[test]
    fn test_rcd_dc_bias_detection() {
        let hc: f64 = 7.95; // coercive field (A/m)
        let le: f64 = 54e-3; // magnetic path length (m)
        let n_fault: f64 = 1.0; // sense winding turns

        println!("DC bias detection:");
        println!("  Hc = {hc} A/m, le = {le} m, N_fault = {n_fault}");

        // Test at different fault currents
        // r = H_dc / Hc = (N_fault * I_residual) / (le * Hc)
        // At Idn = 30mA: H_dc = 1*0.03/0.054 = 0.556 A/m, r = 0.556/7.95 = 0.070
        // Detection threshold in the Python sim uses the comparator, not Hc.
        // For this test, we verify the physics is monotonic and proportional.

        let test_points: Vec<(f64, &str)> = vec![
            (0.0, "no fault"),
            (15.0e-3, "0.5x Idn (15mA)"),
            (30.0e-3, "1.0x Idn (30mA)"),
            (150.0e-3, "5.0x Idn (150mA)"),
            (500.0e-3, "extreme (500mA)"),
        ];

        let mut prev_h_dc = -1.0_f64;
        for (i_residual, label) in &test_points {
            let h_dc = n_fault * i_residual / le;
            let r = h_dc / hc;

            println!("  {label}: H_dc = {h_dc:.3} A/m, r = {r:.4}");

            // H_dc should be monotonically increasing with fault current
            assert!(h_dc > prev_h_dc, "H_dc should increase with fault current");
            prev_h_dc = h_dc;
        }

        // At 500mA: H_dc = 9.26 A/m > Hc = 7.95 → r > 1.0 → oscillator stalls
        let h_dc_extreme = n_fault * 500.0e-3 / le;
        let r_extreme = h_dc_extreme / hc;
        assert!(
            r_extreme > 1.0,
            "at 500mA, r should exceed 1.0 (oscillator stall), got {r_extreme:.3}"
        );

        // At 30mA: H_dc = 0.556 A/m, well below Hc → detectable but not stalling
        let h_dc_rated = n_fault * 30.0e-3 / le;
        assert!(
            h_dc_rated < hc,
            "at Idn, H_dc ({h_dc_rated:.3}) should be below Hc ({hc})"
        );
        assert!(h_dc_rated > 0.0, "at Idn, H_dc should be positive");
    }

    /// WS-D Stage 2 duty-seam fix, precondition pin (steward-required, see
    /// `build_signal_sync`'s comment block): reproduces the general shape of
    /// the oscillator `duty` case — a parameter with a subsystem-local slot, also
    /// read by a signal expression — in isolation, and asserts
    /// `template_omits` actually reports it once `bind_slots` runs. The
    /// steward flagged that this is NOT automatic: `bind_slots`'s eligibility
    /// walk empties the whole omission set the moment ANY FeatureChain /
    /// SlotChainHead appears anywhere in `derivative_exprs` or
    /// `signal_exprs` — so a model with config-Map/LUT-heavy expressions
    /// (like the oscillator fixture's real spec) could plausibly land on `eligible=false` and
    /// silently no-op the fix. This test proves the mechanism directly
    /// rather than inferring it from the black-box chain test's color.
    #[test]
    fn servable_slot_parameter_is_omitted_from_signal_template_when_eligible() {
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
        use sysml_core::ElementId;

        let mut store = SlotStore::new();
        store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::new_v4()),
                Variability::Continuous,
                WriterId::Orchestrator,
                "duty",
                "duty",
            ),
            Value::Float(0.0),
        );

        let mut spec = OdeSpec::new()
            .with_state_var("x", 0.0, parse_derivative("1.0").unwrap())
            .with_param("duty", 0.0);
        spec.signal_exprs
            .insert("I_est".to_owned(), parse_derivative("duty").unwrap());

        let report = spec.bind_slots(&store, None);
        assert!(
            report.unresolved.is_empty(),
            "unexpected unresolved names: {:?}",
            report.unresolved
        );
        assert!(
            spec.scoped_bypass_eligible(),
            "spec should be scoped-bypass-eligible (no FeatureChain/SlotChainHead present)"
        );
        assert!(
            spec.template_omits("duty"),
            "a parameter with a subsystem-local slot must be omitted from the signal \
             template once the spec is eligible — otherwise build_signal_sync's fix \
             silently no-ops"
        );
    }

    /// End-to-end pin for the second half of the duty-seam fix: even with
    /// `duty` correctly omitted from `template_ctx` (previous test), a stale
    /// name-keyed `duty` entry in the `shared` context merged into
    /// `build_signal_sync`'s closure must not shadow the live slot. Models
    /// the exact scenario that defeated the first-half-only fix: `shared`
    /// carries a frozen `duty` = 0.0 (as `context_from_graph` would seed from
    /// a literal model default), while the live value lives only in the slot
    /// store the closure's `out`/slot-reads would otherwise reach.
    #[test]
    fn signal_sync_does_not_let_stale_shared_name_shadow_omitted_slot() {
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
        use sysml_core::ElementId;
        use std::sync::{Arc, RwLock};

        let mut store = SlotStore::new();
        let slot = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::new_v4()),
                Variability::Continuous,
                WriterId::Orchestrator,
                "duty",
                "duty",
            ),
            Value::Float(0.42), // the LIVE value, only ever visible via the slot
        );

        let mut spec = OdeSpec::new()
            .with_state_var("x", 0.0, parse_derivative("1.0").unwrap())
            .with_param("duty", 0.0);
        spec.signal_exprs
            .insert("I_est".to_owned(), parse_derivative("duty").unwrap());
        spec.bind_slots(&store, None);
        assert!(spec.template_omits("duty"), "precondition: see previous test");

        let sync = spec.build_signal_sync().expect("signal_exprs is non-empty");

        // `shared` reproduces the master/orchestrator context: a stale bare
        // "duty" name entry frozen at its literal default, exactly as
        // `context_from_graph` seeds it and as the tracker's write path
        // (which only ever updates the *qualified* key) leaves it forever.
        let mut shared = EvalContext::new();
        shared.set("duty", Value::Float(0.0));
        shared.slots = Some(Arc::new(RwLock::new(store)));

        let mut out = EvalContext::new();
        sync(&[0.0], &shared, &mut out);

        assert_eq!(
            out.get("I_est"),
            Some(&Value::Float(0.42)),
            "I_est must track duty's live SLOT value (0.42), not the stale shared \
             name-map entry (0.0) merge_from would otherwise let win"
        );
        let _ = slot;
    }

    /// Deterministic red test for the actual nondeterminism source behind the
    /// flaky `duty_lut_consumer_chain_responds_to_bias` (ws_d_duty_mint_seam.rs)
    /// — this is the gate; the chain test is corroborating evidence only, run
    /// 10x consecutively in the commit body, not what's asserted here.
    ///
    /// A signal's own target name (`a`, mirroring the fixture's `I_est`) can ALSO be a
    /// member of `template_omitted_params`: it carries a literal default
    /// (`attribute a : Real default 0.0;`-shaped) AND a mint-time-servable
    /// slot (RSC-3.0 cat-2 mints one for every `GetOutput`-derived quantity,
    /// not just externally-tracked parameters like `duty`). A downstream
    /// signal (`b = a + 1.0`, mirroring the fixture's `I_norm = I_est / I_dn`) relies
    /// on `a`'s value carrying forward from the PREVIOUS tick via the name
    /// map (`merge_from(shared)`) — the slot only reflects the previous
    /// tick's write, since `write_signals` batches this tick's slot writes
    /// AFTER the whole signal loop finishes. The first cut of the duty-seam
    /// fix blanket-stripped every name in `template_omitted_params` after
    /// `merge_from`, including a signal's own target name — silently
    /// dropping this carryover. Whether that loss was ever OBSERVABLE
    /// depended on `signal_exprs`' random `HashMap`-derived iteration order
    /// (a `Vec` collected fresh each `build_signal_sync` call) — the
    /// ws_determinism disease family, not a guard/timing bug. This test does
    /// not depend on that randomness at all: it drives two ticks directly and
    /// asserts `a`'s tick-1 value is visible to `b` on tick 2, regardless of
    /// which order `a`/`b` were evaluated in on either tick.
    #[test]
    fn signal_own_target_name_survives_merge_carryover_for_downstream_signal() {
        use std::sync::{Arc, RwLock};

        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
        use sysml_core::ElementId;

        let mut store = SlotStore::new();
        store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::new_v4()),
                Variability::Continuous,
                WriterId::Orchestrator,
                "duty",
                "duty",
            ),
            Value::Float(5.0),
        );
        store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::new_v4()),
                Variability::Continuous,
                WriterId::Orchestrator,
                "a",
                "a",
            ),
            Value::Float(0.0),
        );

        let mut spec = OdeSpec::new()
            .with_state_var("x", 0.0, parse_derivative("1.0").unwrap())
            .with_param("duty", 0.0)
            .with_param("a", 0.0);
        spec.signal_exprs
            .insert("a".to_owned(), parse_derivative("duty").unwrap());
        spec.signal_exprs
            .insert("b".to_owned(), parse_derivative("a + 1.0").unwrap());
        spec.bind_slots(&store, None);
        assert!(
            spec.template_omits("a"),
            "precondition: `a` must land in the omitted set (servable-slot signal \
             target) for this test to exercise the fix"
        );

        let sync = spec.build_signal_sync().expect("signal_exprs is non-empty");
        let store = Arc::new(RwLock::new(store));

        // Tick 1: nothing carried over yet — `a` computes fresh from `duty`'s slot.
        let mut shared1 = EvalContext::new();
        shared1.slots = Some(Arc::clone(&store));
        let mut out1 = EvalContext::new();
        sync(&[0.0], &shared1, &mut out1);
        let a_tick1 = out1
            .get("a")
            .cloned()
            .expect("a should compute from duty's slot on tick 1");

        // Tick 2: `shared` carries `a`'s tick-1 value ONLY via the legacy
        // name map (no slot attached at all) — isolating exactly what the
        // fix protects: a signal's own carried-over NAME-MAP entry must
        // survive `merge_from` + the omission strip. If it doesn't, there is
        // no slot fallback to save it here (deliberately, to keep this test
        // from silently passing via a fallback path the real bug's mechanism
        // doesn't reliably get either).
        let mut shared2 = EvalContext::new();
        shared2.set("a", a_tick1.clone());
        let mut out2 = EvalContext::new();
        sync(&[0.0], &shared2, &mut out2);

        let expected_b = match a_tick1 {
            Value::Float(f) => f + 1.0,
            other => panic!("expected a Float from tick 1, got {other:?}"),
        };
        assert_eq!(
            out2.get("b"),
            Some(&Value::Float(expected_b)),
            "b = a + 1.0 must see `a`'s tick-1 value carried over via the name map \
             — stripping a signal's own target name from the merge (the bug) makes \
             this read silently missing instead of merely one-tick-stale"
        );
    }
}
