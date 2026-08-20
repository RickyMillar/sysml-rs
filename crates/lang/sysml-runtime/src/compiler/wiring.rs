//! Flow bridges, port / message routing, and zero-crossing wiring.

use std::collections::HashMap;

use sysml_core::{ElementKind, Value};

use crate::expressions::ExprIR;
use crate::ode_builder;
use crate::orchestrator::{Orchestrator, SubsystemIndex};
use crate::StateMachineIR;

use super::*;

impl ModelCompiler {
    /// Detect `ZeroCrossingEventDef` usages in parts that specialize
    /// `ContinuousStateSpaceDynamics`. Auto-wires zero-crossing events
    /// from the spec type to the runtime `ZeroCrossingDetector`.
    ///
    /// Called from `build_workspace_orchestrator()` after ODE subsystems are added.
    pub(crate) fn wire_ssr_zero_crossing_events(&self, orchestrator: &mut Orchestrator) {
        use crate::ode_events::{CrossingDirection, ZeroCrossingDetector};

        let subsystem_names = orchestrator.subsystem_names();

        // Collect events grouped by ODE label so we can batch them into one detector.
        let mut events_by_ode: HashMap<String, Vec<(String, ExprIR)>> = HashMap::new();

        // Find ActionDefinitions/ActionUsages/OccurrenceUsages that specialize
        // ZeroCrossingEventDef and have a guard/condition expression.
        for element in self.graph.elements.values() {
            if !matches!(
                element.kind,
                ElementKind::ActionDefinition
                    | ElementKind::ActionUsage
                    | ElementKind::OccurrenceUsage
            ) {
                continue;
            }

            if !self.specializes_name(&element.id, "ZeroCrossingEventDef") {
                continue;
            }

            // Get the guard/condition expression (AST-first).
            let guard_expr =
                sysml_core::expression_pretty::pretty_print_owner(element, &self.graph).or_else(
                    || {
                        element
                            .get_prop("guard")
                            .or_else(|| element.get_prop("condition"))
                            .and_then(|v| v.as_str().map(|s| s.to_string()))
                    },
                );

            let guard_expr = match guard_expr {
                Some(g) => g,
                None => continue,
            };

            let compiled = match ode_builder::parse_derivative(&guard_expr) {
                Ok(expr) => expr,
                Err(_) => continue,
            };

            let event_name = element
                .name
                .clone()
                .unwrap_or_else(|| "zero_crossing".to_string());

            // Find the owning part and match to an ODE subsystem
            let owner_id = match &element.owner {
                Some(id) => id.clone(),
                None => continue,
            };
            let owner_name = self
                .graph
                .get_element(&owner_id)
                .and_then(|o| o.name.clone())
                .unwrap_or_default();

            let ode_label = if subsystem_names.contains(&owner_name) {
                owner_name
            } else {
                "compiled-ode".to_string()
            };

            events_by_ode
                .entry(ode_label)
                .or_default()
                .push((event_name, compiled));
        }

        // RSC-4.2 (L39): `add_crossing_detector` is now keyed by
        // `SubsystemIndex`, not name. This pathway (spec-typed
        // `ZeroCrossingEventDef`, distinct from the `accept when` comparator
        // wiring `wire_when_crossings_for_pair` fixes above) has no
        // per-event target SM to capture — it is OUT OF SCOPE for this arc
        // (not enumerated in the RSC-4.2 plan). To stay byte-identical,
        // resolve each `ode_label` and the injection target to the exact
        // subsystems the OLD name-keyed/"first StateMachine subsystem"
        // behavior would have found: a ContinuousDynamics subsystem is the
        // only kind step 1b's capture ever matched by name, and no further
        // StateMachine registration happens after this call, so the first
        // StateMachine-phase subsystem here is the same one tick-time
        // "first SM" resolution would have found.
        let first_sm_index = orchestrator
            .subsystems()
            .iter()
            .position(|s| s.executor.phase() == crate::orchestrator::ExecutionPhase::StateMachine)
            .map(|i| SubsystemIndex(i as u16));

        // Create detectors and register them
        for (ode_label, events) in events_by_ode {
            let ode_index = orchestrator.subsystems().iter().position(|s| {
                s.name == ode_label
                    && s.executor.phase() == crate::orchestrator::ExecutionPhase::ContinuousDynamics
            });
            let (Some(ode_index), Some(sm_index)) = (ode_index, first_sm_index) else {
                // No matching ContinuousDynamics subsystem (or no SM to
                // target) — the pre-RSC-4.2 detector would have been
                // orphaned here too (step 1b's capture never matches a
                // non-ContinuousDynamics or nonexistent name), so skipping
                // registration is byte-identical, not a behavior change.
                continue;
            };
            let mut detector = ZeroCrossingDetector::new();
            for (event_name, compiled) in events {
                let event_fn: crate::ode_events::EventFn =
                    std::sync::Arc::new(move |_t, _y, ctx| {
                        let evaluator = crate::expressions::ExpressionEvaluator::new();
                        let is_true = match evaluator.eval(&compiled, ctx) {
                            Ok(Value::Bool(b)) => b,
                            Ok(Value::Float(f)) => f != 0.0,
                            Ok(Value::Int(i)) => i != 0,
                            _ => false,
                        };
                        if is_true {
                            1.0
                        } else {
                            -1.0
                        }
                    });
                detector.add_event(event_name, CrossingDirection::Rising, event_fn);
            }
            orchestrator.add_crossing_detector(
                SubsystemIndex(ode_index as u16),
                sm_index,
                detector,
            );
            // RSC-4.3 (ledger L46): the SSR `ZeroCrossingEventDef` path is
            // DELIBERATELY NOT marked re-step-eligible. A `ZeroCrossingEventDef`
            // occurrence expresses no effect binding, so its target SM here is
            // only a positional guess ("first StateMachine subsystem") — handing
            // a synchronous mid-tick re-entry knife to a name/position-derived
            // target would be unsound. This detector therefore keeps the
            // pre-RSC-4.3 post-step `inject_event` (one tick's ODE integration
            // late). The oscillator-vs-SSR asymmetry is made LOUD here rather than
            // silent (addendum). Threading a real target once a model expresses
            // the binding is the runtime half of L46 (steward consult at Wave 2).
            orchestrator.push_compile_warnings([sysml_span::Diagnostic::warning(format!(
                "zero-crossing event on ODE '{ode_label}' uses the SSR \
                 ZeroCrossingEventDef path, which is GATED OFF from the RSC-4.3 \
                 time-accurate mid-tick re-step (it carries no per-event effect \
                 binding — its target state machine is only a positional guess). \
                 The event is delivered one tick late via post-step injection; the \
                 located `accept when` comparator path re-steps in-tick. See ledger L46."
            ))
            .with_code("RS014")]);
        }
    }

