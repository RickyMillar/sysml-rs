//! Canonical orchestrator constructors and subsystem registration.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use sysml_core::{ElementId, ElementKind, Value};

use crate::expressions::{EvalContext, ExprIR};
use crate::ode::Rk4Solver;
use crate::ode_builder::{self, OdeSpec};
use crate::orchestrator::{Orchestrator, OrchestratorConfig, SubsystemIndex};
use crate::statemachine::{StateMachineCompiler, StateMachineRunner};
use crate::StateMachineIR;

use super::*;

/// The graph-invariant compile products of a single-SM ODE orchestrator,
/// produced by [`ModelCompiler::prepare_single_ode`] and consumed by
/// [`ModelCompiler::build_orchestrator_from_prepared`].
///
/// None of these depend on the per-variant parameter `overrides`, so a
/// parameter sweep prepares this once and assembles N orchestrators from it
/// (RSC-6.2) instead of re-walking the graph per variant.
#[derive(Debug, Clone)]
pub struct PreparedSingleOde {
    /// State-machine name this orchestrator drives (the `sm_name` argument).
    pub sm_name: String,
    /// Compiled state-machine IR.
    pub sm_ir: StateMachineIR,
    /// Detected ODE configuration (state vars, params, derivative/signal exprs).
    pub ode: OdeDetection,
    /// Parsed derivative expressions, one per state var (same order as
    /// `ode.state_vars`).
    pub compiled_derivs: Vec<ExprIR>,
    /// Parsed signal expressions keyed by attribute name (best-effort subset of
    /// `ode.signal_exprs`).
    pub compiled_signals: Vec<(String, ExprIR)>,
    /// SampledFunction lookup tables to inject as `__sf_*` context globals.
    pub sampled_functions: Vec<(String, Value)>,
}

// ---------------------------------------------------------------------------
// Context building (moved from sysml-service/src/parse.rs)
// ---------------------------------------------------------------------------