    // -- Composite SSR pattern detection ------------------------------------------

    /// Detect and register flow gates for state machines.
    ///
    /// For each state machine (instance-multiplied or top-level), finds terminal
    /// states (no outgoing transitions). These are registered as gating states —
    /// when the SM enters a terminal state, the flow gate `{sm_name}.__flow_gate`
    /// is set to false.
    ///
    /// Instance-multiplied: gates per-instance, requires both SM and ODE.
    /// Top-level: gates global flows when the model has physics.
    ///
    /// Generic: works for any domain (breakers, valves, relays, clutches).
    pub(crate) fn detect_and_register_flow_gates(
        &self,
        orchestrator: &mut Orchestrator,
        instances: &[InstanceSpec],
    ) {
        // Instance-multiplied SMs: gate per-instance flows
        for inst in instances {
            if inst.sm_names.is_empty() || inst.ode_detections.is_empty() {
                continue;
            }

            for sm_name in &inst.sm_names {
                let sm_subsystem_name = format!("{}.{}", inst.prefix, sm_name);
                self.register_flow_gate_for_sm(orchestrator, sm_name, &sm_subsystem_name);
            }
        }

        // Top-level SMs: gate global flows
        let sm_results = crate::statemachine::StateMachineCompiler::compile_all(&self.graph);
        let multiplied_sm_names: std::collections::HashSet<String> = instances
            .iter()
            .flat_map(|i| i.sm_names.iter().cloned())
            .collect();

        for (name, result) in sm_results {
            if multiplied_sm_names.contains(&name) {
                continue; // Already handled above
            }
            if result.is_ok() {
                self.register_flow_gate_for_sm(orchestrator, &name, &name);
            }
        }
    }

    /// Helper: compile SM, find terminal states, register as flow gate.
    fn register_flow_gate_for_sm(
        &self,
        orchestrator: &mut Orchestrator,
        sm_def_name: &str,
        sm_subsystem_name: &str,
    ) {
        if let Ok(ir) = self.compile_state_machine(sm_def_name) {
            let terminal = ir.terminal_states();
            if !terminal.is_empty() {
                let gating_states: Vec<String> = terminal.iter().map(|s| s.to_string()).collect();
                #[cfg(feature = "tracing")]
                tracing::info!(
                    sm = %sm_subsystem_name,
                    gating = ?gating_states,
                    "registered flow gate"
                );
                orchestrator.add_flow_gate(sm_subsystem_name, gating_states);
            }
        }
    }