impl ModelCompiler {
    /// Build a fully-wired `Orchestrator` from the model.
    ///
    /// This is the unified replacement for the ~80 lines of ODE-build
    /// pipeline that was copy-pasted across multiple service commands.
    ///
    /// Steps:
    /// 1. Compile the named state machine.
    /// 2. Detect ODE metadata (returns error if not found).
    /// 3. Compile derivative and signal expressions.
    /// 4. Build `OdeSpec` and the appropriate solver (RK4 or RK45).
    /// 5. Create `Orchestrator` with config, add SM + ODE subsystems.
    /// 6. Seed the orchestrator context with ODE parameters + overrides.
    pub fn build_orchestrator(
        &self,
        sm_name: &str,
        overrides: &[(String, String)],
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<Orchestrator, CompileError> {
        // Single-call path: prepare the graph-invariant pieces, then assemble.
        // A loop that varies only `overrides` (e.g. `ode_sweep`) should call
        // `prepare_single_ode` ONCE and `build_orchestrator_from_prepared` per
        // variant — see those methods (RSC-6.2). This delegation keeps the
        // single-call result byte-identical to the former inline body.
        let prepared = self.prepare_single_ode(sm_name)?;
        self.build_orchestrator_from_prepared(&prepared, overrides, dt_ms, max_time_ms)
    }

    /// Compile the **graph-invariant** pieces of a single-SM ODE orchestrator
    /// once, for reuse across many [`build_orchestrator_from_prepared`] calls
    /// that differ only in parameter `overrides`.
    ///
    /// State-machine compilation, the unified SSR ODE detection (a full-graph
    /// walk), the derivative/signal expression parse, and the sampled-function
    /// extraction do **not** depend on `overrides` — only the swept parameter
    /// *values* do, and those enter at assembly time as a float map. Per
    /// CLAUDE.md principle 5b (performance is in the DNA), an `ode_sweep` of N
    /// variants must not re-run this graph walk N times; it prepares once and
    /// assembles per variant. The result mirrors how
    /// [`build_workspace_orchestrator`] takes pre-built cached inputs.
    pub fn prepare_single_ode(&self, sm_name: &str) -> Result<PreparedSingleOde, CompileError> {
        // 1. Compile state machine
        let sm_ir = self.compile_state_machine(sm_name)?;

        // 2. Detect ODE via unified SSR + metadata path
        let ode = self.detect_ode().ok_or_else(|| {
            CompileError::from_message(
                "no ODE detected: expected calc def :> GetDerivative (SSR pattern) \
                 in model, optionally with @ToolExecution { toolName } for solver selection",
            )
        })?;
        // Fail hard on any unresolved derivative→state-var mapping recorded at
        // detection (non-match or ambiguity) — never a silent constant-0 RHS.
        ode.ensure_derivatives_matched()?;

        // 3. Compile derivative expressions
        let compiled_derivs: Vec<ExprIR> = ode
            .derivative_exprs
            .iter()
            .map(|expr_str| {
                ode_builder::parse_derivative(expr_str).map_err(|e| {
                    CompileError::from_message(format!("derivative compile error: {e}"))
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        // Compile signal expressions (best-effort — skip failures)
        let compiled_signals: Vec<(String, ExprIR)> = ode
            .signal_exprs
            .iter()
            .filter_map(|(name, expr_str)| {
                ode_builder::parse_derivative(expr_str)
                    .ok()
                    .map(|ir| (name.clone(), ir))
            })
            .collect();

        // Sampled-function lookup tables (`__sf_*`) are a pure graph derivative
        // too — extract them once here rather than per variant (former step 6b).
        let sampled_functions = self.extract_sampled_functions()?;

        Ok(PreparedSingleOde {
            sm_name: sm_name.to_owned(),
            sm_ir,
            ode,
            compiled_derivs,
            compiled_signals,
            sampled_functions,
        })
    }

    /// Assemble a single-SM ODE [`Orchestrator`] from pre-compiled pieces plus
    /// the per-variant parameter `overrides`. The companion of
    /// [`prepare_single_ode`]; together they replace the former monolithic
    /// `build_orchestrator` so a parameter sweep pays the graph walk once.
    ///
    /// Takes `prepared` by reference and clones only the cheap structural IR it
    /// needs, so the same `prepared` may be reused across many variants without
    /// mutation — the result is identical to assembling from a freshly prepared
    /// value (gated by `rsc62_prepared_ode_equivalence`).
    pub fn build_orchestrator_from_prepared(
        &self,
        prepared: &PreparedSingleOde,
        overrides: &[(String, String)],
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<Orchestrator, CompileError> {
        let sm_name = prepared.sm_name.as_str();
        let sm_ir = prepared.sm_ir.clone();
        let ode = &prepared.ode;
        let compiled_derivs = &prepared.compiled_derivs;
        let compiled_signals = &prepared.compiled_signals;

        // Fail hard on a build-time override that names no real target, the
        // compiler-harness counterpart of the session path's RS002
        // (`apply_overrides_with_aliases`). Was a silent no-op: an `ode_sweep`
        // over a mistyped `parameter_name`, or any typo'd/qualified key,
        // baked nothing and produced a *flat* sweep with no error. On this
        // single-ODE path every target is bare (ODE parameter, ODE state var,
        // or SM assignment target); a qualified spelling of an ODE parameter
        // can never reach the RHS-baked constant, so it earns a precise
        // "use the bare key" error rather than the old silent drop.
        let sm_assign_targets = Self::collect_sm_assignment_targets(&sm_ir);
        let mut known_bare: std::collections::HashSet<&str> = std::collections::HashSet::new();
        known_bare.extend(ode.parameters.keys().map(String::as_str));
        known_bare.extend(ode.state_vars.iter().map(String::as_str));
        known_bare.extend(sm_assign_targets.iter().map(String::as_str));
        let known_params: std::collections::HashSet<&str> =
            ode.parameters.keys().map(String::as_str).collect();
        validate_build_override_targets(overrides, &known_bare, &known_params)?;

        // Parse overrides into a float map for merging with ODE params
        let override_map: HashMap<&str, f64> = overrides
            .iter()
            .filter_map(|(k, v)| v.parse::<f64>().ok().map(|f| (k.as_str(), f)))
            .collect();

        // 4. Build ODE spec
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
        for (sig_name, sig_ir) in compiled_signals {
            spec = spec.with_signal(sig_name.clone(), sig_ir.clone());
        }
        // Task #8: fix the canonical dependency evaluation order of the signal
        // expressions ONCE, before any RHS/signal-sync closure captures it — a
        // cyclic definition is a hard CompileError.
        spec.compute_signal_order()?;
        let rhs = spec.build_rhs();

        // 5. Create orchestrator with config
        let dt = dt_ms.unwrap_or(1.0);
        let max_t = max_time_ms.unwrap_or(60_000.0);

        let config = OrchestratorConfig {
            dt_ms: dt,
            max_ticks: (max_t / dt) as u64 + 1,
            max_time_ms: max_t,
            ..Default::default()
        };
        let mut orchestrator = Orchestrator::new(config);
        let (sm_index, sm_targets, sm_payload_ports) =
            Self::register_sm_targets(&mut orchestrator, sm_name, sm_ir);

        // 6. Seed context with ODE parameters + overrides.
        //    Done BEFORE the solver branch so the WS-B4 stiffness classifier
        //    (in the `else` arm) evaluates the RHS against the same seeded
        //    context the integrator will see at run time.
        for (k, v) in &ode.parameters {
            let val = override_map.get(k.as_str()).copied().unwrap_or(*v);
            orchestrator.context.set(k.clone(), Value::Float(val));
        }

        // 6b. RSC-3.7 C.1/C.2: SampledFunction lookup tables are minted as
        //     global `Parameter` slots in `mint_slot_store` and the `bind_slots`
        //     passes rewrite every `interpolate(__sf_…, …)` read to a `SlotRef`,
        //     so the bare `__sf_{name}` context key is no longer injected here —
        //     reads resolve through the slot store. Mirrors
        //     `build_workspace_orchestrator` step 2c. `prepared.sampled_functions`
        //     is still the (graph-invariant) source the mint step re-derives via
        //     `extract_sampled_functions`.
        let _ = &prepared.sampled_functions;

        // Explicit `@ToolExecution` choices honored verbatim; an un-annotated
        // ODE goes through WS-B4 auto-switch (stiff⇒BDF, else⇒RK45) in the
        // `else` arm. The same `spec`/`rhs` (built once in `prepare_single_ode`,
        // config already folded in) feeds whichever solver is selected.
        let ode_label = "compiled-ode";
        let ode_index: SubsystemIndex = if ode.is_bdf() {
            orchestrator.add_bdf(
                ode_label,
                crate::solvers::bdf::BdfSolver::new(
                    ode_label,
                    ode.state_vars.clone(),
                    ode.initial_values.clone(),
                    rhs,
                )
                .with_spec(spec),
            )
        } else if ode.is_rk4() {
            orchestrator.add_ode(
                ode_label,
                Rk4Solver::new(
                    ode_label,
                    ode.state_vars.clone(),
                    ode.initial_values.clone(),
                    rhs,
                )
                .with_spec(spec),
            )
        } else {
            let verdict = crate::solvers::bdf::classify_stiffness_at_state(
                rhs.as_ref(),
                0.0,
                &ode.initial_values,
                &orchestrator.context,
                dt / 1000.0,
            );
            Self::log_auto_solver_choice(ode_label, &verdict);
            if verdict.is_stiff {
                orchestrator.add_bdf(
                    ode_label,
                    crate::solvers::bdf::BdfSolver::new(
                        ode_label,
                        ode.state_vars.clone(),
                        ode.initial_values.clone(),
                        rhs,
                    )
                    .with_spec(spec),
                )
            } else {
                orchestrator.add_ode45(
                    ode_label,
                    crate::ode45::Rk45Solver::new(
                        ode_label,
                        ode.state_vars.clone(),
                        ode.initial_values.clone(),
                        rhs,
                    )
                    .with_spec(spec),
                )
            }
        };

        // 7. Calculation registry + spatial frame registry are now bundled
        //    into the cached `EvalContext` by
        //    `sysml_ide_db::eval_context_seed::context_from_graph` and
        //    propagated by `EvalContext::merge_from`. Per-call compile sites
        //    removed (see S3.T12 cached_calculation_registry + frame_registry).

        // 8-9. Mint the compile-known slot table, bind expressions, and run
        // the RS003/RS004/RS005 gates (shared with `build_sm_orchestrator`
        // — see `mint_bind_and_gate`'s doc comment, ledger L44).
        //
        // `ode` is borrowed from `prepared` (shared across variant reuse —
        // see the doc comment on this fn), so its `subsystem_index` can't be
        // set in place; mint from an owned clone carrying the index
        // captured above (RSC-4.2 L40 — still captured at the call site,
        // never re-derived by name).
        let mut ode_for_mint = ode.clone();
        ode_for_mint.subsystem_index = Some(ode_index);
        // P5: the single primary SM this path ever registers — same shape as
        // `build_workspace_orchestrator`'s `primary_sm_names`, just one entry.
        let primary_sm_names = [(sm_name.to_owned(), sm_index)];
        self.mint_bind_and_gate(
            &mut orchestrator,
            std::slice::from_ref(&ode_for_mint),
            &sm_targets,
            &primary_sm_names,
            &sm_payload_ports,
            &override_map,
        )?;

        Ok(orchestrator)
    }

    /// Build a single-state-machine [`Orchestrator`] with **no** continuous
    /// dynamics — the mint/bind entry point `sysml.simulate.start` (and
    /// therefore `sessions.create`'s `Pick::Simulation` arm) needs for an
    /// SM-only model (ledger L44). Before this existed, `simulate_start`
    /// assembled its orchestrator via bare `Orchestrator::new()` +
    /// `add_state_machine()` — no `mint_slot_store`/`bind_expression_slots`
    /// — so a transition effect's attribute assignment had no slot-routed
    /// writeback path and silently vanished from every snapshot (the
    /// RSC-4 string-identity cull deleted the only other writeback path).
    ///
    /// [`build_orchestrator`](Self::build_orchestrator) can't serve this: it
    /// delegates to [`prepare_single_ode`](Self::prepare_single_ode), which
    /// hard-errors via [`detect_ode`](Self::detect_ode) when there is no
    /// ODE. This method shares SM registration
    /// ([`register_sm_targets`](Self::register_sm_targets)) and the
    /// mint/bind/gate sequence
    /// ([`mint_bind_and_gate`](Self::mint_bind_and_gate)) with
    /// [`build_orchestrator_from_prepared`](Self::build_orchestrator_from_prepared)
    /// — the ODE-specific spec/solver assembly that method does simply has
    /// no counterpart here; `ode_detections` is passed as `&[]`.
    pub fn build_sm_orchestrator(
        &self,
        sm_name: &str,
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<Orchestrator, CompileError> {
        let sm_ir = self.compile_state_machine(sm_name)?;
        // Same timing contract as `build_workspace_orchestrator`: `None`
        // keeps the defaults, and `max_ticks` is derived so a caller that
        // raises the time budget is not then stopped by a stale tick cap.
        let dt = dt_ms.unwrap_or(OrchestratorConfig::default().dt_ms);
        let max_t = max_time_ms.unwrap_or(OrchestratorConfig::default().max_time_ms);
        let mut orchestrator = Orchestrator::new(OrchestratorConfig {
            dt_ms: dt,
            max_ticks: (max_t / dt) as u64 + 1,
            max_time_ms: max_t,
            ..Default::default()
        });
        let (sm_index, sm_targets, sm_payload_ports) =
            Self::register_sm_targets(&mut orchestrator, sm_name, sm_ir);
        let primary_sm_names = [(sm_name.to_owned(), sm_index)];
        let override_map: HashMap<&str, f64> = HashMap::new();
        self.mint_bind_and_gate(
            &mut orchestrator,
            &[],
            &sm_targets,
            &primary_sm_names,
            &sm_payload_ports,
            &override_map,
        )?;
        Ok(orchestrator)
    }

    /// Register a compiled state machine on `orchestrator`, returning its
    /// `SubsystemIndex` plus the `SmAssignTarget`/`SmPayloadPort` lists the
    /// slot-mint step needs. Shared by the ODE-bearing
    /// ([`build_orchestrator_from_prepared`](Self::build_orchestrator_from_prepared))
    /// and SM-only ([`build_sm_orchestrator`](Self::build_sm_orchestrator))
    /// paths. RSC-4.2 (ruling escalation 1): collect the raw name lists
    /// BEFORE registration (they only need `&sm_ir`/`&sm_runner`), register
    /// the SM to get its `SubsystemIndex`, THEN build the `SmAssignTarget`/
    /// `SmPayloadPort` structs — so `subsystem` is captured at the call
    /// site, never re-derived by name later.
    fn register_sm_targets(
        orchestrator: &mut Orchestrator,
        sm_name: &str,
        sm_ir: StateMachineIR,
    ) -> (SubsystemIndex, Vec<SmAssignTarget>, Vec<SmPayloadPort>) {
        let sm_target_names = Self::collect_sm_assignment_targets(&sm_ir);
        let sm_runner = StateMachineRunner::new(sm_ir);
        // RSC-3.5b: compile-static payload slot targets (top-level SM).
        let sm_accept_ports = sm_runner.accept_ports();
        let sm_index = orchestrator.add_state_machine(sm_name, sm_runner);
        let sm_targets: Vec<SmAssignTarget> = sm_target_names
            .into_iter()
            .map(|target| SmAssignTarget {
                runtime_name: target.clone(),
                bare_name: target,
                instance: None,
                subsystem: sm_index,
            })
            .collect();
        let sm_payload_ports: Vec<SmPayloadPort> = sm_accept_ports
            .into_iter()
            .map(|port| SmPayloadPort {
                runtime_base: port.clone(),
                canonical_base: port.clone(),
                port,
                subsystem: sm_index,
                instance: None,
            })
            .collect();
        (sm_index, sm_targets, sm_payload_ports)
    }

    /// Mint the compile-known slot table, attach it to `orchestrator`, bind
    /// expressions to slots, and run the RS003/RS004/RS005 diagnostic gates.
    /// Shared by [`build_orchestrator_from_prepared`](Self::build_orchestrator_from_prepared)
    /// (which passes its one ODE's detection) and
    /// [`build_sm_orchestrator`](Self::build_sm_orchestrator) (which passes
    /// `&[]` — no continuous dynamics). Every other `SlotMintInputs` field
    /// single-model builders leave empty/`None` (no instances, no link
    /// graph, no port registry, no physics) is fixed here too — only the
    /// ODE detections and the SM-derived lists vary by caller.
    fn mint_bind_and_gate(
        &self,
        orchestrator: &mut Orchestrator,
        ode_detections: &[OdeDetection],
        sm_targets: &[SmAssignTarget],
        primary_sm_names: &[(String, SubsystemIndex)],
        sm_payload_ports: &[SmPayloadPort],
        override_map: &HashMap<&str, f64>,
    ) -> Result<(), CompileError> {
        let empty_multiplied: HashSet<String> = HashSet::new();
        // Single-model builders never call `wire_zero_crossing_detectors`
        // (no `accept when` comparator wiring here), so this is empty today
        // — collected generally rather than hardcoded `&HashSet::new()` so a
        // future path that does wire duty trackers here doesn't silently
        // drop them.
        let duty_tracker_odes: HashSet<SubsystemIndex> =
            orchestrator.duty_tracker_indices().collect();
        let slot_store = self.mint_slot_store(&SlotMintInputs {
            instances: &[],
            ode_detections,
            multiplied_ode_names: &empty_multiplied,
            computed_targets: &[],
            sm_targets,
            primary_sm_names,
            // Single-model builders multiply no instances, so they have no
            // prefixed SMs needing guard-read slots.
            sm_guard_reads: &[],
            sm_payload_ports,
            override_map,
            // Single-model builders build no link graph; signal
            // slots/propagation are a workspace-path (build_workspace_orchestrator)
            // concern.
            link_graph: None,
            port_registry: None,
            // Single-model builders have no physics executor.
            physics_write_targets: None,
            physics_writer: None,
            duty_tracker_odes: &duty_tracker_odes,
            // Single-model builders register no discrete subsystems — the
            // discrete SSR lanes are a workspace-path concern (step 4b of
            // `build_workspace_orchestrator`).
            discrete_detections: &[],
        })?;
        // RS001 hard error: a slot with two distinct tick-time writers is
        // a compile failure (design doc D-2.0.3 rule 1).
        self.check_multi_writer_conflicts(&slot_store, &orchestrator.subsystem_names())?;
        orchestrator.set_slot_store(slot_store);

        // RSC-2.3/2.5: bind compiled expressions to slots (after minting,
        // before any tick). RS003 is a HARD ERROR since RSC-2.5: a name in
        // a bound scope resolving to neither a slot nor a graph feature
        // fails the compile — the eval-time graph name-scan that used to
        // soft-serve such names is deleted.
        orchestrator.bind_expression_slots(Some(self.graph.as_ref()));
        let (rs003_errors, rs003_warnings) = self.rs003_diagnostics(orchestrator);
        if !rs003_errors.is_empty() {
            return Err(CompileError::from_diagnostics(rs003_errors));
        }
        orchestrator.push_compile_warnings(rs003_warnings);

        // RS004 (RSC-3.5f.3): every prefixed subsystem must be scoped-view-
        // bypass-eligible now that build_scoped_context is gone.
        let rs004_errors = self.rs004_diagnostics(orchestrator);
        if !rs004_errors.is_empty() {
            return Err(CompileError::from_diagnostics(rs004_errors));
        }

        // RS005 (A2): every strict-`apply` write must mint a claimed slot —
        // hard-fail the build rather than silently drop the write in release.
        let rs005_errors = self.rs005_diagnostics(orchestrator);
        if !rs005_errors.is_empty() {
            return Err(CompileError::from_diagnostics(rs005_errors));
        }

        Ok(())
    }

    // `detect_spatial_frames` moved to `sysml_core::spatial::detect_spatial_frames`
    // as a free function (S3.T12). The frame registry is now built in the
    // salsa-cached `EvalContext` seed and propagated via `merge_from`.

    /// Build a fully-wired `Orchestrator` from ALL subsystems in the workspace.
    ///
    /// Unlike [`build_orchestrator`] (which compiles one SM + one ODE), this
    /// discovers ALL state machine definitions and ALL ODE annotations in the
    /// graph and wires them into a single orchestrator.
    ///
    /// This is the entry point for large multi-file, multi-subsystem models.
    ///
    /// `base_ctx` is the pre-built `EvalContext` seed produced by
    /// `sysml_ide_db::eval_context_seed::context_from_graph` (or one of its
    /// salsa-tracked variants). Passing it in eliminates the duplicate
    /// seed-walk that this method used to do internally, and keeps the
    /// graph-walk in ide-db where it can be cached by the salsa db.
    ///
    /// `precompiled_constraints` is the pre-built constraint set produced
    /// by `sysml_ide_db::precompiled_constraints::workspace_precompiled_constraints_with_library`
    /// (or one of its variants). Pass `Some` to wire continuous constraint
    /// monitoring into the orchestrator — every tick will then evaluate
    /// these constraints and surface results in `ExecutionSnapshot`. Pass
    /// `None` to skip monitoring (the field stays empty and per-tick
    /// `evaluate_constraints` is a no-op). Per ADR-011 §3, taking the
    /// pre-built set keeps the extract-and-precompile walk in ide-db
    /// where salsa can memoize it across orchestrator builds.
    ///
    /// `port_flow` is the pre-built port+flow bundle produced by
    /// `sysml_ide_db::port_flow_runtime::workspace_port_flow_runtime_with_library`
    /// (or one of its variants). Pass `Some` to skip the per-call
    /// `compile_ports` + `compile_flows` graph walk and reuse the
    /// salsa-cached resources directly. Pass `None` to let this method
    /// walk the graph in place (the original behaviour, preserved for
    /// non-workspace callers and tests that don't go through ide-db).
    /// Per ADR-011 §3 rows RT-17 / RT-18 / RT-19, the walk is a pure
    /// graph derivative and naturally lives in the cached upstream.
    #[allow(clippy::too_many_arguments)]
    pub fn build_workspace_orchestrator(
        &self,
        base_ctx: EvalContext,
        precompiled_constraints: Option<std::sync::Arc<PrecompiledConstraintSet>>,
        port_flow: Option<std::sync::Arc<crate::flows::PortFlowResources>>,
        gated_expressions: Option<std::sync::Arc<Vec<GatedExprSpec>>>,
        ref_resolve_cache: Option<
            std::sync::Arc<std::sync::Mutex<crate::expressions::RefResolveCache>>,
        >,
        overrides: &[(String, String)],
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<Orchestrator, CompileError> {
        let dt = dt_ms.unwrap_or(1.0);
        let max_t = max_time_ms.unwrap_or(60_000.0);

        let config = OrchestratorConfig {
            dt_ms: dt,
            max_ticks: (max_t / dt) as u64 + 1,
            max_time_ms: max_t,
            ..Default::default()
        };
        let mut orchestrator = Orchestrator::new(config);
        if let Some(constraints) = precompiled_constraints {
            orchestrator.set_constraints(constraints);
        }
        // Snapshot-scoped ref-resolve cache (ADR-011 §6, S3.T14).
        // Wired through when the caller has a salsa-cached Arc handy;
        // otherwise the orchestrator keeps the fresh empty cache it
        // installed in `Orchestrator::new`.
        if let Some(cache) = ref_resolve_cache {
            orchestrator.set_ref_resolve_cache(cache);
        }

        let override_map: HashMap<&str, f64> = overrides
            .iter()
            .filter_map(|(k, v)| v.parse::<f64>().ok().map(|f| (k.as_str(), f)))
            .collect();

        // 0. Pre-compute instance expansion to know which definitions get multiplied.
        // Multiplied definitions (e.g., ThermalProtectionModel used by circuit1..circuit10)
        // are only added as prefixed instances, NOT as top-level subsystems.
        // RSC-4.2 (L40, escalation 2): both are mutated in place during their
        // registration loops below, so each `OdeDetection` carries its own
        // `SubsystemIndex` captured directly at the call site — never
        // re-derived by name later.
        let mut instances = self.expand_part_instances();
        let mut ode_detections = self.detect_all_odes_unified();

        // Fail hard on any unresolved derivative→state-var mapping recorded at
        // detection (non-match or ambiguity), on every ODE in the workspace and
        // on the per-instance templates. This mirrors the single-ODE path in
        // `prepare_single_ode` so a malformed ODE aborts compilation on every
        // entry path — never a silent constant-0 RHS or substring-collision.
        for ode in ode_detections.iter().chain(
            instances
                .iter()
                .flat_map(|inst| inst.ode_detections.iter()),
        ) {
            ode.ensure_derivatives_matched()?;
        }

        use std::collections::HashSet;

        // GAP 2 (workspace path): fail hard on a build-time override whose
        // target resolves to nothing — the compiler-harness counterpart of the
        // session path's RS002, on the workspace entry point. Was silently
        // dumped into context (step 3 below) and ignored; an `ode_sweep` /
        // what-if over a mistyped key produced no effect and no diagnostic.
        //
        // An override key is legal when its BARE TAIL (the segment after the
        // last `.`, or the whole key when unqualified) names a real ODE
        // parameter, ODE state variable, or an already-seeded context variable
        // — the last covers SM assignment targets, which are declared
        // attributes seeded by `context_from_graph`. This tail rule preserves
        // all three working spellings without enumerating instance prefixes:
        // instance-prefixed (`station1.P_heater`), canonical tree-path
        // (`ProductionCell.station1.P_heater`), and bare (`P_heater`).
        //
        // A BARE key remains a BROADCAST to every instance of a multiplied
        // definition — the natural global what-if semantic ("set every
        // station's P_heater"), relied on by the fixtures' gates. See the
        // per-instance seeding loop below (`override_map.get(prefixed).or bare`)
        // where the broadcast is applied; this is designed behavior.
        if !overrides.is_empty() {
            let all_odes = ode_detections
                .iter()
                .chain(instances.iter().flat_map(|inst| inst.ode_detections.iter()));
            // Targets an ODE override may name (for the error hint + validation).
            let mut param_targets: HashSet<&str> = HashSet::new();
            for ode in all_odes {
                param_targets.extend(ode.parameters.keys().map(String::as_str));
                param_targets.extend(ode.state_vars.iter().map(String::as_str));
            }
            // Full legal-tail set also includes every seeded context variable
            // (SM assignment targets and any other declared attribute) — both
            // its whole key and its bare tail, so a qualified attribute spelling
            // validates too.
            let mut known_tails: HashSet<&str> = param_targets.clone();
            for key in base_ctx.variables.keys() {
                known_tails.insert(key.as_str());
                if let Some((_, tail)) = key.rsplit_once('.') {
                    known_tails.insert(tail);
                }
            }
            for (key, _) in overrides {
                let tail = key
                    .rsplit_once('.')
                    .map(|(_, t)| t)
                    .unwrap_or(key.as_str());
                if !known_tails.contains(tail) && !known_tails.contains(key.as_str()) {
                    let mut hint: Vec<&str> = param_targets.iter().copied().collect();
                    hint.sort_unstable();
                    return Err(CompileError::from_message(format!(
                        "unknown override target '{key}': its name resolves to no ODE \
                         parameter, ODE state variable, or model attribute. ODE targets: {hint:?}"
                    )));
                }
            }
        }
        let multiplied_sm_names: HashSet<String> = instances
            .iter()
            .flat_map(|i| i.sm_names.iter().cloned())
            .collect();
        let multiplied_ode_names: HashSet<String> = instances
            .iter()
            .flat_map(|i| i.ode_detections.iter().filter_map(|o| o.name.clone()))
            .collect();

        // Pre-build a name → ElementId map for subsystem source tracking (ADR-006).
        let element_id_by_name: HashMap<String, ElementId> = self
            .graph
            .elements
            .values()
            .filter_map(|e| {
                let n = e.name.as_ref()?;
                Some((n.clone(), e.id.clone()))
            })
            .collect();

        // Slot-minting inputs collected as compilation proceeds (RSC-2.1).
        let mut sm_targets: Vec<SmAssignTarget> = Vec::new();
        // RSC-3.5b: compile-static port-payload slot targets.
        let mut sm_payload_ports: Vec<SmPayloadPort> = Vec::new();
        // RSC-3.6: per-instance guard/trigger read-leaf slot targets for
        // prefixed SMs (so their guards bind instance-local → bypass-eligible).
        let mut sm_guard_reads: Vec<SmGuardRead> = Vec::new();
        // WS-A2: bare (non-multiplied) SM names + their SubsystemIndex,
        // captured at registration below, so `wire_zero_crossing_detectors`
        // can wire `accept when` crossings for top-level SM+ODE pairs (as
        // well as instance-multiplied ones) targeting the exact registered
        // SM (RSC-4.2 L39) instead of re-deriving it by name at wire time.
        let mut primary_sm_names: Vec<(String, SubsystemIndex)> = Vec::new();

        // 1. Compile state machines (skip definitions that will be multiplied)
        let sm_results = StateMachineCompiler::compile_all(&self.graph);
        let mut sm_count = 0;
        for (name, result) in sm_results {
            if multiplied_sm_names.contains(&name) {
                continue; // Will be added as circuit1.X, circuit2.X, etc.
            }
            match result {
                Ok(ir) => {
                    // RSC-4.2 (ruling escalation 1): collect the target/port
                    // name lists before registration (only `&ir`/`&runner`
                    // needed), register to get the SubsystemIndex, then build
                    // the SmAssignTarget/SmPayloadPort structs from it.
                    let target_names = Self::collect_sm_assignment_targets(&ir);
                    let runner = StateMachineRunner::new(ir);
                    // RSC-3.5b: top-level SM — payload slot names are bare
                    // (`{port}.payload`), canonical == runtime.
                    let accept_ports = runner.accept_ports();
                    let sm_index = orchestrator.add_state_machine(&name, runner);
                    primary_sm_names.push((name.clone(), sm_index));
                    for target in target_names {
                        sm_targets.push(SmAssignTarget {
                            runtime_name: target.clone(),
                            bare_name: target,
                            instance: None,
                            subsystem: sm_index,
                        });
                    }
                    for port in accept_ports {
                        sm_payload_ports.push(SmPayloadPort {
                            runtime_base: port.clone(),
                            canonical_base: port.clone(),
                            port,
                            subsystem: sm_index,
                            instance: None,
                        });
                    }
                    // GAP 2: reconcile the SM-subsystem name (`{state-def}`) to
                    // the part-usage instance whose port a transfer addresses,
                    // so a routed `accept … via <port>` message reaches this SM.
                    // Package-level state defs fall back to identity.
                    let owner_key = self
                        .part_usage_owner_of_state_def(&name)
                        .unwrap_or_else(|| name.clone());
                    orchestrator.register_owner_subsystem(owner_key, name.clone());
                    if let Some(eid) = element_id_by_name.get(&name) {
                        orchestrator.set_last_source_element_id(eid.clone());
                    }
                    sm_count += 1;
                }
                Err(diags) => {
                    #[cfg(feature = "tracing")]
                    tracing::warn!(
                        sm = %name,
                        errors = diags.len(),
                        "skipping state machine that failed to compile"
                    );
                    let _ = diags;
                }
            }
        }

        // 1b. Compile root action subsystems (RSC port-flow Wave B).
        //
        // A bare `action def` is a type (vocabulary), not an Occurrence; only a
        // root action USAGE is a Performance that executes (Performances.kerml
        // :63/190, Actions.sysml:180). Unlike state machines — keyed on the
        // `state def` because every one is exhibited (`exhibit state` is itself
        // a PerformActionUsage) — actions have no `exhibit` shortcut, so we
        // anchor on root `ActionUsage` / `PerformActionUsage` features owned
        // directly by a PartDefinition (the part's behaviour) or a package.
        // `is_root_part_or_package_action` excludes subperformances (nested in
        // another action), SM-owned accepts (nested in a `state def`), and
        // case/calc/constraint behaviours. SSR / dynamics actions are already
        // ODE subsystems and are skipped. Each action subsystem is reconciled
        // to its owning part usage exactly like an SM, so a routed `accept …
        // via <port>` message reaches it. Byte-identical on the corpus: no
        // model declares a root action usage today.
        let multiplied_instance_prefixes: HashSet<String> =
            instances.iter().map(|i| i.prefix.clone()).collect();
        let mut action_count = 0;
        for elem in self.graph.elements.values() {
            if !matches!(
                elem.kind,
                ElementKind::ActionUsage | ElementKind::PerformActionUsage
            ) {
                continue;
            }
            // Standard-library performance/action features (kernel `do`/`entry`/
            // `exit`/`accept`/`transitionActions`/… from Performances.kerml and
            // friends) are vocabulary, not executable user subsystems. Over a
            // library-overlaid workspace graph they would otherwise be compiled
            // as spurious action subsystems — mirror the seed walk's
            // `is_library_element` filter.
            if self.graph.is_library_element(&elem.id) {
                continue;
            }
            if !self.is_root_part_or_package_action(elem) {
                continue;
            }
            // A dynamics action (`action … :> StateSpaceDynamics`, incl. the
            // Continuous/Discrete refinements) is a continuous/discrete-dynamics
            // subsystem handled by the ODE/discrete lane — never a plain action.
            // Exclude it STRUCTURALLY, regardless of whether SSR detection
            // actually built it: if detection missed it (e.g. the usage `:>`
            // form isn't detected yet), the model stays non-compiling (#1 fail
            // hard) rather than silently ticking its dynamics as a generic
            // action graph.
            if self.is_dynamics_action(elem) {
                continue;
            }
            let Some(name) = elem.name.clone() else {
                continue;
            };
            let ir = match self.compile_action(&name) {
                Ok(ir) => ir,
                Err(_) => continue,
            };
            let owner_key = self
                .part_usage_owner_of_behavior_def(elem)
                .unwrap_or_else(|| name.clone());
            // A multiplied owner is expanded per-instance in section 4; skip the
            // top-level copy to avoid a duplicate. (No corpus model multiplies a
            // part that carries a root action today — documented follow-up.)
            if multiplied_instance_prefixes.contains(&owner_key) {
                continue;
            }
            orchestrator.add_action(&name, crate::actions::ActionRunner::new(ir));
            orchestrator.register_owner_subsystem(owner_key, name.clone());
            if let Some(eid) = element_id_by_name.get(&name) {
                orchestrator.set_last_source_element_id(eid.clone());
            }
            action_count += 1;
        }

        // 2. Seed context from pre-built base (built by Snapshot via the
        //    ide-db `eval_context_seed::context_from_graph` function).
        orchestrator.context.merge_from(&base_ctx);
        orchestrator.context.graph = Some(Arc::clone(&self.graph));

        // 2b. Calculation registry + spatial frame registry are bundled
        //     into the cached `EvalContext` by
        //     `sysml_ide_db::eval_context_seed::context_from_graph` (S3.T12)
        //     and propagated by `EvalContext::merge_from` above.

        // 2c. RSC-3.7 C.1/C.2: SampledFunction lookup tables are minted as
        // global `Parameter` slots in `mint_slot_store` (step 10) and the
        // `bind_slots` passes rewrite every `interpolate(__sf_…, …)` read to a
        // `SlotRef`, so the bare `__sf_{name}` context key is no longer
        // injected for the ODE RHS — those reads resolve through the slot store
        // (`get_slot`) in every context (master `slots`, per-prefix
        // `build_slot_read_context`, ODE RHS scratch via `merge_from`
        // slot_reader propagation). Extraction still runs here so a malformed
        // `@DataSource` fails loud at compile time (the mint step swallows the
        // re-walk result, but this `?` does not).
        //
        // WS-D / WS-A2 fix: the zero-crossing detector's `event_fn`
        // (`wire_when_crossings_for_pair`) evaluates the *raw* signal-comparator
        // expression against a clone of `orchestrator.context` — a string-keyed
        // `EvalContext`, NOT the slot store, and NOT slot-bound. A comparator
        // that references a SampledFunction (e.g. the oscillator fixture's `i_drive` square-wave
        // comparator, whose `interpolateSaturating(HfromB_ascending, B)` output
        // is the fault-detection duty signal) therefore failed to evaluate once
        // C.1/C.2 removed the bare context binding — the interpolate call raised
        // `UndefinedVariable`, the residual fell back to the guard-false sign,
        // and the located crossing never fired (only the B-saturation safety
        // crossings did, which the duty tracker correctly ignores). The
        // detector's own contract already assumes these tables live in the
        // master context ("params, sampled fns, V_applied, …"); restore that
        // binding under the SampledFunction's DECLARED name so signal
        // comparators re-evaluate. `WriterId::External`, read-only injected data
        // — same status as ODE parameters; it is never written at tick time.
        for (sf_name, sf_value) in self.extract_sampled_functions()? {
            orchestrator.context.set(sf_name, sf_value);
        }

        // 3. Compile ODE subsystems (skip definitions that will be multiplied)
        let mut ode_count = 0;
        for ode in &mut ode_detections {
            if let Some(name) = &ode.name {
                if multiplied_ode_names.contains(name) {
                    continue; // Will be added as circuit1.X, circuit2.X, etc.
                }
            }

            let ode_label = ode.name.as_deref().unwrap_or("compiled-ode").to_owned();

            // Build config maps for this ODE (e.g., config → ThermalProtectionConfig defaults)
            let config_maps = ode
                .name
                .as_deref()
                .map(|n| self.build_config_maps(n))
                .unwrap_or_default();

            // RSC-4.3 (L47): restep-eligibility structurally requires RK45
            // (only `Rk45Solver`/`Rk4Solver` implement the time-accurate
            // re-step protocol; `BdfSolver`'s multistep history makes true
            // rollback FORBIDDEN new solver machinery, Q9). Decide this
            // BEFORE the solver branch, using the exact test
            // `wire_when_crossings_for_pair` uses later to call
            // `mark_restep_eligible` — never a second heuristic.
            let restep_eligible = self.any_sm_has_qualifying_when_crossing(
                primary_sm_names.iter().map(|(n, _)| n.as_str()),
                ode,
            );
            if restep_eligible && ode.is_bdf() {
                return Err(CompileError::from_message(format!(
                    "ODE '{ode_label}' is annotated `@ToolExecution` for the implicit \
                     BDF solver, but a paired state machine has a qualifying `accept \
                     when` crossing trigger, which requires the time-accurate mid-tick \
                     re-step (RSC-4.3). BDF cannot support that protocol (its \
                     multistep history cannot be rolled back — out of Wave-1 scope). \
                     Remove the BDF annotation (RK45 is the default) or remove the \
                     `accept when` comparator trigger."
                )));
            }

            // Explicit `@ToolExecution` choices are honored verbatim; an
            // un-annotated ODE goes through WS-B4 auto-switch (stiff⇒BDF,
            // else⇒RK45) — the `else` arm below. A restep-eligible ODE skips
            // the stiffness classifier entirely and pins RK45 (loud
            // diagnostic if it would otherwise have picked BDF).
            let add_result = if ode.is_bdf() {
                Self::build_bdf_solver(&ode_label, ode, &override_map, &config_maps)
                    .map(|solver| orchestrator.add_bdf(&ode_label, solver))
            } else if ode.is_rk4() {
                Self::build_ode_solver_with_config(&ode_label, ode, &override_map, &config_maps)
                    .map(|solver| orchestrator.add_ode(&ode_label, solver))
            } else if restep_eligible {
                if let Some(verdict) = Self::classify_stiffness_verdict(
                    ode,
                    &override_map,
                    &config_maps,
                    &orchestrator.context,
                    dt / 1000.0,
                ) {
                    Self::log_restep_pin_over_stiffness(&ode_label, &verdict);
                }
                Self::build_ode45_solver(&ode_label, ode, &override_map, &config_maps)
                    .map(|solver| orchestrator.add_ode45(&ode_label, solver))
            } else if Self::auto_select_stiff(
                &ode_label,
                ode,
                &override_map,
                &config_maps,
                &orchestrator.context,
                dt / 1000.0,
            ) {
                Self::build_bdf_solver(&ode_label, ode, &override_map, &config_maps)
                    .map(|solver| orchestrator.add_bdf(&ode_label, solver))
            } else {
                Self::build_ode45_solver(&ode_label, ode, &override_map, &config_maps)
                    .map(|solver| orchestrator.add_ode45(&ode_label, solver))
            };

            match add_result {
                Ok(index) => {
                    // RSC-4.2 (L40): captured directly at the registration
                    // call site above — never re-derived by name in mint.
                    ode.subsystem_index = Some(index);
                    if let Some(eid) = element_id_by_name.get(&ode_label) {
                        orchestrator.set_last_source_element_id(eid.clone());
                    }
                    for (k, v) in &ode.parameters {
                        let val = override_map.get(k.as_str()).copied().unwrap_or(*v);
                        orchestrator.context.set(k.clone(), Value::Float(val));
                    }
                    ode_count += 1;
                }
                Err(e) => {
                    // Do NOT skip. Slot-minting (step 10) hard-fails on any
                    // detection without a registered subsystem, so a "skipped"
                    // ODE was never survivable — it only surfaced later as
                    // `internal: … has no registered subsystem at slot-mint
                    // time (RSC-4.2 mint-gap)`, which blames the mint step for
                    // a failure that happened here and buries the reason the
                    // solver could not be built. Report the real cause, at the
                    // stage that produced it.
                    return Err(CompileError::from_message(format!(
                        "ODE subsystem '{ode_label}' could not be built: {}",
                        e.message
                    )));
                }
            }
        }

        // Apply overrides
        for (k, v) in &override_map {
            orchestrator.context.set(k.to_string(), Value::Float(*v));
        }

        // 4. Instance multiplication — create prefixed subsystem copies
        for (inst_idx, inst) in instances.iter_mut().enumerate() {
            for sm_name in &inst.sm_names {
                if let Ok(ir) = self.compile_state_machine(sm_name) {
                    let subsystem_name = format!("{}.{}", inst.prefix, sm_name);
                    // RSC-4.2 (ruling escalation 1): collect the raw name
                    // lists that only need `&ir`/`&runner` first, register to
                    // get the SubsystemIndex, then build the
                    // SmAssignTarget/SmPayloadPort structs from it below.
                    let target_names = Self::collect_sm_assignment_targets(&ir);
                    // RSC-3.5b (leftover-C): the SM's instance-scoped
                    // assignment-target slots carry a canonical tree-path
                    // spelling (`{container}.{instance}.{var}`) distinct from
                    // their `{instance}.{var}` runtime key. Pass the canonical
                    // prefix (`{container}.{instance}`) so WriteRoute::resolve
                    // matches the slot's canonical name instead of refusing the
                    // route on the mismatch and falling back to the name-keyed
                    // path. The SM target attribute is declared directly on the
                    // instance type (no ODE-style sub-part chain), so the
                    // canonical prefix is exactly container + instance.
                    let sm_canonical_prefix = match inst.container_name.as_deref() {
                        Some(container) => format!("{container}.{}", inst.prefix),
                        None => String::new(),
                    };
                    let runner = StateMachineRunner::new(ir);
                    // RSC-3.6: per-instance guard/trigger read-leaf slots. This
                    // SM is prefixed (instance-multiplied), so each key its
                    // guard / `When` / `AfterExpr` reads must resolve through
                    // the instance's OWN namespace for the executor to be
                    // scoped-view-bypass-eligible. Mint one slot per read leaf
                    // (`{prefix}.{leaf}` runtime, `{container}.{prefix}.{leaf}`
                    // canonical) so `bind_expression_slots` binds the guard to
                    // an instance-local `SlotRef`. Uncompilable guards
                    // contribute no leaves (event-name comparison reads nothing).
                    for leaf in runner.guard_trigger_reads() {
                        let runtime_name = format!("{}.{}", inst.prefix, leaf);
                        let canonical_name = if sm_canonical_prefix.is_empty() {
                            runtime_name.clone()
                        } else {
                            format!("{sm_canonical_prefix}.{leaf}")
                        };
                        sm_guard_reads.push(SmGuardRead {
                            runtime_name,
                            canonical_name,
                            leaf,
                            instance: Some(inst_idx),
                        });
                    }
                    // RSC-3.5b: per-instance payload slot targets. Runtime
                    // spelling `{prefix}.{port}.payload`; canonical spelling
                    // `{container}.{prefix}.{port}.payload` (== runtime when
                    // there is no container). Bases are precomputed here
                    // (before `sm_canonical_prefix`/`runner` are moved into
                    // the registration call below) and turned into
                    // SmPayloadPort structs afterward, once the
                    // SubsystemIndex is known.
                    let port_bases: Vec<(String, String, String)> = runner
                        .accept_ports()
                        .into_iter()
                        .map(|port| {
                            let runtime_base = format!("{}.{}", inst.prefix, port);
                            let canonical_base = if sm_canonical_prefix.is_empty() {
                                runtime_base.clone()
                            } else {
                                format!("{sm_canonical_prefix}.{port}")
                            };
                            (port, runtime_base, canonical_base)
                        })
                        .collect();
                    // GAP 2: a multiplied SM's owner key is its instance prefix
                    // (`breaker1` → subsystem `breaker1.Logic`), so a transfer to
                    // `breaker1.tripIn` reaches only this instance's SM.
                    orchestrator
                        .register_owner_subsystem(inst.prefix.clone(), subsystem_name.clone());
                    let sm_index = orchestrator.add_state_machine_prefixed_with_canonical(
                        subsystem_name,
                        runner,
                        &inst.prefix,
                        sm_canonical_prefix,
                    );
                    // RSC-4.2 (L39): captured directly at the registration
                    // call site above — `wire_zero_crossing_detectors`
                    // consumes this so `accept when` crossing wiring for a
                    // multiplied SM+ODE pair targets the exact registered SM
                    // instead of "first StateMachine subsystem".
                    inst.sm_subsystem_indices.insert(sm_name.clone(), sm_index);
                    for target in target_names {
                        sm_targets.push(SmAssignTarget {
                            runtime_name: format!("{}.{}", inst.prefix, target),
                            bare_name: target,
                            instance: Some(inst_idx),
                            subsystem: sm_index,
                        });
                    }
                    for (port, runtime_base, canonical_base) in port_bases {
                        sm_payload_ports.push(SmPayloadPort {
                            port,
                            runtime_base,
                            canonical_base,
                            subsystem: sm_index,
                            instance: Some(inst_idx),
                        });
                    }
                    if let Some(eid) = element_id_by_name.get(sm_name) {
                        orchestrator.set_last_source_element_id(eid.clone());
                    }
                    sm_count += 1;
                }
            }
            // RSC-4.2 (L40): index-based (not `&mut inst.ode_detections`)
            // because `instance_canonical_prefix(inst, ode)` below needs a
            // shared `&InstanceSpec` (whole-struct) alongside `ode` — two
            // shared borrows coexist fine, but a live `&mut` element borrow
            // from the loop would conflict with it. `ode.subsystem_index` is
            // written back via a direct indexed assignment at the end of the
            // success branch, once every read through `ode` in this
            // iteration is done.
            for ode_idx in 0..inst.ode_detections.len() {
                let ode = &inst.ode_detections[ode_idx];
                let label = format!("{}.{}", inst.prefix, ode.name.as_deref().unwrap_or("ode"));
                let config_maps = ode
                    .name
                    .as_deref()
                    .map(|n| self.build_config_maps(n))
                    .unwrap_or_default();
                // Build a prefixed override map so per-instance overrides resolve.
                // E.g., "circuit1.loadCurrent" → "loadCurrent" for this instance's solver.
                let mut instance_overrides = override_map.clone();
                let prefix_dot = format!("{}.", inst.prefix);
                for (k, v) in &override_map {
                    if let Some(unprefixed) = k.strip_prefix(prefix_dot.as_str()) {
                        instance_overrides.insert(unprefixed, *v);
                    }
                }

                // Build the canonical tree-path prefix. The frontend
                // tree's `ownerPath` for an ODE's state var is
                // `{container}.{instance}.{sub_parts…}.{var}` — we
                // reproduce that so scalar_vars keys match and the
                // UI finds the live value instead of the compiler's
                // static bare-name binding.
                let (canonical_prefix, _sub_path) = self.instance_canonical_prefix(inst, ode);

                // RSC-4.3 (L47): same restep-eligibility pin as the primary
                // ODE loop — see the comment there. `inst.sm_names` are this
                // instance's paired SMs.
                let restep_eligible = self.any_sm_has_qualifying_when_crossing(
                    inst.sm_names.iter().map(String::as_str),
                    ode,
                );
                if restep_eligible && ode.is_bdf() {
                    return Err(CompileError::from_message(format!(
                        "ODE '{label}' is annotated `@ToolExecution` for the implicit \
                         BDF solver, but a paired state machine has a qualifying \
                         `accept when` crossing trigger, which requires the \
                         time-accurate mid-tick re-step (RSC-4.3). BDF cannot support \
                         that protocol (its multistep history cannot be rolled back — \
                         out of Wave-1 scope). Remove the BDF annotation (RK45 is the \
                         default) or remove the `accept when` comparator trigger."
                    )));
                }

                // Explicit `@ToolExecution` choices honored verbatim; an
                // un-annotated instance ODE goes through WS-B4 auto-switch
                // (stiff⇒BDF, else⇒RK45) — the final arm. All routed through
                // the prefixed+canonical adder so the multiplied instance's
                // slots resolve. A restep-eligible ODE skips the stiffness
                // classifier and pins RK45 (loud diagnostic if it would
                // otherwise have picked BDF).
                let add_result: Result<SubsystemIndex, _> = if ode.is_bdf() {
                    Self::build_bdf_solver(&label, ode, &instance_overrides, &config_maps).map(
                        |s| {
                            orchestrator.add_bdf_prefixed_with_canonical(
                                &label,
                                s,
                                &inst.prefix,
                                canonical_prefix.clone(),
                            )
                        },
                    )
                } else if ode.is_rk4() {
                    Self::build_ode_solver_with_config(
                        &label,
                        ode,
                        &instance_overrides,
                        &config_maps,
                    )
                    .map(|s| {
                        orchestrator.add_ode_prefixed_with_canonical(
                            &label,
                            s,
                            &inst.prefix,
                            canonical_prefix.clone(),
                        )
                    })
                } else if restep_eligible {
                    if let Some(verdict) = Self::classify_stiffness_verdict(
                        ode,
                        &instance_overrides,
                        &config_maps,
                        &orchestrator.context,
                        dt / 1000.0,
                    ) {
                        Self::log_restep_pin_over_stiffness(&label, &verdict);
                    }
                    Self::build_ode45_solver(&label, ode, &instance_overrides, &config_maps).map(
                        |s| {
                            orchestrator.add_ode45_prefixed_with_canonical(
                                &label,
                                s,
                                &inst.prefix,
                                canonical_prefix.clone(),
                            )
                        },
                    )
                } else if Self::auto_select_stiff(
                    &label,
                    ode,
                    &instance_overrides,
                    &config_maps,
                    &orchestrator.context,
                    dt / 1000.0,
                ) {
                    Self::build_bdf_solver(&label, ode, &instance_overrides, &config_maps).map(
                        |s| {
                            orchestrator.add_bdf_prefixed_with_canonical(
                                &label,
                                s,
                                &inst.prefix,
                                canonical_prefix.clone(),
                            )
                        },
                    )
                } else {
                    Self::build_ode45_solver(&label, ode, &instance_overrides, &config_maps).map(
                        |s| {
                            orchestrator.add_ode45_prefixed_with_canonical(
                                &label,
                                s,
                                &inst.prefix,
                                canonical_prefix.clone(),
                            )
                        },
                    )
                };
                // Same contract as the top-level arm above: a per-instance
                // detection that fails to build is fatal at mint, so report
                // the build error here rather than deferring to the
                // instance-flavoured mint-gap message.
                let index = add_result.map_err(|e| {
                    CompileError::from_message(format!(
                        "ODE subsystem '{}.{}' could not be built: {}",
                        inst.prefix,
                        ode.name.as_deref().unwrap_or("ode"),
                        e.message
                    ))
                })?;
                if let Some(ode_name) = &ode.name {
                    if let Some(eid) = element_id_by_name.get(ode_name) {
                        orchestrator.set_last_source_element_id(eid.clone());
                    }
                }
                for (k, v) in &ode.parameters {
                    let prefixed_key = format!("{}.{}", inst.prefix, k);
                    let val = override_map
                        .get(prefixed_key.as_str())
                        .or_else(|| override_map.get(k.as_str()))
                        .copied()
                        .unwrap_or(*v);
                    orchestrator.context.set(prefixed_key, Value::Float(val));
                    // Also bind the parameter under its canonical
                    // tree-path key so the UI's `ownerPath`-based
                    // lookup sees the live value instead of falling
                    // back to the compiler's static bare-name
                    // initial binding. Skipped when canonical and
                    // var-prefix coincide (no extra container or
                    // sub-part segments to add).
                    if !canonical_prefix.is_empty() && canonical_prefix != inst.prefix {
                        let canonical_key = format!("{}.{}", canonical_prefix, k);
                        orchestrator.context.set(canonical_key, Value::Float(val));
                    }
                }
                // RSC-4.2 (L40): captured directly at the registration
                // call site above — never re-derived by name in mint.
                // Written last (indexed, not through `ode`) so every
                // read through `ode` in this iteration is done first.
                inst.ode_detections[ode_idx].subsystem_index = Some(index);
                ode_count += 1;
            }

            // 4b. Seed instance-specific config overrides from inline PartUsage attributes.
            // e.g., circuit1's config { ratedCurrent = 16.0 } → circuit1.config.ratedCurrent
            self.seed_instance_config_into(
                &mut orchestrator.context,
                &inst.prefix,
                &ode_detections,
            )?;
        }

        // 4b. Detect discrete-time state-space dynamics.
        //
        // Each detection's `SubsystemIndex` is captured at its own
        // `add_discrete` call site and carried into slot minting (step 10), so
        // the solver's state vector gets Executor-owned slots. Without that the
        // subsystem registered fine and then panicked on its first writeback —
        // see `DiscreteDetection`.
        let mut discrete_detections: Vec<DiscreteDetection> = Vec::new();
        let mut discrete_count = 0;
        let discrete_lanes = self
            .detect_composite_discrete_ssr() // action :> DiscreteStateSpaceDynamics
            .into_iter()
            .chain(self.detect_composite_state_space_dynamics()) // :> StateSpaceDynamics
            .chain(self.detect_discrete_from_ssr()); // loose calc def :> GetDifference
        for (mut claim, solver) in discrete_lanes {
            // Fail hard BEFORE registering: a subsystem whose state variables
            // could not be resolved to calc returns would otherwise tick a
            // broadcast or constant-0 equation and report numbers that are not
            // the model's. Mirrors `ensure_derivatives_matched` on the
            // continuous lane.
            claim.ensure_states_matched()?;
            let index = orchestrator.add_discrete(&claim.label, solver);
            claim.subsystem_index = Some(index);
            if let Some(eid) = element_id_by_name.get(&claim.label) {
                orchestrator.set_last_source_element_id(eid.clone());
            }
            discrete_detections.push(claim);
            discrete_count += 1;
        }

        // 5. Auto-detect computed expressions from = bindings, plus the
        //    instance-scoped duplicates that 5b would normally add. When the
        //    caller supplies the salsa-cached bundle, replay it verbatim and
        //    skip both walks plus the per-expression parse. Otherwise fall
        //    back to the legacy two-step path. See S3.T12 cached_gated_expressions
        //    (ADR-011 §3 RT-16).
        let mut computed_target_names: Vec<String> = Vec::new();
        if let Some(cached_gated) = gated_expressions {
            for spec in cached_gated.iter() {
                computed_target_names.push(spec.target.clone());
                match &spec.scope_prefix {
                    Some(prefix) => orchestrator.add_instance_computed_expression(
                        spec.target.clone(),
                        spec.expr.clone(),
                        prefix.clone(),
                    ),
                    None => orchestrator
                        .add_computed_expression(spec.target.clone(), spec.expr.clone()),
                }
            }
        } else {
            let computed = Self::detect_computed_expressions(&self.graph);
            for (target, expr_str) in &computed {
                if let Ok(expr) = ode_builder::parse_derivative(expr_str) {
                    computed_target_names.push(target.clone());
                    orchestrator.add_computed_expression(target, expr);
                }
            }

            // 5b. Instance-scoped computed expressions: duplicate `= expr` bindings
            // from multiplied part definitions with instance prefixes.
            // E.g., CircuitPath { attribute tripped = bimetalTemp >= 150; } generates
            // circuit1.tripped, circuit2.tripped, etc.
            computed_target_names
                .extend(self.detect_instance_scoped_expressions(&mut orchestrator, &instances));
        }

        // 5c. Wire SampledFunction waveforms to ODE inputs.
        // Generic: splits SF attribute name on `_` to get (instance, param_hint),
        // then finds matching ODE variable via case-insensitive substring match.
        self.wire_scenario_waveforms(&mut orchestrator);

        // 6. Compile ports for runtime value propagation. Prefer the salsa-
        //    cached bundle when the caller supplies it; otherwise walk the graph
        //    in place (preserves the path for tests, benches, and reparse).
        //    See S3.T12 cached_port_flow_runtime (ADR-011 §3 RT-17).
        let port_registry = if let Some(cached) = port_flow {
            (*cached).registry.clone()
        } else {
            crate::flows::compile_ports(&self.graph)
        };

        // 6a. RSC-3.1: build the classified link graph (design doc D-3.0.1).
        // RSC-3.5e.5 W3: classify_links now folds in the flow producer — it
        // walks the flow elements (FlowUsage/SuccessionFlowUsage/Flow) itself,
        // so there is no separate FlowConnectionIR list. Built once here, where
        // the elaborated graph + classified PortRegistry are alive, and reused
        // by the registry-attach guard, the contract-diagnostic pass, both flow
        // bridges (5d/6b), and the physics executor. FL017 (unresolved link
        // class) is folded into the compile warnings so it surfaces alongside
        // the session like RS003 does.
        let (link_graph, link_diags) = crate::links::classify_links(&self.graph, &port_registry);

        // Attach the port registry when there is anything to route — a non-empty
        // registry, or any flow-derived link (the W3 stand-in for the former
        // `!flow_connections.is_empty()` half of this guard).
        if !port_registry.is_empty()
            || link_graph
                .iter()
                .any(|l| l.kind == crate::links::LinkSourceKind::FlowUsage)
        {
            orchestrator.set_port_registry(port_registry.clone());
        }

        // 6a'. RSC-3.3b: spec Transfer contract enforcement (design doc
        // D-3.0.4 D3/D5/D6). FL018 (interface topology), FL019 (direction),
        // and FL020 (static payload conformance) are HARD ERRORS per the
        // §6 Q2 decision (RS001 playbook — the corpus was pre-cleared in
        // e2ca2546 + the 3.3b model fixes; inventory is zero).
        let contract_diags =
            crate::links::transfer_contract_diagnostics(&self.graph, &port_registry, &link_graph);
        let (contract_errors, contract_warnings): (Vec<_>, Vec<_>) = contract_diags
            .into_iter()
            .partition(|d| d.severity == sysml_span::Severity::Error);
        if !contract_errors.is_empty() {
            return Err(CompileError::from_diagnostics(contract_errors));
        }
        if !contract_warnings.is_empty() {
            orchestrator.push_compile_warnings(contract_warnings);
        }

        if !link_diags.is_empty() {
            orchestrator.push_compile_warnings(link_diags);
        }

        // 5d (moved past 6a so it can consume the link graph). Bridge instance
        // ODE state vars to flow-connected port variables. Order vs 6b is
        // preserved (5d before 6b) so the computed-expression sequence is
        // byte-identical to the pre-W3 ordering.
        self.wire_port_flow_bridge(&mut orchestrator, &instances, &link_graph);

        // 6b. Generic flow bridge: walk the flow-derived links and create
        // port-to-port computed expressions based on physics domain
        // classification.
        self.wire_generic_flow_bridge(&mut orchestrator, &link_graph);

        // 7. Build physics executor from the PowerBond subset of the classified
        // link graph (RSC-3.5f.2 / L30). Borrow `link_graph` here, *before*
        // moving it into the orchestrator below, so the physics topology reuses
        // the classification already computed at step 6a (no second pass).
        // RSC-3.4 / L31: extract write_targets BEFORE consuming the executor so
        // mint_slot_store can pre-mint physics slots in step 8.
        let mut has_physics = false;
        // RSC-4.2 (L40): the physics writer index, captured directly from
        // `add_physics`'s return at its call site (the landed L31
        // `physics_writer: Option<WriterId>` precedent) — never re-derived
        // by name later.
        let mut physics_subsystem_index: Option<SubsystemIndex> = None;
        let mut physics_write_targets: Vec<String> = Vec::new();
        if let Some(cached) = &self.cached_physics_executor {
            // RSC-6.4: the salsa-aware caller pre-built this executor (from the
            // same elaborated graph + link classification) via the
            // `workspace_physics_executor` query. Clone it instead of
            // reconstructing — byte-identical, and the build cost is paid once
            // per graph version rather than once per orchestrator build.
            let executor = cached.clone_concrete();
            physics_write_targets = executor.write_targets();
            physics_subsystem_index = Some(orchestrator.add_physics("physics", executor));
            has_physics = true;
        } else {
            match crate::physics::executor::PhysicsExecutor::from_graph_with_links(
                &self.graph,
                &link_graph,
            ) {
                Ok((executor, physics_diags)) => {
                    for d in &physics_diags {
                        #[cfg(feature = "tracing")]
                        tracing::debug!("physics: {}", d);
                        let _ = d;
                    }
                    physics_write_targets = executor.write_targets();
                    physics_subsystem_index = Some(orchestrator.add_physics("physics", executor));
                    has_physics = true;
                }
                Err(diags) => {
                    // Non-fatal: model has no physics topology, continue without.
                    // Log diagnostics so failures are visible during debugging.
                    for d in &diags {
                        #[cfg(feature = "tracing")]
                        tracing::info!("physics skipped: {}", d);
                        let _ = d;
                    }
                }
            }
        }

        // The orchestrator takes ownership of the link graph after the physics
        // executor has consumed its PowerBond subset.
        if !link_graph.is_empty() {
            orchestrator.set_link_graph(link_graph);
        }

        // 7b. Auto-enable convergence when physics + ODE + SM are all present.
        // Multi-domain coupling (e.g., electrical → thermal feedback) requires
        // iterative convergence to reach a consistent state each tick.
        if has_physics
            && ode_count > 0
            && sm_count > 0
            && orchestrator.convergence_iterations() == 0
        {
            orchestrator.set_convergence_iterations(3);
        }

        // 8. Wire zero-crossing detectors for `accept when` SM triggers over
        // continuous ODE state — both instance-multiplied and primary (bare)
        // SM+ODE pairs. Primary ODEs are those not multiplied into instances.
        let primary_ode_detections: Vec<OdeDetection> = ode_detections
            .iter()
            .filter(|ode| {
                ode.name
                    .as_deref()
                    .map(|n| !multiplied_ode_names.contains(n))
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        self.wire_zero_crossing_detectors(
            &mut orchestrator,
            &instances,
            &primary_ode_detections,
            &primary_sm_names,
        );

        // 8b. Wire spec-typed ZeroCrossingEventDef from SSR models.
        self.wire_ssr_zero_crossing_events(&mut orchestrator);

        // 9. Auto-detect flow gates from state machines.
        // For each instance that has both SMs and ODE/flows, find terminal
        // states in the SM IR and register them as gating states.
        self.detect_and_register_flow_gates(&mut orchestrator, &instances);

        // 10. Mint the compile-known slot table (RSC-2.2: live storage —
        // attached to the master context for write-through routing). All
        // subsystems are registered by now, so writer assignment resolves
        // executor indices directly, and the RS001 multi-writer gate runs
        // before the store is installed.
        // WS-D Stage 2 duty-seam fix: step 8 (`wire_zero_crossing_detectors`,
        // above) has already registered every duty tracker this workspace
        // needs, so the full set is available here, before mint.
        let duty_tracker_odes: HashSet<SubsystemIndex> =
            orchestrator.duty_tracker_indices().collect();
        let slot_store = self.mint_slot_store(&SlotMintInputs {
            instances: &instances,
            ode_detections: &ode_detections,
            multiplied_ode_names: &multiplied_ode_names,
            computed_targets: &computed_target_names,
            sm_targets: &sm_targets,
            primary_sm_names: &primary_sm_names,
            sm_guard_reads: &sm_guard_reads,
            sm_payload_ports: &sm_payload_ports,
            override_map: &override_map,
            // RSC-3.2: link graph was just installed (step 6a); read it back
            // so signal-link endpoint port features mint as slots.
            link_graph: Some(orchestrator.link_graph()),
            port_registry: Some(&port_registry),
            // RSC-3.4 / L31: pre-mint physics write slots so WriteRoute::resolve
            // hard-asserts on all known targets rather than silently creating them.
            physics_write_targets: if has_physics {
                Some(&physics_write_targets)
            } else {
                None
            },
            physics_writer: physics_subsystem_index.map(crate::slots::WriterId::from),
            duty_tracker_odes: &duty_tracker_odes,
            discrete_detections: &discrete_detections,
        })?;
        self.check_multi_writer_conflicts(&slot_store, &orchestrator.subsystem_names())?;
        orchestrator.set_slot_store(slot_store);

        // 10a. RSC-3.2: compile the SignalLink directed-propagation plan now
        // the slot table is live (design doc D-3.0.3). Cycles surface as a
        // compile diagnostic (RS010) folded into the session warnings; the
        // interim per-tick pass then falls back to interning order.
        {
            let store_guard = orchestrator.slot_store();
            let (signal_prop, signal_diags) = crate::links::compile_signal_propagation(
                orchestrator.link_graph(),
                &store_guard,
                &port_registry,
            );
            drop(store_guard);
            orchestrator.set_signal_propagation(signal_prop);
            if !signal_diags.is_empty() {
                orchestrator.push_compile_warnings(signal_diags);
            }
        }

        // 10b. RSC-3.5d (steward-ruled — ledger L26): wire the discrete message
        // router from the classified link graph, so occurrence-addressed
        // `Transfer`s actually deliver on a compiled model. Runs after the slot
        // table (step 10) and signal-propagation plan (step 10a) are live, and
        // after all subsystems are registered (steps 1–3b) so the acceptor
        // topology is complete.
        self.wire_message_router(&mut orchestrator);

        // 11. RSC-2.3/2.5: bind compiled expressions to slots (after
        // minting, before any tick). Orchestrator-level expressions
        // (computed/gated, constraints) bind in the global scope; each
        // subsystem's retained ODE spec binds in its instance-local scope.
        // RS003 is a HARD ERROR since RSC-2.5: names resolving to neither
        // a slot nor a graph feature fail the compile (the eval-time
        // graph name-scan fallback is deleted).
        orchestrator.bind_expression_slots(Some(self.graph.as_ref()));
        let (rs003_errors, rs003_warnings) = self.rs003_diagnostics(&orchestrator);
        if !rs003_errors.is_empty() {
            return Err(CompileError::from_diagnostics(rs003_errors));
        }
        orchestrator.push_compile_warnings(rs003_warnings);

        // RS004 (RSC-3.5f.3): every prefixed subsystem must be scoped-view-
        // bypass-eligible now that build_scoped_context is gone.
        let rs004_errors = self.rs004_diagnostics(&orchestrator);
        if !rs004_errors.is_empty() {
            return Err(CompileError::from_diagnostics(rs004_errors));
        }

        // RS005 (A2): every strict-`apply` write must mint a claimed slot —
        // hard-fail the build rather than silently drop the write in release.
        let rs005_errors = self.rs005_diagnostics(&orchestrator);
        if !rs005_errors.is_empty() {
            return Err(CompileError::from_diagnostics(rs005_errors));
        }

        if sm_count == 0 && ode_count == 0 && discrete_count == 0 && action_count == 0 {
            return Err(CompileError::from_message(
                "no state machines, ODE, discrete, or action subsystems found in the workspace graph",
            ));
        }

        Ok(orchestrator)
    }

    /// Mirror of [`context_from_graph`]'s variable walk that keeps the
    /// declaring `ElementId` alongside each bound value. Same element
    /// iteration, same binding rules (value prop → default prop → literal
    /// child → sticky `Value::Ref`), same ISQ tagging.
    pub(crate) fn collect_bare_bindings(&self) -> HashMap<String, (ElementId, Value)> {
        let mut out: HashMap<String, (ElementId, Value)> = HashMap::new();
        for element in self.graph.elements.values() {
            if is_expression_ast_kind(&element.kind) {
                continue;
            }
            // Calc-def-internal features are invocation-scoped — never global
            // bare-name bindings (and never slot-minted). See
            // `is_calc_scoped_seed_feature`.
            if is_calc_scoped_seed_feature(&self.graph, element) {
                continue;
            }
            let Some(name) = &element.name else { continue };
            if let Some(val) = element.get_prop("value") {
                let val = maybe_tag_isq(&self.graph, element, val.clone());
                out.insert(name.clone(), (element.id.clone(), val));
                continue;
            }
            if let Some(val) = element.get_prop("default") {
                let val = maybe_tag_isq(&self.graph, element, val.clone());
                out.insert(name.clone(), (element.id.clone(), val));
                continue;
            }
            let mut found_literal = false;
            for child in self.graph.children_of(&element.id) {
                if matches!(
                    child.kind,
                    ElementKind::LiteralInteger
                        | ElementKind::LiteralRational
                        | ElementKind::LiteralBoolean
                        | ElementKind::LiteralString
                ) {
                    if let Some(val) = child.get_prop("value") {
                        let val = maybe_tag_isq(&self.graph, element, val.clone());
                        out.insert(name.clone(), (element.id.clone(), val));
                        found_literal = true;
                        break;
                    }
                }
            }
            if !found_literal {
                let already_concrete = out
                    .get(name)
                    .is_some_and(|(_, v)| !matches!(v, Value::Ref(_)));
                if !already_concrete {
                    out.insert(
                        name.clone(),
                        (element.id.clone(), Value::Ref(element.id.clone())),
                    );
                }
            }
        }
        out
    }

}

/// Fail hard on a build-time override key that names no real target — the
/// compiler-harness counterpart of the session override path's RS002
/// (`Orchestrator::apply_overrides_with_aliases`). Prior to this the
/// build-time `overrides` param was applied by a bare-keyed `HashMap` lookup
/// with an `unwrap_or(default)` fallback, so a typo'd or wrongly-qualified key
/// silently baked nothing (e.g. an `ode_sweep` over a mistyped `parameter_name`
/// produced a *flat* sweep with no diagnostic).
///
/// - `known_bare` — every bare name a build override may legally set on this
///   path (ODE parameters, ODE state variables, SM assignment targets).
/// - `known_params` — the subset that are ODE PARAMETERS. These are baked into
///   the solver RHS from their *bare* key ([`OdeSpec::build_rhs`]), so a
///   qualified spelling of one cannot reach the baked constant; it earns a
///   precise "use the bare key" error rather than a silent no-op.
fn validate_build_override_targets(
    overrides: &[(String, String)],
    known_bare: &std::collections::HashSet<&str>,
    known_params: &std::collections::HashSet<&str>,
) -> Result<(), CompileError> {
    for (key, _value) in overrides {
        if known_bare.contains(key.as_str()) {
            continue;
        }
        if let Some((_prefix, tail)) = key.rsplit_once('.') {
            if known_params.contains(tail) {
                return Err(CompileError::from_message(format!(
                    "override target '{key}': qualified param override not supported for \
                     ODE parameters (baked into the solver from the bare key) — use the \
                     bare key '{tail}'"
                )));
            }
        }
        let mut available: Vec<&str> = known_bare.iter().copied().collect();
        available.sort_unstable();
        return Err(CompileError::from_message(format!(
            "unknown override target '{key}': matches no ODE parameter, ODE state variable, \
             or SM assignment target on this model. Known targets: {available:?}"
        )));
    }
    Ok(())
}