    /// Wire zero-crossing detectors for `accept when <comparator>` SM triggers
    /// over continuous ODE state (WS-A2 / hybrid event location).
    ///
    /// A `when` trigger compiles to [`TriggerKind::When`], which the SM runner
    /// samples at tick boundaries (rising edge). For a stiff or fast hybrid
    /// system a threshold crossing happens mid-tick and the boundary sample
    /// misses it — the SM freezes. The spec's `ChangeSignal`/`TriggerWhen`
    /// semantics (Triggers.kerml, Observation.kerml `ObserveChange`) fire at the
    /// *instant* the condition becomes true, which is exactly what zero-crossing
    /// location computes.
    ///
    /// For each co-located SM + ODE this scans the SM's `accept when` triggers
    /// (`event == "when <comparator>"`). A trigger qualifies for crossing
    /// location when (steward ruling, plan §5 WS-A2):
    /// 1. it is a threshold comparison `lhs <op> rhs`, `op ∈ {<, <=, >, >=}`
    ///    (equality is excluded — measure-zero crossings can't be bisected), and
    /// 2. it references at least one continuous quantity of *this* ODE — a state
    ///    variable or a `GetOutput` signal (a pure function of the state).
    ///
    /// For a qualifying trigger it builds a SMOOTH residual `g = lhs − rhs`
    /// (`Rising` for `>`/`>=`, `Falling` for `<`/`<=`), recomputing any ODE
    /// output signals from the *interpolated* state at each bisection point
    /// (reusing the model's signal expressions — gap vs. the old boolean ±1
    /// event fn that read stale post-step context), registers it on the owning
    /// ODE's detector, and rewrites the transition's trigger to
    /// [`TriggerKind::WhenLocated`] so the located crossing event fires it.
    ///
    /// Non-qualifying `when` triggers (equality, or referencing discrete /
    /// non-continuous variables) are left as tick-sampled `When` — the best
    /// available approximation for the discrete case (SPEC-SILENT).
    ///
    /// RSC-4.2 (L39): each located crossing is injected into the exact
    /// target `SubsystemIndex` captured at the SM's own registration —
    /// never "the first StateMachine subsystem" — so this now also routes
    /// correctly for multiple instances of one SM+ODE pair (previously a
    /// documented multi-instance limitation; crossing event names are still
    /// unique per (SM, transition), and now so is the injection target).
    pub(crate) fn wire_zero_crossing_detectors(
        &self,
        orchestrator: &mut Orchestrator,
        instances: &[InstanceSpec],
        primary_ode_detections: &[OdeDetection],
        primary_sm_names: &[(String, SubsystemIndex)],
    ) {
        // Instance-multiplied pairs: subsystem names carry the instance prefix.
        for inst in instances {
            if inst.sm_names.is_empty() || inst.ode_detections.is_empty() {
                continue;
            }
            for sm_name in &inst.sm_names {
                let Ok(ir) = self.compile_state_machine(sm_name) else {
                    continue;
                };
                // Captured at this SM's registration in the instance-
                // multiplication step (above); a name that compiled
                // successfully here compiled successfully there too (same
                // deterministic `compile_state_machine` over the same
                // graph), so the entry is guaranteed present — RSC-4.2
                // ruling 4 fail-hard, not a silent skip.
                let sm_index = *inst.sm_subsystem_indices.get(sm_name).unwrap_or_else(|| {
                    panic!(
                        "RSC-4.2 wiring bug: instance-multiplied SM '{sm_name}' \
                         (instance '{}') compiled successfully during crossing-\
                         detector wiring but has no registered SubsystemIndex — \
                         registration and wiring must observe the same compile \
                         result.",
                        inst.prefix
                    )
                });
                let sm_subsystem = format!("{}.{}", inst.prefix, sm_name);
                for ode in &inst.ode_detections {
                    // A `None` here means this ODE detection failed to
                    // register a subsystem (compiler.rs step 4's `Err(e)`
                    // arm) — nothing exists to locate crossings against, so
                    // skip it exactly as the orphaned pre-RSC-4.2 detector
                    // would have (it could never have fired either, since
                    // step 1b's capture only matches a real ContinuousDynamics
                    // subsystem).
                    let Some(ode_index) = ode.subsystem_index else {
                        continue;
                    };
                    let ode_label =
                        format!("{}.{}", inst.prefix, ode.name.as_deref().unwrap_or("ode"));
                    self.wire_when_crossings_for_pair(
                        orchestrator,
                        &ir,
                        &sm_subsystem,
                        sm_index,
                        &ode_label,
                        ode,
                        ode_index,
                        &inst.prefix,
                    );
                }
            }
        }

        // Primary (non-multiplied) pairs: bare subsystem names (e.g. the
        // oscillator fixture's top-level `OscillatorStateMachine` + `CoreODE`).
        for (sm_name, sm_index) in primary_sm_names {
            let Ok(ir) = self.compile_state_machine(sm_name) else {
                continue;
            };
            for ode in primary_ode_detections {
                let Some(ode_index) = ode.subsystem_index else {
                    continue;
                };
                let ode_label = ode.name.as_deref().unwrap_or("compiled-ode").to_owned();
                self.wire_when_crossings_for_pair(
                    orchestrator,
                    &ir,
                    sm_name,
                    *sm_index,
                    &ode_label,
                    ode,
                    ode_index,
                    "",
                );
            }
        }
    }

    /// RSC-4.3 (L47): does `event` (an SM transition's raw event string)
    /// lower to a located zero-crossing trigger against a continuous
    /// quantity in `continuous`? Returns the parsed comparator + crossing
    /// direction when it qualifies. Shared by
    /// [`Self::wire_when_crossings_for_pair`] (which builds the actual
    /// detector from the result) and
    /// [`Self::sm_has_qualifying_when_crossing`] (which only needs to know
    /// whether ANY transition qualifies, ahead of solver selection) — the
    /// same test in both places, never a second drift-prone copy.
    fn qualifying_when_crossing(
        event: Option<&str>,
        continuous: &std::collections::HashSet<&str>,
    ) -> Option<(ExprIR, crate::ode_events::CrossingDirection)> {
        use crate::expressions::BinOp;
        use crate::ode_events::CrossingDirection;

        // `accept when X` lowers to event == "when X" (guard None).
        let inner = event.and_then(when_comparator)?;
        let cmp = ode_builder::parse_derivative(inner).ok()?;
        // (1) threshold comparison only.
        let ExprIR::BinaryOp { op, .. } = &cmp else {
            return None;
        };
        let direction = match op {
            BinOp::GreaterThan | BinOp::GreaterEqual => CrossingDirection::Rising,
            BinOp::LessThan | BinOp::LessEqual => CrossingDirection::Falling,
            _ => return None, // equality / other → not located
        };
        // (2) must reference a continuous quantity of THIS ODE.
        let free = cmp.free_variables();
        if !free.iter().any(|v| continuous.contains(v.as_str())) {
            return None;
        }
        Some((cmp, direction))
    }

    /// RSC-4.3 (L47): true if `ir`'s state machine has at least one
    /// qualifying `accept when` comparator trigger against `ode`'s
    /// continuous quantities — the exact condition under which
    /// [`Self::wire_when_crossings_for_pair`] will later call
    /// `mark_restep_eligible` for this ODE. Solver selection (which runs
    /// BEFORE wiring, at ODE-construction time) uses this to pin RK45 ahead
    /// of WS-B4's auto-stiffness switch: restep-eligibility structurally
    /// requires RK45 in Wave 1 (ledger L47) — only `Rk45Solver`/`Rk4Solver`
    /// implement the re-step protocol; `BdfSolver`'s multistep history makes
    /// true rollback out of scope (FORBIDDEN new solver machinery, Q9).
    pub(crate) fn sm_has_qualifying_when_crossing(ir: &StateMachineIR, ode: &OdeDetection) -> bool {
        let signal_exprs: Vec<(String, ExprIR)> = ode
            .signal_exprs
            .iter()
            .filter_map(|(n, s)| ode_builder::parse_derivative(s).ok().map(|e| (n.clone(), e)))
            .collect();
        let continuous: std::collections::HashSet<&str> = ode
            .state_vars
            .iter()
            .map(String::as_str)
            .chain(signal_exprs.iter().map(|(n, _)| n.as_str()))
            .collect();
        ir.transitions
            .iter()
            .any(|t| Self::qualifying_when_crossing(t.event.as_deref(), &continuous).is_some())
    }

    /// RSC-4.3 (L47): true if any SM in `sm_names` will end up paired with
    /// `ode` via a qualifying `accept when` crossing trigger once
    /// `wire_zero_crossing_detectors` runs later in the pipeline — i.e.
    /// `ode` will be marked restep-eligible. Re-compiles each SM (mirrors
    /// the existing re-compile-at-wire-time pattern in
    /// `wire_zero_crossing_detectors`); a name that fails to compile here
    /// fails identically at wire time, so treating it as non-qualifying is
    /// consistent, not a silent skip of a real pairing.
    pub(crate) fn any_sm_has_qualifying_when_crossing<'a>(
        &self,
        sm_names: impl IntoIterator<Item = &'a str>,
        ode: &OdeDetection,
    ) -> bool {
        sm_names.into_iter().any(|sm_name| {
            self.compile_state_machine(sm_name)
                .is_ok_and(|ir| Self::sm_has_qualifying_when_crossing(&ir, ode))
        })
    }

    /// Build and register a zero-crossing detector for the qualifying
    /// `accept when <comparator>` triggers of one SM that reference one ODE's
    /// continuous quantities, rewriting each to [`TriggerKind::WhenLocated`].
    /// Shared by the instance-multiplied and primary paths
    /// (`prefix` is `""` for primary). See [`Self::wire_zero_crossing_detectors`].
    #[allow(clippy::too_many_arguments)]
    fn wire_when_crossings_for_pair(
        &self,
        orchestrator: &mut Orchestrator,
        ir: &StateMachineIR,
        sm_subsystem: &str,
        sm_index: SubsystemIndex,
        ode_label: &str,
        ode: &OdeDetection,
        ode_index: SubsystemIndex,
        prefix: &str,
    ) {
        use crate::expressions::{BinOp, ExpressionEvaluator};
        use crate::ode_events::{CrossingDirection, ZeroCrossingDetector};

        // This ODE's state vars (index-aligned with the executor's state
        // vector) and its output-signal expressions.
        let state_var_names = ode.state_vars.clone();
        let mut signal_exprs: Vec<(String, ExprIR)> = ode
            .signal_exprs
            .iter()
            .filter_map(|(n, s)| {
                ode_builder::parse_derivative(s)
                    .ok()
                    .map(|e| (n.clone(), e))
            })
            .collect();
        // Task #8: deterministic order for the crossing-location event_fn's
        // signal recomputation — same `HashMap`-iteration hazard the ODE RHS /
        // signal-sync fix addresses. This path re-parses the raw `ode.signal_exprs`
        // strings (not the spec's bound exprs), so it can't reuse `OdeSpec`'s
        // dependency order directly; a name sort is the deterministic minimum
        // (the located comparator residual reads a single continuous quantity,
        // so cross-signal order does not affect the located crossing, but a
        // fixed order removes the last per-process signal-eval variation here).
        signal_exprs.sort_by(|a, b| a.0.cmp(&b.0));
        // The continuous quantities a comparator may reference and still be
        // locatable from the interpolated state.
        let continuous: std::collections::HashSet<&str> = state_var_names
            .iter()
            .map(String::as_str)
            .chain(signal_exprs.iter().map(|(n, _)| n.as_str()))
            .collect();

        let signal_names: std::collections::HashSet<&str> =
            signal_exprs.iter().map(|(n, _)| n.as_str()).collect();

        // Task #10 fix: the same stale-bare-name-shadow hazard
        // `build_signal_sync`'s `template_omitted_params` strip
        // (`ode_builder.rs`) exists to fix — a PARAMETER (not one of this
        // closure's own signal targets) that also has a live, externally-
        // tracked value under a QUALIFIED key (the confirmed case: `duty`,
        // which `DutyCycleTracker` only ever publishes as `{ode_name}.duty`,
        // never the bare name) is shadowed by the bare name's stale,
        // once-seeded-at-t=0 default sitting in the master context's
        // `variables` map forever after. `build_signal_sync` fixes this by
        // STRIPPING the stale name and letting its `SlotRef`-bound (post
        // `bind_slots`) expressions fall through to the live slot. This
        // closure's `signal_exprs` are a fresh `parse_derivative` re-parse —
        // never `bind_slots`-rewritten — so a bare `ExprIR::FeatureRef` has NO
        // slot fallback (`evaluator.rs`'s `FeatureRef` arm only ever consults
        // `ctx.get(name)`); stripping here would just make evaluation fail
        // (`UndefinedVariable`), not fall through correctly. The equivalent
        // fix is REFRESH: overwrite each such name with its live slot value
        // before evaluating (below, inside `event_fn`).
        let shadow_params: std::collections::HashSet<String> = signal_exprs
            .iter()
            .flat_map(|(_, expr)| expr.free_variables())
            .filter(|v| ode.parameters.contains_key(v) && !signal_names.contains(v.as_str()))
            .collect();

        let mut detector = ZeroCrossingDetector::new();
        let mut registered = 0usize;
        // WS-D Stage 2: comparator metadata for the duty-cycle tracker.
        // Per registered crossing: (event_name, direction, comparator variable,
        // is the comparator variable an output signal?). The square-wave
        // comparator is the variable carrying BOTH a Rising and a Falling
        // crossing (a complete ±threshold comparator); see below.
        let mut cmp_meta: Vec<(String, CrossingDirection, String, bool)> = Vec::new();

        for (idx, t) in ir.transitions.iter().enumerate() {
            let Some((cmp, direction)) =
                Self::qualifying_when_crossing(t.event.as_deref(), &continuous)
            else {
                continue;
            };
            let ExprIR::BinaryOp { left, right, .. } = &cmp else {
                unreachable!("qualifying_when_crossing only returns BinaryOp comparators")
            };
            let free = cmp.free_variables();

            // Smooth residual g = lhs − rhs. A `Rising` crossing (−→+) means
            // `lhs >= rhs` just became true; `Falling` (+→−) means `lhs <= rhs`.
            let residual = ExprIR::BinaryOp {
                op: BinOp::Subtract,
                left: left.clone(),
                right: right.clone(),
            };
            // On eval failure return the "guard-false" sign so a transient
            // evaluation gap can't fabricate a crossing.
            let false_sign = match direction {
                CrossingDirection::Rising => -1.0,
                _ => 1.0,
            };

            let event_name = format!("__zc::{sm_subsystem}::{idx}");
            let prefix = prefix.to_owned();
            let state_names = state_var_names.clone();
            let sigs = signal_exprs.clone();
            let resid = residual.clone();
            let shadow = shadow_params.clone();

            let event_fn: crate::ode_events::EventFn = std::sync::Arc::new(move |_t, y, ctx| {
                let evaluator = ExpressionEvaluator::new();
                // Scoped, prefix-stripped view of the master context (params,
                // sampled fns, V_applied, …). Empty prefix → no-op strip; the
                // bare keys from the clone are used directly.
                //
                // Task #10 write-leak fix (cull-arc W3): this is the one scratch
                // site whose base IS the master context, so a live clone would
                // share the `slots` handle and every `scoped.set()` below would
                // write straight through to the SAME production `SlotStore` —
                // silently corrupting real per-tick state (e.g. `I_est`) from
                // speculative, throwaway bisection root-finding. `scratch_snapshot`
                // copies then demotes the handle to read-only (fusing the former
                // explicit `demote_to_read_only`): reads stay live via
                // `slot_reader`, writes stay local. The assert guards the demotion.
                let mut scoped = ctx.scratch_snapshot();
                debug_assert!(
                    scoped.slots.is_none(),
                    "event_fn must never retain a live slot write handle — bisection \
                     root-finding is speculative and must not mutate production state"
                );
                if !prefix.is_empty() {
                    let dot_prefix = format!("{prefix}.");
                    for (key, val) in ctx.variables.iter() {
                        if let Some(stripped) = key.strip_prefix(&dot_prefix) {
                            scoped.set(stripped.to_owned(), val.clone());
                        }
                    }
                }
                // Overlay the INTERPOLATED state at this bisection point,
                // overwriting the stale post-step values.
                for (i, name) in state_names.iter().enumerate() {
                    if i < y.len() {
                        scoped.set(name.clone(), Value::Float(y[i]));
                    }
                }
                // Refresh every parameter this closure's OWN signal
                // expressions reference against its live slot value (see the
                // `shadow_params` comment above) — `duty` is the confirmed
                // case. Prefix-qualify the lookup the same way
                // `WriteRoute::resolve` does, so a prefixed (instance-
                // multiplied) ODE reads its own instance's slot, never a
                // foreign one.
                for name in &shadow {
                    let qualified = if prefix.is_empty() {
                        name.clone()
                    } else {
                        format!("{prefix}.{name}")
                    };
                    if let Some(v) = scoped.slot_value_if_enabled(&qualified) {
                        scoped.set(name.clone(), v);
                    }
                }
                // Recompute output signals from the interpolated state so signal
                // comparators (e.g. i_drive) are located, not read stale.
                for (sn, se) in &sigs {
                    match evaluator.eval(se, &scoped) {
                        Ok(Value::Float(f)) => scoped.set(sn.clone(), Value::Float(f)),
                        Ok(Value::Int(i)) => scoped.set(sn.clone(), Value::Float(i as f64)),
                        _ => {}
                    }
                }
                match evaluator.eval(&resid, &scoped) {
                    Ok(Value::Float(f)) => f,
                    Ok(Value::Int(i)) => i as f64,
                    _ => false_sign,
                }
            });

            detector.add_event(event_name.clone(), direction, event_fn);
            // Route the located crossing to the transition. RSC-4.2 L39: the
            // exact SubsystemIndex captured at this SM's registration
            // replaces the old prefixed-then-bare name-guess pair — no
            // ambiguity to retry through.
            orchestrator.set_sm_located_trigger(sm_index, idx, &event_name);
            // The comparator variable is the continuous free var (state var or
            // signal); the constant side (thresholds like `I_threshold`,
            // `0.999 * Bs`) is a parameter, not continuous.
            if let Some(cmp_var) = free.iter().find(|v| continuous.contains(v.as_str())) {
                let on_signal = signal_names.contains(cmp_var.as_str());
                cmp_meta.push((event_name.clone(), direction, cmp_var.clone(), on_signal));
            }
            registered += 1;
        }

        if registered > 0 {
            #[allow(clippy::print_stderr)]
            if std::env::var_os("SYSML_TRACE_WS_A2").is_some() {
                eprintln!(
                    "[WS-A2] registered {registered} located crossing(s) on ODE '{ode_label}' for SM '{sm_subsystem}'"
                );
            }
            orchestrator.add_crossing_detector(ode_index, sm_index, detector);
            // RSC-4.3: the `accept when` comparator path carries a real
            // per-transition target-SM binding (`set_sm_located_trigger` above),
            // so its located crossings are eligible for the time-accurate
            // mid-tick re-step. (The SSR `ZeroCrossingEventDef` path is NOT — see
            // `wire_ssr_zero_crossing_events` and ledger L46.)
            orchestrator.mark_restep_eligible(ode_index);

            // WS-D Stage 2 (SPEC-SILENT): if a comparator variable carries BOTH
            // a Rising (signal ↑ +threshold) and a Falling (signal ↓ −threshold)
            // crossing, it forms a square wave whose duty-cycle asymmetry is the
            // fault signature. Prefer an output signal (firmware measures the
            // drive signal, e.g. `i_drive`) over a bare state var (e.g. a
            // `B`-saturation safety backstop), so the detection metric tracks
            // the comparator, not the safety limit.
            if let Some(tracker) = build_duty_tracker(&cmp_meta) {
                orchestrator.add_duty_tracker(ode_index, tracker);
            }
        }
    }

    /// Generate computed expressions from flow connections by walking the
    /// `FlowConnectionIR` list. For each flow `A.portX → B.portY` in a
    /// classified physics domain, creates:
    ///   `B.portY.flow_feature = A.portX.flow_feature`
    ///
    /// The flow feature is determined by the physics domain of the port
    /// (e.g., "current" for electrical, "heatFlow" for thermal).
    ///
    /// This is a generic replacement for hardcoded port-to-variable bridges.
    ///
    /// COMPLEMENTARY-PATHS INVARIANT (RSC-3.5d, steward-ruled — ledger L26):
    /// this continuous path and the discrete [`ExchangePlane`] router are two
    /// non-overlapping delivery mechanisms over the same classified
    /// [`LinkGraph`], NOT duplicates:
    ///   * Continuous (here + `compile_signal_propagation`): per-tick copy of a
    ///     domain-classified flow feature, `tgt.feat = src.feat`. Only links
    ///     whose ports resolve to a physics domain get a computed expression;
    ///     `MessageChannel` (item/event) links are unclassifiable here and
    ///     `continue` — so they are NEVER continuously copied.
    ///   * Discrete (router, wired in `wire_message_router`): occurrence-
    ///     addressed `Transfer` delivery driven by `route_pending()`. The class
    ///     gate routes `MessageChannel`/`SignalLink`/`Unknown` and skips
    ///     `PowerBond`. A `SignalLink` therefore sits in the routing table but
    ///     its delivery gate is INERT unless an SM explicitly `send()`s to a
    ///     signal source key — its per-tick value still flows via this
    ///     continuous path. The inertness is load-bearing: it is the spec-silent
    ///     reason the two paths never double-deliver, NOT enforced by an ingest
    ///     filter (filtering would invent a non-spec "SignalLink is unroutable"
    ///     distinction — `Transfers.kerml:76` makes `FlowTransfer` routable).
    pub(crate) fn wire_generic_flow_bridge(
        &self,
        orchestrator: &mut Orchestrator,
        link_graph: &crate::links::LinkGraph,
    ) {
        use crate::physics::connection::{
            classify_port_def_by_name, find_port_definition_for_name,
        };

        // RSC-3.5e.5 W3: iterate the flow-derived subset of the classified link
        // graph (interning order == the former `compile_flows` order).
        for flow in link_graph
            .iter()
            .filter(|l| l.kind == crate::links::LinkSourceKind::FlowUsage)
        {
            let src_key = flow.source.key();
            let tgt_key = flow.target.key();

            // Classify source port
            let src_port_name = flow.source.port.as_str();
            let tgt_port_name = flow.target.port.as_str();

            let src_domain = find_port_definition_for_name(src_port_name, &self.graph)
                .and_then(|def| classify_port_def_by_name(&def));
            let tgt_domain = find_port_definition_for_name(tgt_port_name, &self.graph)
                .and_then(|def| classify_port_def_by_name(&def));

            // Determine domain and flow feature from either endpoint
            let flow_feat: String = match (src_domain, tgt_domain) {
                (Some((d1, f)), Some((d2, _))) if d1 == d2 => f.to_owned(),
                (Some((_d, f)), None) | (None, Some((_d, f))) => f.to_owned(),
                _ => continue, // Can't classify — skip this flow
            };

            // Forward propagation: target.feature = source.feature
            let src_var = format!("{}.{}", src_key, flow_feat);
            let tgt_var = format!("{}.{}", tgt_key, flow_feat);

            if let Ok(expr) = ode_builder::parse_derivative(&src_var) {
                orchestrator.add_computed_expression(&tgt_var, expr);
            }
        }
    }

    /// Wire the discrete message router (RSC-3.5d, steward-ruled — ledger L26).
    ///
    /// The classified [`LinkGraph`](crate::links::LinkGraph) was installed on
    /// the orchestrator at step 7a; the [`ExchangePlane`](crate::exchange::ExchangePlane)
    /// adopts a clone of it as its routing table (the LinkGraph IS the routing
    /// table, design-doc §8) via
    /// [`ingest_classified`](crate::exchange::ExchangePlane::ingest_classified)
    /// so discrete occurrence-addressed `Transfer`s deliver on a compiled model.
    /// Before this step the production router was an empty `ExchangePlane::new()`
    /// (every `ingest_classified`/`set_exchange_plane` caller was test-only), so
    /// `route_pending()` had no routing table and SM port-message triggers
    /// (`accept … via <port>`, L38) and the `isMove` move-clear (L26) were
    /// dormant on parsed models — only ever proven via hand-wired tests.
    ///
    /// `flow_ids` is the dense-parallel label vector `ingest_classified`
    /// requires; it is synthesized here from
    /// [`LinkIR::display_label`](crate::links::LinkIR::display_label) in
    /// interning order. The label is router display metadata (consumed by
    /// `flow_inspect`), not link identity, so it lives at the wiring site rather
    /// than on `LinkIR` (steward §8 Q5).
    ///
    /// A clone — not a shared `Arc` — is correct: the router owns mutable
    /// delivery-queue state while the orchestrator keeps its `link_graph` field
    /// for read-only inspection (signal propagation, physics, `flow_inspect`).
    /// The clone is an O(links) one-time build cost, never in the hot loop.
    /// [`set_exchange_plane`](crate::orchestrator::Orchestrator::set_exchange_plane)
    /// re-registers every already-built subsystem's accepting surfaces on the
    /// new plane (acceptor topology follows the subsystems, not the backend
    /// instance), so the `accept`-port acceptors survive the install.
    pub(crate) fn wire_message_router(&self, orchestrator: &mut Orchestrator) {
        // Step 7a only installs the link graph when non-empty; mirror that gate
        // so models with no flows leave the router as the empty default (no
        // behavioural change, byte-identical).
        if orchestrator.link_graph().is_empty() {
            return;
        }
        let link_graph = orchestrator.link_graph().clone();
        let flow_ids: Vec<String> = link_graph
            .iter()
            .map(|link| link.display_label(&self.graph))
            .collect();
        let mut plane = crate::exchange::ExchangePlane::new();
        plane.ingest_classified(link_graph, flow_ids);
        orchestrator.set_exchange_plane(plane);
    }

    /// Bridge instance ODE state variables to flow-connected port variables.
    ///
    /// For each instance, finds ODE state vars and searches the flow graph
    /// to locate port variables on connected parts. Generates computed
    /// expressions that propagate state var values to port flow features.
    ///
    /// Generic: derives all variable names from model structure (instances,
    /// ODE detections, flow connections). No domain-specific string matching.
    pub(crate) fn wire_port_flow_bridge(
        &self,
        orchestrator: &mut Orchestrator,
        instances: &[InstanceSpec],
        link_graph: &crate::links::LinkGraph,
    ) {
        let instance_prefixes: Vec<&str> = instances.iter().map(|i| i.prefix.as_str()).collect();
        if instance_prefixes.is_empty() {
            return;
        }

        // Collect all ODE input parameter names per instance (from detections)
        for inst in instances {
            for ode in &inst.ode_detections {
                // For each ODE state var, find flow connections where the
                // instance is the target. Bridge the state var to the source
                // port's flow feature.
                for state_var in &ode.state_vars {
                    let prefixed_var = format!("{}.{}", inst.prefix, state_var);

                    // RSC-3.5e.5 W3: search the flow-derived links for ports
                    // connecting TO this instance. The classified link graph is
                    // built once by the caller — this replaces the former
                    // per-state-var `compile_flows` recompile.
                    for flow in link_graph
                        .iter()
                        .filter(|l| l.kind == crate::links::LinkSourceKind::FlowUsage)
                    {
                        let tgt_key = flow.target.key();
                        // If the flow target is within this instance's scope
                        if tgt_key.starts_with(&format!("{}.", inst.prefix))
                            || flow.target.owner == inst.prefix
                        {
                            // Classify the source port to get the flow feature name
                            let src_port_name = flow.source.port.as_str();
                            let flow_feat =
                                crate::physics::connection::find_port_definition_for_name(
                                    src_port_name,
                                    &self.graph,
                                )
                                .and_then(|def| {
                                    crate::physics::connection::classify_port_def_by_name(&def)
                                })
                                .map(|(_, f)| f);

                            if let Some(feat) = flow_feat {
                                let port_var = format!("{}.{}", flow.source.key(), feat);
                                if let Ok(expr) = ode_builder::parse_derivative(&prefixed_var) {
                                    orchestrator.add_computed_expression(&port_var, expr);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    /// Wire SampledFunction waveforms to ODE inputs via scenario bindings.
    ///
    /// Finds scenario part defs containing `prefix_suffix : SampledFunction = waveformRef`
    /// bindings. Splits the attribute name on `_` to get the instance prefix and
    /// a parameter hint. Searches the instance's ODE parameters and state vars
    /// for a matching input variable (case-insensitive substring match on the
    /// hint). Generates `prefix.matched_var = interpolateLinear(__sf_ref, __clock_time)`.
    ///
    /// Generic: no hardcoded parameter name mapping. The suffix is matched against
    /// actual ODE variables found in the model.
    pub(crate) fn wire_scenario_waveforms(&self, orchestrator: &mut Orchestrator) {
        // RSC-3.7 C.1/C.2: SampledFunction lookup tables are minted as global
        // `__sf_{name}` slots (in `mint_slot_store`) instead of bare context
        // keys, but the slot store is not attached until after this wiring runs.
        // The "is the SF data present?" guard below therefore checks model
        // membership — the SAME `extract_sampled_functions` set the mint uses —
        // rather than the (now-absent) bare `__sf_` context key, so a waveform is
        // wired iff a slot will exist to serve its read.
        let available_sf: std::collections::HashSet<String> = self
            .extract_sampled_functions()
            .unwrap_or_default()
            .into_iter()
            .map(|(name, _)| name)
            .collect();

        // Collect ODE input parameter names per instance for matching
        let instances = self.expand_part_instances();
        let mut instance_ode_params: HashMap<String, Vec<String>> = HashMap::new();
        for inst in &instances {
            let mut params = Vec::new();
            for ode in &inst.ode_detections {
                params.extend(ode.state_vars.iter().cloned());
                params.extend(ode.parameters.keys().cloned());
            }
            instance_ode_params.insert(inst.prefix.clone(), params);
        }

        for attr in self.graph.elements_by_kind(&ElementKind::AttributeUsage) {
            let attr_name = match &attr.name {
                Some(n) => n.clone(),
                None => continue,
            };

            // Must be typed as SampledFunction
            let is_sf = attr
                .get_prop("unresolvedTypeName")
                .and_then(|v| match v {
                    Value::String(s) => Some(s.contains("SampledFunction")),
                    _ => None,
                })
                .unwrap_or(false)
                || self.graph.children_of(&attr.id).any(|c| {
                    c.kind == ElementKind::FeatureTyping
                        && c.get_prop("unresolved_type")
                            .and_then(|v| v.as_str())
                            .is_some_and(|s| s.contains("SampledFunction"))
                });

            if !is_sf {
                continue;
            }

            // Must have a reference value (e.g., = lightingWaveform) —
            // AST-first via the attribute's expression subtree, then
            // legacy `default` string prop.
            let waveform_ref = sysml_core::expression_pretty::pretty_print_owner(attr, &self.graph)
                .or_else(|| {
                    attr.get_prop("default")
                        .and_then(|v| v.as_str().map(|s| s.to_owned()))
                });

            let Some(waveform_name) = waveform_ref else {
                continue;
            };

            // Split name on last `_` to get (instance_prefix, param_hint)
            if let Some(underscore_pos) = attr_name.rfind('_') {
                let instance_prefix = &attr_name[..underscore_pos];
                let param_hint = &attr_name[underscore_pos + 1..];
                let hint_lower = param_hint.to_ascii_lowercase();

                // Find matching ODE parameter: case-insensitive substring match
                let target_param = instance_ode_params
                    .get(instance_prefix)
                    .and_then(|params| {
                        params
                            .iter()
                            .find(|p| p.to_ascii_lowercase().contains(&hint_lower))
                    })
                    .cloned()
                    // Fallback: use the hint directly
                    .unwrap_or_else(|| param_hint.to_owned());

                let sf_key = format!("__sf_{}", waveform_name);
                let target_var = format!("{}.{}", instance_prefix, target_param);

                // Only wire if the SF data exists (model membership; the slot is
                // minted under `sf_key` and serves the generated read).
                if available_sf.contains(&waveform_name) {
                    let expr_str = format!("interpolateLinear({}, __clock_time)", sf_key);
                    if let Ok(expr) = ode_builder::parse_derivative(&expr_str) {
                        orchestrator.add_computed_expression(&target_var, expr);
                        #[cfg(feature = "tracing")]
                        tracing::info!(
                            "Wired waveform: {} → {} via {}",
                            waveform_name,
                            target_var,
                            sf_key
                        );
                    }
                }
            }
        }
    }

}
