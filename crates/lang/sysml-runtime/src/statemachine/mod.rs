//! # sysml-run-statemachine
//!
//! State machine compilation and execution for SysML v2.
//!
//! This crate provides:
//! - Compilation from ModelGraph state machines to StateMachineIR
//! - A simple runner that executes the IR
//! - Parallel state machine runner for composite state machines with concurrent regions

#![allow(clippy::indexing_slicing)]
// State machine module uses expect/unwrap for invariant-checked operations.
#![allow(clippy::expect_used, clippy::unwrap_used)]
pub mod action_parser;
pub mod health;
pub mod parallel;

pub use action_parser::parse_action;
pub use health::{state_machine_health_diagnostics, state_machine_health_diagnostics_for_name};
pub use parallel::ParallelStateMachineRunner;

use crate::expressions::{compile_simple_expression, EvalContext, ExprIR, ExpressionEvaluator};
use crate::{
    CompileToIR, RegionIR, Runner, StateIR, StateMachineIR, StepResult, TransitionActionIR,
    TransitionIR,
};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use sysml_core::element_ordering::sort_elements_by_source_order;
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, RelationshipKind, Value};
use sysml_span::{Diagnostic, DiagnosticTier};

/// Diagnostic information about a guard evaluation — explains WHY a transition is blocked or enabled.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct GuardDiagnosis {
    /// The raw guard expression string.
    pub guard_expr: String,
    /// The transition: (from_state, to_state).
    pub transition: (String, String),
    /// Event required for this transition (if any).
    pub event: Option<String>,
    /// Variable names the guard reads.
    pub dependencies: std::collections::HashSet<String>,
    /// Current values of those variables.
    pub dependency_values: std::collections::HashMap<String, sysml_core::Value>,
    /// Whether the guard is currently satisfied.
    pub satisfied: bool,
    /// Human-readable explanation, e.g., "blocked: machineReady = 0, needs > 0"
    pub explanation: String,
}

/// Evaluate a guard expression string against an evaluation context.
///
/// First attempts to compile the guard string as an `ExprIR` and evaluate it.
/// If compilation or evaluation fails, falls back to string equality matching
/// against the event name (backward compatibility).
pub(crate) fn evaluate_guard(guard: &str, ctx: &EvalContext, event: Option<&str>) -> bool {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        guard = guard,
        event = ?event,
        binding_count = ctx.variables.len(),
        "evaluating transition guard"
    );

    match compile_simple_expression(guard) {
        Ok(expr) => {
            let evaluator = ExpressionEvaluator::new();
            match evaluator.eval(&expr, ctx) {
                Ok(Value::Bool(b)) => b,
                Ok(_) => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!(
                        guard = guard,
                        event = ?event,
                        "guard evaluated to non-boolean, falling back to event-name match"
                    );
                    // Non-boolean result — fall back to string matching
                    event == Some(guard)
                }
                Err(_) => {
                    #[cfg(feature = "tracing")]
                    tracing::debug!(
                        guard = guard,
                        event = ?event,
                        "guard evaluation failed, falling back to event-name match"
                    );
                    // Evaluation error (e.g., undefined variable) — fall back
                    event == Some(guard)
                }
            }
        }
        Err(_) => {
            #[cfg(feature = "tracing")]
            tracing::debug!(
                guard = guard,
                event = ?event,
                "guard compilation failed, falling back to event-name match"
            );
            // Compilation error — fall back to string matching
            event == Some(guard)
        }
    }
}

/// Statically-known assignment targets of a compiled state machine
/// (structured transition/entry/do/exit actions, nested machines and
/// parallel regions included). `Simple` string actions (entry/do/exit and
/// effect text the SM compiler did not structure) are normalized through the
/// same `action_parser` the runner uses at execution time, so `x = 1; y = 2`
/// strings surface their targets too.
///
/// This is the SM's **compiled write-set** (RSC-2.4b): the compiler claims
/// these names as `Discrete / Executor(owning SM)` slots (RSC-2.2), and the
/// runner's slot-routed writeback publishes exactly these targets (plus the
/// runtime-dynamic keys it binds itself: port payload bindings and the
/// local-clock `__clock_time`) instead of the legacy whole-context diff.
pub(crate) fn collect_assignment_targets(ir: &StateMachineIR) -> Vec<String> {
    fn from_action(action: &TransitionActionIR, out: &mut Vec<String>) {
        let push_assignments = |assignments: &[crate::AssignmentIR], out: &mut Vec<String>| {
            for a in assignments {
                if !out.contains(&a.variable) {
                    out.push(a.variable.clone());
                }
            }
        };
        match action {
            TransitionActionIR::Structured { assignments, .. } => {
                push_assignments(assignments, out);
            }
            TransitionActionIR::Simple(s) => {
                if let TransitionActionIR::Structured { assignments, .. } =
                    action_parser::parse_action(s)
                {
                    push_assignments(&assignments, out);
                }
            }
        }
    }
    fn from_states(states: &[StateIR], out: &mut Vec<String>) {
        for s in states {
            for action in [&s.entry_action, &s.do_action, &s.exit_action]
                .into_iter()
                .flatten()
            {
                from_action(action, out);
            }
            if let Some(sub) = &s.sub_machine {
                walk(sub, out);
            }
        }
    }
    fn walk(ir: &StateMachineIR, out: &mut Vec<String>) {
        from_states(&ir.states, out);
        for t in &ir.transitions {
            if let Some(action) = &t.action {
                from_action(action, out);
            }
        }
        for r in &ir.regions {
            from_states(&r.states, out);
            for t in &r.transitions {
                if let Some(action) = &t.action {
                    from_action(action, out);
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(ir, &mut out);
    out
}

/// RSC-2.4b: whether every variable read in `expr` is a bound
/// [`ExprIR::SlotRef`] that `slot_ok(name, slot)` accepts (servable through
/// the read-only slot handle without the scoped-context view — the
/// predicate additionally pins SlotRefs to the subsystem's OWN namespace,
/// because the binder's global fall-through would silently read a top-level
/// default instead of the instance's value once the merged view is gone).
/// Deliberately conservative — chain heads (graph-navigated tails),
/// function calls (stdlib temporal/occurrence functions read the
/// trace/registries/context), lambda forms and constructor/meta access all
/// disqualify.
fn expr_fully_slot_bound(
    expr: &ExprIR,
    slot_ok: &dyn Fn(&str, crate::slots::SlotId) -> bool,
) -> bool {
    match expr {
        ExprIR::LiteralInt(_)
        | ExprIR::LiteralReal(_)
        | ExprIR::LiteralBool(_)
        | ExprIR::LiteralString(_)
        | ExprIR::LiteralNull => true,
        ExprIR::SlotRef { slot, name } => slot_ok(name, *slot),
        ExprIR::BinaryOp { left, right, .. } => {
            expr_fully_slot_bound(left, slot_ok) && expr_fully_slot_bound(right, slot_ok)
        }
        ExprIR::UnaryOp { operand, .. } => expr_fully_slot_bound(operand, slot_ok),
        ExprIR::Conditional {
            condition,
            then_expr,
            else_expr,
        } => {
            expr_fully_slot_bound(condition, slot_ok)
                && expr_fully_slot_bound(then_expr, slot_ok)
                && expr_fully_slot_bound(else_expr, slot_ok)
        }
        ExprIR::NullCoalescing { expr, default } => {
            expr_fully_slot_bound(expr, slot_ok) && expr_fully_slot_bound(default, slot_ok)
        }
        ExprIR::Index { sequence, index } => {
            expr_fully_slot_bound(sequence, slot_ok) && expr_fully_slot_bound(index, slot_ok)
        }
        ExprIR::Range { lower, upper } => {
            expr_fully_slot_bound(lower, slot_ok) && expr_fully_slot_bound(upper, slot_ok)
        }
        ExprIR::Sequence(items) => items
            .iter()
            .all(|item| expr_fully_slot_bound(item, slot_ok)),
        _ => false,
    }
}

/// RSC-2.4b: precomputed slot write-set of one state-machine executor —
/// one [`WriteRoute`](crate::slots::WriteRoute) per compiled assignment
/// target, plus the legacy prefix formatting for the name-keyed dynamic-key
/// fallback. Built by `Executor::prepare_slot_writeback`.
#[derive(Debug, Clone)]
struct SmWriteSet {
    /// `(bare target name, route)` for every compiled assignment target
    /// (collection order = [`collect_assignment_targets`] order).
    targets: Vec<(String, crate::slots::WriteRoute)>,
    /// `"{prefix}."` when the owning subsystem is instance-prefixed —
    /// dynamic keys are published under it (mirrors the legacy
    /// orchestrator string loop byte-for-byte).
    dot_prefix: Option<String>,
    /// `"{canonical_prefix}."` when it differs from the var prefix. RSC-3.5b
    /// (leftover-C): instance SM subsystems now carry the canonical tree-path
    /// prefix (`{container}.{instance}`) so their assignment-target routes
    /// resolve against the slot's canonical spelling instead of being refused
    /// on a canonical-mismatch and falling back to the name-keyed path.
    canonical_dot: Option<String>,
    /// RSC-3.5b: one [`WriteRoute`](crate::slots::WriteRoute) per port-payload
    /// key (`{port}.payload`, `{port}_payload`) the SM can bind at tick time.
    /// The receiving ports are compile-static (`accept_ports()`), so these
    /// keys are pre-minted as Discrete slots and routed by SlotId — draining
    /// them out of the `dynamic_keys` name-keyed fallback class.
    payload_routes: Vec<(String, crate::slots::WriteRoute)>,
}

/// Trigger kinds supported by the state machine runner.
///
/// Corresponds to SysML v2 trigger types: event triggers, time-based
/// `after(duration)` triggers, and condition-based `when(condition)` triggers.
#[derive(Debug, Clone)]
pub enum TriggerKind {
    /// Standard event trigger — matches an event by name.
    Event(String),
    /// Time-based trigger — fires after the given duration has elapsed in the current state.
    After(Duration),
    /// Expression-based after trigger — evaluates expression to get duration (in seconds).
    /// Used for `after(t_dead)` where the duration comes from a model parameter.
    /// The expression is evaluated when the trigger is checked; the result (seconds)
    /// is compared to `state_elapsed`.
    AfterExpr(ExprIR),
    /// Absolute time trigger — fires when the elapsed time reaches the given value.
    /// Aligned with the spec's `TriggerAt(timeInstant, receiver, clock)`.
    At(f64),
    /// Condition-based trigger — fires when the expression evaluates to true.
    /// With rising-edge detection: only fires on false→true transition (per spec's
    /// `TriggerWhen` which fires when "condition changes from false to true").
    ///
    /// SPEC-SILENT approximation: the spec's `TriggerWhen`/`ChangeSignal`
    /// (Triggers.kerml, Observation.kerml `ObserveChange`) fires at the *instant*
    /// the Boolean condition changes false→true — an event model, not a sampled
    /// one. This variant samples the condition at tick boundaries (rising-edge
    /// via `prev_when_values`). It is the best available approximation for
    /// conditions over discrete / non-continuous variables. When the condition
    /// is a threshold predicate over continuous ODE state or ODE-output signals,
    /// the compiler instead rewrites it to [`TriggerKind::WhenLocated`] so the
    /// crossing instant is located precisely (the spec-faithful behavior).
    When(ExprIR),
    /// Located change trigger — a `when` threshold predicate over continuous ODE
    /// state / output signals whose crossing instant is located by the
    /// orchestrator's zero-crossing detector (`ode_events::ZeroCrossingDetector`)
    /// and injected as the named discrete event. Fires when that event is
    /// delivered, exactly like [`TriggerKind::Event`].
    ///
    /// CONFORMS-REQUIRED: this implements the spec's `ChangeSignal` semantics for
    /// the continuous case — the trigger occurs at the false→true crossing
    /// instant, not at a tick sample. The compiler synthesizes this from a
    /// qualifying `accept when <comparator>` (see `wire_zero_crossing_detectors`);
    /// the `String` is the synthesized crossing-event name (`__zc::<sm>::<idx>`),
    /// retained for diagnostics and round-trip fidelity.
    WhenLocated(String),
    /// Port message trigger — fires when a message arrives at the named port.
    /// The payload is captured and can be bound to transition effect context.
    ///
    /// SPEC-SILENT approximation (L34 / RSC-3.5b): the spec keys this trigger on
    /// the receiver **Occurrence** bound by the `via` expression
    /// (TransitionPerformances.kerml:28-46) and accepts a transfer only when its
    /// payload conforms to the accepted typing (SysML-vocab.ttl:2495,
    /// TransitionPerformances.kerml:131 `acceptable`). This implementation uses a
    /// port-**name** string match (see `matches_with_extension`) and does NOT yet
    /// check payload-type conformance — `payload_type` is carried but unused by
    /// matching. Payload-type matching is the documented follow-up.
    PortMessage {
        /// Port name to monitor (e.g., "currentSensorHigh")
        port_name: String,
        /// Optional payload type constraint (carried; not yet matched — see above)
        payload_type: Option<String>,
        /// The accept parameter name (`accept <name> via <port>`), if declared.
        /// Bound at tick time to the delivered payload Value so a guard/effect
        /// can read it (`<name>` resolves to `{port}.payload`). Spec: the
        /// accepted transfer's payload binds the accept parameter
        /// (Transfers.kerml:254-266 `binding payload = acceptedTransfer.payload`).
        param_name: Option<String>,
    },
}

/// Extended state information stored alongside the base `StateIR`.
///
/// This holds per-state data that the `sysml-run` crate's `StateIR` does not
/// natively carry, such as do-actions (executed each step while in the state).
#[derive(Debug, Clone, Default)]
pub struct StateExtension {
    /// Action executed each step while the state is active.
    pub do_action: Option<TransitionActionIR>,
}

/// Extended transition information stored alongside the base `TransitionIR`.
///
/// Holds rich trigger information that goes beyond a plain event string.
#[derive(Debug, Clone)]
pub struct TransitionExtension {
    /// The parsed trigger kind, if this transition uses a non-event trigger.
    pub trigger: Option<TriggerKind>,
}

/// Compiler for state machines.
pub struct StateMachineCompiler;

impl StateMachineCompiler {
    /// Collect all elements that can act as state machine roots.
    ///
    /// Per SysML v2 spec, any named element that contains child `StateUsage`
    /// elements is a runnable state machine — this includes `StateDefinition`,
    /// `StateUsage` (with children), `PartDefinition` with exhibit states, etc.
    fn sorted_state_definitions(graph: &ModelGraph) -> Vec<&Element> {
        let mut defs: Vec<&Element> = graph
            .elements_by_kind(&ElementKind::StateDefinition)
            .collect();

        // Also include any other named element that contains child StateUsage
        // (StateUsage-as-machine, PartDefinition with inline states, etc.)
        for elem in graph.elements_by_kind(&ElementKind::StateUsage) {
            if elem.name.is_some() {
                let has_child_states = graph
                    .children_of(&elem.id)
                    .any(|c| matches!(c.kind, ElementKind::StateUsage));
                if has_child_states {
                    defs.push(elem);
                }
            }
        }
        for elem in graph.elements_by_kind(&ElementKind::PartDefinition) {
            if elem.name.is_some() {
                let has_child_states = graph
                    .children_of(&elem.id)
                    .any(|c| matches!(c.kind, ElementKind::StateUsage));
                if has_child_states {
                    defs.push(elem);
                }
            }
        }
        for elem in graph.elements_by_kind(&ElementKind::PartUsage) {
            if elem.name.is_some() {
                let has_child_states = graph
                    .children_of(&elem.id)
                    .any(|c| matches!(c.kind, ElementKind::StateUsage));
                if has_child_states {
                    defs.push(elem);
                }
            }
        }

        sort_elements_by_source_order(&mut defs);
        defs
    }

    fn compile_selected(
        graph: &ModelGraph,
        sm: &Element,
    ) -> Result<StateMachineIR, Vec<Diagnostic>> {
        let sm_name = sm.name.clone().unwrap_or_else(|| "StateMachine".to_owned());

        // Check if this should be compiled as a parallel state machine
        if let Some(regions) = Self::detect_parallel_regions(graph, sm) {
            let _region_count = regions.len();
            let result = Self::compile_parallel(graph, sm, sm_name, regions);
            #[cfg(feature = "tracing")]
            if let Ok(ref ir) = result {
                tracing::debug!(
                    regions = _region_count,
                    states = ir.states.len(),
                    transitions = ir.transitions.len(),
                    "compiled parallel state machine"
                );
            }
            result
        } else {
            let result = Self::compile_simple(graph, sm, sm_name);
            #[cfg(feature = "tracing")]
            if let Ok(ref ir) = result {
                tracing::debug!(
                    states = ir.states.len(),
                    transitions = ir.transitions.len(),
                    "compiled simple state machine"
                );
            }
            result
        }
    }

    /// Compile a specific state machine by name.
    pub fn compile_named(
        graph: &ModelGraph,
        sm_name: &str,
    ) -> Result<StateMachineIR, Vec<Diagnostic>> {
        let defs = Self::sorted_state_definitions(graph);
        let Some(sm) = defs
            .iter()
            .copied()
            .find(|e| e.name.as_deref() == Some(sm_name))
        else {
            let mut diag = Diagnostic::error(format!("state machine '{}' not found", sm_name))
                .with_code("SM007")
                .with_tier(DiagnosticTier::Semantic);
            let names: Vec<String> = defs.iter().filter_map(|e| e.name.clone()).collect();
            if !names.is_empty() {
                diag = diag.with_note(format!("available state machines: {}", names.join(", ")));
            }
            return Err(vec![diag]);
        };

        Self::compile_selected(graph, sm)
    }

    /// Return the names of all state machine definitions in the graph.
    ///
    /// Used by `build_workspace_orchestrator()` to discover all SMs.
    pub fn list_state_machine_names(graph: &ModelGraph) -> Vec<String> {
        Self::sorted_state_definitions(graph)
            .iter()
            .filter_map(|e| e.name.clone())
            .collect()
    }

    /// Compile ALL state machines found in the graph.
    ///
    /// Returns a vec of `(name, StateMachineIR)` pairs. Compilation errors
    /// for individual SMs are collected; only SMs that compile successfully
    /// are included.
    pub fn compile_all(
        graph: &ModelGraph,
    ) -> Vec<(String, Result<StateMachineIR, Vec<Diagnostic>>)> {
        let defs = Self::sorted_state_definitions(graph);
        defs.iter()
            .filter_map(|sm| {
                // Standard-library state/performance definitions are vocabulary,
                // not executable user subsystems. Over a library-overlaid
                // workspace graph they would otherwise be compiled as spurious
                // orchestrator subsystems (kernel `StateAction`/performance
                // features). The eval-context seed already filters library
                // elements the same way (`is_library_element`).
                if graph.is_library_element(&sm.id) {
                    return None;
                }
                let name = sm.name.clone()?;
                let result = Self::compile_selected(graph, sm);
                Some((name, result))
            })
            .collect()
    }

    /// Compile a simple (non-parallel) state machine.
    fn compile_simple(
        graph: &ModelGraph,
        sm: &Element,
        sm_name: String,
    ) -> Result<StateMachineIR, Vec<Diagnostic>> {
        let mut diagnostics = Vec::new();

        // Find all states that are children of this state machine
        let mut states: Vec<_> = graph
            .children_of(&sm.id)
            .filter(|e| matches!(e.kind, ElementKind::StateUsage))
            .collect();
        sort_elements_by_source_order(&mut states);

        if states.is_empty() {
            diagnostics.push(Diagnostic::error("State machine has no states"));
            return Err(diagnostics);
        }

        // Find initial state (first state or one marked as initial)
        let initial_state = states
            .iter()
            .find(|s| {
                s.get_prop("initial")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
            .or_else(|| states.first())
            .expect("invariant: states verified non-empty above");

        let initial_name = initial_state
            .name
            .clone()
            .unwrap_or_else(|| "initial".to_owned());

        // Build the IR
        let mut ir = StateMachineIR::new(sm_name, initial_name);

        // Add states
        for state in &states {
            let state_ir = Self::compile_state(state, graph);
            ir = ir.with_state(state_ir);
        }

        // Find transitions. Prefer explicit TransitionUsage children owned by the
        // state machine, because multiple distinct triggers may share the same
        // source/target pair. The elaborated relationship graph can collapse those
        // into a single Transition relationship, which would otherwise lose
        // comparator vs safety transitions.
        let state_ids: HashSet<_> = states.iter().map(|s| s.id.clone()).collect();
        let mut owned_transition_pairs: HashSet<(String, String)> = HashSet::new();
        for child in graph.children_of(&sm.id) {
            if child.kind == ElementKind::TransitionUsage {
                if let Some(transition) = Self::compile_transition_usage(graph, child, &state_ids) {
                    owned_transition_pairs.insert((transition.from.clone(), transition.to.clone()));
                    ir = ir.with_transition(transition);
                }
            }
        }

        for rel in graph.relationships_by_kind(&RelationshipKind::Transition) {
            if !state_ids.contains(&rel.source) || !state_ids.contains(&rel.target) {
                continue;
            }

            let source = graph.get_element(&rel.source);
            let target = graph.get_element(&rel.target);

            if let (Some(src), Some(tgt)) = (source, target) {
                let from = src.name.clone().unwrap_or_else(|| src.id.to_string());
                let to = tgt.name.clone().unwrap_or_else(|| tgt.id.to_string());

                if owned_transition_pairs.contains(&(from.clone(), to.clone())) {
                    continue;
                }

                let mut transition = TransitionIR::new(from.clone(), to.clone());

                let has_trigger = rel
                    .props
                    .get("has_trigger")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if let Some(event) = rel.props.get("event").and_then(|v| v.as_str()) {
                    transition = transition.with_event(event);
                }

                if let Some(guard) = rel.props.get("guard").and_then(|v| v.as_str()) {
                    transition = transition.with_guard(guard);
                }

                // Handle "trigger" prop — parser stores `accept when <expr>` as trigger
                // on the TransitionUsage element (not the Transition relationship).
                // A "when <expr>" trigger is a condition-based trigger (SysML v2 TriggerWhen):
                // it fires automatically when the expression becomes true, without an event.
                // Extract the expression as a guard and mark guard-only.
                //
                // Check both the relationship props and the TransitionUsage element.
                // The parser stores `accept when <expr>` as "trigger" on the TransitionUsage
                // element, not on the Transition relationship. Find the TransitionUsage by
                // matching source/target props against the relationship endpoints.
                let trigger_str = rel
                    .props
                    .get("trigger")
                    .and_then(|v| v.as_str().map(|s| s.to_owned()))
                    .or_else(|| {
                        graph
                            .elements_by_kind(&sysml_core::ElementKind::TransitionUsage)
                            .find(|e| {
                                let src_match =
                                    e.get_prop("source").and_then(|v| v.as_str()) == Some(&from);
                                let tgt_match =
                                    e.get_prop("target").and_then(|v| v.as_str()) == Some(&to);
                                src_match && tgt_match
                            })
                            .and_then(|e| graph.transition_feature_text(&e.id, "trigger"))
                    });
                if let Some(trigger) = trigger_str.as_deref() {
                    if let Some(expr) = trigger.strip_prefix("when ") {
                        if transition.guard.is_none() {
                            transition = transition.with_guard(expr);
                        }
                        transition = transition.guard_only();
                    } else if transition.event.is_none() {
                        // Non-when trigger: treat as an event name
                        transition = transition.with_event(trigger);
                    }
                }
                // Accept-parameter name (`accept <name> via <port>`), same
                // source as `trigger`: relationship props, else the matching
                // TransitionUsage's trigger-action payload ReferenceUsage.
                let accept_param = rel
                    .props
                    .get("accept_param")
                    .and_then(|v| v.as_str().map(|s| s.to_owned()))
                    .or_else(|| {
                        graph
                            .elements_by_kind(&sysml_core::ElementKind::TransitionUsage)
                            .find(|e| {
                                e.get_prop("source").and_then(|v| v.as_str()) == Some(&from)
                                    && e.get_prop("target").and_then(|v| v.as_str()) == Some(&to)
                            })
                            .and_then(|e| graph.transition_accept_param(&e.id))
                    });
                if let Some(param) = accept_param {
                    transition = transition.with_accept_param(param);
                }

                if let Some(action) = rel.props.get("action").and_then(|v| v.as_str()) {
                    transition = transition.with_action(parse_action(action));
                }

                // A transition with a guard but no real trigger is guard-only
                if transition.guard.is_some() && !has_trigger {
                    transition = transition.guard_only();
                }

                ir = ir.with_transition(transition);
            }
        }

        // Post-pass: mark completion transitions.
        // A null-event transition whose source state has a do-action is a completion
        // transition — it should only fire after the do-activity completes.
        for transition in &mut ir.transitions {
            if transition.event.is_none() {
                let source_has_do = ir
                    .states
                    .iter()
                    .find(|s| s.name == transition.from)
                    .map(|s| s.do_action.is_some())
                    .unwrap_or(false);
                if source_has_do {
                    transition.is_completion = true;
                }
            }
        }

        Ok(ir)
    }

    /// Compile a parallel state machine with multiple concurrent regions.
    fn compile_parallel(
        graph: &ModelGraph,
        _sm: &Element,
        sm_name: String,
        region_elements: Vec<&Element>,
    ) -> Result<StateMachineIR, Vec<Diagnostic>> {
        let mut ir = StateMachineIR::parallel(sm_name);

        for region_elem in region_elements {
            let region_name = region_elem
                .name
                .clone()
                .unwrap_or_else(|| region_elem.id.to_string());

            // Find states within this region
            let mut states: Vec<_> = graph
                .children_of(&region_elem.id)
                .filter(|e| matches!(e.kind, ElementKind::StateUsage))
                .collect();
            sort_elements_by_source_order(&mut states);

            if states.is_empty() {
                continue; // Skip regions with no states
            }

            // Find initial state for this region
            let initial_state = states
                .iter()
                .find(|s| {
                    s.get_prop("initial")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false)
                })
                .or_else(|| states.first())
                .unwrap();

            let initial_name = initial_state
                .name
                .clone()
                .unwrap_or_else(|| "initial".to_owned());

            let mut region = RegionIR::new(&region_name, initial_name);

            // Add states to region
            for state in &states {
                let state_ir = Self::compile_state(state, graph);
                region = region.with_state(state_ir);
            }

            // Find transitions within this region
            // Collect state IDs for this region
            let region_state_ids: std::collections::HashSet<_> =
                states.iter().map(|s| &s.id).collect();

            for rel in graph.relationships_by_kind(&RelationshipKind::Transition) {
                // Only include transitions where source is in this region
                if !region_state_ids.contains(&rel.source) {
                    continue;
                }

                let source = graph.get_element(&rel.source);
                let target = graph.get_element(&rel.target);

                if let (Some(src), Some(tgt)) = (source, target) {
                    let from = src.name.clone().unwrap_or_else(|| src.id.to_string());
                    let to = tgt.name.clone().unwrap_or_else(|| tgt.id.to_string());

                    let mut transition = TransitionIR::new(from, to);

                    let has_trigger = rel
                        .props
                        .get("has_trigger")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    if let Some(event) = rel.props.get("event").and_then(|v| v.as_str()) {
                        transition = transition.with_event(event);
                    }
                    if let Some(param) = rel.props.get("accept_param").and_then(|v| v.as_str()) {
                        transition = transition.with_accept_param(param);
                    }

                    if let Some(guard) = rel.props.get("guard").and_then(|v| v.as_str()) {
                        transition = transition.with_guard(guard);
                    }

                    if let Some(action) = rel.props.get("action").and_then(|v| v.as_str()) {
                        transition = transition.with_action(parse_action(action));
                    }

                    if transition.guard.is_some() && !has_trigger {
                        transition = transition.guard_only();
                    }

                    region = region.with_transition(transition);
                }
            }

            // Post-pass: mark completion transitions in this region.
            for transition in &mut region.transitions {
                if transition.event.is_none() {
                    let source_has_do = region
                        .states
                        .iter()
                        .find(|s| s.name == transition.from)
                        .map(|s| s.do_action.is_some())
                        .unwrap_or(false);
                    if source_has_do {
                        transition.is_completion = true;
                    }
                }
            }

            ir = ir.with_region(region);
        }

        Ok(ir)
    }

    /// Compile a single state element into StateIR.
    ///
    /// Entry/do/exit actions are sourced from:
    /// 1. The state's `entry`/`exit`/`do_action` string property (from elaboration)
    /// 2. Child ActionUsage elements with `stateSubactionKind` (from parser)
    ///    — their child AssignmentActionUsage elements are compiled to structured actions
    fn compile_state(state: &Element, graph: &ModelGraph) -> StateIR {
        let name = state.name.clone().unwrap_or_else(|| state.id.to_string());
        let is_final = state
            .get_prop("final")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut state_ir = StateIR::new(&name);

        // Entry/do/exit subactions are all sourced the same way (SysML §7.18.1):
        // prefer the elaboration string property when it carries a real body,
        // otherwise walk the tagged child ActionUsage elements (the structured
        // body the parser emits per `process_state_subaction`). All three share
        // one path so exit/do bodies execute exactly like entry — previously
        // only entry had the children-fallback, so exit/do bodies were dropped
        // to `Simple("")` (GAP-SM-EXEC).
        if let Some(action) = Self::compile_state_subaction(state, graph, "entry", "entry") {
            state_ir = state_ir.with_entry_action(action);
        }
        if let Some(action) = Self::compile_state_subaction(state, graph, "exit", "exit") {
            state_ir = state_ir.with_exit_action(action);
        }
        if let Some(action) = Self::compile_state_subaction(state, graph, "do_action", "do") {
            state_ir = state_ir.with_do_action(action);
        }

        if is_final {
            state_ir = state_ir.final_state();
        }

        // Check if this state has child states → composite state
        let child_state_count = graph
            .children_of(&state.id)
            .filter(|c| c.kind == ElementKind::StateUsage)
            .count();

        if child_state_count > 0 {
            // Recursively compile child states into a sub-machine
            match Self::compile_selected(graph, state) {
                Ok(sub_ir) => {
                    state_ir = state_ir.with_sub_machine(sub_ir);
                }
                Err(_diags) => {
                    // Sub-machine compilation errors are non-fatal — opt in to
                    // stderr output via SYSML_TRACE_SM for a quieter default.
                    #[cfg(debug_assertions)]
                    #[allow(clippy::print_stderr)]
                    if std::env::var_os("SYSML_TRACE_SM").is_some() {
                        for d in &_diags {
                            eprintln!("[SM] sub-machine compilation warning: {}", d.message);
                        }
                    }
                }
            }
        }

        state_ir
    }

    /// Compile one of a state's entry/do/exit subactions (SysML §7.18.1).
    ///
    /// `string_prop` is the elaboration string key (`entry`/`exit`/`do_action`);
    /// `kind_name` is the `stateSubactionKind` tag on the parser's child
    /// ActionUsage (`entry`/`exit`/`do`). The string property is used only when
    /// it carries a real body (an assignment or send); otherwise — including the
    /// common case where elaboration leaves just an action name or an empty
    /// marker — we walk the tagged children to recover the structured body. This
    /// is the path that makes exit/do action bodies execute (not just entry).
    fn compile_state_subaction(
        state: &Element,
        graph: &ModelGraph,
        string_prop: &str,
        kind_name: &str,
    ) -> Option<crate::TransitionActionIR> {
        if let Some(s) = state.get_prop(string_prop).and_then(|v| v.as_str()) {
            let action = parse_action(s);
            if !action.is_simple() || s.contains('=') || s.contains("send(") {
                return Some(action);
            }
            // String is just an action name / empty marker — prefer the
            // structured body from the tagged children, else keep the parsed string.
            return Self::compile_action_from_children(&state.id, kind_name, graph)
                .or(Some(action));
        }
        Self::compile_action_from_children(&state.id, kind_name, graph)
    }

    /// Walk child ActionUsage elements of a state to build a structured action.
    /// Looks for ActionUsage children with `stateSubactionKind` matching `kind_name`,
    /// then walks their child AssignmentActionUsage elements to build assignments.
    fn compile_action_from_children(
        state_id: &sysml_core::ElementId,
        kind_name: &str,
        graph: &ModelGraph,
    ) -> Option<crate::TransitionActionIR> {
        // Find the ActionUsage child tagged with the right subaction kind
        let action_elem = graph.children_of(state_id).find(|e| {
            e.kind == sysml_core::ElementKind::ActionUsage
                && e.get_prop("stateSubactionKind")
                    .and_then(|v| v.as_str())
                    .map(|k| k == kind_name)
                    .unwrap_or(false)
        })?;

        // Walk child elements looking for assignments and sends
        let mut assignments = Vec::new();
        let mut sends = Vec::new();
        let mut port_send_ops: Vec<(String, crate::expressions::ExprIR)> = Vec::new();

        // `children_of` iterates an unordered set (owner_to_children is a
        // HashSet), so the child action statements come back in hash order, not
        // source order. An action body executes its statements in sequence
        // (SysML §7.18.1 / §8.4.13), so a body like `aExit = seq; seq = seq + 1`
        // must compile in that order — otherwise the read sees the post-increment
        // value. Sort the statement children by source span before lowering.
        let mut ordered_children: Vec<&Element> = graph.children_of(&action_elem.id).collect();
        ordered_children.sort_by_key(|c| c.spans.first().map_or(usize::MAX, |s| s.start));

        for child in ordered_children {
            match child.kind {
                sysml_core::ElementKind::AssignmentActionUsage => {
                    // Extract variable name and value from assignment
                    if let Some(var_name) = child
                        .name
                        .as_deref()
                        .or_else(|| child.get_prop("target").and_then(|v| v.as_str()))
                    {
                        let assignment = if let Some(value) = child.get_prop("value") {
                            crate::AssignmentIR::set(var_name, value.clone())
                        } else if let Ok(expr) =
                            crate::expressions::compile_expression(child, graph)
                        {
                            // AST-first: walk the assignment's expression subtree.
                            let source =
                                sysml_core::expression_pretty::pretty_print_owner(child, graph)
                                    .unwrap_or_else(|| format!("{:?}", expr));
                            crate::AssignmentIR::from_expr(
                                var_name,
                                crate::AssignmentOp::Set,
                                &source,
                                expr,
                            )
                        } else {
                            crate::AssignmentIR::set(var_name, sysml_core::Value::Null)
                        };

                        assignments.push(assignment);
                    }
                }
                sysml_core::ElementKind::SendActionUsage => {
                    // SM-send: `send <payload> via <port>`. The parser stamps
                    // `via_port` and projects the payload as the send's
                    // payloadArgument subtree. We keep the `send via <port>`
                    // string in `sends` purely as the trace/snapshot surface,
                    // and carry the real payload through `port_send_ops`:
                    // compile the payload expression now, evaluate it at tick
                    // time, and route the resulting Value as the addressed
                    // MessageTransfer payload (A-scalar: the string no longer
                    // routes — the Value channel does). A `via`-send with no
                    // compilable payload still routes (LiteralNull) so the bare
                    // `send via p` message still occurs.
                    if let Some(port) = child
                        .get_prop("via_port")
                        .and_then(|v| v.as_str())
                        .map(str::trim)
                        .filter(|p| !p.is_empty())
                    {
                        sends.push(format!("send via {}", port));
                        let payload = crate::expressions::compile_expression(child, graph)
                            .unwrap_or(crate::expressions::ExprIR::LiteralNull);
                        port_send_ops.push((port.to_owned(), payload));
                    } else if let Some(event_name) = child.name.as_deref() {
                        sends.push(event_name.to_owned());
                    }
                }
                _ => {}
            }
        }

        if assignments.is_empty() && sends.is_empty() {
            None
        } else {
            Some(crate::TransitionActionIR::structured_with_ports(
                assignments,
                sends,
                port_send_ops,
            ))
        }
    }

    /// Check if a state machine should be compiled as parallel.
    /// Returns the region elements if parallel, None otherwise.
    fn detect_parallel_regions<'a>(
        graph: &'a ModelGraph,
        sm: &'a Element,
    ) -> Option<Vec<&'a Element>> {
        // Check isParallel property
        let is_parallel = sm
            .get_prop("isParallel")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_parallel {
            // Top-level StateUsage children are regions
            let regions: Vec<_> = graph
                .children_of(&sm.id)
                .filter(|e| matches!(e.kind, ElementKind::StateUsage))
                .collect();
            if !regions.is_empty() {
                return Some(regions);
            }
        }

        // Check for multiple top-level states that each have their own substates
        // This indicates a parallel structure
        let top_level_states: Vec<_> = graph
            .children_of(&sm.id)
            .filter(|e| matches!(e.kind, ElementKind::StateUsage))
            .collect();

        // If multiple top-level states each have child states, treat as parallel
        let regions_with_substates: Vec<_> = top_level_states
            .iter()
            .filter(|s| {
                graph
                    .children_of(&s.id)
                    .any(|c| matches!(c.kind, ElementKind::StateUsage))
            })
            .copied()
            .collect();

        if regions_with_substates.len() > 1 {
            return Some(regions_with_substates);
        }

        None
    }

    /// Compile a composite state machine from a part that contains
    /// sub-parts exhibiting state machines.
    ///
    /// This traverses the part hierarchy to find all `exhibit state` declarations,
    /// extracts the state definitions they reference, and builds a parallel
    /// state machine with one region per exhibited state machine.
    ///
    /// # Arguments
    ///
    /// * `graph` - The model graph containing the parsed SysML model
    /// * `part_id` - The ID of the root part (e.g., the installation)
    ///
    /// # Returns
    ///
    /// A `StateMachineIR` with one region per exhibited state machine, or
    /// diagnostics if compilation fails.
    pub fn compile_from_part(
        graph: &ModelGraph,
        part_id: &ElementId,
    ) -> Result<StateMachineIR, Vec<Diagnostic>> {
        let mut diagnostics = Vec::new();

        let Some(part) = graph.get_element(part_id) else {
            diagnostics.push(Diagnostic::error("Part not found"));
            return Err(diagnostics);
        };

        let part_name = part
            .name
            .clone()
            .unwrap_or_else(|| "CompositeStateMachine".to_owned());

        // Find all descendants with exhibit state declarations
        let mut exhibit_states = Vec::new();
        let mut visited = HashSet::new();
        Self::collect_exhibit_states(graph, part_id, "", &mut exhibit_states, &mut visited);

        if exhibit_states.is_empty() {
            diagnostics.push(Diagnostic::error(
                "No exhibit state declarations found in part hierarchy",
            ));
            return Err(diagnostics);
        }

        // Build parallel state machine
        let mut ir = StateMachineIR::parallel(part_name);

        for (region_name, exhibit_id) in exhibit_states {
            // Find the type of this exhibit state (the state definition it references)
            if let Some(state_def_id) = Self::find_exhibit_state_type(graph, &exhibit_id) {
                if let Some(region) = Self::state_def_to_region(graph, &state_def_id, &region_name)
                {
                    ir = ir.with_region(region);
                }
            }
        }

        if ir.regions.is_empty() {
            diagnostics.push(Diagnostic::error("No valid state machines found"));
            return Err(diagnostics);
        }

        Ok(ir)
    }

    /// Recursively collect all exhibit state usages in the part tree.
    ///
    /// Builds a list of (region_name, exhibit_id) pairs, where region_name
    /// is a simplified name based on the containing part.
    fn collect_exhibit_states(
        graph: &ModelGraph,
        element_id: &ElementId,
        path_prefix: &str,
        result: &mut Vec<(String, ElementId)>,
        visited: &mut HashSet<ElementId>,
    ) {
        if !visited.insert(element_id.clone()) {
            return;
        }

        // Get the element's name for path building
        let element_name = graph
            .get_element(element_id)
            .and_then(|e| e.name.clone())
            .unwrap_or_default();

        // Check all children
        for child in graph.children_of(element_id) {
            match child.kind {
                ElementKind::ExhibitStateUsage => {
                    // Found an exhibit state - use the element name or the exhibit's name
                    let region_name = if let Some(name) = &child.name {
                        // If exhibit has a name, use it
                        name.clone()
                    } else if !element_name.is_empty() {
                        // Use the containing part's name
                        element_name.clone()
                    } else {
                        // Fallback to path-based name
                        format!("{}region_{}", path_prefix, result.len())
                    };
                    result.push((region_name, child.id.clone()));
                }
                ElementKind::PartUsage | ElementKind::PartDefinition => {
                    // Recurse into parts
                    let new_prefix = if path_prefix.is_empty() {
                        element_name.clone()
                    } else if !element_name.is_empty() {
                        format!("{}.{}", path_prefix, element_name)
                    } else {
                        path_prefix.to_owned()
                    };
                    Self::collect_exhibit_states(graph, &child.id, &new_prefix, result, visited);
                }
                _ => {
                    // Recurse into other structural elements
                    Self::collect_exhibit_states(graph, &child.id, path_prefix, result, visited);
                }
            }
        }
    }

    /// Find the state definition type for an exhibit state usage.
    ///
    /// Looks for FeatureTyping children with a resolved `type` property
    /// or an `unresolved_type` property that we can try to resolve.
    fn find_exhibit_state_type(graph: &ModelGraph, exhibit_id: &ElementId) -> Option<ElementId> {
        // Look for FeatureTyping children
        for child in graph.children_of(exhibit_id) {
            if child.kind == ElementKind::FeatureTyping
                || child.kind.is_subtype_of(ElementKind::FeatureTyping)
            {
                // Check for resolved type
                if let Some(type_ref) = child.props.get("type") {
                    if let Some(type_id) = type_ref.as_ref() {
                        return Some(type_id.clone());
                    }
                }

                // Check for unresolved type and try to find it
                if let Some(unresolved) = child.props.get("unresolved_type") {
                    if let Some(type_name) = unresolved.as_str() {
                        // Try to find the state definition by name
                        if let Some(state_def) =
                            Self::find_state_definition_by_name(graph, type_name)
                        {
                            return Some(state_def);
                        }
                    }
                }
            }
        }

        None
    }

    /// Find a state definition by name (potentially qualified).
    fn find_state_definition_by_name(graph: &ModelGraph, name: &str) -> Option<ElementId> {
        // Extract the simple name (last part of qualified name)
        let simple_name = name.rsplit("::").next().unwrap_or(name);

        // Search for StateDefinition elements with this name
        for element in graph.elements_by_kind(&ElementKind::StateDefinition) {
            if let Some(elem_name) = &element.name {
                if elem_name == simple_name || elem_name == name {
                    return Some(element.id.clone());
                }
            }
        }

        None
    }

    /// Convert a state definition to a RegionIR.
    fn state_def_to_region(
        graph: &ModelGraph,
        state_def_id: &ElementId,
        region_name: &str,
    ) -> Option<RegionIR> {
        let _state_def = graph.get_element(state_def_id)?;

        // Find all states within this state definition
        let mut states: Vec<_> = graph
            .children_of(state_def_id)
            .filter(|e| matches!(e.kind, ElementKind::StateUsage))
            .collect();
        sort_elements_by_source_order(&mut states);

        if states.is_empty() {
            return None;
        }

        // Find initial state (first state or one marked initial)
        let initial_state = states
            .iter()
            .find(|s| {
                s.get_prop("initial")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false)
            })
            .or_else(|| states.first())
            .expect("invariant: states verified non-empty above");

        let initial_name = initial_state
            .name
            .clone()
            .unwrap_or_else(|| "initial".to_owned());

        let mut region = RegionIR::new(region_name, &initial_name);

        // Add states
        for state in &states {
            let state_ir = Self::compile_state(state, graph);
            region = region.with_state(state_ir);
        }

        // Build a map of state names to IDs for transition lookup
        let state_ids: HashSet<_> = states.iter().map(|s| s.id.clone()).collect();

        // Find transitions within this state definition
        // Look for TransitionUsage elements owned by the state definition
        for child in graph.children_of(state_def_id) {
            if child.kind == ElementKind::TransitionUsage {
                if let Some(transition) = Self::compile_transition_usage(graph, child, &state_ids) {
                    region = region.with_transition(transition);
                }
            }
        }

        // Also check for transitions as relationships
        for rel in graph.relationships_by_kind(&RelationshipKind::Transition) {
            if state_ids.contains(&rel.source) {
                let source = graph.get_element(&rel.source);
                let target = graph.get_element(&rel.target);

                if let (Some(src), Some(tgt)) = (source, target) {
                    let from = src.name.clone().unwrap_or_else(|| src.id.to_string());
                    let to = tgt.name.clone().unwrap_or_else(|| tgt.id.to_string());

                    let mut transition = TransitionIR::new(from, to);

                    let has_trigger = rel
                        .props
                        .get("has_trigger")
                        .and_then(|v| v.as_bool())
                        .unwrap_or(false);

                    if let Some(event) = rel.props.get("event").and_then(|v| v.as_str()) {
                        transition = transition.with_event(event);
                    }
                    if let Some(param) = rel.props.get("accept_param").and_then(|v| v.as_str()) {
                        transition = transition.with_accept_param(param);
                    }

                    if let Some(guard) = rel.props.get("guard").and_then(|v| v.as_str()) {
                        transition = transition.with_guard(guard);
                    }

                    if let Some(action) = rel.props.get("action").and_then(|v| v.as_str()) {
                        transition = transition.with_action(parse_action(action));
                    }

                    if transition.guard.is_some() && !has_trigger {
                        transition = transition.guard_only();
                    }

                    region = region.with_transition(transition);
                }
            }
        }

        // Post-pass: mark completion transitions in this region.
        for transition in &mut region.transitions {
            if transition.event.is_none() {
                let source_has_do = region
                    .states
                    .iter()
                    .find(|s| s.name == transition.from)
                    .map(|s| s.do_action.is_some())
                    .unwrap_or(false);
                if source_has_do {
                    transition.is_completion = true;
                }
            }
        }

        Some(region)
    }

    /// Compile a TransitionUsage element to TransitionIR.
    fn compile_transition_usage(
        graph: &ModelGraph,
        transition: &Element,
        _state_ids: &HashSet<ElementId>,
    ) -> Option<TransitionIR> {
        // Get source and target from properties
        let source_name = transition.props.get("source").and_then(|v| {
            // Could be a Ref or a String
            v.as_ref()
                .and_then(|id| graph.get_element(id))
                .and_then(|e| e.name.clone())
                .or_else(|| v.as_str().map(String::from))
        });

        let target_name = transition.props.get("target").and_then(|v| {
            v.as_ref()
                .and_then(|id| graph.get_element(id))
                .and_then(|e| e.name.clone())
                .or_else(|| v.as_str().map(String::from))
        });

        // Try unresolved properties
        let source_name = source_name.or_else(|| {
            transition
                .props
                .get("unresolved_source")
                .and_then(|v| v.as_str().map(String::from))
        });

        let target_name = target_name.or_else(|| {
            transition
                .props
                .get("unresolved_target")
                .and_then(|v| v.as_str().map(String::from))
        });

        let (Some(from), Some(to)) = (source_name, target_name) else {
            // Can't determine source/target without explicit properties
            // Future: could try parsing from transition name patterns
            return None;
        };

        let mut ir = TransitionIR::new(from, to);

        // Extract event from the trigger child (AcceptActionUsage wrapped in
        // TransitionFeatureMembership kind=trigger; its `text` prop carries the
        // canonical trigger string, SysML v2 §8.3.18.8).
        let trigger_text = graph.transition_feature_text(&transition.id, "trigger");
        let has_trigger = trigger_text.is_some();
        if let Some(trigger) = trigger_text {
            ir = ir.with_event(trigger);
        }
        // Carry the accept-parameter name (`accept <name> via <port>`) — the
        // canonical trigger string can't hold it; it is the trigger action's
        // payload ReferenceUsage child, bound to the payload at tick.
        if let Some(param) = graph.transition_accept_param(&transition.id) {
            ir = ir.with_accept_param(param);
        }

        // Extract guard (owned Expression child, kind=guard; `text` prop)
        if let Some(guard) = graph.transition_feature_text(&transition.id, "guard") {
            ir = ir.with_guard(guard);
        }

        // Extract the transition effect (SysML §7.18.3). The effect child's
        // `text` prop holds the raw CST text of the effect clause, e.g.
        // `do action { eff = 1; }` — the parser mints the ActionUsage child but
        // does not lower the effect body into further child elements. Unwrap an
        // optional `{ … }` body so the inner statement list parses into a
        // Structured action whose assignments execute; a wrapper-less
        // named-action effect stays Simple. Previously the wrapper made
        // `parse_action` drop the body to `Simple("")` (GAP-SM-EXEC).
        if let Some(effect) = graph.transition_feature_text(&transition.id, "effect") {
            ir = ir.with_action(parse_action(unwrap_action_body(&effect)));
        }

        // Guard-only: has guard but no real trigger
        if ir.guard.is_some() && !has_trigger {
            ir = ir.guard_only();
        }

        Some(ir)
    }
}

impl CompileToIR<StateMachineIR> for StateMachineCompiler {
    fn compile(graph: &ModelGraph) -> Result<StateMachineIR, Vec<Diagnostic>> {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            element_count = graph.element_count(),
            relationship_count = graph.relationship_count(),
            "compiling state machine"
        );

        // Find the first state machine element in deterministic source order.
        let defs = StateMachineCompiler::sorted_state_definitions(graph);
        let Some(sm) = defs.first().copied() else {
            return Err(vec![Diagnostic::error("No state machine found in model")]);
        };
        Self::compile_selected(graph, sm)
    }
}

/// Strip an optional `{ … }` body wrapper (and any leading `do`/`action`
/// keywords) from a transition effect's raw CST text, returning the inner
/// statement list. `do action { eff = 1; }` → `eff = 1;`. A string with no
/// braces is returned trimmed unchanged (a bare named-action effect stays a
/// `Simple` action). Uses the outermost brace pair.
fn unwrap_action_body(s: &str) -> &str {
    match (s.find('{'), s.rfind('}')) {
        (Some(open), Some(close)) if close > open => s[open + 1..close].trim(),
        _ => s.trim(),
    }
}

/// Execute a structured action: apply assignments to the eval context and collect sends.
/// Returns the list of send events generated.
/// Execute a transition action against `ctx`, returning the string send-trace
/// (`sends`) and appending any port-addressed payload sends — `(port, value)`
/// from the action's `port_send_ops`, evaluated against the live `ctx` — to
/// `port_sends`. The string trace is the snapshot surface; `port_sends` carries
/// the real payload Value the string cannot.
fn execute_action(
    action: &TransitionActionIR,
    ctx: &mut crate::expressions::EvalContext,
    port_sends: &mut Vec<(String, sysml_core::Value)>,
) -> Vec<String> {
    match action {
        TransitionActionIR::Simple(_name) => Vec::new(),
        TransitionActionIR::Structured {
            assignments,
            sends,
            port_send_ops,
        } => {
            let evaluator = ExpressionEvaluator::new();
            for assign in assignments {
                let evaluated = assign
                    .value_expr
                    .as_ref()
                    .and_then(|expr| evaluator.eval(expr, ctx).ok());
                let new_value = match assign.operator {
                    crate::AssignmentOp::Set => evaluated.unwrap_or_else(|| assign.value.clone()),
                    crate::AssignmentOp::Add | crate::AssignmentOp::Subtract => {
                        // Arithmetic operators: extract f64 from both current and assigned value
                        let current = ctx
                            .get(&assign.variable)
                            .and_then(|v| match v {
                                sysml_core::Value::Float(f) => Some(*f),
                                sysml_core::Value::Int(i) => Some(*i as f64),
                                _ => None,
                            })
                            .unwrap_or(0.0);

                        let operand = match evaluated.as_ref().unwrap_or(&assign.value) {
                            sysml_core::Value::Float(f) => *f,
                            sysml_core::Value::Int(i) => *i as f64,
                            _ => assign.as_f64().unwrap_or(0.0),
                        };
                        let result = if assign.operator == crate::AssignmentOp::Add {
                            current + operand
                        } else {
                            current - operand
                        };
                        sysml_core::Value::Float(result)
                    }
                };

                ctx.set(assign.variable.clone(), new_value);
            }
            // Evaluate port-addressed payload sends against the live context.
            // A payload that fails to evaluate routes as Null rather than
            // dropping the send (the message still occurs; only its payload is
            // empty) — symmetric with the receive side carrying-but-not-yet-
            // typing the payload.
            for (port, expr) in port_send_ops {
                let value = evaluator.eval(expr, ctx).unwrap_or(sysml_core::Value::Null);
                port_sends.push((port.clone(), value));
            }
            sends.clone()
        }
    }
}

/// Format an action for output.
fn format_action(action: &TransitionActionIR) -> String {
    match action {
        TransitionActionIR::Simple(s) => s.clone(),
        TransitionActionIR::Structured {
            assignments, sends, ..
        } => {
            let mut parts = Vec::new();
            for assign in assignments {
                let op = match assign.operator {
                    crate::AssignmentOp::Set => "=",
                    crate::AssignmentOp::Add => "+=",
                    crate::AssignmentOp::Subtract => "-=",
                };
                let rhs = assign
                    .value_source
                    .as_deref()
                    .map(str::to_owned)
                    .unwrap_or_else(|| format!("{}", assign.value));
                parts.push(format!("{} {} {}", assign.variable, op, rhs));
            }
            for send in sends {
                parts.push(format!("send('{}')", send));
            }
            parts.join("; ")
        }
    }
}

/// Recursively search a `StateMachineIR` tree for a state with the given name.
/// Returns `true` if the state exists anywhere in the tree (direct or nested).
fn find_state_in_tree(sm: &StateMachineIR, name: &str) -> bool {
    for state in &sm.states {
        if state.name == name {
            return true;
        }
        if let Some(sub) = &state.sub_machine {
            if find_state_in_tree(sub, name) {
                return true;
            }
        }
    }
    false
}

/// Find the direct parent of a state anywhere in the `StateMachineIR` tree.
/// A state P is the direct parent of state S if P's `sub_machine` contains S
/// in its immediate `states` list (not deeper).
/// Returns `None` if the state is a top-level state of `sm` or not found.
fn find_direct_parent_in_tree(sm: &StateMachineIR, target: &str) -> Option<String> {
    for state in &sm.states {
        if let Some(sub) = &state.sub_machine {
            // Check if target is a direct child of this state's sub-machine
            if sub.states.iter().any(|s| s.name == target) {
                return Some(state.name.clone());
            }
            // Recurse into deeper levels
            if let Some(parent) = find_direct_parent_in_tree(sub, target) {
                return Some(parent);
            }
        }
    }
    None
}

/// Find a state's entry or exit action anywhere in the `StateMachineIR` tree.
/// If `entry` is true, returns the entry action; otherwise returns the exit action.
fn find_state_action_in_tree(
    sm: &StateMachineIR,
    name: &str,
    entry: bool,
) -> Option<TransitionActionIR> {
    for state in &sm.states {
        if state.name == name {
            return if entry {
                state.entry_action.clone()
            } else {
                state.exit_action.clone()
            };
        }
        if let Some(sub) = &state.sub_machine {
            if let Some(action) = find_state_action_in_tree(sub, name, entry) {
                return Some(action);
            }
        }
    }
    None
}

/// Configuration for the state machine runner.
#[derive(Debug, Clone)]
pub struct RunnerConfig {
    /// Maximum number of steps before the runner halts.
    /// Default: 10,000.
    pub max_steps: usize,
}

impl Default for RunnerConfig {
    fn default() -> Self {
        Self { max_steps: 10_000 }
    }
}

/// A state machine runner with expression-based guard evaluation,
/// do-actions, and enhanced trigger support.
///
/// This runner extends the basic IR-driven execution with:
/// - **Expression guards**: guard strings are compiled to `ExprIR` and evaluated
///   against an `EvalContext`. Falls back to string matching for unparseable guards.
/// - **Do-actions**: an optional action that executes each step while a state is
///   active (before checking transitions).
/// - **Enhanced triggers**: `after(duration)` and `when(condition)` triggers in
///   addition to standard event triggers.
/// - **Time tracking**: an elapsed clock that can be advanced via `advance_time`.
pub struct StateMachineRunner {
    pub(crate) ir: StateMachineIR,
    current_state: String,
    completed: bool,
    /// Evaluation context for expression-based guards and when-triggers.
    pub eval_ctx: EvalContext,
    /// Per-state extensions (do-actions).
    state_extensions: HashMap<String, StateExtension>,
    /// Per-transition extensions indexed by (from, to) for trigger enrichment.
    transition_extensions: Vec<TransitionExtension>,
    /// Total elapsed time for the runner.
    elapsed: Duration,
    /// Time spent in the current state (reset on each transition).
    state_elapsed: Duration,
    /// Runner configuration (step limits, etc.).
    config: RunnerConfig,
    /// Number of steps executed since creation or last reset.
    step_count: usize,
    /// Whether the current state's do-action has been executed (for completion transitions).
    do_action_executed: bool,
    /// Sub-runners for composite states (state_name -> sub-runner).
    sub_runners: HashMap<String, Box<StateMachineRunner>>,
    /// History: maps composite state name -> last active sub-state path.
    /// Shallow history uses path[0], deep history restores the full path.
    state_history: HashMap<String, Vec<String>>,
    /// Previous boolean values for When triggers (rising-edge detection).
    /// Maps transition index -> was-true-last-step.
    /// Uses RefCell for interior mutability since trigger checking happens
    /// during iteration over immutably-borrowed transition data.
    prev_when_values: std::sync::Mutex<HashMap<usize, bool>>,
    /// Events deferred by the current state, replayed on state exit.
    deferred_queue: Vec<String>,
    /// RSC-2.4b: guard `ExprIR` cache parallel to `ir.transitions`,
    /// compiled ONCE at construction (closing the RSC-2.3 deferred item —
    /// guards no longer recompile from strings per evaluation). `None` =
    /// the guard string did not compile → the event-string fallback
    /// semantics of [`evaluate_guard`] are preserved exactly (counted via
    /// [`guard_string_fallbacks`](Self::guard_string_fallbacks)).
    /// Bound to slots by `Executor::bind_expression_slots`; evaluation
    /// stays context-name-first (RSC-2.3 invariant), so bound and unbound
    /// caches produce identical verdicts wherever the name is in context.
    guard_cache: Vec<Option<ExprIR>>,
    /// RSC-2.4b: precomputed slot write-set (compiled assignment targets →
    /// routes). `None` until `Executor::prepare_slot_writeback` runs —
    /// the orchestrator then keeps the legacy whole-context-diff path.
    write_set: Option<SmWriteSet>,
    /// RSC-2.4b: runtime-dynamic context keys this runner bound during
    /// `tick()` that are NOT compile-enumerable as a slot claim — currently
    /// only the local-clock `__clock_time` key (a Phase-4 item, design doc
    /// §8 caveat (d)). Published through the name-keyed fallback, reported
    /// via `Executor::slot_write_fallbacks`. BTreeSet for deterministic
    /// publish order.
    ///
    /// RSC-3.5b: port payload bindings (`{port}.payload`/`{port}_payload`)
    /// were drained OUT of this set — the SET of accept ports a SM instance
    /// can receive payloads on IS compile-static (only the message VALUES are
    /// dynamic), so they are pre-minted as Discrete slots and routed through
    /// [`payload_keys`](Self::payload_keys) instead of this fallback class.
    dynamic_keys: std::collections::BTreeSet<String>,
    /// RSC-3.5b: port payload context keys this runner bound during `tick()`
    /// (`{port}.payload`/`{port}_payload`). Drained out of `dynamic_keys`:
    /// the receiving ports are compile-static (`accept_ports()`), so the
    /// compiler pre-mints `{port}.payload` Discrete slots and
    /// `prepare_slot_writeback` resolves a [`WriteRoute`] per key
    /// ([`SmWriteSet::payload_routes`]). Routed payloads leave the
    /// name-keyed fallback class entirely. BTreeSet for deterministic order.
    payload_keys: std::collections::BTreeSet<String>,
    /// RSC-2.4b: outcome of the guard/trigger slot-binding pass (kept for
    /// introspection; `unresolved` is intentionally NOT fed to RS003 —
    /// guards carry event-string fallback semantics, so an unresolvable
    /// guard name is legal, not a compile defect).
    guard_bind_report: crate::expressions::BindReport,
    /// RSC-2.4b: every read this runner performs at tick time is provably
    /// slot-servable → the orchestrator may skip the per-prefix
    /// scoped-context clone (same conservative eligibility shape as the
    /// RSC-2.4a ODE bypass). Latched by `prepare_slot_writeback`.
    bypass_scoped: bool,
    /// RSC-3.6 step (2): the instance-local slot seed for structured-action
    /// reads. `execute_action` reads/writes assignment operands by bare
    /// context name against [`eval_ctx`](Self::eval_ctx) (so within-action
    /// read-after-write stays coherent on a single surface). The thin
    /// slot-read view a prefixed runner reads through carries no variable
    /// map, so those bare names are not pre-populated — each tick `tick()`
    /// re-seeds exactly these `(bare_name, SlotId)` pairs from the slots
    /// (the bounded, compile-enumerated subset the actions actually read).
    /// (Pre-RSC-3.5f.3 the legacy `build_scoped_context` clone populated
    /// them instead; that path is deleted — the slot seed is now the sole
    /// mechanism.) Collected at bind time; non-empty only for prefixed,
    /// bypass-eligible SMs whose every action read is a bare name resolving
    /// to an instance-local slot.
    action_seed: Vec<(String, crate::slots::SlotId)>,
    /// RSC-3.6 step (2): true when every structured-action read this runner
    /// performs is a bare name that binds to an instance-local slot (so the
    /// `action_seed` fully reconstructs the action's read environment without
    /// the scoped clone). Vacuously true when the runner has no structured
    /// actions. Gates the structured-action arm of `scoped_bypass_eligible`:
    /// an action whose RHS reads a chain, a non-slot name, or a global
    /// fall-through stays bypass-INELIGIBLE (surfaced, not papered).
    actions_slot_seedable: bool,
}

impl Clone for StateMachineRunner {
    fn clone(&self) -> Self {
        // Snapshot the Mutex contents so the clone has its own independent
        // map. If another thread panicked while holding the mutex the
        // lock is poisoned — recover the inner guard anyway rather than
        // silently dropping the rising-edge state, which would cause
        // spurious `when()` fires on the first post-fork step.
        let prev_when_snapshot = match self.prev_when_values.lock() {
            Ok(guard) => guard.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        };

        Self {
            ir: self.ir.clone(),
            current_state: self.current_state.clone(),
            completed: self.completed,
            eval_ctx: self.eval_ctx.alias_live(),
            state_extensions: self.state_extensions.clone(),
            transition_extensions: self.transition_extensions.clone(),
            elapsed: self.elapsed,
            state_elapsed: self.state_elapsed,
            config: self.config.clone(),
            step_count: self.step_count,
            do_action_executed: self.do_action_executed,
            // sub_runners are StateMachineRunner themselves — recursive deep clone.
            sub_runners: self
                .sub_runners
                .iter()
                .map(|(k, v)| (k.clone(), Box::new((**v).clone())))
                .collect(),
            state_history: self.state_history.clone(),
            prev_when_values: std::sync::Mutex::new(prev_when_snapshot),
            deferred_queue: self.deferred_queue.clone(),
            guard_cache: self.guard_cache.clone(),
            write_set: self.write_set.clone(),
            dynamic_keys: self.dynamic_keys.clone(),
            payload_keys: self.payload_keys.clone(),
            guard_bind_report: self.guard_bind_report.clone(),
            bypass_scoped: self.bypass_scoped,
            action_seed: self.action_seed.clone(),
            actions_slot_seedable: self.actions_slot_seedable,
        }
    }
}

impl StateMachineRunner {
    /// Create a new runner from IR.
    pub fn new(ir: StateMachineIR) -> Self {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            machine = %ir.name,
            states = ir.states.len(),
            transitions = ir.transitions.len(),
            regions = ir.regions.len(),
            "creating state machine runner"
        );

        let initial = ir.initial.clone();
        let mut state_extensions = HashMap::new();
        for state in &ir.states {
            if let Some(do_action) = &state.do_action {
                state_extensions.insert(
                    state.name.clone(),
                    StateExtension {
                        do_action: Some(do_action.clone()),
                    },
                );
            }
        }
        let mut sub_runners = HashMap::new();

        // If the initial state is composite, create a sub-runner for it
        if let Some(state_ir) = ir.find_state(&initial) {
            if let Some(sub_machine) = &state_ir.sub_machine {
                let sub = StateMachineRunner::new((**sub_machine).clone());
                sub_runners.insert(initial.clone(), Box::new(sub));
            }
        }

        let mut runner = StateMachineRunner {
            ir,
            current_state: initial,
            completed: false,
            eval_ctx: EvalContext::new(),
            state_extensions,
            transition_extensions: Vec::new(),
            elapsed: Duration::ZERO,
            state_elapsed: Duration::ZERO,
            config: RunnerConfig::default(),
            step_count: 0,
            do_action_executed: false,
            sub_runners,
            state_history: HashMap::new(),
            prev_when_values: std::sync::Mutex::new(HashMap::new()),
            deferred_queue: Vec::new(),
            guard_cache: Vec::new(),
            write_set: None,
            dynamic_keys: std::collections::BTreeSet::new(),
            payload_keys: std::collections::BTreeSet::new(),
            guard_bind_report: crate::expressions::BindReport::default(),
            bypass_scoped: false,
            action_seed: Vec::new(),
            // Vacuously seedable until the bind pass discovers a structured
            // action whose read does NOT resolve to an instance-local slot.
            actions_slot_seedable: true,
        };
        runner.wire_triggers();
        runner.compile_guard_cache();
        runner
    }

    /// Create a new runner from IR with a custom configuration.
    pub fn with_config(ir: StateMachineIR, config: RunnerConfig) -> Self {
        let mut runner = Self::new(ir);
        runner.config = config;
        runner
    }

    /// Create a runner by compiling a model graph.
    pub fn from_graph(graph: &ModelGraph) -> Result<Self, Vec<Diagnostic>> {
        let ir = StateMachineCompiler::compile(graph)?;
        Ok(Self::new(ir))
    }

    /// Create a runner for a specific named state machine.
    pub fn from_graph_named(graph: &ModelGraph, sm_name: &str) -> Result<Self, Vec<Diagnostic>> {
        let ir = StateMachineCompiler::compile_named(graph, sm_name)?;
        Ok(Self::new(ir))
    }

    /// Set a do-action for a named state.
    pub fn set_do_action(&mut self, state: &str, action: TransitionActionIR) {
        self.state_extensions
            .entry(state.to_owned())
            .or_default()
            .do_action = Some(action);
    }

    /// Register a trigger for a transition at the given index.
    ///
    /// The index corresponds to the position in `ir.transitions`.
    pub fn set_transition_trigger(&mut self, index: usize, trigger: TriggerKind) {
        // Ensure the extensions vec is large enough
        while self.transition_extensions.len() <= index {
            self.transition_extensions
                .push(TransitionExtension { trigger: None });
        }
        self.transition_extensions[index].trigger = Some(trigger);
    }

    /// Rewrite the transition at `index` from a tick-sampled [`TriggerKind::When`]
    /// to a located [`TriggerKind::WhenLocated`] that fires on the named
    /// zero-crossing event (WS-A2 / hybrid event location).
    ///
    /// Returns `true` only when the transition currently carries a `When`
    /// trigger — the compiler classified its comparator as a continuous-state
    /// threshold predicate and registered a matching crossing detector. A
    /// non-`When` trigger (or out-of-range index) is left untouched and returns
    /// `false`, so the caller can detect a classification/indexing mismatch.
    pub fn rewrite_when_to_located(&mut self, index: usize, event_name: &str) -> bool {
        match self.transition_extensions.get_mut(index) {
            Some(ext) if matches!(ext.trigger, Some(TriggerKind::When(_))) => {
                ext.trigger = Some(TriggerKind::WhenLocated(event_name.to_owned()));
                true
            }
            _ => false,
        }
    }

    /// Examine each transition's event string and register time/condition triggers.
    fn wire_triggers(&mut self) {
        let triggers: Vec<(usize, TriggerKind)> = self
            .ir
            .transitions
            .iter()
            .enumerate()
            .filter_map(|(idx, transition)| {
                transition
                    .event
                    .as_deref()
                    .and_then(|event| {
                        parse_trigger_from_event(event, transition.accept_param.as_deref())
                    })
                    .map(|trigger| (idx, trigger))
            })
            .collect();
        for (idx, trigger) in triggers {
            self.set_transition_trigger(idx, trigger);
        }
    }

    /// RSC-2.4b: compile every transition guard string into persistent
    /// `ExprIR` ONCE (the per-evaluation `compile_simple_expression` call in
    /// [`evaluate_guard`] is the RSC-2.3 deferred item this closes).
    /// `compile_simple_expression` is deterministic, so the cached IR is
    /// exactly what per-eval compilation would have produced.
    fn compile_guard_cache(&mut self) {
        self.guard_cache = self
            .ir
            .transitions
            .iter()
            .map(|t| {
                t.guard
                    .as_deref()
                    .and_then(|g| compile_simple_expression(g).ok())
            })
            .collect();
    }

    /// RSC-2.4b: evaluate the guard of transition `idx` through the
    /// compiled (and, post-bind, slot-bound) cache. Semantics are
    /// byte-compatible with [`evaluate_guard`]:
    /// - cached IR evaluates to `Bool(b)` → `b`;
    /// - non-boolean result or evaluation error → event-name fallback
    ///   (`event == Some(guard)`);
    /// - no cache entry (guard didn't compile, or the index drifted from
    ///   `ir.transitions` — e.g. a test mutated the IR post-construction)
    ///   → full legacy [`evaluate_guard`] path, which re-derives the same
    ///   fallback behaviour from the string.
    fn evaluate_guard_at(&self, idx: usize, guard: &str, event: Option<&str>) -> bool {
        match self.guard_cache.get(idx).and_then(|c| c.as_ref()) {
            Some(expr) => {
                let evaluator = ExpressionEvaluator::new();
                match evaluator.eval(expr, &self.eval_ctx) {
                    Ok(Value::Bool(b)) => b,
                    // Non-boolean / evaluation error — the event-string
                    // backward-compat fallback, exactly as evaluate_guard.
                    _ => event == Some(guard),
                }
            }
            None => evaluate_guard(guard, &self.eval_ctx, event),
        }
    }

    /// RSC-2.4b: how many guarded transitions stay on the string/event
    /// fallback path because their guard string did not compile.
    pub fn guard_string_fallbacks(&self) -> usize {
        self.ir
            .transitions
            .iter()
            .enumerate()
            .filter(|(idx, t)| {
                t.guard.is_some() && self.guard_cache.get(*idx).map_or(true, |c| c.is_none())
            })
            .count()
    }

    /// RSC-2.4b: outcome of the guard/trigger slot-binding pass.
    pub fn guard_bind_report(&self) -> &crate::expressions::BindReport {
        &self.guard_bind_report
    }

    /// RSC-3.6: the slot-bindable read keys of every compiled guard +
    /// persistent `When` / `AfterExpr` trigger expression — the full dotted
    /// chains [`bind_expression_slots`](Self::bind_expression_slots) will try
    /// to resolve. The compiler mints a per-instance slot for each so a
    /// prefixed (instance-multiplied) SM's guard binds to instance-local
    /// `SlotRef`s and becomes scoped-view-bypass-eligible (the read it
    /// performs is then provably servable from the instance's own slots
    /// instead of the prefix-stripped scoped-context clone). Uncompilable
    /// guards (`guard_cache` `None`, event-name comparison only) read nothing
    /// and contribute no keys.
    pub(crate) fn guard_trigger_reads(&self) -> std::collections::HashSet<String> {
        let mut reads = std::collections::HashSet::new();
        for expr in self.guard_cache.iter().flatten() {
            reads.extend(expr.slot_bindable_reads());
        }
        for ext in &self.transition_extensions {
            match &ext.trigger {
                Some(TriggerKind::When(expr)) | Some(TriggerKind::AfterExpr(expr)) => {
                    reads.extend(expr.slot_bindable_reads());
                }
                _ => {}
            }
        }
        reads
    }

    /// RSC-3.6 step (2): the bare context names every structured action in
    /// this runner reads at execution time — assignment RHS free variables
    /// (`value_expr`) plus, for `+=`/`-=`, the target's implicit current-value
    /// read. These are the keys [`execute_action`] looks up through
    /// [`EvalContext::get`](crate::expressions::EvalContext::get) (the bare-name
    /// variable map), so they must be seeded into the runner's `eval_ctx` for
    /// the action to evaluate identically without the scoped-context clone.
    ///
    /// Walks exactly the action sites [`scoped_bypass_eligible`] inspects:
    /// each top-level state's entry/do/exit action, the `state_extensions`
    /// do-actions, and each transition action. (Composite states are already
    /// disqualified from the bypass by the `sub_machine.is_none()` gate, so a
    /// nested sub-machine's actions need not be collected here.)
    pub(crate) fn action_slot_reads(&self) -> Vec<String> {
        fn push_action(action: &TransitionActionIR, out: &mut Vec<String>) {
            if let TransitionActionIR::Structured { assignments, .. } = action {
                for a in assignments {
                    if let Some(expr) = &a.value_expr {
                        out.extend(expr.slot_bindable_reads());
                    }
                    // `+=` / `-=` read the target's current value (see
                    // `execute_action`'s arithmetic arm) before writing it.
                    if matches!(
                        a.operator,
                        crate::AssignmentOp::Add | crate::AssignmentOp::Subtract
                    ) {
                        out.push(a.variable.clone());
                    }
                }
            }
        }
        let mut reads = Vec::new();
        for s in &self.ir.states {
            for act in [&s.entry_action, &s.do_action, &s.exit_action]
                .into_iter()
                .flatten()
            {
                push_action(act, &mut reads);
            }
        }
        for ext in self.state_extensions.values() {
            if let Some(act) = &ext.do_action {
                push_action(act, &mut reads);
            }
        }
        for t in &self.ir.transitions {
            if let Some(act) = &t.action {
                push_action(act, &mut reads);
            }
        }
        reads
    }

    /// RSC-2.4b — scoped-clone bypass eligibility (same conservative shape
    /// as the RSC-2.4a ODE rule): every read this runner performs at tick
    /// time must be provably servable without the orchestrator's
    /// prefix-stripped scoped-context view. That holds only when:
    ///
    /// - every guarded transition's guard compiled AND every read in it is
    ///   a bound `SlotRef` resolving through the subsystem's OWN namespace
    ///   (`SlotBinder::resolve_subsystem_local` — the binder's global
    ///   fall-through must not survive bypass, where it would read a
    ///   top-level default instead of the instance's value; same rule the
    ///   RSC-2.4a ODE eligibility applies to template parameters). Chain
    ///   heads / function calls / unbound names may consult flat scoped
    ///   keys, the graph, the trace or registries — disqualified;
    /// - every persistent trigger expression (`When` / `AfterExpr`) is
    ///   fully bound the same way; `At` triggers read `__clock_time` by
    ///   name; `Event` / `After` / `PortMessage` triggers read nothing;
    /// - every structured action's reads are slot-seedable
    ///   ([`actions_slot_seedable`](Self::actions_slot_seedable)): each bare
    ///   context name [`execute_action`] reads (assignment RHS free vars,
    ///   `+=`/`-=` current-value lookups) binds to an instance-local slot, so
    ///   `tick()` re-seeds them from the slots (the bounded subset of the
    ///   scoped clone the actions read) and `execute_action` evaluates
    ///   identically off `eval_ctx`. An action reading a chain, a non-slot
    ///   name, or a global fall-through is NOT seedable and disqualifies the
    ///   SM (RSC-3.6 step 2). `Simple` actions are execution no-ops and stay
    ///   eligible regardless;
    /// - no composite states or parallel regions (their sub-runners hold
    ///   unbound guard caches that resolve through the merged view).
    ///
    /// Guards whose verdict legitimately falls back to event-name matching
    /// stay eligible — that comparison reads no context.
    fn scoped_bypass_eligible(
        &self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
    ) -> bool {
        // RSC-3.6 step (2): a structured action is bypass-safe only when every
        // bare name it reads was seeded into `eval_ctx` from an instance-local
        // slot (`actions_slot_seedable`, latched by `bind_expression_slots`).
        // The flag is runner-global / all-or-nothing, so it is threaded into
        // the per-site `action_is_pure` check uniformly.
        let actions_ok = self.actions_slot_seedable;
        fn action_is_pure(action: &TransitionActionIR, actions_ok: bool) -> bool {
            match action {
                // Execution no-op (see execute_action).
                TransitionActionIR::Simple(_) => true,
                // Empty assignments read nothing; otherwise every read must be
                // slot-seedable (RSC-3.6 step 2).
                TransitionActionIR::Structured { assignments, .. } => {
                    assignments.is_empty() || actions_ok
                }
            }
        }
        fn states_eligible(
            states: &[StateIR],
            exts: &HashMap<String, StateExtension>,
            actions_ok: bool,
        ) -> bool {
            states.iter().all(|s| {
                s.sub_machine.is_none()
                    && [&s.entry_action, &s.do_action, &s.exit_action]
                        .into_iter()
                        .flatten()
                        .all(|a| action_is_pure(a, actions_ok))
                    && exts
                        .get(&s.name)
                        .and_then(|e| e.do_action.as_ref())
                        .map_or(true, |a| action_is_pure(a, actions_ok))
            })
        }

        if !self.ir.regions.is_empty() {
            return false;
        }
        if !states_eligible(&self.ir.states, &self.state_extensions, actions_ok) {
            return false;
        }
        // SlotRefs must have bound through the instance's own namespace —
        // the binder's global fall-through reads the wrong value once the
        // merged scoped view is gone.
        let binder = crate::expressions::SlotBinder::for_subsystem(store, var_prefix);
        let local_slot = |name: &str, slot: crate::slots::SlotId| {
            binder.resolve_subsystem_local(name) == Some(slot)
        };
        for (idx, t) in self.ir.transitions.iter().enumerate() {
            if let Some(action) = &t.action {
                if !action_is_pure(action, actions_ok) {
                    return false;
                }
            }
            if t.guard.is_some() {
                match self.guard_cache.get(idx).and_then(|c| c.as_ref()) {
                    Some(expr) => {
                        if !expr_fully_slot_bound(expr, &local_slot) {
                            return false;
                        }
                    }
                    // Uncompilable guard → event-name comparison only:
                    // reads no context, stays eligible.
                    None => {}
                }
            }
            if let Some(ext) = self.transition_extensions.get(idx) {
                match &ext.trigger {
                    Some(TriggerKind::When(expr)) | Some(TriggerKind::AfterExpr(expr)) => {
                        if !expr_fully_slot_bound(expr, &local_slot) {
                            return false;
                        }
                    }
                    Some(TriggerKind::At(_)) => return false,
                    _ => {}
                }
            }
        }
        true
    }

    /// Advance the elapsed time by the given duration.
    ///
    /// This also advances the per-state elapsed timer, allowing
    /// `after(duration)` triggers to fire.
    pub fn advance_time(&mut self, dt: Duration) {
        self.elapsed += dt;
        self.state_elapsed += dt;
    }

    /// Get the total elapsed time.
    pub fn elapsed(&self) -> Duration {
        self.elapsed
    }

    /// Get the time spent in the current state.
    pub fn state_elapsed(&self) -> Duration {
        self.state_elapsed
    }

    /// Force a transition to the given target state (Phase 15: guard-only transitions).
    ///
    /// Executes exit actions from source up through the hierarchy to the lowest common
    /// ancestor (LCA), then the optional transition action, then entry actions from the
    /// LCA down to the target. For same-level transitions the LCA is implicit (no
    /// intermediate states to traverse). For inter-level transitions the full
    /// exit/entry chain through composite states is executed.
    ///
    /// Returns the `sends` accumulated by every exit/transition/entry action run
    /// during the move (L26): the guard-only transition path previously discarded
    /// them, so a `send … via <port>` on a state entered by a guard-only
    /// transition never reached the orchestrator's router. The single production
    /// caller (`<dyn Executor>::tick`) merges these into the tick's `sends`.
    #[allow(clippy::needless_pass_by_value)] // Public API; callers construct the Option inline
    pub fn force_transition(
        &mut self,
        target: &str,
        action: Option<TransitionActionIR>,
    ) -> (Vec<String>, Vec<(String, sysml_core::Value)>) {
        let mut sends: Vec<String> = Vec::new();
        let mut port_sends: Vec<(String, sysml_core::Value)> = Vec::new();
        let source = self.current_state.clone();

        // Check if the target is nested inside the current composite state.
        // If so, the LCA is the current state itself and we only need to
        // rebuild the sub-runner chain (no top-level state change needed).
        let target_is_nested_in_source = self.ir.states.iter().any(|s| {
            s.name == source
                && s.sub_machine
                    .as_ref()
                    .is_some_and(|sm| find_state_in_tree(sm, target))
        });

        if target_is_nested_in_source {
            // Cross-branch or deeper transition within the same composite state.
            // Exit the current sub-runner chain, execute transition action,
            // then rebuild sub-runners targeting the new leaf state.
            self.exit_sub_runners_recursively(&source);

            if let Some(act) = &action {
                sends.extend(execute_action(act, &mut self.eval_ctx, &mut port_sends));
            }

            // Build entry chain from the current composite down to the target
            let entry_chain = self.entry_chain_within(&source, target);
            for state_name in &entry_chain {
                // Find the action in the sub-machine tree
                if let Some(action_ir) = find_state_action_in_tree(&self.ir, state_name, true) {
                    sends.extend(execute_action(
                        &action_ir,
                        &mut self.eval_ctx,
                        &mut port_sends,
                    ));
                }
            }

            // Rebuild sub-runner chain
            self.enter_composite_chain_from_current(&source, target);

            self.state_elapsed = Duration::ZERO;
            self.do_action_executed = false;
            return (sends, port_sends);
        }

        // Standard case: source and target are at the same level or
        // the target is nested inside a different top-level state.
        let exit_chain = self.exit_chain(&source, target);
        let entry_chain = self.entry_chain(&source, target);

        // Execute exit actions from source up to (but not including) LCA
        for state_name in &exit_chain {
            // Exit sub-runner if this is a composite state we are leaving
            self.exit_sub_runners_recursively(state_name);

            if let Some(state) = self.ir.find_state(state_name) {
                if let Some(exit) = &state.exit_action {
                    sends.extend(execute_action(exit, &mut self.eval_ctx, &mut port_sends));
                }
            }
        }

        // Execute transition action
        if let Some(act) = &action {
            sends.extend(execute_action(act, &mut self.eval_ctx, &mut port_sends));
        }

        // Execute entry actions from LCA down to target
        for state_name in &entry_chain {
            if let Some(state) = self.ir.find_state(state_name) {
                if let Some(entry) = &state.entry_action {
                    sends.extend(execute_action(entry, &mut self.eval_ctx, &mut port_sends));
                }
            }
        }

        // Determine the new top-level state.
        // If the target is a top-level state, use it directly.
        // If the target is nested, use the outermost composite in the entry chain.
        let is_top_level = self.ir.states.iter().any(|s| s.name == target);
        let top_level_target = if is_top_level {
            target.to_owned()
        } else {
            // Find which top-level state contains the target
            entry_chain
                .first()
                .cloned()
                .unwrap_or_else(|| target.to_owned())
        };
        self.current_state = top_level_target;
        self.state_elapsed = Duration::ZERO;
        self.do_action_executed = false;

        // Set up sub-runners if the target is nested
        if !is_top_level {
            self.enter_composite_chain(&entry_chain, target);
        }

        // Check if the target state is final
        if let Some(state) = self.ir.find_state(&self.current_state) {
            if state.is_final {
                self.completed = true;
            }
        }

        (sends, port_sends)
    }

    /// Get the chain of ancestor state names from a state up to the root of this
    /// state machine. Returns `[state, parent, grandparent, ...]`.
    ///
    /// Walks the `sub_machine` hierarchy: a state S is a child of state P if P's
    /// `sub_machine` contains S in its direct `states` list. The search considers
    /// all levels of nesting, not just the top-level states.
    fn ancestor_chain(&self, state: &str) -> Vec<String> {
        let mut chain = vec![state.to_owned()];
        let mut current = state.to_owned();
        while let Some(parent_name) = find_direct_parent_in_tree(&self.ir, &current) {
            chain.push(parent_name.clone());
            current = parent_name;
        }
        chain
    }

    /// Find the lowest common ancestor of two states in the hierarchy.
    /// Returns `None` if the states share no common ancestor (both are
    /// top-level, or one is the root itself).
    fn find_lca(&self, state_a: &str, state_b: &str) -> Option<String> {
        let ancestors_a = self.ancestor_chain(state_a);
        let ancestors_b = self.ancestor_chain(state_b);
        // Find the deepest ancestor of A that also appears in B's chain
        for a in &ancestors_a {
            if ancestors_b.contains(a) {
                return Some(a.clone());
            }
        }
        None
    }

    /// Compute the states to exit when transitioning from `source` to `target`.
    /// Returns states from source up to (but not including) the LCA.
    /// If there is no LCA (same-level transition), returns just `[source]`.
    fn exit_chain(&self, source: &str, target: &str) -> Vec<String> {
        let ancestors_source = self.ancestor_chain(source);
        match self.find_lca(source, target) {
            Some(ref lca) => {
                // Collect ancestors from source up to but not including LCA
                ancestors_source
                    .into_iter()
                    .take_while(|s| s != lca)
                    .collect()
            }
            None => {
                // No shared ancestor — exit the full source chain
                ancestors_source
            }
        }
    }

    /// Compute the states to enter when transitioning from `source` to `target`.
    /// Returns states from just below the LCA down to target (inclusive).
    /// If there is no LCA (same-level transition), returns just `[target]`.
    fn entry_chain(&self, source: &str, target: &str) -> Vec<String> {
        let ancestors_target = self.ancestor_chain(target);
        match self.find_lca(source, target) {
            Some(ref lca) => {
                // Collect ancestors from target up to but not including LCA, then reverse
                let mut chain: Vec<String> = ancestors_target
                    .into_iter()
                    .take_while(|s| s != lca)
                    .collect();
                chain.reverse();
                chain
            }
            None => {
                // No shared ancestor — enter the full target chain (reversed to top-down)
                let mut chain = ancestors_target;
                chain.reverse();
                chain
            }
        }
    }

    /// Create sub-runners for entering a chain of composite states.
    /// `chain` is in top-down order (outermost first), `leaf_target` is the
    /// deepest state we want to land in.
    fn enter_composite_chain(&mut self, chain: &[String], leaf_target: &str) {
        // Walk the chain: for each composite state, create a sub-runner and
        // position it at the next state in the chain (or leaf_target for the last).
        let mut current_ir = &self.ir;
        let mut current_runners = &mut self.sub_runners;

        for (i, state_name) in chain.iter().enumerate() {
            let next_state = if i + 1 < chain.len() {
                &chain[i + 1]
            } else {
                leaf_target
            };

            // Find this state in the current IR level to get its sub_machine
            let state_ir = current_ir.states.iter().find(|s| s.name == *state_name);
            let sub_machine = state_ir.and_then(|s| s.sub_machine.as_ref());

            if let Some(sm) = sub_machine {
                let mut sub = StateMachineRunner::new((**sm).clone());
                sub.eval_ctx = self.eval_ctx.alias_live();
                sub.current_state = next_state.to_owned();
                sub.state_elapsed = Duration::ZERO;
                sub.do_action_executed = false;

                current_runners.insert(state_name.clone(), Box::new(sub));

                // Descend into the sub-runner for the next iteration
                let sub_runner = current_runners.get_mut(state_name).unwrap();
                current_ir = &sub_runner.ir;
                current_runners = &mut sub_runner.sub_runners;
            } else {
                // Not a composite state — nothing more to descend into
                break;
            }
        }
    }

    /// Recursively exit and remove all sub-runners for a composite state.
    /// Executes exit actions on the active sub-state at each nesting level.
    fn exit_sub_runners_recursively(&mut self, state_name: &str) {
        if let Some(mut sub) = self.sub_runners.remove(state_name) {
            // Recursively exit deeper sub-runners first
            let sub_current = {
                use crate::Runner;
                Runner::current_state(sub.as_ref()).to_owned()
            };
            sub.exit_sub_runners_recursively(&sub_current);

            // Execute the exit action on the sub-runner's current state.
            // Sends from exit-during-teardown are dropped here (as the string
            // `sends` already were before this change) — a throwaway sink keeps
            // that behavior; routing exit-teardown sends is out of scope.
            if let Some(sub_state) = sub.ir.find_state(&sub_current) {
                if let Some(exit) = &sub_state.exit_action {
                    execute_action(exit, &mut self.eval_ctx, &mut Vec::new());
                }
            }
        }
    }

    /// Compute the entry chain from a composite state down to a target nested within it.
    /// Returns the states in top-down order, excluding the composite state itself but
    /// including the target.
    fn entry_chain_within(&self, composite: &str, target: &str) -> Vec<String> {
        // Get the ancestor chain of target, which goes [target, ..., composite]
        let ancestors = self.ancestor_chain(target);
        // Take everything up to (but not including) the composite, then reverse
        let mut chain: Vec<String> = ancestors
            .into_iter()
            .take_while(|s| s != composite)
            .collect();
        chain.reverse();
        chain
    }

    /// Create sub-runners starting from the current composite state down to a
    /// nested target state. Similar to `enter_composite_chain` but starts from
    /// the sub-machine of an existing top-level state.
    fn enter_composite_chain_from_current(&mut self, composite: &str, leaf_target: &str) {
        let chain = self.entry_chain_within(composite, leaf_target);
        if chain.is_empty() {
            return;
        }

        // The composite state's sub-machine is our starting point
        let state_ir = self.ir.states.iter().find(|s| s.name == composite);
        let sub_machine = state_ir.and_then(|s| s.sub_machine.as_ref());

        if let Some(sm) = sub_machine {
            let mut sub = StateMachineRunner::new((**sm).clone());
            sub.eval_ctx = self.eval_ctx.alias_live();
            sub.current_state = chain[0].clone();
            sub.state_elapsed = Duration::ZERO;
            sub.do_action_executed = false;

            // If there are deeper levels, recursively build sub-runners
            if chain.len() > 1 {
                sub.enter_composite_chain(&chain, leaf_target);
            }

            self.sub_runners.insert(composite.to_owned(), Box::new(sub));
        }
    }

    /// Return the name of the state machine.
    pub fn name(&self) -> &str {
        &self.ir.name
    }

    /// Return available transitions from the current state.
    ///
    /// Each entry is `(event_or_name, target_state)` where the first element
    /// is the transition event (if set) or `"<from>_to_<to>"` as fallback.
    pub fn available_transitions(&self) -> Vec<(&str, &str)> {
        self.ir
            .transitions_from(&self.current_state)
            .into_iter()
            .map(|t| {
                let label = t.event.as_deref().unwrap_or(&t.to);
                (label, t.to.as_str())
            })
            .collect()
    }

    /// Return all state names in the compiled state machine.
    pub fn all_states(&self) -> Vec<&str> {
        self.ir.states.iter().map(|s| s.name.as_str()).collect()
    }

    /// Return all compiled transitions as `(from, event, guard, to)`.
    ///
    /// `event` is `None` for automatic transitions.
    /// `guard` is the raw guard expression string, if present.
    pub fn all_transitions(&self) -> Vec<(&str, Option<&str>, Option<&str>, &str)> {
        self.ir
            .transitions
            .iter()
            .map(|t| {
                (
                    t.from.as_str(),
                    t.event.as_deref(),
                    t.guard.as_deref(),
                    t.to.as_str(),
                )
            })
            .collect()
    }

    /// Diagnose all guarded transitions from the current state.
    /// Returns a GuardDiagnosis for each transition that has a guard expression.
    pub fn diagnose_guards(&self, event: Option<&str>) -> Vec<GuardDiagnosis> {
        let current = &self.current_state;
        let mut diagnoses = Vec::new();

        for (idx, transition) in self.ir.transitions.iter().enumerate() {
            if transition.from != *current {
                continue;
            }

            if let Some(guard) = &transition.guard {
                // Dependency names stay string-derived (RSC-2.4b invariant:
                // analyze_guard_dependencies keeps producing the same names
                // regardless of slot binding).
                let deps = crate::expressions::analyze_guard_dependencies(guard);
                let dep_values: std::collections::HashMap<String, sysml_core::Value> = deps
                    .iter()
                    .filter_map(|name| self.eval_ctx.get(name).map(|v| (name.clone(), v.clone())))
                    .collect();

                // Verdict through the same compiled/bound cache the runner
                // executes with, so diagnosis matches runtime truth.
                let satisfied = self.evaluate_guard_at(idx, guard, event);

                let explanation = if satisfied {
                    format!("satisfied: {}", guard)
                } else {
                    let var_summary: Vec<String> = deps
                        .iter()
                        .map(|name| {
                            let val = dep_values
                                .get(name)
                                .map(|v| format!("{:?}", v))
                                .unwrap_or_else(|| "undefined".to_owned());
                            format!("{} = {}", name, val)
                        })
                        .collect();
                    if var_summary.is_empty() {
                        format!("blocked: {} (no variable bindings)", guard)
                    } else {
                        format!("blocked: {} ({})", guard, var_summary.join(", "))
                    }
                };

                diagnoses.push(GuardDiagnosis {
                    guard_expr: guard.clone(),
                    transition: (transition.from.clone(), transition.to.clone()),
                    event: transition.event.clone(),
                    dependencies: deps,
                    dependency_values: dep_values,
                    satisfied,
                    explanation,
                });
            }
        }
        diagnoses
    }

    /// Check if the current state's do-activity has completed.
    ///
    /// Returns true if:
    /// - The state has no do-action (neither in IR nor in state_extensions), or
    /// - The do-action is Simple (descriptive, always "complete"), or
    /// - The do-action is Structured and has been executed at least once.
    fn do_activity_completed(&self) -> bool {
        // Check state_extensions first (includes dynamically set do-actions),
        // then fall back to ir.states for completeness.
        let do_action = self
            .state_extensions
            .get(&self.current_state)
            .and_then(|ext| ext.do_action.as_ref())
            .or_else(|| {
                self.ir
                    .states
                    .iter()
                    .find(|s| s.name == self.current_state)
                    .and_then(|s| s.do_action.as_ref())
            });

        match do_action {
            None => true,
            Some(TransitionActionIR::Simple(_)) => true,
            Some(TransitionActionIR::Structured { .. }) => self.do_action_executed,
        }
    }

    /// Check if a transition's trigger (enhanced or standard) matches the current conditions.
    fn transition_matches(
        &self,
        transition_idx: usize,
        transition: &TransitionIR,
        event: Option<&str>,
    ) -> bool {
        // Check for enhanced trigger
        if let Some(ext) = self.transition_extensions.get(transition_idx) {
            if let Some(trigger) = &ext.trigger {
                return match trigger {
                    TriggerKind::Event(e) => event.is_some_and(|ev| ev == e),
                    // Located change trigger: the orchestrator's zero-crossing
                    // detector injects the synthesized crossing event by name;
                    // fire exactly like an event match (the crossing instant IS
                    // the trigger occurrence — spec ChangeSignal semantics).
                    TriggerKind::WhenLocated(e) => event.is_some_and(|ev| ev == e),
                    TriggerKind::After(d) => self.state_elapsed >= *d,
                    TriggerKind::AfterExpr(expr) => {
                        let evaluator = ExpressionEvaluator::new();
                        match evaluator.eval(expr, &self.eval_ctx) {
                            Ok(Value::Float(secs)) => {
                                self.state_elapsed >= Duration::from_secs_f64(secs)
                            }
                            Ok(Value::Int(ms)) => {
                                self.state_elapsed >= Duration::from_millis(ms as u64)
                            }
                            _ => false, // Can't evaluate → don't fire
                        }
                    }
                    TriggerKind::At(target_time) => {
                        // Prefer clock time from orchestrator if available
                        let current_time = self
                            .eval_ctx
                            .get("__clock_time")
                            .and_then(|v| match v {
                                Value::Float(f) => Some(*f),
                                Value::Int(i) => Some(*i as f64),
                                _ => None,
                            })
                            .unwrap_or(self.elapsed.as_secs_f64());
                        current_time >= *target_time
                    }
                    TriggerKind::When(expr) => {
                        // Undefined-variable diagnostic: runs per-guard per-tick
                        // so it's gated behind SYSML_TRACE_GUARDS to avoid
                        // flooding stderr on large models where some guards
                        // reference vars that aren't in every scoped context.
                        #[cfg(debug_assertions)]
                        #[allow(clippy::print_stderr)]
                        if std::env::var_os("SYSML_TRACE_GUARDS").is_some() {
                            let free_vars = expr.free_variables();
                            for var in &free_vars {
                                if self.eval_ctx.get(var).is_none() {
                                    eprintln!(
                                        "[sysml-runtime] When trigger on transition {} ({}->{}) references undefined variable '{}'",
                                        transition_idx,
                                        transition.from,
                                        transition.to,
                                        var,
                                    );
                                }
                            }
                        }
                        // Rising-edge detection: only fire on false→true transition
                        let evaluator = ExpressionEvaluator::new();
                        let current =
                            matches!(evaluator.eval(expr, &self.eval_ctx), Ok(Value::Bool(true)));
                        let prev = self
                            .prev_when_values
                            .lock()
                            .unwrap()
                            .get(&transition_idx)
                            .copied()
                            .unwrap_or(false);
                        self.prev_when_values
                            .lock()
                            .unwrap()
                            .insert(transition_idx, current);
                        current && !prev // Only fire on rising edge
                    }
                    TriggerKind::PortMessage { port_name, .. } => {
                        event.is_some_and(|ev| ev == port_name)
                    }
                };
            }
        }
        // Fall back to standard event matching
        transition.matches(event)
    }

    /// All `PortMessage` accept ports anywhere in this machine, including
    /// sub-machines (RSC-3.3c D4). These are the machine's accepting
    /// surfaces for occurrence-addressed MessageTransfers — the
    /// orchestrator registers each as `"{subsystem}.{port}"` on the
    /// [`ExchangePlane`](crate::exchange::ExchangePlane).
    pub fn accept_ports(&self) -> Vec<String> {
        let mut ports = Vec::new();
        for ext in &self.transition_extensions {
            if let Some(TriggerKind::PortMessage { port_name, .. }) = &ext.trigger {
                if !ports.contains(port_name) {
                    ports.push(port_name.clone());
                }
            }
        }
        for sub in self.sub_runners.values() {
            for port in sub.accept_ports() {
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
        ports
    }

    /// The accept-parameter name declared for `port` by an
    /// `accept <name> via <port>` trigger, if any (searches this machine and
    /// any active sub-runners). Used to bind the delivered payload to `<name>`.
    fn accept_param_for_port(&self, port: &str) -> Option<String> {
        for ext in &self.transition_extensions {
            if let Some(TriggerKind::PortMessage {
                port_name,
                param_name: Some(name),
                ..
            }) = &ext.trigger
            {
                if port_name == port {
                    return Some(name.clone());
                }
            }
        }
        for sub in self.sub_runners.values() {
            if let Some(name) = sub.accept_param_for_port(port) {
                return Some(name);
            }
        }
        None
    }

    /// The `PortMessage` accept ports ARMED right now (RSC-3.3c U1):
    /// ports of `PortMessage`-triggered transitions whose source state is
    /// the current state (or the active sub-state of a composite). The
    /// orchestrator polls these each tick and pull-initiates parked
    /// pull-mode transfers ([`ExchangePlane::pull`](crate::exchange::ExchangePlane::pull))
    /// for them — "delivery happens when the target's accept becomes
    /// enabled" (design doc D-3.0.4 row U1, labeled extension).
    pub fn armed_accept_ports(&self) -> Vec<String> {
        let mut ports = Vec::new();
        for (idx, t) in self.ir.transitions.iter().enumerate() {
            if t.from != self.current_state {
                continue;
            }
            if let Some(TransitionExtension {
                trigger: Some(TriggerKind::PortMessage { port_name, .. }),
            }) = self.transition_extensions.get(idx)
            {
                if !ports.contains(port_name) {
                    ports.push(port_name.clone());
                }
            }
        }
        if let Some(sub) = self.sub_runners.get(&self.current_state) {
            for port in sub.armed_accept_ports() {
                if !ports.contains(&port) {
                    ports.push(port);
                }
            }
        }
        ports
    }

    /// Get the full state path including sub-states.
    ///
    /// For a composite state like `Operational` containing sub-state `Idle`,
    /// this returns `["Operational", "Idle"]`.
    pub fn current_state_path(&self) -> Vec<String> {
        let mut path = vec![self.current_state.clone()];
        if let Some(sub) = self.sub_runners.get(&self.current_state) {
            path.extend(sub.current_state_path());
        }
        path
    }

    /// Check all triggers for readiness issues and return a list of diagnostic messages.
    ///
    /// Inspects each transition's trigger and reports problems such as:
    /// - `When` triggers that reference variables not present in the `EvalContext`
    ///
    /// This is useful for pre-simulation diagnostics to catch configuration issues
    /// before stepping the state machine.
    pub fn check_trigger_readiness(&self) -> Vec<String> {
        let mut issues = Vec::new();
        for (idx, ext) in self.transition_extensions.iter().enumerate() {
            if let Some(TriggerKind::When(expr)) = &ext.trigger {
                let free_vars = expr.free_variables();
                let (from, to) = if let Some(t) = self.ir.transitions.get(idx) {
                    (t.from.as_str(), t.to.as_str())
                } else {
                    ("?", "?")
                };
                for var in &free_vars {
                    if self.eval_ctx.get(var).is_none() {
                        issues.push(format!(
                            "When trigger on transition {} ({}->{}) references undefined variable '{}'",
                            idx, from, to, var,
                        ));
                    }
                }
            }
        }
        issues
    }
}

impl StateMachineRunner {
    /// Fire at most one transition. Returns the step result.
    fn step_inner(&mut self, event: Option<&str>) -> StepResult {
        #[cfg(feature = "tracing")]
        tracing::trace!(
            machine = %self.ir.name,
            state = self.current_state.as_str(),
            event = ?event,
            elapsed_ms = self.elapsed.as_millis(),
            state_elapsed_ms = self.state_elapsed.as_millis(),
            "state machine step start"
        );

        // Check step limit
        if self.step_count >= self.config.max_steps {
            return StepResult::new(&self.current_state)
                .with_outputs(vec![format!(
                    "error: step limit exceeded (max: {})",
                    self.config.max_steps
                )])
                .completed();
        }
        self.step_count += 1;

        if self.completed {
            #[cfg(feature = "tracing")]
            tracing::trace!(
                machine = %self.ir.name,
                state = self.current_state.as_str(),
                "state machine already completed"
            );
            return StepResult::new(&self.current_state).completed();
        }

        let mut outputs = Vec::new();
        let mut sends = Vec::new();
        let mut port_sends: Vec<(String, sysml_core::Value)> = Vec::new();

        // Check if the current state defers this event
        if let Some(ev) = event {
            if let Some(state_ir) = self.ir.find_state(&self.current_state) {
                if state_ir.deferred_events.iter().any(|d| d == ev) {
                    self.deferred_queue.push(ev.to_owned());
                    return StepResult::new(&self.current_state);
                }
            }
        }

        // Execute do-action for current state (runs each step while in state)
        if let Some(ext) = self.state_extensions.get(&self.current_state) {
            if let Some(do_action) = &ext.do_action {
                outputs.push(format!("do: {}", format_action(do_action)));
                sends.extend(execute_action(
                    do_action,
                    &mut self.eval_ctx,
                    &mut port_sends,
                ));
                self.do_action_executed = true;
            }
        }

        // Delegate to sub-runner if current state is composite
        let mut event_consumed = false;
        if let Some(sub) = self.sub_runners.get_mut(&self.current_state) {
            let prev_sub_state = {
                use crate::Runner;
                Runner::current_state(sub.as_ref()).to_owned()
            };
            let sub_result = {
                use crate::Runner;
                Runner::step(sub.as_mut(), event)
            };
            outputs.extend(sub_result.outputs);
            sends.extend(sub_result.sends);
            port_sends.extend(sub_result.port_sends);
            // Sync context back from sub-runner
            self.eval_ctx.merge_from(&sub.eval_ctx);

            if sub_result.state != prev_sub_state {
                event_consumed = true;
            }
            if sub_result.completed {
                // Sub-machine completed → enables completion transition on outer state
                self.do_action_executed = true;
            }
        }

        if event_consumed {
            // Sub-runner consumed the event — don't check outer transitions
            self.step_count += 1;
            let mut result = StepResult::new(&self.current_state)
                .with_outputs(outputs)
                .with_sends(sends)
                .with_port_sends(port_sends);
            if self.completed {
                result = result.completed();
            }
            return result;
        }

        // Find a matching transition, considering guards and triggers.
        // We need to collect info before mutating self.
        let transitions: Vec<(usize, String, Option<String>, Option<TransitionActionIR>)> = self
            .ir
            .transitions
            .iter()
            .enumerate()
            .filter(|(_, t)| t.from == self.current_state)
            .map(|(i, t)| (i, t.to.clone(), t.guard.clone(), t.action.clone()))
            .collect();

        let mut matched_to = None;
        let mut matched_guard = None;
        let mut matched_action = None;
        // The triggering transfer recorded on entry (incomingTransitionTrigger).
        // Only a *message* trigger (event / port message) qualifies — time- and
        // condition-based triggers (after/at/when) and completion/guard-only
        // transitions are not MessageTransfers (StatePerformances.kerml:48).
        let mut matched_trigger_event = None;

        for (idx, to, guard, action) in &transitions {
            // Check trigger/event match
            if !self.transition_matches(*idx, &self.ir.transitions[*idx], event) {
                continue;
            }

            // Check guard (compiled-once cache, RSC-2.4b)
            if let Some(g) = guard {
                if !self.evaluate_guard_at(*idx, g, event) {
                    continue;
                }
            }

            matched_to = Some(to.clone());
            matched_guard = guard.clone();
            matched_action = action.clone();

            let t = &self.ir.transitions[*idx];
            let is_message_trigger = match self
                .transition_extensions
                .get(*idx)
                .and_then(|ext| ext.trigger.as_ref())
            {
                Some(TriggerKind::Event(_) | TriggerKind::PortMessage { .. }) => true,
                Some(_) => false, // after / at / when are not MessageTransfers
                None => t.event.is_some() && !t.is_completion && !t.is_guard_only,
            };
            if is_message_trigger {
                matched_trigger_event = event.map(str::to_owned);
            }
            break;
        }

        // If no regular transition matched, check completion transitions.
        // Completion transitions fire only when the source state's do-activity has completed.
        let current_state = self.current_state.clone();
        if matched_to.is_none() && self.do_activity_completed() {
            for (idx, transition) in self.ir.transitions.iter().enumerate() {
                if transition.from == current_state && transition.is_completion {
                    let guard_ok = match &transition.guard {
                        Some(g) => self.evaluate_guard_at(idx, g, None),
                        None => true,
                    };
                    if guard_ok {
                        matched_to = Some(transition.to.clone());
                        matched_guard = transition.guard.clone();
                        matched_action = transition.action.clone();
                        break;
                    }
                }
            }
        }

        let transition_matched = matched_to.is_some();

        if let Some(to) = matched_to {
            let _ = matched_guard; // used above for evaluation

            let _from_state = self.current_state.clone();

            // Record history before exiting composite state
            if let Some(state_ir) = self.ir.find_state(&self.current_state) {
                if state_ir.history.is_some() {
                    if let Some(sub) = self.sub_runners.get(&self.current_state) {
                        self.state_history
                            .insert(self.current_state.clone(), sub.current_state_path());
                    }
                }
            }

            // Exit sub-runner if leaving a composite state
            if let Some(sub) = self.sub_runners.remove(&self.current_state) {
                // Run sub-state's exit action
                let sub_current = {
                    use crate::Runner;
                    Runner::current_state(sub.as_ref()).to_owned()
                };
                if let Some(sub_state) = sub.ir.find_state(&sub_current) {
                    if let Some(exit) = &sub_state.exit_action {
                        sends.extend(execute_action(exit, &mut self.eval_ctx, &mut port_sends));
                    }
                }
            }

            // Execute exit action of current state
            if let Some(state) = self.ir.find_state(&self.current_state) {
                if let Some(exit) = &state.exit_action {
                    outputs.push(format!("exit: {}", format_action(exit)));
                    sends.extend(execute_action(exit, &mut self.eval_ctx, &mut port_sends));
                }
            }

            // Execute transition action
            if let Some(action) = &matched_action {
                outputs.push(format!("action: {}", format_action(action)));
                sends.extend(execute_action(action, &mut self.eval_ctx, &mut port_sends));
            }

            // Move to new state
            self.current_state = to;
            self.state_elapsed = Duration::ZERO;
            self.do_action_executed = false;

            // Execute entry action of new state
            if let Some(state) = self.ir.find_state(&self.current_state) {
                if let Some(entry) = &state.entry_action {
                    outputs.push(format!("entry: {}", format_action(entry)));
                    sends.extend(execute_action(entry, &mut self.eval_ctx, &mut port_sends));
                }

                if state.is_final {
                    self.completed = true;
                }
            }

            // Create sub-runner for composite states
            if let Some(state_ir) = self.ir.find_state(&self.current_state) {
                if let Some(sub_machine) = &state_ir.sub_machine {
                    let mut sub = StateMachineRunner::new((**sub_machine).clone());
                    sub.eval_ctx = self.eval_ctx.alias_live();

                    // Check history for this composite state
                    let history_path = state_ir
                        .history
                        .and_then(|_| self.state_history.get(&self.current_state))
                        .cloned();

                    if let Some(path) = history_path {
                        match state_ir.history {
                            Some(crate::HistoryKind::Deep) => {
                                // Deep: restore the full nested path recursively
                                restore_deep_history(
                                    &mut sub,
                                    &path,
                                    &mut outputs,
                                    &mut sends,
                                    &mut port_sends,
                                );
                            }
                            _ => {
                                // Shallow: only restore the direct sub-state (path[0])
                                if let Some(target) = path.first() {
                                    sub.current_state = target.clone();
                                    if let Some(restored_state) =
                                        sub.ir.find_state(&sub.current_state)
                                    {
                                        if let Some(entry) = &restored_state.entry_action {
                                            let action_sends = execute_action(
                                                entry,
                                                &mut sub.eval_ctx,
                                                &mut port_sends,
                                            );
                                            outputs
                                                .push(format!("entry: {}", format_action(entry)));
                                            sends.extend(action_sends);
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        // Normal entry: step to initial sub-state
                        let entry_result = {
                            use crate::Runner;
                            Runner::step(&mut sub, None)
                        };
                        outputs.extend(entry_result.outputs);
                        sends.extend(entry_result.sends);
                        port_sends.extend(entry_result.port_sends);
                    }

                    self.eval_ctx.merge_from(&sub.eval_ctx);
                    self.sub_runners
                        .insert(self.current_state.clone(), Box::new(sub));
                }
            }

            #[cfg(feature = "tracing")]
            tracing::debug!(
                machine = %self.ir.name,
                from = _from_state.as_str(),
                to = self.current_state.as_str(),
                event = ?event,
                completed = self.completed,
                "state machine transition fired"
            );

            // Replay deferred events: try each queued event against the new state.
            // Events still deferred by the new state stay in the queue.
            if !self.deferred_queue.is_empty() {
                let queue = std::mem::take(&mut self.deferred_queue);
                let mut still_deferred = Vec::new();
                let new_state_defers: Vec<String> = self
                    .ir
                    .find_state(&self.current_state)
                    .map(|s| s.deferred_events.clone())
                    .unwrap_or_default();

                for deferred_ev in queue {
                    if new_state_defers.iter().any(|d| d == &deferred_ev) {
                        // New state also defers this event — keep it queued
                        still_deferred.push(deferred_ev);
                    } else {
                        // Try to process the deferred event
                        let replay_result = self.step_inner(Some(&deferred_ev));
                        outputs.extend(replay_result.outputs);
                        sends.extend(replay_result.sends);
                        port_sends.extend(replay_result.port_sends);
                        if replay_result.completed {
                            self.completed = true;
                        }
                    }
                }
                self.deferred_queue = still_deferred;
            }
        }

        let mut result = StepResult::new(&self.current_state)
            .with_outputs(outputs)
            .with_sends(sends)
            .with_port_sends(port_sends)
            .with_incoming_trigger(matched_trigger_event);

        // When an event was provided but no transition matched, surface available
        // transitions so callers (e.g. AI agents via MCP) know what events are valid.
        if !transition_matched && event.is_some() {
            result.available_transitions = self
                .available_transitions()
                .into_iter()
                .map(|(ev, tgt)| (ev.to_owned(), tgt.to_owned()))
                .collect();
        }

        if self.completed {
            result = result.completed();
        }

        #[cfg(feature = "tracing")]
        tracing::trace!(
            machine = %self.ir.name,
            state = self.current_state.as_str(),
            outputs = result.outputs.len(),
            completed = self.completed,
            "state machine step complete"
        );

        result
    }
}

impl Runner for StateMachineRunner {
    fn reset(&mut self) {
        self.current_state = self.ir.initial.clone();
        self.completed = false;
        self.eval_ctx = EvalContext::new();
        self.elapsed = Duration::ZERO;
        self.state_elapsed = Duration::ZERO;
        self.step_count = 0;
        self.do_action_executed = false;
        self.sub_runners.clear();
        self.state_history.clear();
        self.prev_when_values.lock().unwrap().clear();
        self.deferred_queue.clear();
        // RSC-2.4b: the bindings these keys tracked were just dropped with
        // eval_ctx. The compiled write-set and guard cache survive — they
        // are compile-time state, like the orchestrator's slot table.
        self.dynamic_keys.clear();
        // RSC-3.5b: payload bindings are equally per-run state.
        self.payload_keys.clear();
    }

    fn step(&mut self, event: Option<&str>) -> StepResult {
        let mut result = self.step_inner(event);

        // Run-to-completion: auto-fire null-event transitions until stable
        let mut chain_count = 0;
        const MAX_CHAIN: usize = 100;
        let mut visited = std::collections::HashSet::new();
        visited.insert(result.state.clone());

        while chain_count < MAX_CHAIN && !result.completed {
            let prev_state = result.state.clone();
            let step_count_before = self.step_count;
            let auto_result = self.step_inner(None);

            if auto_result.state == prev_state {
                // No transition fired — undo the step count increment and stop
                self.step_count = step_count_before;
                break;
            }

            // Cycle detection
            if visited.contains(&auto_result.state) {
                break; // Already visited this state — would loop
            }
            visited.insert(auto_result.state.clone());

            // Accumulate results
            result.state = auto_result.state;
            result.outputs.extend(auto_result.outputs);
            result.sends.extend(auto_result.sends);
            result.port_sends.extend(auto_result.port_sends);
            result.completed = auto_result.completed;
            chain_count += 1;
        }

        result
    }

    fn current_state(&self) -> &str {
        &self.current_state
    }

    fn is_completed(&self) -> bool {
        self.completed
    }
}

// ---------------------------------------------------------------------------
// Executor trait implementation (Phase 3)
// ---------------------------------------------------------------------------

impl crate::orchestrator::Executor for StateMachineRunner {
    fn phase(&self) -> crate::orchestrator::ExecutionPhase {
        crate::orchestrator::ExecutionPhase::StateMachine
    }

    fn kind_label(&self) -> &'static str {
        "stateMachine"
    }

    fn rewrite_when_to_located(&mut self, index: usize, event_name: &str) -> bool {
        StateMachineRunner::rewrite_when_to_located(self, index, event_name)
    }

    fn tick(
        &mut self,
        ctx: &crate::orchestrator::TickContext<'_>,
    ) -> crate::orchestrator::TickOutput {
        // RSC-3.6 step (2): on the scoped-view bypass path the orchestrator
        // hands this runner the thin slot-read view (no variable map), so the
        // instance-local names `execute_action` reads by bare key are absent
        // from `eval_ctx`. Re-seed exactly the compile-enumerated action reads
        // from the slots — the bounded subset of the scoped clone the actions
        // touch — keeping `execute_action` byte-identical to the scoped-clone
        // path while reading/writing one coherent surface (`eval_ctx`). No-op
        // off the bypass path (seed empty) and when a slot is unreadable
        // (`get_slot` → `None`). Runs before the guard-only check and step so
        // the action's read environment is in place before it executes.
        if self.bypass_scoped {
            for (name, slot) in &self.action_seed {
                if let Some(value) = self.eval_ctx.get_slot(*slot) {
                    self.eval_ctx.set(name.clone(), value);
                }
            }
        }

        // Override local clock time if present
        if let Some(local_t) = ctx.local_clock_time {
            self.eval_ctx
                .set("__clock_time".to_owned(), sysml_core::Value::Float(local_t));
            // RSC-2.4b: runtime-dynamic write — published through the
            // name-keyed fallback (not compile-enumerable as a slot claim).
            self.dynamic_keys.insert("__clock_time".to_owned());
        }

        // Bind port event payloads into eval context (Phase 12)
        for (port_name, payload) in ctx.port_payloads {
            let dot_key = format!("{}.payload", port_name);
            let underscore_key = format!("{}_payload", port_name);
            self.eval_ctx.set(dot_key.clone(), payload.clone());
            self.eval_ctx.set(underscore_key.clone(), payload.clone());
            // RSC-3.5b: the receiving port SET is compile-static, so these
            // keys are pre-minted as `{port}.payload` Discrete slots and
            // routed by SlotId at writeback time (drained out of the
            // `dynamic_keys` name-keyed fallback). Only the message VALUE is
            // runtime-dynamic — which is exactly what the routed slot stores.
            self.payload_keys.insert(dot_key);
            self.payload_keys.insert(underscore_key);
            // A-scalar: bind the accept-parameter name (`accept <name> via
            // <port>`) to the same delivered payload Value, so a guard/effect
            // can read it by bare name (`if cmd`, `cmd.field`). Spec: the
            // accepted transfer's payload binds the accept parameter
            // (Transfers.kerml:254-266).
            if let Some(param) = self.accept_param_for_port(port_name) {
                self.eval_ctx.set(param.clone(), payload.clone());
                self.payload_keys.insert(param);
            }
        }

        // Advance internal clock for time-based triggers
        self.advance_time(std::time::Duration::from_secs_f64(ctx.dt));

        // Guard-only transitions: fire when guard becomes true (no event needed)
        // L26: capture the sends the forced move runs (e.g. the entered state's
        // entry `send … via <port>`) — previously discarded, so a guard-only
        // transition into a sending state never routed.
        let mut forced_sends: Vec<String> = Vec::new();
        let mut forced_port_sends: Vec<(String, sysml_core::Value)> = Vec::new();
        if ctx.event.is_none() {
            let current = self.current_state.clone();
            for (idx, t) in self.ir.transitions.iter().enumerate() {
                if t.from != current {
                    continue;
                }
                if !t.is_guard_only {
                    continue;
                }
                let Some(guard) = &t.guard else { continue };
                if self.evaluate_guard_at(idx, guard, None) {
                    let (fs, fps) = self.force_transition(&t.to.clone(), t.action.clone());
                    forced_sends = fs;
                    forced_port_sends = fps;
                    break;
                }
            }
        }

        // RSC-4.3: the `max_steps` budget guards the WITHIN-tick run-to-
        // completion auto-chain (`Runner::step` fires null-event transitions
        // until stable — see the MAX_CHAIN loop), NOT the number of
        // orchestrator ticks. The orchestrator drives this executor once per
        // tick, so left un-reset `step_count` accumulates one per tick and
        // trips `step_count >= max_steps` (default 10_000) after 10_000 ticks —
        // silently freezing EVERY transition (the located zero-crossing event
        // is delivered but `step_inner` early-returns "step limit exceeded"
        // before evaluating it). That dropped `accept when` crossings on any
        // run exceeding 10_000 ticks (e.g. fine-dt hybrid models). Reset the
        // per-tick budget here so each orchestrator tick gets a fresh
        // run-to-completion allowance; direct `Runner::step` callers (the
        // standalone step-limit unit tests) are unaffected — they never route
        // through this executor `tick()`.
        self.step_count = 0;

        // Step the state machine
        let mut result = Runner::step(self, ctx.event);
        // Merge guard-only forced-transition sends ahead of this tick's step sends.
        if !forced_sends.is_empty() {
            forced_sends.extend(result.sends);
            result.sends = forced_sends;
        }
        // Same for the port-addressed payload sends (the Value-carrying channel).
        if !forced_port_sends.is_empty() {
            forced_port_sends.extend(result.port_sends);
            result.port_sends = forced_port_sends;
        }

        let avail = self
            .available_transitions()
            .into_iter()
            .map(|(ev, tgt)| (ev.to_owned(), tgt.to_owned()))
            .collect();

        crate::orchestrator::TickOutput {
            current_state: result.state,
            completed: result.completed,
            available_transitions: avail,
            outputs: result.outputs,
            sends: result.sends,
            port_sends: result.port_sends,
            messages: Vec::new(),
            addressed_messages: Vec::new(),
            incoming_trigger: result.incoming_trigger,
        }
    }

    fn accept_ports(&self) -> Vec<String> {
        StateMachineRunner::accept_ports(self)
    }

    fn armed_accept_ports(&self) -> Vec<String> {
        StateMachineRunner::armed_accept_ports(self)
    }

    fn reset_executor(&mut self) {
        Runner::reset(self);
    }

    fn is_completed(&self) -> bool {
        self.completed
    }

    fn clone_boxed(&self) -> Box<dyn crate::orchestrator::Executor> {
        Box::new(self.clone())
    }

    fn sync_context_in(&mut self, shared: &EvalContext) {
        self.eval_ctx.merge_from(shared);
    }

    /// RSC-2.4b: slot-routed writeback restricted to the SM's compiled
    /// write-set. Replaces the legacy whole-context diff — which, for
    /// prefixed instances, republished EVERY internal-context key under
    /// `{prefix}.*` (echoing merged globals like `t_ms` into per-instance
    /// keys nothing declared). The restricted set is:
    ///
    /// - **compiled assignment targets** ([`collect_assignment_targets`]):
    ///   the only context keys [`execute_action`] can write. Published
    ///   through precomputed [`WriteRoute`](crate::slots::WriteRoute)s —
    ///   by `SlotId` where the slot table claims the target for this
    ///   executor (set_slot dual-spelling mirror), name-keyed otherwise.
    ///   A target the SM has not (yet) assigned still re-publishes its
    ///   merged value — idempotent on the map, matching the legacy diff's
    ///   final state byte-for-byte.
    /// - **port-payload keys** (`{port}.payload`/`{port}_payload`) the runner
    ///   bound in `tick()`: RSC-3.5b routes them through precomputed
    ///   [`payload_routes`](SmWriteSet::payload_routes) (the receiving ports
    ///   are compile-static) — by `SlotId` where the pre-minted Discrete
    ///   payload slot is claimed, name-keyed otherwise. Drained out of the
    ///   `dynamic_keys` fallback class.
    /// - **runtime-dynamic keys** the runner itself bound in `tick()`
    ///   (local-clock `__clock_time` only, RSC-3.5b): name-keyed fallback
    ///   with the exact legacy prefix formatting, reported via
    ///   [`slot_write_fallbacks`](Self::slot_write_fallbacks). `__clock_time`
    ///   stays here deliberately — it is a runtime-dynamic local-clock key,
    ///   not a payload, and is a Phase-4 item (design doc §8 caveat (d)).
    fn sync_context_out_slots(
        &self,
        shared: &mut EvalContext,
        _mode: crate::ode::SignalEvalMode,
    ) -> bool {
        let Some(ws) = &self.write_set else {
            return false;
        };
        for (bare, route) in &ws.targets {
            if let Some(v) = self.eval_ctx.get(bare) {
                route.apply(shared, v.clone());
            }
        }
        // RSC-3.5b: route port-payload keys through their pre-minted slots.
        // Port payloads are RUNTIME-DYNAMIC (bound from a delivery in `tick`),
        // not compile-claimed state — a payload whose `{port}.payload` slot was
        // never minted legitimately name-keys through `apply_name_keyed` (the
        // sanctioned runtime-dynamic category, alongside physics port/flow and
        // ODE signal outputs), so map parity holds even before/without minting.
        // A minted payload slot still routes by `SlotId`.
        for (key, route) in &ws.payload_routes {
            if let Some(v) = self.eval_ctx.get(key) {
                route.apply_name_keyed(shared, v.clone());
            }
        }
        for key in &self.dynamic_keys {
            if let Some(v) = self.eval_ctx.get(key) {
                match &ws.dot_prefix {
                    Some(dot) => {
                        shared.set(format!("{dot}{key}"), v.clone());
                        if let Some(canonical) = &ws.canonical_dot {
                            shared.set(format!("{canonical}{key}"), v.clone());
                        }
                    }
                    None => shared.set(key.clone(), v.clone()),
                }
            }
        }
        true
    }

    /// RSC-2.4b: build the precomputed slot write-set from the compiled
    /// assignment targets and latch the scoped-clone bypass eligibility.
    fn prepare_slot_writeback(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
        canonical_prefix: Option<&str>,
        writer: crate::slots::WriterId,
    ) {
        let targets: Vec<(String, crate::slots::WriteRoute)> = collect_assignment_targets(&self.ir)
            .into_iter()
            .map(|target| {
                let route = crate::slots::WriteRoute::resolve(
                    store,
                    var_prefix,
                    canonical_prefix,
                    writer,
                    &target,
                );
                (target, route)
            })
            .collect();
        // RSC-3.5b: resolve a route per compile-static port-payload key. The
        // receiving ports are `accept_ports()` (PortMessage triggers); the
        // compiler pre-mints a `{port}.payload` Discrete slot per port owned
        // by this SM executor, so `resolve` (the hard-assert variant) routes
        // them by SlotId and they leave the `dynamic_keys` fallback class.
        let payload_routes: Vec<(String, crate::slots::WriteRoute)> = self
            .accept_ports()
            .into_iter()
            .flat_map(|port| [format!("{port}.payload"), format!("{port}_payload")])
            .map(|key| {
                let route = crate::slots::WriteRoute::resolve(
                    store,
                    var_prefix,
                    canonical_prefix,
                    writer,
                    &key,
                );
                (key, route)
            })
            .collect();
        self.write_set = Some(SmWriteSet {
            targets,
            dot_prefix: var_prefix.map(|p| format!("{p}.")),
            canonical_dot: canonical_prefix
                .filter(|c| var_prefix != Some(*c))
                .map(|c| format!("{c}.")),
            payload_routes,
        });
        self.bypass_scoped = var_prefix.is_some() && self.scoped_bypass_eligible(store, var_prefix);
    }

    fn scoped_view_bypass(&self) -> bool {
        self.bypass_scoped
    }

    /// RSC-2.4b: writeback keys still on the name-keyed fallback —
    /// compiled targets whose slot route was refused (unminted target,
    /// canonical-spelling mismatch for instance-scoped SMs, foreign
    /// placeholder writer) plus the runtime-dynamic keys. Observability
    /// hook for the RSC-2.5 deletion gate, surfaced through
    /// `Orchestrator::sm_slot_fallbacks`.
    ///
    /// RSC-3.5b: port-payload routes that resolved to a slot are NO LONGER
    /// reported (drained); only an UNROUTED payload route (slot not minted /
    /// canonical mismatch) reports. `__clock_time` (the lone remaining
    /// `dynamic_keys` member when a local clock is active) is NOT a payload
    /// and stays in the fallback set — a Phase-4 item (design doc §8 (d)).
    fn slot_write_fallbacks(&self) -> Vec<String> {
        let Some(ws) = &self.write_set else {
            return Vec::new();
        };
        let mut out: Vec<String> = ws
            .targets
            .iter()
            .filter(|(_, route)| !route.is_routed())
            .map(|(_, route)| route.runtime_key().to_owned())
            .collect();
        // RSC-3.5b: only payload routes still on the name-keyed path report.
        for (_, route) in &ws.payload_routes {
            if !route.is_routed() {
                out.push(route.runtime_key().to_owned());
            }
        }
        for key in &self.dynamic_keys {
            out.push(match &ws.dot_prefix {
                Some(dot) => format!("{dot}{key}"),
                None => key.clone(),
            });
        }
        out
    }

    /// A2 / RS005: ONLY the assignment `targets` whose strict-`apply` route
    /// failed to mint a slot — the subset of `slot_write_fallbacks` that would
    /// silently drop in release. Deliberately excludes `payload_routes` (the
    /// name-keyed `apply_name_keyed` port-payload path, L34-gated) and
    /// `dynamic_keys` (`__clock_time` — runtime-dynamic, not a mint gap).
    fn unrouted_slot_writes(&self) -> Vec<String> {
        let Some(ws) = &self.write_set else {
            return Vec::new();
        };
        ws.targets
            .iter()
            .filter(|(_, route)| !route.is_routed())
            .map(|(_, route)| route.runtime_key().to_owned())
            .collect()
    }

    /// RSC-2.4b (closes the RSC-2.3 deferred item): bind the compiled
    /// guard cache and the persistent When/AfterExpr trigger expressions
    /// to slots in the subsystem-local scope. Evaluation stays
    /// context-name-first, so verdicts are unchanged wherever the name is
    /// present in the internal context; the slot serves the read when it
    /// is not (the precondition for the scoped-view bypass).
    ///
    /// The returned report intentionally clears `unresolved`: guard
    /// strings carry event-name fallback semantics (`event == guard`), so
    /// a name that resolves nowhere is legal guard input, not an RS003
    /// candidate. The unfiltered report stays available through
    /// [`guard_bind_report`](Self::guard_bind_report).
    ///
    /// Sub-runners minted later for composite states keep compiled-but-
    /// unbound caches (identical verdicts through the context-name path).
    fn bind_expression_slots(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
    ) -> crate::expressions::BindReport {
        use crate::expressions::{bind_slots, BindReport, SlotBinder};

        let binder = SlotBinder::for_subsystem(store, var_prefix);
        let mut report = BindReport::default();
        for expr in self.guard_cache.iter_mut().flatten() {
            bind_slots(expr, &binder, &mut report);
        }
        for ext in &mut self.transition_extensions {
            match &mut ext.trigger {
                Some(TriggerKind::When(expr)) | Some(TriggerKind::AfterExpr(expr)) => {
                    bind_slots(expr, &binder, &mut report);
                }
                _ => {}
            }
        }

        // RSC-3.6 step (2): collect the instance-local slot seed for
        // structured-action reads. Unlike guards (rewritten to `SlotRef` and
        // read by-`SlotId` at eval time), `execute_action` reads/writes by
        // bare context name against `eval_ctx` so within-action
        // read-after-write stays coherent on one surface. Each bare name an
        // action reads must therefore resolve to an instance-local slot — the
        // same `resolve_subsystem_local` test the bypass guards use — so the
        // thin-view tick can re-seed it from the slots. A read that is a chain
        // (`name.contains('.')`, not a bare-name `FeatureRef` the var map can
        // serve) or resolves only through the binder's global fall-through
        // leaves the SM bypass-INELIGIBLE: it stays on the scoped-clone path.
        let mut seed: Vec<(String, crate::slots::SlotId)> = Vec::new();
        let mut all_seedable = true;
        for name in self.action_slot_reads() {
            match binder.resolve_subsystem_local(&name) {
                Some(slot) if !name.contains('.') => seed.push((name, slot)),
                _ => all_seedable = false,
            }
        }
        seed.sort();
        seed.dedup();
        self.action_seed = seed;
        self.actions_slot_seedable = all_seedable;

        self.guard_bind_report = report.clone();
        let mut public = report;
        public.unresolved.clear();
        public
    }

    /// RSC-4.1: read-set = the compiler-resolved slots this runner reads at
    /// tick time, harvested from its already-bound IR (after
    /// [`bind_expression_slots`](Self::bind_expression_slots)):
    /// - every compiled guard expression's `SlotRef` / `SlotChainHead`,
    /// - every persistent `When` / `AfterExpr` trigger expression's slots,
    /// - the instance-local slots of structured-action reads
    ///   (`action_seed`, the `(name, SlotId)` pairs the bind pass resolved
    ///   subsystem-local for `execute_action`'s by-name reads).
    ///
    /// Mirrors the name-level `guard_trigger_reads` + `action_slot_reads`
    /// surface, but returns the resolved slot identities instead of names
    /// (§9 Q2). Empty before binding / for uncompilable guards (event-name
    /// comparison reads no context).
    fn read_slots(&self) -> Vec<crate::slots::SlotId> {
        let mut v = Vec::new();
        for expr in self.guard_cache.iter().flatten() {
            v.extend(expr.slot_reads());
        }
        for ext in &self.transition_extensions {
            match &ext.trigger {
                Some(TriggerKind::When(expr)) | Some(TriggerKind::AfterExpr(expr)) => {
                    v.extend(expr.slot_reads());
                }
                _ => {}
            }
        }
        for (_, slot) in &self.action_seed {
            v.push(*slot);
        }
        v.sort();
        v.dedup();
        v
    }

    fn current_state_name(&self) -> &str {
        &self.current_state
    }

    fn diagnose_guards(&self, event: Option<&str>) -> Vec<GuardDiagnosis> {
        self.diagnose_guards(event)
    }

    fn eval_context(&self) -> Option<&EvalContext> {
        Some(&self.eval_ctx)
    }

    fn transitions(&self) -> Option<&[crate::TransitionIR]> {
        Some(&self.ir.transitions)
    }

    fn deferred_event_count(&self) -> usize {
        self.deferred_queue.len()
    }
}

/// Recursively restore deep history by walking the saved state path.
///
/// `path` is the full state path from `current_state_path()`, e.g. `["A", "B", "C"]`.
/// This sets the runner's current state to `path[0]`, creates a sub-runner for it
/// if it's composite, sets that sub-runner's state to `path[1]`, and so on.
fn restore_deep_history(
    runner: &mut StateMachineRunner,
    path: &[String],
    outputs: &mut Vec<String>,
    sends: &mut Vec<String>,
    port_sends: &mut Vec<(String, sysml_core::Value)>,
) {
    if path.is_empty() {
        return;
    }

    runner.current_state = path[0].clone();

    // Execute entry action of restored state
    if let Some(state_ir) = runner.ir.find_state(&runner.current_state) {
        if let Some(entry) = &state_ir.entry_action {
            let action_sends = execute_action(entry, &mut runner.eval_ctx, port_sends);
            outputs.push(format!("entry: {}", format_action(entry)));
            sends.extend(action_sends);
        }

        // If there's more path to restore AND this state is composite, recurse
        if path.len() > 1 {
            if let Some(sub_machine) = &state_ir.sub_machine {
                let mut sub = StateMachineRunner::new((**sub_machine).clone());
                sub.eval_ctx = runner.eval_ctx.alias_live();
                restore_deep_history(&mut sub, &path[1..], outputs, sends, port_sends);
                runner.eval_ctx.merge_from(&sub.eval_ctx);
                runner
                    .sub_runners
                    .insert(runner.current_state.clone(), Box::new(sub));
            }
        }
    }
}

/// Parse time/condition triggers from event strings.
/// Recognizes "after(5s)", "after(500ms)", "after 5s", "when(x > 10)".
fn parse_trigger_from_event(event: &str, accept_param: Option<&str>) -> Option<TriggerKind> {
    let trimmed = event.trim();

    // Handle "accept via <port>" — a port-message trigger (`accept ... via P`).
    // The parser (ast_builder/states.rs) lowers the grammar's `via_port` field
    // to this canonical string. SPEC-SILENT: the spec keys the trigger on the
    // receiver Occurrence (TransitionPerformances.kerml:28-46) and requires
    // payload-type conformance (SysML-vocab.ttl:2495); we approximate that with
    // a port-NAME string match (`PortMessage` fires when a delivered message's
    // name equals `port_name`) and defer payload-type matching (`payload_type`
    // left None — L34/RSC-3.5b follow-up). The port-name match is what the
    // ExchangePlane acceptor registration keys on (`accept_ports`).
    if let Some(port) = trimmed.strip_prefix("accept via ") {
        let port_name = port.trim();
        if !port_name.is_empty() {
            return Some(TriggerKind::PortMessage {
                port_name: port_name.to_owned(),
                payload_type: None,
                param_name: accept_param.map(str::to_owned),
            });
        }
    }

    // Handle "after(...)" format — try literal duration first, then expression
    if trimmed.starts_with("after(") && trimmed.ends_with(')') {
        let inner = trimmed[6..trimmed.len() - 1].trim();
        if let Some(duration) = parse_duration_literal(inner) {
            return Some(TriggerKind::After(duration));
        }
        // Not a literal — try as an expression (e.g., "after(t_dead)")
        if let Ok(expr) = compile_simple_expression(inner) {
            return Some(TriggerKind::AfterExpr(expr));
        }
    }

    // Handle "after Ns" or "after Nms" format (space-separated)
    if let Some(stripped) = trimmed.strip_prefix("after ") {
        let inner = stripped.trim();
        if let Some(duration) = parse_duration_literal(inner) {
            return Some(TriggerKind::After(duration));
        }
        // Not a literal — try as expression
        if let Ok(expr) = compile_simple_expression(inner) {
            return Some(TriggerKind::AfterExpr(expr));
        }
    }

    // Handle "at(...)" format — absolute time trigger (Phase 15F)
    if trimmed.starts_with("at(") && trimmed.ends_with(')') {
        let inner = trimmed[3..trimmed.len() - 1].trim();
        // Parse as duration literal then convert to seconds
        if let Some(duration) = parse_duration_literal(inner) {
            return Some(TriggerKind::At(duration.as_secs_f64()));
        }
        // Also accept bare float as seconds
        if let Ok(secs) = inner.parse::<f64>() {
            return Some(TriggerKind::At(secs));
        }
    }

    // Handle "when(...)" format (parenthesized)
    if trimmed.starts_with("when(") && trimmed.ends_with(')') {
        let inner = trimmed[5..trimmed.len() - 1].trim();
        if let Ok(expr) = compile_simple_expression(inner) {
            return Some(TriggerKind::When(expr));
        }
    }

    // Handle "when expr" format (space-separated, from PEG parser)
    if let Some(stripped) = trimmed.strip_prefix("when ") {
        let inner = stripped.trim();
        if let Ok(expr) = compile_simple_expression(inner) {
            return Some(TriggerKind::When(expr));
        }
    }

    None
}

/// Parse a duration literal: "5s", "500ms", "5000" (bare number = milliseconds)
fn parse_duration_literal(s: &str) -> Option<Duration> {
    let s = s.trim();
    // Check multi-char suffixes first (longest match)
    if let Some(ns) = s.strip_suffix("ns") {
        return ns
            .trim()
            .parse::<f64>()
            .ok()
            .map(|n| Duration::from_secs_f64(n * 1e-9));
    }
    if let Some(us) = s.strip_suffix("us").or_else(|| s.strip_suffix("μs")) {
        return us
            .trim()
            .parse::<f64>()
            .ok()
            .map(|n| Duration::from_secs_f64(n * 1e-6));
    }
    if let Some(ms) = s.strip_suffix("ms") {
        return ms
            .trim()
            .parse::<f64>()
            .ok()
            .map(|n| Duration::from_secs_f64(n * 1e-3));
    }
    if let Some(secs) = s.strip_suffix('s') {
        return secs.trim().parse::<f64>().ok().map(Duration::from_secs_f64);
    }
    // Bare number: treat as seconds (float) for scientific notation like 5.6e-7
    s.parse::<f64>().ok().map(Duration::from_secs_f64)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_core::{Element, Relationship};

    fn create_traffic_light_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();

        // Create state machine
        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("TrafficLight");
        let sm_id = graph.add_element(sm);

        // Create states
        let red = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Red")
            .with_owner(sm_id.clone())
            .with_prop("initial", true);
        let red_id = graph.add_element(red);

        let green = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Green")
            .with_owner(sm_id.clone());
        let green_id = graph.add_element(green);

        let yellow = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Yellow")
            .with_owner(sm_id.clone());
        let yellow_id = graph.add_element(yellow);

        // Create transitions
        let t1 = Relationship::new(
            RelationshipKind::Transition,
            red_id.clone(),
            green_id.clone(),
        )
        .with_prop("event", "timer");
        graph.add_relationship(t1);

        let t2 = Relationship::new(RelationshipKind::Transition, green_id, yellow_id.clone())
            .with_prop("event", "timer");
        graph.add_relationship(t2);

        let t3 = Relationship::new(RelationshipKind::Transition, yellow_id, red_id)
            .with_prop("event", "timer");
        graph.add_relationship(t3);

        graph
    }

    #[test]
    fn compile_state_machine() {
        let graph = create_traffic_light_graph();
        let ir = StateMachineCompiler::compile(&graph).unwrap();

        assert_eq!(ir.name, "TrafficLight");
        assert_eq!(ir.states.len(), 3);
        assert_eq!(ir.transitions.len(), 3);
        assert_eq!(ir.initial, "Red");
    }

    #[test]
    fn compile_named_state_machine() {
        let mut graph = ModelGraph::new();

        let sm_alpha = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("Alpha")
            .with_span(sysml_span::Span::new("file:///test.sysml", 10, 20));
        let sm_alpha_id = graph.add_element(sm_alpha);
        let alpha_state = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("A")
            .with_owner(sm_alpha_id)
            .with_prop("initial", true);
        graph.add_element(alpha_state);

        let sm_beta = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("Beta")
            .with_span(sysml_span::Span::new("file:///test.sysml", 30, 40));
        let sm_beta_id = graph.add_element(sm_beta);
        let beta_state = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("B")
            .with_owner(sm_beta_id)
            .with_prop("initial", true);
        graph.add_element(beta_state);

        let ir = StateMachineCompiler::compile_named(&graph, "Beta").unwrap();
        assert_eq!(ir.name, "Beta");

        let runner = StateMachineRunner::from_graph_named(&graph, "Alpha").unwrap();
        assert_eq!(runner.name(), "Alpha");
    }

    #[test]
    fn compile_initial_state_uses_source_order() {
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("Ordered");
        let sm_id = graph.add_element(sm);

        // Add "Late" first, but with a later source span.
        let late = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Late")
            .with_owner(sm_id.clone())
            .with_span(sysml_span::Span::new("file:///test.sysml", 100, 110));
        graph.add_element(late);

        // Add "Early" second, but with an earlier source span.
        let early = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Early")
            .with_owner(sm_id)
            .with_span(sysml_span::Span::new("file:///test.sysml", 10, 20));
        graph.add_element(early);

        let ir = StateMachineCompiler::compile(&graph).unwrap();
        assert_eq!(ir.initial, "Early");
    }

    #[test]
    fn runner_initial_state() {
        let graph = create_traffic_light_graph();
        let runner = StateMachineRunner::from_graph(&graph).unwrap();

        assert_eq!(runner.current_state(), "Red");
        assert!(!runner.is_completed());
    }

    #[test]
    fn runner_introspection_lists_states_and_transitions() {
        let graph = create_traffic_light_graph();
        let runner = StateMachineRunner::from_graph(&graph).unwrap();

        let states = runner.all_states();
        assert!(states.contains(&"Red"));
        assert!(states.contains(&"Green"));
        assert!(states.contains(&"Yellow"));

        let transitions = runner.all_transitions();
        assert_eq!(transitions.len(), 3);
        assert!(transitions
            .iter()
            .any(|(from, event, _guard, to)| *from == "Red"
                && *to == "Green"
                && *event == Some("timer")));
    }

    #[test]
    fn runner_step() {
        let graph = create_traffic_light_graph();
        let mut runner = StateMachineRunner::from_graph(&graph).unwrap();

        // Step with timer event
        let result = runner.step(Some("timer"));
        assert_eq!(result.state, "Green");

        // Another step
        let result = runner.step(Some("timer"));
        assert_eq!(result.state, "Yellow");

        // Back to red
        let result = runner.step(Some("timer"));
        assert_eq!(result.state, "Red");
    }

    #[test]
    fn runner_no_matching_event() {
        let graph = create_traffic_light_graph();
        let mut runner = StateMachineRunner::from_graph(&graph).unwrap();

        // Step with non-matching event
        let result = runner.step(Some("unknown"));
        assert_eq!(result.state, "Red"); // Should stay in Red
    }

    #[test]
    fn runner_reset() {
        let graph = create_traffic_light_graph();
        let mut runner = StateMachineRunner::from_graph(&graph).unwrap();

        runner.step(Some("timer"));
        assert_eq!(runner.current_state(), "Green");

        runner.reset();
        assert_eq!(runner.current_state(), "Red");
    }

    #[test]
    fn compile_no_state_machine() {
        let graph = ModelGraph::new();
        let result = StateMachineCompiler::compile(&graph);

        assert!(result.is_err());
        let diags = result.unwrap_err();
        assert!(diags[0].message.contains("No state machine"));
    }

    fn create_parallel_state_machine_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();

        // Create parallel state machine
        let sm = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("HybridSystem")
            .with_prop("isParallel", true);
        let sm_id = graph.add_element(sm);

        // Region 1: Grid
        let grid = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("grid")
            .with_owner(sm_id.clone());
        let grid_id = graph.add_element(grid);

        let grid_energized = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("energized")
            .with_owner(grid_id.clone())
            .with_prop("initial", true);
        let grid_energized_id = graph.add_element(grid_energized);

        let grid_deenergized = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("deEnergized")
            .with_owner(grid_id.clone());
        let grid_deenergized_id = graph.add_element(grid_deenergized);

        // Grid transitions
        let t1 = Relationship::new(
            RelationshipKind::Transition,
            grid_energized_id.clone(),
            grid_deenergized_id.clone(),
        )
        .with_prop("event", "gridFail");
        graph.add_relationship(t1);

        let t2 = Relationship::new(
            RelationshipKind::Transition,
            grid_deenergized_id,
            grid_energized_id,
        )
        .with_prop("event", "gridRestore");
        graph.add_relationship(t2);

        // Region 2: Relay
        let relay = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("relay")
            .with_owner(sm_id.clone());
        let relay_id = graph.add_element(relay);

        let relay_closed = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("closed")
            .with_owner(relay_id.clone())
            .with_prop("initial", true);
        let relay_closed_id = graph.add_element(relay_closed);

        let relay_open = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("open")
            .with_owner(relay_id.clone());
        let relay_open_id = graph.add_element(relay_open);

        // Relay transitions
        let t3 = Relationship::new(
            RelationshipKind::Transition,
            relay_closed_id.clone(),
            relay_open_id.clone(),
        )
        .with_prop("event", "gridFail")
        .with_prop("action", "t += 20");
        graph.add_relationship(t3);

        let t4 = Relationship::new(RelationshipKind::Transition, relay_open_id, relay_closed_id)
            .with_prop("event", "gridRestore");
        graph.add_relationship(t4);

        graph
    }

    #[test]
    fn compile_parallel_state_machine() {
        let graph = create_parallel_state_machine_graph();
        let ir = StateMachineCompiler::compile(&graph).unwrap();

        assert_eq!(ir.name, "HybridSystem");
        assert!(ir.is_parallel());
        assert_eq!(ir.regions.len(), 2);

        // Check grid region
        let grid_region = ir.find_region("grid").unwrap();
        assert_eq!(grid_region.initial, "energized");
        assert_eq!(grid_region.states.len(), 2);
        assert!(grid_region.find_state("energized").is_some());
        assert!(grid_region.find_state("deEnergized").is_some());

        // Check relay region
        let relay_region = ir.find_region("relay").unwrap();
        assert_eq!(relay_region.initial, "closed");
        assert_eq!(relay_region.states.len(), 2);
    }

    #[test]
    fn parallel_runner_from_compiled_graph() {
        let graph = create_parallel_state_machine_graph();
        let ir = StateMachineCompiler::compile(&graph).unwrap();
        let mut runner = ParallelStateMachineRunner::new(ir);

        // Initial states
        assert_eq!(runner.region_state("grid"), Some("energized"));
        assert_eq!(runner.region_state("relay"), Some("closed"));

        // Send gridFail event
        runner.send("gridFail");

        // Both regions should transition
        assert_eq!(runner.region_state("grid"), Some("deEnergized"));
        assert_eq!(runner.region_state("relay"), Some("open"));

        // Check timing context was updated
        assert_eq!(runner.get_context("t"), Some(20.0));
    }

    #[test]
    fn parallel_runner_restore() {
        let graph = create_parallel_state_machine_graph();
        let ir = StateMachineCompiler::compile(&graph).unwrap();
        let mut runner = ParallelStateMachineRunner::new(ir);

        // Fail then restore
        runner.send("gridFail");
        runner.send("gridRestore");

        assert_eq!(runner.region_state("grid"), Some("energized"));
        assert_eq!(runner.region_state("relay"), Some("closed"));
    }

    #[test]
    fn action_parsing_in_compiled_transitions() {
        let graph = create_parallel_state_machine_graph();
        let ir = StateMachineCompiler::compile(&graph).unwrap();

        // Find relay region and check transition action
        let relay_region = ir.find_region("relay").unwrap();
        let transitions = relay_region.transitions_from("closed");
        assert_eq!(transitions.len(), 1);

        let transition = transitions[0];
        assert!(transition.action.is_some());

        // The action should be parsed as structured
        if let Some(TransitionActionIR::Structured { assignments, .. }) = &transition.action {
            assert_eq!(assignments.len(), 1);
            assert_eq!(assignments[0].variable, "t");
            assert_eq!(assignments[0].value, sysml_core::Value::Float(20.0));
        } else {
            panic!("Expected structured action");
        }
    }

    #[test]
    fn compile_state_with_do_action() {
        let mut graph = ModelGraph::new();
        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("SM");
        let sm_id = graph.add_element(sm);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("monitoring")
            .with_owner(sm_id.clone())
            .with_prop("initial", true)
            .with_prop("entry", "init()")
            .with_prop("do_action", "check_sensors()")
            .with_prop("exit", "cleanup()");
        graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("done")
            .with_owner(sm_id)
            .with_prop("final", true);
        let s2_id = graph.add_element(s2);

        // Add a transition
        let s1_id = graph
            .elements
            .values()
            .find(|e| e.name.as_deref() == Some("monitoring"))
            .unwrap()
            .id
            .clone();
        let t = Relationship::new(RelationshipKind::Transition, s1_id, s2_id)
            .with_prop("event", "stop");
        graph.add_relationship(t);

        let ir = StateMachineCompiler::compile(&graph).unwrap();
        let monitoring = ir.find_state("monitoring").unwrap();
        assert!(monitoring.do_action.is_some());
        assert_eq!(
            monitoring.do_action.as_ref().unwrap().as_simple(),
            Some("check_sensors()")
        );

        // Also test that runner auto-populates do_action from IR
        let mut runner = StateMachineRunner::new(ir);
        let result = runner.step(None);
        assert!(result
            .outputs
            .iter()
            .any(|o| o.contains("do: check_sensors()")));
    }

    // ===================================================================
    // Phase 1: Expression guard tests
    // ===================================================================

    /// Helper: creates a simple 2-state machine with a guarded transition.
    fn create_guarded_ir(guard: &str) -> StateMachineIR {
        StateMachineIR::new("GuardedMachine", "idle")
            .with_state(StateIR::new("idle"))
            .with_state(StateIR::new("active"))
            .with_transition(
                TransitionIR::new("idle", "active")
                    .with_event("go")
                    .with_guard(guard),
            )
    }

    #[test]
    fn guard_expr_simple_comparison() {
        let ir = create_guarded_ir("speed < 100");
        let mut runner = StateMachineRunner::new(ir);
        runner.eval_ctx.set("speed", Value::Float(85.0));

        let result = runner.step(Some("go"));
        assert_eq!(
            result.state, "active",
            "guard speed < 100 should pass when speed=85"
        );
    }

    #[test]
    fn guard_expr_boolean_and() {
        let ir = create_guarded_ir("x > 0 and y > 0");
        let mut runner = StateMachineRunner::new(ir);
        runner.eval_ctx.set("x", Value::Int(5));
        runner.eval_ctx.set("y", Value::Int(3));

        let result = runner.step(Some("go"));
        assert_eq!(result.state, "active", "guard x>0 and y>0 should pass");
    }

    #[test]
    fn compile_guard_from_graph() {
        // Build a graph with a guarded transition
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("GuardedSM");
        let sm_id = graph.add_element(sm);

        let s1 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("idle")
            .with_owner(sm_id.clone())
            .with_prop("initial", true);
        let s1_id = graph.add_element(s1);

        let s2 = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("running")
            .with_owner(sm_id);
        let s2_id = graph.add_element(s2);

        let t = Relationship::new(RelationshipKind::Transition, s1_id, s2_id)
            .with_prop("event", "start")
            .with_prop("guard", "temp < 100");
        graph.add_relationship(t);

        let ir = StateMachineCompiler::compile(&graph).unwrap();
        let mut runner = StateMachineRunner::new(ir);
        runner.eval_ctx.set("temp", Value::Float(50.0));

        let result = runner.step(Some("start"));
        assert_eq!(result.state, "running");
    }

    #[test]
    fn runner_guard_blocks_transition() {
        let ir = create_guarded_ir("speed < 100");
        let mut runner = StateMachineRunner::new(ir);
        runner.eval_ctx.set("speed", Value::Float(120.0));

        let result = runner.step(Some("go"));
        assert_eq!(
            result.state, "idle",
            "guard speed < 100 should block when speed=120"
        );
    }

    #[test]
    fn runner_guard_allows_transition() {
        let ir = create_guarded_ir("speed < 100");
        let mut runner = StateMachineRunner::new(ir);
        runner.eval_ctx.set("speed", Value::Float(50.0));

        let result = runner.step(Some("go"));
        assert_eq!(
            result.state, "active",
            "guard speed < 100 should allow when speed=50"
        );
    }

    #[test]
    fn guard_string_backward_compat() {
        // When guard string can't be parsed as expression, fall back to string match
        let ir = create_guarded_ir("some_opaque_guard");
        let mut runner = StateMachineRunner::new(ir);

        // The string "some_opaque_guard" can't be compiled as an expression comparison.
        // However, it can be compiled as a FeatureRef. If the variable exists as a bool
        // in the eval context, it works. But if it doesn't, we fall back to string
        // matching against the event.
        // Since the event is "go" != "some_opaque_guard", this should NOT transition.
        let result = runner.step(Some("go"));
        assert_eq!(
            result.state, "idle",
            "opaque guard should fall back to string match"
        );

        // If event matches the guard string, it should transition (backward compat)
        let ir2 = create_guarded_ir("go");
        let mut runner2 = StateMachineRunner::new(ir2);
        let result2 = runner2.step(Some("go"));
        // "go" compiles as FeatureRef("go"), which won't be in context -> eval fails -> fallback
        // fallback: event "go" == guard "go" -> true
        assert_eq!(
            result2.state, "active",
            "guard matching event string should transition"
        );
    }

    // ===================================================================
    // Phase 2: Do-action tests
    // ===================================================================

    #[test]
    fn do_action_executes_while_in_state() {
        let ir = StateMachineIR::new("DoActionMachine", "active")
            .with_state(StateIR::new("active"))
            .with_state(StateIR::new("done"))
            .with_transition(TransitionIR::new("active", "done").with_event("finish"));

        let mut runner = StateMachineRunner::new(ir);
        runner.set_do_action("active", TransitionActionIR::simple("tick()"));

        // Step without a matching event — should stay in "active" and execute do-action
        let result = runner.step(None);
        assert_eq!(result.state, "active");
        assert!(result.outputs.iter().any(|o| o.contains("do: tick()")));

        // Step again — do-action fires again
        let result = runner.step(None);
        assert_eq!(result.state, "active");
        assert!(result.outputs.iter().any(|o| o.contains("do: tick()")));
    }

    #[test]
    fn do_action_interrupted_by_event() {
        let ir = StateMachineIR::new("DoInterruptMachine", "active")
            .with_state(StateIR::new("active"))
            .with_state(StateIR::new("done"))
            .with_transition(TransitionIR::new("active", "done").with_event("finish"));

        let mut runner = StateMachineRunner::new(ir);
        runner.set_do_action("active", TransitionActionIR::simple("tick()"));

        // Step with matching event — do-action fires, then transition happens
        let result = runner.step(Some("finish"));
        assert_eq!(result.state, "done");
        // Do-action should have run before the transition
        assert!(result.outputs.iter().any(|o| o.contains("do: tick()")));
    }

    #[test]
    fn state_with_do_action() {
        let ir = StateMachineIR::new("DoActionStateMachine", "monitoring")
            .with_state(StateIR::new("monitoring"))
            .with_state(StateIR::new("alarm"))
            .with_transition(TransitionIR::new("monitoring", "alarm").with_event("alert"));

        let mut runner = StateMachineRunner::new(ir);
        runner.set_do_action("monitoring", TransitionActionIR::simple("check_sensors()"));

        // First step: do-action runs, no transition
        let r1 = runner.step(None);
        assert_eq!(r1.state, "monitoring");
        assert!(r1.outputs.iter().any(|o| o.contains("do: check_sensors()")));

        // Second step: do-action runs, then transition via alert
        let r2 = runner.step(Some("alert"));
        assert_eq!(r2.state, "alarm");
        assert!(r2.outputs.iter().any(|o| o.contains("do: check_sensors()")));

        // Third step: no do-action for alarm state
        let r3 = runner.step(None);
        assert_eq!(r3.state, "alarm");
        assert!(!r3.outputs.iter().any(|o| o.contains("do:")));
    }

    // ===================================================================
    // Phase 3: Enhanced trigger tests
    // ===================================================================

    #[test]
    fn after_trigger_fires() {
        let ir = StateMachineIR::new("AfterTriggerMachine", "waiting")
            .with_state(StateIR::new("waiting"))
            .with_state(StateIR::new("timeout"))
            .with_transition(TransitionIR::new("waiting", "timeout"));

        let mut runner = StateMachineRunner::new(ir);
        // Set after(5s) trigger on the transition (index 0)
        runner.set_transition_trigger(0, TriggerKind::After(Duration::from_secs(5)));

        // Advance time past the threshold
        runner.advance_time(Duration::from_secs(6));

        let result = runner.step(None);
        assert_eq!(
            result.state, "timeout",
            "after(5s) trigger should fire after 6s"
        );
    }

    #[test]
    fn after_trigger_not_yet() {
        let ir = StateMachineIR::new("AfterTriggerMachine", "waiting")
            .with_state(StateIR::new("waiting"))
            .with_state(StateIR::new("timeout"))
            .with_transition(TransitionIR::new("waiting", "timeout"));

        let mut runner = StateMachineRunner::new(ir);
        runner.set_transition_trigger(0, TriggerKind::After(Duration::from_secs(5)));

        // Advance time but not past the threshold
        runner.advance_time(Duration::from_secs(3));

        let result = runner.step(None);
        assert_eq!(
            result.state, "waiting",
            "after(5s) trigger should NOT fire after 3s"
        );
    }

    #[test]
    fn when_trigger_condition_met() {
        let ir = StateMachineIR::new("WhenTriggerMachine", "idle")
            .with_state(StateIR::new("idle"))
            .with_state(StateIR::new("hot"))
            .with_transition(TransitionIR::new("idle", "hot"));

        let mut runner = StateMachineRunner::new(ir);
        let condition = compile_simple_expression("temp > 100").unwrap();
        runner.set_transition_trigger(0, TriggerKind::When(condition));
        runner.eval_ctx.set("temp", Value::Float(120.0));

        let result = runner.step(None);
        assert_eq!(
            result.state, "hot",
            "when(temp > 100) should fire when temp=120"
        );
    }

    #[test]
    fn when_trigger_condition_not_met() {
        let ir = StateMachineIR::new("WhenTriggerMachine", "idle")
            .with_state(StateIR::new("idle"))
            .with_state(StateIR::new("hot"))
            .with_transition(TransitionIR::new("idle", "hot"));

        let mut runner = StateMachineRunner::new(ir);
        let condition = compile_simple_expression("temp > 100").unwrap();
        runner.set_transition_trigger(0, TriggerKind::When(condition));
        runner.eval_ctx.set("temp", Value::Float(50.0));

        let result = runner.step(None);
        assert_eq!(
            result.state, "idle",
            "when(temp > 100) should NOT fire when temp=50"
        );
    }

    #[test]
    fn advance_time_fires_after_triggers() {
        let ir = StateMachineIR::new("TimedMachine", "start")
            .with_state(StateIR::new("start"))
            .with_state(StateIR::new("step1"))
            .with_state(StateIR::new("step2"))
            .with_transition(TransitionIR::new("start", "step1"))
            .with_transition(TransitionIR::new("step1", "step2"));

        let mut runner = StateMachineRunner::new(ir);
        runner.set_transition_trigger(0, TriggerKind::After(Duration::from_millis(100)));
        runner.set_transition_trigger(1, TriggerKind::After(Duration::from_millis(200)));

        // Not enough time yet
        runner.advance_time(Duration::from_millis(50));
        let r1 = runner.step(None);
        assert_eq!(r1.state, "start");

        // Enough for first trigger
        runner.advance_time(Duration::from_millis(60));
        let r2 = runner.step(None);
        assert_eq!(r2.state, "step1");
        // state_elapsed resets on transition
        assert_eq!(runner.state_elapsed(), Duration::ZERO);

        // Not enough for second trigger (state_elapsed just reset)
        runner.advance_time(Duration::from_millis(100));
        let r3 = runner.step(None);
        assert_eq!(r3.state, "step1");

        // Now enough for second trigger
        runner.advance_time(Duration::from_millis(150));
        let r4 = runner.step(None);
        assert_eq!(r4.state, "step2");
    }

    // ===================================================================
    // M2: Cross-crate integration — Expressions → State Machine Guards
    // ===================================================================

    #[test]
    fn statemachine_with_expression_guard() {
        // Verify that compound expression guards involving logical 'and'
        // and multiple variables are correctly evaluated by the state machine
        // runner through the expression evaluator (sysml-run-expressions).
        //
        // Guard: "temperature > 100 and pressure < maxPressure"
        // Context: temperature=105, pressure=90, maxPressure=150
        // → 105 > 100 = true AND 90 < 150 = true → guard passes → transition fires
        let ir = StateMachineIR::new("MultiGuardMachine", "normal")
            .with_state(StateIR::new("normal"))
            .with_state(StateIR::new("alert"))
            .with_transition(
                TransitionIR::new("normal", "alert")
                    .with_event("check")
                    .with_guard("temperature > 100 and pressure < maxPressure"),
            );

        let mut runner = StateMachineRunner::new(ir);
        runner.eval_ctx.set("temperature", Value::Float(105.0));
        runner.eval_ctx.set("pressure", Value::Float(90.0));
        runner.eval_ctx.set("maxPressure", Value::Float(150.0));

        let result = runner.step(Some("check"));
        assert_eq!(
            result.state, "alert",
            "guard 'temperature > 100 and pressure < maxPressure' should pass (105>100 and 90<150)"
        );

        // Reset and verify that a failing guard blocks the transition
        runner.reset();
        runner.eval_ctx.set("temperature", Value::Float(80.0)); // below threshold
        runner.eval_ctx.set("pressure", Value::Float(90.0));
        runner.eval_ctx.set("maxPressure", Value::Float(150.0));

        let result = runner.step(Some("check"));
        assert_eq!(
            result.state, "normal",
            "guard should block when temperature=80 (not > 100)"
        );

        // Also verify when pressure condition fails
        runner.reset();
        runner.eval_ctx.set("temperature", Value::Float(105.0));
        runner.eval_ctx.set("pressure", Value::Float(200.0)); // exceeds maxPressure
        runner.eval_ctx.set("maxPressure", Value::Float(150.0));

        let result = runner.step(Some("check"));
        assert_eq!(
            result.state, "normal",
            "guard should block when pressure=200 (not < 150)"
        );
    }

    // ===================================================================
    // F046: Step limit tests
    // ===================================================================

    #[test]
    fn step_limit_enforced() {
        let ir = StateMachineIR::new("StepLimitMachine", "a")
            .with_state(StateIR::new("a"))
            .with_state(StateIR::new("b"))
            .with_transition(TransitionIR::new("a", "b").with_event("go"))
            .with_transition(TransitionIR::new("b", "a").with_event("go"));

        let config = RunnerConfig { max_steps: 5 };
        let mut runner = StateMachineRunner::with_config(ir, config);

        // Run 5 steps (within limit)
        for _ in 0..5 {
            let result = runner.step(Some("go"));
            assert!(
                !result.outputs.iter().any(|o| o.contains("step limit")),
                "should not hit step limit within 5 steps"
            );
        }

        // 6th step should be blocked
        let result = runner.step(Some("go"));
        assert!(
            result.outputs.iter().any(|o| o.contains("step limit")),
            "6th step should report step limit exceeded"
        );
        assert!(
            result.completed,
            "runner should be marked completed after step limit"
        );
    }

    #[test]
    fn step_limit_reset() {
        let ir = StateMachineIR::new("StepLimitResetMachine", "a")
            .with_state(StateIR::new("a"))
            .with_state(StateIR::new("b"))
            .with_transition(TransitionIR::new("a", "b").with_event("go"))
            .with_transition(TransitionIR::new("b", "a").with_event("go"));

        let config = RunnerConfig { max_steps: 3 };
        let mut runner = StateMachineRunner::with_config(ir, config);

        // Exhaust the limit
        for _ in 0..3 {
            runner.step(Some("go"));
        }
        let result = runner.step(Some("go"));
        assert!(result.outputs.iter().any(|o| o.contains("step limit")));

        // Reset should clear the counter
        runner.reset();
        let result = runner.step(Some("go"));
        assert!(
            !result.outputs.iter().any(|o| o.contains("step limit")),
            "after reset, step limit should be cleared"
        );
    }

    #[test]
    fn auto_chain_fires_in_single_step() {
        // A -> B -> C (both transitions have no event = auto)
        let ir = StateMachineIR::new("test", "A")
            .with_state(StateIR::new("A"))
            .with_state(StateIR::new("B"))
            .with_state(StateIR::new("C").final_state())
            .with_transition(TransitionIR::new("A", "B")) // no event = auto
            .with_transition(TransitionIR::new("B", "C")); // no event = auto

        let mut runner = StateMachineRunner::new(ir);

        // Single step(None) should chain through A -> B -> C
        let result = Runner::step(&mut runner, None);
        assert_eq!(
            result.state, "C",
            "Should reach C in one step via auto-chaining"
        );
        assert!(result.completed, "C is final");
    }

    #[test]
    fn auto_chain_stops_at_event_transition() {
        // A -> B (auto) -> C (requires "go")
        let ir = StateMachineIR::new("test", "A")
            .with_state(StateIR::new("A"))
            .with_state(StateIR::new("B"))
            .with_state(StateIR::new("C").final_state())
            .with_transition(TransitionIR::new("A", "B")) // auto
            .with_transition(TransitionIR::new("B", "C").with_event("go")); // requires event

        let mut runner = StateMachineRunner::new(ir);

        // step(None) should chain A -> B but stop there (B->C needs "go")
        let result = Runner::step(&mut runner, None);
        assert_eq!(result.state, "B");
        assert!(!result.completed);

        // step("go") fires B -> C
        let result2 = Runner::step(&mut runner, Some("go"));
        assert_eq!(result2.state, "C");
        assert!(result2.completed);
    }

    #[test]
    fn auto_chain_cycle_detection() {
        // A -> B (auto) -> A (auto) — infinite cycle
        let ir = StateMachineIR::new("test", "A")
            .with_state(StateIR::new("A"))
            .with_state(StateIR::new("B"))
            .with_transition(TransitionIR::new("A", "B")) // auto
            .with_transition(TransitionIR::new("B", "A")); // auto — creates cycle

        let mut runner = StateMachineRunner::new(ir);

        // Should NOT infinite loop — cycle detection breaks.
        // step_inner fires A->B (visited={B}), auto-chain fires B->A (visited={B,A}),
        // then A->B would revisit B, so cycle detection stops at A.
        let result = Runner::step(&mut runner, None);
        assert!(
            result.state == "A" || result.state == "B",
            "Should stop at a visited state, got: {}",
            result.state
        );
    }

    // ===================================================================
    // Gap 3: Completion transition tests
    // ===================================================================

    #[test]
    fn completion_transition_waits_for_do_action() {
        // State A has a do-action and a completion transition to B.
        // With our model, Structured do-actions complete immediately after first execution,
        // so step(None) should: execute do, mark do_completed, then auto-chain fires completion.
        let ir = StateMachineIR::new("test", "A")
            .with_state(
                StateIR::new("A").with_do_action(TransitionActionIR::structured(
                    vec![crate::AssignmentIR::set("x", 42.0)],
                    vec![],
                )),
            )
            .with_state(StateIR::new("B").final_state())
            .with_transition(TransitionIR::new("A", "B").completion());

        let mut runner = StateMachineRunner::new(ir);

        // step(None): do-action executes and immediately completes (Structured = one-shot),
        // so the completion transition fires in the same step via auto-chaining.
        let result = Runner::step(&mut runner, None);
        assert_eq!(result.state, "B");
        assert!(
            runner.eval_ctx.get("x").is_some(),
            "do-action should have set x"
        );
    }

    #[test]
    fn auto_transition_without_do_fires_immediately() {
        // State A has NO do-action, transition A->B has no event (auto, not completion).
        let ir = StateMachineIR::new("test", "A")
            .with_state(StateIR::new("A")) // no do-action
            .with_state(StateIR::new("B").final_state())
            .with_transition(TransitionIR::new("A", "B")); // is_completion defaults to false

        let mut runner = StateMachineRunner::new(ir);
        let result = Runner::step(&mut runner, None);
        assert_eq!(result.state, "B", "Auto-transition should fire immediately");
    }

    #[test]
    fn event_transition_with_do_still_works() {
        // State A has do-action AND an event-triggered transition to B.
        // Event transitions should fire regardless of do-action completion state.
        let ir = StateMachineIR::new("test", "A")
            .with_state(
                StateIR::new("A").with_do_action(TransitionActionIR::structured(vec![], vec![])),
            )
            .with_state(StateIR::new("B").final_state())
            .with_transition(TransitionIR::new("A", "B").with_event("go"));

        let mut runner = StateMachineRunner::new(ir);

        // step(None) should NOT fire (event required)
        let r1 = Runner::step(&mut runner, None);
        assert_eq!(r1.state, "A");

        // step("go") should fire regardless of do-action state
        let r2 = Runner::step(&mut runner, Some("go"));
        assert_eq!(r2.state, "B");
    }

    #[test]
    fn completion_transition_not_marked_fires_as_auto() {
        // Verify that a transition with no event and is_completion=false (default)
        // still fires immediately as an auto-transition.
        let ir = StateMachineIR::new("test", "A")
            .with_state(
                StateIR::new("A").with_do_action(TransitionActionIR::structured(
                    vec![crate::AssignmentIR::set("y", 10.0)],
                    vec![],
                )),
            )
            .with_state(StateIR::new("B").final_state())
            .with_transition(TransitionIR::new("A", "B")); // NOT marked as completion

        let mut runner = StateMachineRunner::new(ir);

        // Without is_completion, this is a plain auto-transition that fires immediately.
        let result = Runner::step(&mut runner, None);
        assert_eq!(
            result.state, "B",
            "Non-completion auto-transition should fire immediately"
        );
    }

    #[test]
    fn completion_transition_with_guard() {
        // Completion transition with a guard that initially blocks, then passes.
        let ir = StateMachineIR::new("test", "A")
            .with_state(
                StateIR::new("A").with_do_action(TransitionActionIR::structured(
                    vec![crate::AssignmentIR::set("count", 1.0)],
                    vec![],
                )),
            )
            .with_state(StateIR::new("B").final_state())
            .with_transition(
                TransitionIR::new("A", "B")
                    .completion()
                    .with_guard("count > 0"),
            );

        let mut runner = StateMachineRunner::new(ir);

        // Initially count is not set, so guard should fail even after do-action sets it to 1.
        // Actually: do-action sets count=1 first, then completion check runs, guard count>0 passes.
        let result = Runner::step(&mut runner, None);
        assert_eq!(
            result.state, "B",
            "Guard should pass after do-action sets count=1"
        );
    }

    #[test]
    fn composite_state_enters_initial_substate() {
        // Outer: Broken, Operational { Idle, Active }
        // Transition: Broken → Operational on "repair"
        let sub_machine = StateMachineIR::new("Operational_sub", "Idle")
            .with_state(StateIR::new("Idle"))
            .with_state(StateIR::new("Active"))
            .with_transition(TransitionIR::new("Idle", "Active").with_event("start"));

        let ir = StateMachineIR::new("test", "Broken")
            .with_state(StateIR::new("Broken"))
            .with_state(StateIR::new("Operational").with_sub_machine(sub_machine))
            .with_transition(TransitionIR::new("Broken", "Operational").with_event("repair"));

        let mut runner = StateMachineRunner::new(ir);
        assert_eq!(Runner::current_state(&runner), "Broken");

        // Fire "repair" → enters Operational, sub-machine enters Idle
        let result = Runner::step(&mut runner, Some("repair"));
        assert_eq!(result.state, "Operational");

        let path = runner.current_state_path();
        assert_eq!(path, vec!["Operational".to_string(), "Idle".to_string()]);
    }

    #[test]
    fn composite_state_delegates_events_to_substate() {
        let sub_machine = StateMachineIR::new("sub", "Idle")
            .with_state(StateIR::new("Idle"))
            .with_state(StateIR::new("Active"))
            .with_transition(TransitionIR::new("Idle", "Active").with_event("start"));

        let ir = StateMachineIR::new("test", "Operational")
            .with_state(StateIR::new("Operational").with_sub_machine(sub_machine))
            .with_state(StateIR::new("Done").final_state())
            .with_transition(TransitionIR::new("Operational", "Done").with_event("finish"));

        let mut runner = StateMachineRunner::new(ir);
        // Auto-enter Operational → sub enters Idle
        let _ = Runner::step(&mut runner, None);

        // Fire "start" → delegated to sub-machine: Idle → Active
        let result = Runner::step(&mut runner, Some("start"));
        assert_eq!(result.state, "Operational"); // Outer state unchanged
        let path = runner.current_state_path();
        assert_eq!(path, vec!["Operational".to_string(), "Active".to_string()]);
    }

    #[test]
    fn composite_state_outer_transition_preempts() {
        let sub_machine = StateMachineIR::new("sub", "Idle")
            .with_state(StateIR::new("Idle"))
            .with_state(StateIR::new("Active"))
            .with_transition(TransitionIR::new("Idle", "Active").with_event("start"));

        let ir = StateMachineIR::new("test", "Operational")
            .with_state(StateIR::new("Operational").with_sub_machine(sub_machine))
            .with_state(StateIR::new("Broken"))
            .with_transition(TransitionIR::new("Operational", "Broken").with_event("break"));

        let mut runner = StateMachineRunner::new(ir);
        let _ = Runner::step(&mut runner, None); // Enter Operational.Idle

        // Fire "break" → outer transition preempts, exits composite
        let result = Runner::step(&mut runner, Some("break"));
        assert_eq!(result.state, "Broken");
        assert!(
            runner.current_state_path().len() == 1,
            "No sub-runner after exiting composite"
        );
    }

    #[test]
    fn composite_state_unhandled_event_propagates_to_outer() {
        // Sub-machine has no transition for "break", outer does
        let sub_machine = StateMachineIR::new("sub", "Idle")
            .with_state(StateIR::new("Idle"))
            .with_state(StateIR::new("Active"))
            .with_transition(TransitionIR::new("Idle", "Active").with_event("start"));

        let ir = StateMachineIR::new("test", "Operational")
            .with_state(StateIR::new("Operational").with_sub_machine(sub_machine))
            .with_state(StateIR::new("Broken"))
            .with_transition(TransitionIR::new("Operational", "Broken").with_event("break"));

        let mut runner = StateMachineRunner::new(ir);
        let _ = Runner::step(&mut runner, None);

        // Fire "break" — sub-machine can't handle it, propagates to outer
        let result = Runner::step(&mut runner, Some("break"));
        assert_eq!(result.state, "Broken");
    }

    #[test]
    fn test_shallow_history_restores_last_sub_state() {
        // Composite state A with history=Shallow, sub-states X (initial), Y
        // Transition path: (initial) -> A/X -> (event "next") -> A/Y -> (event "leave") -> B -> (event "back") -> A
        // On re-entry to A, should resume at Y (not X)

        let sub_machine = StateMachineIR {
            name: "A_sub".to_string(),
            states: vec![StateIR::new("X"), StateIR::new("Y")],
            transitions: vec![TransitionIR {
                from: "X".into(),
                to: "Y".into(),
                event: Some("next".into()),
                guard: None,
                action: None,
                is_completion: false,
                is_guard_only: false,
                accept_param: None,
            }],
            initial: "X".to_string(),
            regions: vec![],
        };

        let ir = StateMachineIR {
            name: "test".to_string(),
            states: vec![
                StateIR::new("A")
                    .with_history(crate::HistoryKind::Shallow)
                    .with_sub_machine(sub_machine),
                StateIR::new("B"),
            ],
            transitions: vec![
                TransitionIR {
                    from: "A".into(),
                    to: "B".into(),
                    event: Some("leave".into()),
                    guard: None,
                    action: None,
                    is_completion: false,
                    is_guard_only: false,
                    accept_param: None,
                },
                TransitionIR {
                    from: "B".into(),
                    to: "A".into(),
                    event: Some("back".into()),
                    guard: None,
                    action: None,
                    is_completion: false,
                    is_guard_only: false,
                    accept_param: None,
                },
            ],
            initial: "A".to_string(),
            regions: vec![],
        };

        let mut runner = StateMachineRunner::new(ir);

        // Enter A, should be at sub-state X
        let _r = Runner::step(&mut runner, None);
        // Move to Y within A
        let _r = Runner::step(&mut runner, Some("next"));
        // Leave A -> B
        let _r = Runner::step(&mut runner, Some("leave"));
        assert_eq!(runner.current_state(), "B");
        // Re-enter A via "back" - should restore to Y (not X)
        let _r = Runner::step(&mut runner, Some("back"));
        // The sub-runner should be at Y, not X
        if let Some(sub) = runner.sub_runners.get("A") {
            assert_eq!(sub.current_state(), "Y", "History should restore to Y");
        } else {
            panic!("Expected sub-runner for composite state A");
        }
    }

    #[test]
    fn test_no_history_always_starts_at_initial() {
        // Same as above but WITHOUT history - should always start at X
        let sub_machine = StateMachineIR {
            name: "A_sub".to_string(),
            states: vec![StateIR::new("X"), StateIR::new("Y")],
            transitions: vec![TransitionIR {
                from: "X".into(),
                to: "Y".into(),
                event: Some("next".into()),
                guard: None,
                action: None,
                is_completion: false,
                is_guard_only: false,
                accept_param: None,
            }],
            initial: "X".to_string(),
            regions: vec![],
        };

        let ir = StateMachineIR {
            name: "test".to_string(),
            states: vec![
                StateIR::new("A").with_sub_machine(sub_machine), // No history!
                StateIR::new("B"),
            ],
            transitions: vec![
                TransitionIR {
                    from: "A".into(),
                    to: "B".into(),
                    event: Some("leave".into()),
                    guard: None,
                    action: None,
                    is_completion: false,
                    is_guard_only: false,
                    accept_param: None,
                },
                TransitionIR {
                    from: "B".into(),
                    to: "A".into(),
                    event: Some("back".into()),
                    guard: None,
                    action: None,
                    is_completion: false,
                    is_guard_only: false,
                    accept_param: None,
                },
            ],
            initial: "A".to_string(),
            regions: vec![],
        };

        let mut runner = StateMachineRunner::new(ir);
        Runner::step(&mut runner, None);
        Runner::step(&mut runner, Some("next")); // Move to Y
        Runner::step(&mut runner, Some("leave")); // Leave to B
        Runner::step(&mut runner, Some("back")); // Re-enter A
                                                 // Without history, should be at X (initial)
        if let Some(sub) = runner.sub_runners.get("A") {
            assert_eq!(
                sub.current_state(),
                "X",
                "Without history, should start at initial X"
            );
        }
    }

    #[test]
    fn test_deep_history_restores_full_path() {
        // Nested composite: A contains B (initial) and C
        // B contains X (initial) and Y
        // Path: enter A -> A/B/X -> next1 -> A/B/Y -> leave -> D -> back -> A
        // Deep history should restore to A/B/Y (not A/B/X)

        let inner_sub = StateMachineIR {
            name: "B_sub".to_string(),
            states: vec![StateIR::new("X"), StateIR::new("Y")],
            transitions: vec![TransitionIR {
                from: "X".into(),
                to: "Y".into(),
                event: Some("next1".into()),
                guard: None,
                action: None,
                is_completion: false,
                is_guard_only: false,
                accept_param: None,
            }],
            initial: "X".to_string(),
            regions: vec![],
        };

        let outer_sub = StateMachineIR {
            name: "A_sub".to_string(),
            states: vec![
                StateIR::new("B").with_sub_machine(inner_sub),
                StateIR::new("C"),
            ],
            transitions: vec![TransitionIR {
                from: "B".into(),
                to: "C".into(),
                event: Some("next2".into()),
                guard: None,
                action: None,
                is_completion: false,
                is_guard_only: false,
                accept_param: None,
            }],
            initial: "B".to_string(),
            regions: vec![],
        };

        let ir = StateMachineIR {
            name: "test".to_string(),
            states: vec![
                StateIR::new("A")
                    .with_history(crate::HistoryKind::Deep)
                    .with_sub_machine(outer_sub),
                StateIR::new("D"),
            ],
            transitions: vec![
                TransitionIR {
                    from: "A".into(),
                    to: "D".into(),
                    event: Some("leave".into()),
                    guard: None,
                    action: None,
                    is_completion: false,
                    is_guard_only: false,
                    accept_param: None,
                },
                TransitionIR {
                    from: "D".into(),
                    to: "A".into(),
                    event: Some("back".into()),
                    guard: None,
                    action: None,
                    is_completion: false,
                    is_guard_only: false,
                    accept_param: None,
                },
            ],
            initial: "A".to_string(),
            regions: vec![],
        };

        let mut runner = StateMachineRunner::new(ir);

        // Enter A -> A/B/X
        Runner::step(&mut runner, None);
        assert_eq!(runner.current_state_path(), vec!["A", "B", "X"]);

        // Move inner to Y: A/B/Y
        Runner::step(&mut runner, Some("next1"));
        assert_eq!(runner.current_state_path(), vec!["A", "B", "Y"]);

        // Leave A -> D
        Runner::step(&mut runner, Some("leave"));
        assert_eq!(runner.current_state(), "D");

        // Re-enter A via deep history -> should restore A/B/Y
        Runner::step(&mut runner, Some("back"));
        assert_eq!(
            runner.current_state_path(),
            vec!["A".to_string(), "B".to_string(), "Y".to_string()],
            "Deep history should restore full nested path A/B/Y"
        );
    }

    // ===================================================================
    // Trigger parsing tests
    // ===================================================================

    #[test]
    fn test_parse_trigger_after_seconds() {
        let trigger = parse_trigger_from_event("after(5s)", None);
        assert!(trigger.is_some());
        match trigger.unwrap() {
            TriggerKind::After(d) => assert_eq!(d, std::time::Duration::from_secs(5)),
            _ => panic!("expected After trigger"),
        }
    }

    #[test]
    fn test_parse_trigger_after_milliseconds() {
        let trigger = parse_trigger_from_event("after(500ms)", None);
        assert!(trigger.is_some());
        match trigger.unwrap() {
            TriggerKind::After(d) => assert_eq!(d, std::time::Duration::from_millis(500)),
            _ => panic!("expected After trigger"),
        }
    }

    #[test]
    fn test_parse_trigger_after_space_format() {
        let trigger = parse_trigger_from_event("after 2s", None);
        assert!(trigger.is_some());
        match trigger.unwrap() {
            TriggerKind::After(d) => assert_eq!(d, std::time::Duration::from_secs(2)),
            _ => panic!("expected After trigger"),
        }
    }

    #[test]
    fn test_parse_trigger_when() {
        let trigger = parse_trigger_from_event("when(x > 10)", None);
        assert!(trigger.is_some());
        match trigger.unwrap() {
            TriggerKind::When(_) => {} // Just verify it parsed
            _ => panic!("expected When trigger"),
        }
    }

    #[test]
    fn test_parse_trigger_regular_event() {
        let trigger = parse_trigger_from_event("buttonPressed", None);
        assert!(trigger.is_none());
    }

    #[test]
    fn test_parse_trigger_bare_number_as_seconds() {
        // Bare numbers are seconds (SI base unit), consistent with the
        // orchestrator clock and ODE solver which both use seconds.
        let trigger = parse_trigger_from_event("after(1000)", None);
        assert!(trigger.is_some());
        match trigger.unwrap() {
            TriggerKind::After(d) => assert_eq!(d, std::time::Duration::from_secs(1000)),
            _ => panic!("expected After trigger"),
        }
    }

    #[test]
    fn test_parse_trigger_at_seconds() {
        let trigger = parse_trigger_from_event("at(5s)", None);
        match trigger {
            Some(TriggerKind::At(t)) => {
                assert!((t - 5.0).abs() < 1e-10, "expected 5.0s, got {}", t)
            }
            other => panic!("expected TriggerKind::At, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_trigger_at_milliseconds() {
        let trigger = parse_trigger_from_event("at(500ms)", None);
        match trigger {
            Some(TriggerKind::At(t)) => {
                assert!((t - 0.5).abs() < 1e-10, "expected 0.5s, got {}", t)
            }
            other => panic!("expected TriggerKind::At, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_trigger_at_bare_float() {
        let trigger = parse_trigger_from_event("at(2.5)", None);
        match trigger {
            Some(TriggerKind::At(t)) => {
                assert!((t - 2.5).abs() < 1e-10, "expected 2.5s, got {}", t)
            }
            other => panic!("expected TriggerKind::At, got {:?}", other),
        }
    }

    #[test]
    fn test_wire_triggers_auto_parses_after_event() {
        let ir = StateMachineIR::new("AutoWired", "A")
            .with_state(StateIR::new("A"))
            .with_state(StateIR::new("B"))
            .with_transition(TransitionIR::new("A", "B").with_event("after(100ms)"));

        let mut runner = StateMachineRunner::new(ir);

        // Without advancing time, transition should NOT fire
        let r1 = runner.step(None);
        assert_eq!(r1.state, "A");

        // Advance enough time
        runner.advance_time(Duration::from_millis(110));
        let r2 = runner.step(None);
        assert_eq!(r2.state, "B");
    }

    #[test]
    fn test_deferred_events() {
        // State "running" defers "maintenance" events
        let ir = StateMachineIR::new("deferTest", "idle")
            .with_state(StateIR::new("idle"))
            .with_state(StateIR::new("running").with_deferred(vec!["maintenance".to_string()]))
            .with_state(StateIR::new("stopped"))
            .with_transition(TransitionIR::new("idle", "running").with_event("start"))
            .with_transition(TransitionIR::new("running", "stopped").with_event("stop"))
            .with_transition(TransitionIR::new("stopped", "idle").with_event("maintenance"));

        let mut runner = StateMachineRunner::new(ir);
        runner.step(Some("start")); // idle -> running
        assert_eq!(runner.current_state(), "running");

        // "maintenance" is deferred while in "running"
        let _result = runner.step(Some("maintenance"));
        assert_eq!(runner.current_state(), "running"); // Still in running

        // "stop" is not deferred — fires running -> stopped
        // Deferred "maintenance" replays: stopped -> idle
        runner.step(Some("stop"));
        assert_eq!(runner.current_state(), "idle");
    }

    #[test]
    fn test_deferred_events_cleared_on_reset() {
        let ir = StateMachineIR::new("resetTest", "s1")
            .with_state(StateIR::new("s1").with_deferred(vec!["ev".to_string()]))
            .with_state(StateIR::new("s2"))
            .with_transition(TransitionIR::new("s1", "s2").with_event("go"));

        let mut runner = StateMachineRunner::new(ir);
        runner.step(Some("ev")); // deferred
        assert_eq!(runner.current_state(), "s1");
        runner.reset();
        // Queue should be empty after reset
        assert_eq!(runner.current_state(), "s1");
        // "ev" should not replay — it was cleared by reset
        let result = runner.step(None);
        assert_eq!(result.state, "s1");
    }

    #[test]
    fn test_deferred_event_not_replayed_if_new_state_also_defers() {
        // Both "running" and "paused" defer "maintenance"
        let ir = StateMachineIR::new("deferChain", "running")
            .with_state(StateIR::new("running").with_deferred(vec!["maintenance".to_string()]))
            .with_state(StateIR::new("paused").with_deferred(vec!["maintenance".to_string()]))
            .with_state(StateIR::new("idle"))
            .with_transition(TransitionIR::new("running", "paused").with_event("pause"))
            .with_transition(TransitionIR::new("paused", "idle").with_event("stop"))
            .with_transition(TransitionIR::new("idle", "running").with_event("maintenance"));

        let mut runner = StateMachineRunner::new(ir);

        // Defer maintenance in "running"
        runner.step(Some("maintenance"));
        assert_eq!(runner.current_state(), "running");

        // Transition to "paused" — which also defers maintenance, so no replay
        runner.step(Some("pause"));
        assert_eq!(runner.current_state(), "paused");

        // Transition to "idle" — which doesn't defer maintenance, so replay fires
        runner.step(Some("stop"));
        assert_eq!(runner.current_state(), "running"); // idle -> running via maintenance replay
    }

    #[test]
    fn test_deferred_non_matching_event_processed_normally() {
        // Only "maintenance" is deferred; "stop" should process normally
        let ir = StateMachineIR::new("deferNormal", "running")
            .with_state(StateIR::new("running").with_deferred(vec!["maintenance".to_string()]))
            .with_state(StateIR::new("stopped"));

        let mut runner = StateMachineRunner::new(ir);

        // "stop" is not deferred — but no transition matches, so state stays
        let result = runner.step(Some("stop"));
        assert_eq!(result.state, "running");
    }

    #[test]
    fn test_when_trigger_with_context() {
        // Build SM with When trigger: when(temperature > 80)
        // Set temperature in context, verify trigger fires
        let ir = StateMachineIR::new("tempWatch", "idle")
            .with_state(StateIR::new("idle"))
            .with_state(StateIR::new("alert"))
            .with_transition(TransitionIR::new("idle", "alert"));

        let mut runner = StateMachineRunner::new(ir);
        let condition = compile_simple_expression("temperature > 80").unwrap();
        runner.set_transition_trigger(0, TriggerKind::When(condition));

        // Without setting the variable, trigger should not fire
        let result = runner.step(None);
        assert_eq!(
            result.state, "idle",
            "should stay idle when variable is missing"
        );

        // Set temperature above threshold
        runner.eval_ctx.set("temperature", Value::Float(95.0));
        let result = runner.step(None);
        assert_eq!(
            result.state, "alert",
            "should transition when temperature > 80"
        );
    }

    #[test]
    fn test_at_trigger_with_clock_time() {
        // Build SM with At(5.0) trigger
        // Set __clock_time in context, verify it uses clock time instead of elapsed
        let ir = StateMachineIR::new("clockTest", "waiting")
            .with_state(StateIR::new("waiting"))
            .with_state(StateIR::new("done"))
            .with_transition(TransitionIR::new("waiting", "done"));

        let mut runner = StateMachineRunner::new(ir);
        runner.set_transition_trigger(0, TriggerKind::At(5.0));

        // elapsed is 0, no clock time set — should not fire
        let result = runner.step(None);
        assert_eq!(result.state, "waiting", "should not fire at time 0");

        // Set clock time below threshold — should still not fire
        runner.eval_ctx.set("__clock_time", Value::Float(3.0));
        let result = runner.step(None);
        assert_eq!(result.state, "waiting", "should not fire at clock time 3.0");

        // Set clock time at threshold — should fire (even though self.elapsed is still 0)
        runner.eval_ctx.set("__clock_time", Value::Float(5.0));
        let result = runner.step(None);
        assert_eq!(
            result.state, "done",
            "should fire when clock time reaches 5.0"
        );
    }

    #[test]
    fn test_at_trigger_with_int_clock_time() {
        // Verify __clock_time works with Value::Int as well
        let ir = StateMachineIR::new("clockIntTest", "waiting")
            .with_state(StateIR::new("waiting"))
            .with_state(StateIR::new("done"))
            .with_transition(TransitionIR::new("waiting", "done"));

        let mut runner = StateMachineRunner::new(ir);
        runner.set_transition_trigger(0, TriggerKind::At(5.0));

        runner.eval_ctx.set("__clock_time", Value::Int(6));
        let result = runner.step(None);
        assert_eq!(
            result.state, "done",
            "should fire when int clock time >= target"
        );
    }

    #[test]
    fn test_trigger_readiness_check() {
        // Build SM with When trigger referencing undefined variable
        // Call check_trigger_readiness(), verify it reports the issue
        let ir = StateMachineIR::new("readinessCheck", "idle")
            .with_state(StateIR::new("idle"))
            .with_state(StateIR::new("alert"))
            .with_transition(TransitionIR::new("idle", "alert"));

        let mut runner = StateMachineRunner::new(ir);
        let condition = compile_simple_expression("temperature > 80").unwrap();
        runner.set_transition_trigger(0, TriggerKind::When(condition));

        // temperature is not set — should report an issue
        let issues = runner.check_trigger_readiness();
        assert_eq!(issues.len(), 1, "should report exactly one issue");
        assert!(
            issues[0].contains("temperature"),
            "issue should mention the undefined variable 'temperature', got: {}",
            issues[0],
        );
        assert!(
            issues[0].contains("idle->alert"),
            "issue should mention the transition, got: {}",
            issues[0],
        );

        // Set the variable — issues should clear
        runner.eval_ctx.set("temperature", Value::Float(50.0));
        let issues = runner.check_trigger_readiness();
        assert!(
            issues.is_empty(),
            "should have no issues after setting variable"
        );
    }

    // ---- Feature 1.7: Inter-level transitions ----

    #[test]
    fn test_ancestor_chain_top_level() {
        let ir = StateMachineIR::new("test", "A")
            .with_state(StateIR::new("A"))
            .with_state(StateIR::new("B"));

        let runner = StateMachineRunner::new(ir);
        // Top-level state has no parent — chain is just [A]
        assert_eq!(runner.ancestor_chain("A"), vec!["A".to_string()]);
    }

    #[test]
    fn test_ancestor_chain_nested() {
        let sub = StateMachineIR::new("P_sub", "child").with_state(StateIR::new("child"));

        let ir = StateMachineIR::new("test", "P")
            .with_state(StateIR::new("P").with_sub_machine(sub))
            .with_state(StateIR::new("top"));

        let runner = StateMachineRunner::new(ir);
        assert_eq!(
            runner.ancestor_chain("child"),
            vec!["child".to_string(), "P".to_string()]
        );
    }

    #[test]
    fn test_ancestor_chain_deeply_nested() {
        let grandchild_sm =
            StateMachineIR::new("c1_sub", "grandchild").with_state(StateIR::new("grandchild"));

        let child_sm = StateMachineIR::new("P_sub", "child1")
            .with_state(StateIR::new("child1").with_sub_machine(grandchild_sm));

        let ir = StateMachineIR::new("test", "parent")
            .with_state(StateIR::new("parent").with_sub_machine(child_sm))
            .with_state(StateIR::new("top_level"));

        let runner = StateMachineRunner::new(ir);
        assert_eq!(
            runner.ancestor_chain("grandchild"),
            vec![
                "grandchild".to_string(),
                "child1".to_string(),
                "parent".to_string(),
            ]
        );
    }

    #[test]
    fn test_find_lca_same_parent() {
        // parent { branch_a { leaf_a }, branch_b { leaf_b } }
        let branch_a_sm =
            StateMachineIR::new("ba_sub", "leaf_a").with_state(StateIR::new("leaf_a"));
        let branch_b_sm =
            StateMachineIR::new("bb_sub", "leaf_b").with_state(StateIR::new("leaf_b"));

        let parent_sm = StateMachineIR::new("p_sub", "branch_a")
            .with_state(StateIR::new("branch_a").with_sub_machine(branch_a_sm))
            .with_state(StateIR::new("branch_b").with_sub_machine(branch_b_sm));

        let ir = StateMachineIR::new("test", "parent")
            .with_state(StateIR::new("parent").with_sub_machine(parent_sm))
            .with_state(StateIR::new("other"));

        let runner = StateMachineRunner::new(ir);
        assert_eq!(
            runner.find_lca("leaf_a", "leaf_b"),
            Some("parent".to_string()),
        );
    }

    #[test]
    fn test_find_lca_no_common_ancestor() {
        let ir = StateMachineIR::new("test", "A")
            .with_state(StateIR::new("A"))
            .with_state(StateIR::new("B"));

        let runner = StateMachineRunner::new(ir);
        // Both are top-level — no LCA (they share the implicit root only)
        assert_eq!(runner.find_lca("A", "B"), None);
    }

    #[test]
    fn test_exit_chain_same_level() {
        let ir = StateMachineIR::new("test", "A")
            .with_state(StateIR::new("A"))
            .with_state(StateIR::new("B"));

        let runner = StateMachineRunner::new(ir);
        // Same-level: no LCA, exit chain is the full source ancestor chain
        assert_eq!(runner.exit_chain("A", "B"), vec!["A".to_string()]);
    }

    #[test]
    fn test_entry_chain_same_level() {
        let ir = StateMachineIR::new("test", "A")
            .with_state(StateIR::new("A"))
            .with_state(StateIR::new("B"));

        let runner = StateMachineRunner::new(ir);
        assert_eq!(runner.entry_chain("A", "B"), vec!["B".to_string()]);
    }

    #[test]
    fn test_inter_level_deep_to_top() {
        // Hierarchy: parent { child1 { grandchild } }, top_level
        // Transition from grandchild -> top_level
        // Should execute: exit(grandchild), exit(child1), exit(parent), entry(top_level)
        let grandchild_sm = StateMachineIR::new("c1_sub", "grandchild")
            .with_state(StateIR::new("grandchild").with_exit("exit_grandchild"));

        let parent_sm = StateMachineIR::new("p_sub", "child1").with_state(
            StateIR::new("child1")
                .with_exit("exit_child1")
                .with_sub_machine(grandchild_sm),
        );

        let ir = StateMachineIR::new("test", "parent")
            .with_state(
                StateIR::new("parent")
                    .with_exit("exit_parent")
                    .with_sub_machine(parent_sm),
            )
            .with_state(StateIR::new("top_level").with_entry("entry_top_level"))
            .with_transition(TransitionIR::new("parent", "top_level").with_event("go"));

        let mut runner = StateMachineRunner::new(ir);
        // Enter parent -> child1 -> grandchild
        let _ = Runner::step(&mut runner, None);
        assert_eq!(runner.current_state(), "parent");

        // Force inter-level transition from deep sub-state context
        // The exit chain should be: grandchild, child1, parent
        // The entry chain should be: top_level
        runner.force_transition("top_level", None);
        assert_eq!(runner.current_state(), "top_level");
    }

    #[test]
    fn test_inter_level_top_to_deep() {
        // Hierarchy: top_level, parent { child1 { grandchild } }
        // Transition from top_level -> grandchild
        // Should execute: exit(top_level), entry(parent), entry(child1), entry(grandchild)
        let grandchild_sm = StateMachineIR::new("c1_sub", "grandchild")
            .with_state(StateIR::new("grandchild").with_entry("entry_grandchild"));

        let parent_sm = StateMachineIR::new("p_sub", "child1").with_state(
            StateIR::new("child1")
                .with_entry("entry_child1")
                .with_sub_machine(grandchild_sm),
        );

        let ir = StateMachineIR::new("test", "top_level")
            .with_state(StateIR::new("top_level").with_exit("exit_top_level"))
            .with_state(
                StateIR::new("parent")
                    .with_entry("entry_parent")
                    .with_sub_machine(parent_sm),
            );

        let mut runner = StateMachineRunner::new(ir);
        assert_eq!(runner.current_state(), "top_level");

        // Force transition to deep nested state
        runner.force_transition("grandchild", None);
        assert_eq!(runner.current_state(), "parent");

        // Verify sub-runner chain was created
        let path = runner.current_state_path();
        assert_eq!(
            path,
            vec![
                "parent".to_string(),
                "child1".to_string(),
                "grandchild".to_string(),
            ]
        );
    }

    #[test]
    fn test_inter_level_cross_branch() {
        // Hierarchy: parent { branch_a { leaf_a }, branch_b { leaf_b } }
        // Transition from leaf_a -> leaf_b
        // Should execute: exit(leaf_a), exit(branch_a), entry(branch_b), entry(leaf_b)
        let branch_a_sm = StateMachineIR::new("ba_sub", "leaf_a")
            .with_state(StateIR::new("leaf_a").with_exit("exit_leaf_a"));

        let branch_b_sm = StateMachineIR::new("bb_sub", "leaf_b")
            .with_state(StateIR::new("leaf_b").with_entry("entry_leaf_b"));

        let parent_sm = StateMachineIR::new("p_sub", "branch_a")
            .with_state(
                StateIR::new("branch_a")
                    .with_exit("exit_branch_a")
                    .with_sub_machine(branch_a_sm),
            )
            .with_state(
                StateIR::new("branch_b")
                    .with_entry("entry_branch_b")
                    .with_sub_machine(branch_b_sm),
            );

        let ir = StateMachineIR::new("test", "parent")
            .with_state(StateIR::new("parent").with_sub_machine(parent_sm));

        let mut runner = StateMachineRunner::new(ir);
        // Enter: parent -> branch_a -> leaf_a
        let _ = Runner::step(&mut runner, None);

        // The runner is at "parent" with sub_runner at branch_a/leaf_a
        assert_eq!(runner.current_state(), "parent");
        let path = runner.current_state_path();
        assert_eq!(
            path,
            vec![
                "parent".to_string(),
                "branch_a".to_string(),
                "leaf_a".to_string(),
            ]
        );

        // Force cross-branch transition leaf_a -> leaf_b
        // The LCA is "parent", so:
        //   exit: leaf_a, branch_a (up to parent, exclusive)
        //   entry: branch_b, leaf_b (from parent down, exclusive of parent)
        runner.force_transition("leaf_b", None);

        // Should now be at parent -> branch_b -> leaf_b
        assert_eq!(runner.current_state(), "parent");
        let path = runner.current_state_path();
        assert_eq!(
            path,
            vec![
                "parent".to_string(),
                "branch_b".to_string(),
                "leaf_b".to_string(),
            ]
        );
    }

    #[test]
    fn test_inter_level_same_level_unchanged() {
        // Verify that same-level transitions still work correctly
        // (no regression from the LCA logic)
        let ir = StateMachineIR::new("test", "A")
            .with_state(StateIR::new("A").with_exit("exit_A"))
            .with_state(StateIR::new("B").with_entry("entry_B"))
            .with_transition(TransitionIR::new("A", "B").with_event("go"));

        let mut runner = StateMachineRunner::new(ir);
        assert_eq!(runner.current_state(), "A");

        runner.force_transition("B", None);
        assert_eq!(runner.current_state(), "B");
    }

    #[test]
    fn test_inter_level_exit_actions_use_context() {
        // Verify that structured exit/entry actions execute and affect EvalContext
        // during inter-level transitions.
        use crate::AssignmentIR;

        let exit_action = TransitionActionIR::structured(
            vec![AssignmentIR::set("exited_deep", Value::Bool(true))],
            vec![],
        );
        let entry_action = TransitionActionIR::structured(
            vec![AssignmentIR::set("entered_top", Value::Bool(true))],
            vec![],
        );

        let sub_sm = StateMachineIR::new("p_sub", "deep")
            .with_state(StateIR::new("deep").with_exit_action(exit_action));

        let ir = StateMachineIR::new("test", "parent")
            .with_state(StateIR::new("parent").with_sub_machine(sub_sm))
            .with_state(StateIR::new("top").with_entry_action(entry_action));

        let mut runner = StateMachineRunner::new(ir);
        let _ = Runner::step(&mut runner, None); // Enter parent -> deep

        runner.force_transition("top", None);
        assert_eq!(runner.current_state(), "top");
        assert_eq!(
            runner.eval_ctx.get("entered_top"),
            Some(&Value::Bool(true)),
            "entry action on target should have executed",
        );
    }

    #[test]
    fn test_inter_level_transition_action_fires() {
        // Verify that the transition action fires between exit and entry chains
        use crate::AssignmentIR;

        let sub_sm = StateMachineIR::new("p_sub", "deep").with_state(StateIR::new("deep"));

        let ir = StateMachineIR::new("test", "parent")
            .with_state(StateIR::new("parent").with_sub_machine(sub_sm))
            .with_state(StateIR::new("top"));

        let mut runner = StateMachineRunner::new(ir);
        let _ = Runner::step(&mut runner, None);

        let trans_action = TransitionActionIR::structured(
            vec![AssignmentIR::set("transit", Value::Bool(true))],
            vec![],
        );
        runner.force_transition("top", Some(trans_action));
        assert_eq!(runner.current_state(), "top");
        assert_eq!(
            runner.eval_ctx.get("transit"),
            Some(&Value::Bool(true)),
            "transition action should have fired",
        );
    }

    // -----------------------------------------------------------------------
    // RSC-2.4b: compiled-once guard cache + restricted slot writeback
    // -----------------------------------------------------------------------

    /// Guard-binding equivalence: the compiled-once cache must produce the
    /// SAME verdict as the legacy per-evaluation string compilation across a
    /// matrix of guard shapes (boolean, non-boolean, eval-error fallback,
    /// uncompilable string fallback) × contexts × events.
    #[test]
    fn rsc24b_guard_cache_matches_string_compilation_across_matrix() {
        let guards = [
            "x > 5",           // boolean comparison
            "flag and x < 20", // boolean conjunction (KerML `and`)
            "x - 5",           // compiles, non-boolean result → event fallback
            "powerOn",         // bare identifier: Bool when bound, fallback when not
            "x +",             // does not compile → string/event fallback
        ];
        let mut ir = StateMachineIR::new("GuardMatrix", "a").with_state(StateIR::new("a"));
        for g in &guards {
            ir = ir.with_transition(TransitionIR::new("a", "b").with_guard(*g));
        }
        let mut runner = StateMachineRunner::new(ir);

        let contexts: Vec<Vec<(&str, Value)>> = vec![
            vec![("x", Value::Float(10.0)), ("flag", Value::Bool(true))],
            vec![("x", Value::Float(0.0)), ("flag", Value::Bool(false))],
            vec![("x", Value::Int(7))],
            vec![("powerOn", Value::Bool(true))],
            vec![], // nothing bound — every guard exercises its fallback
        ];
        let events = [
            None,
            Some("powerOn"),
            Some("x +"),
            Some("x - 5"),
            Some("other"),
        ];

        for ctx_vals in &contexts {
            runner.eval_ctx = EvalContext::new();
            for (k, v) in ctx_vals {
                runner.eval_ctx.set((*k).to_owned(), v.clone());
            }
            for (idx, g) in guards.iter().enumerate() {
                for ev in &events {
                    assert_eq!(
                        runner.evaluate_guard_at(idx, g, *ev),
                        evaluate_guard(g, &runner.eval_ctx, *ev),
                        "verdict drift for guard '{g}' under ctx {ctx_vals:?} event {ev:?}"
                    );
                }
            }
        }

        assert_eq!(
            runner.guard_string_fallbacks(),
            1,
            "exactly the uncompilable 'x +' stays on the string path"
        );
    }

    /// Guard-binding equivalence after the slot-bind pass: binding rewrites
    /// reads to `SlotRef` but evaluation stays context-name-first, so
    /// verdicts are identical to the string path wherever names are in
    /// context — and the slot serves the read when they are not.
    #[test]
    fn rsc24b_bound_guards_keep_string_verdicts() {
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
        use sysml_core::ElementId;

        let mut store = SlotStore::new();
        let x_slot = store.intern(
            SlotMeta::new(
                RuntimeId::top_level(ElementId::from_string("decl:x".to_owned())),
                Variability::Discrete,
                WriterId::Orchestrator,
                "x",
                "x",
            ),
            Value::Float(10.0),
        );

        let guards = ["x > 5", "x <= 5"];
        let mut ir = StateMachineIR::new("BoundGuards", "a").with_state(StateIR::new("a"));
        for g in &guards {
            ir = ir.with_transition(TransitionIR::new("a", "b").with_guard(*g));
        }
        let mut runner = StateMachineRunner::new(ir);
        let report =
            crate::orchestrator::Executor::bind_expression_slots(&mut runner, &store, None);
        assert_eq!(report.bound_refs, 2, "both guard reads bind to the x slot");
        assert!(
            report.unresolved.is_empty(),
            "guard binding never feeds RS003: {report:?}"
        );
        assert_eq!(runner.guard_bind_report().bound_refs, 2);

        // Name present in context → identical verdicts to the string path.
        runner.eval_ctx.set("x", Value::Float(10.0));
        for (idx, g) in guards.iter().enumerate() {
            assert_eq!(
                runner.evaluate_guard_at(idx, g, None),
                evaluate_guard(g, &runner.eval_ctx, None),
                "bound guard '{g}' must agree with the string path"
            );
        }

        // Name absent from context → the bound IR is served by the slot
        // (this is what makes the scoped-view bypass possible at all).
        runner.eval_ctx = EvalContext::new();
        runner.eval_ctx.slot_reader = Some(std::sync::Arc::new(std::sync::RwLock::new(store)));
        let _ = x_slot;
        assert!(
            runner.evaluate_guard_at(0, "x > 5", None),
            "slot-served read: x = 10.0 → x > 5"
        );
        assert!(!runner.evaluate_guard_at(1, "x <= 5", None));
    }

    /// Cull-arc W3 (REQ 2): composite-state write-through coverage. A sub-state
    /// ENTRY effect fired inside a composite must reach the shared slot store
    /// after the runner steps + `sync_context_out_slots`. This is the positive
    /// guard for the composite sub-runner `eval_ctx` clones (`:2418`/`:2489`/
    /// `:3087`/`:3786`, classified `alias_live`): the pre-existing composite
    /// tests assert only the state PATH, never that a sub-state assignment
    /// propagates out. If composite context propagation regressed, the store
    /// would keep its seed value instead of the assigned one.
    #[test]
    fn cull_w3_composite_substate_assignment_writes_through() {
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
        use crate::AssignmentIR;

        // Composite `Op` contains a sub-machine Idle --start--> Active; entering
        // `Active` runs an entry effect assigning `tripCount := 42`.
        let sub_machine = StateMachineIR::new("sub", "Idle")
            .with_state(StateIR::new("Idle"))
            .with_state(StateIR::new("Active").with_entry(
                TransitionActionIR::structured(
                    vec![AssignmentIR::set("tripCount", Value::Float(42.0))],
                    vec![],
                ),
            ))
            .with_transition(TransitionIR::new("Idle", "Active").with_event("start"));
        let ir = StateMachineIR::new("Comp", "Op")
            .with_state(StateIR::new("Op").with_sub_machine(sub_machine));
        let mut runner = StateMachineRunner::new(ir);

        // Mint the compiled assignment target as an Executor-owned slot so the
        // routed writeback resolves it by SlotId (production mints it too).
        let mut store = SlotStore::new();
        store.intern(
            SlotMeta::new(
                RuntimeId::top_level(sysml_core::ElementId::from_string("decl:tripCount")),
                Variability::Discrete,
                WriterId::Executor(0),
                "u1.tripCount",
                "u1.tripCount",
            ),
            Value::Float(0.0),
        );
        crate::orchestrator::Executor::prepare_slot_writeback(
            &mut runner,
            &store,
            Some("u1"),
            None,
            WriterId::Executor(0),
        );
        let shared_store: crate::slots::SharedSlotStore =
            std::sync::Arc::new(std::sync::RwLock::new(store));

        // Enter Op → sub Idle, then fire "start" → Idle→Active runs the entry effect.
        let _ = Runner::step(&mut runner, None);
        let _ = Runner::step(&mut runner, Some("start"));

        let mut shared = EvalContext::new();
        shared.slots = Some(std::sync::Arc::clone(&shared_store));
        crate::orchestrator::Executor::sync_context_out_slots(
            &runner,
            &mut shared,
            crate::ode::SignalEvalMode::FreshState,
        );

        assert_eq!(
            shared.get("u1.tripCount"),
            Some(&Value::Float(42.0)),
            "composite sub-state entry assignment must write through to the shared store"
        );
    }

    /// RSC-2.4b write-set restriction: the slot-routed writeback publishes
    /// the compiled assignment targets and the runner's own dynamic
    /// bindings — NOT the rest of the drifted internal context (the legacy
    /// whole-diff's echo of merged globals).
    ///
    /// Updated for the string-identity cull: the compiled SM assignment target
    /// (`tripCount`) is now MINTED as an `Executor`-owned slot and routes by
    /// `SlotId` (the deleted name-keyed fallback for unminted SM targets no
    /// longer exists — an unrouted SM target hard-errors). The `__clock_time`
    /// dynamic key stays on the runtime-dynamic name-keyed path. The core
    /// assertion under test — merged globals are NOT re-echoed — is unchanged.
    #[test]
    fn rsc24b_writeback_publishes_write_set_not_context_echo() {
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
        use crate::AssignmentIR;

        let ir = StateMachineIR::new("Restricted", "a")
            .with_state(StateIR::new("a").with_entry(TransitionActionIR::structured(
                vec![AssignmentIR::set("tripCount", Value::Float(1.0))],
                vec![],
            )))
            .with_state(StateIR::new("b"));
        let mut runner = StateMachineRunner::new(ir);

        // Mint the compiled target slot `u1.tripCount` owned by this SM so the
        // routed writeback resolves it by `SlotId` (production mints it too).
        let mut store = SlotStore::new();
        store.intern(
            SlotMeta::new(
                RuntimeId::top_level(sysml_core::ElementId::from_string("decl:tripCount")),
                Variability::Discrete,
                WriterId::Executor(0),
                "u1.tripCount",
                "u1.tripCount",
            ),
            Value::Float(0.0),
        );
        crate::orchestrator::Executor::prepare_slot_writeback(
            &mut runner,
            &store,
            Some("u1"),
            None,
            WriterId::Executor(0),
        );
        // The routed write lands by `SlotId` into the shared context's store
        // (as it does through the orchestrator's master context in production).
        let shared_store: crate::slots::SharedSlotStore =
            std::sync::Arc::new(std::sync::RwLock::new(store));

        // Drifted internal context: merged globals + an SM write.
        runner.eval_ctx.set("t_ms", Value::Float(40.0));
        runner.eval_ctx.set("someGlobal", Value::Float(7.0));
        runner.eval_ctx.set("tripCount", Value::Float(3.0));
        runner.dynamic_keys.insert("__clock_time".to_owned());
        runner.eval_ctx.set("__clock_time", Value::Float(0.04));

        let mut shared = EvalContext::new();
        shared.slots = Some(std::sync::Arc::clone(&shared_store));
        assert!(crate::orchestrator::Executor::sync_context_out_slots(
            &runner,
            &mut shared,
            crate::ode::SignalEvalMode::FreshState
        ));

        assert_eq!(
            shared.get("u1.tripCount"),
            Some(&Value::Float(3.0)),
            "compiled target published under the instance prefix (routed by SlotId)"
        );
        assert_eq!(
            shared.get("u1.__clock_time"),
            Some(&Value::Float(0.04)),
            "dynamic key published through the runtime-dynamic name-keyed path"
        );
        assert!(
            shared.get("u1.t_ms").is_none() && shared.get("u1.someGlobal").is_none(),
            "merged-global echo keys are NOT republished (the legacy \
             whole-diff pollution this cutover removes); got {:?}",
            shared.variables
        );

        // The routed target is NO LONGER a fallback; only the runtime-dynamic
        // `__clock_time` remains name-keyed.
        let fallbacks = crate::orchestrator::Executor::slot_write_fallbacks(&runner);
        assert!(
            !fallbacks.contains(&"u1.tripCount".to_owned()),
            "minted SM target routes by SlotId, not name-keyed: {fallbacks:?}"
        );
        assert!(
            fallbacks.contains(&"u1.__clock_time".to_owned()),
            "the runtime-dynamic clock key stays name-keyed: {fallbacks:?}"
        );
    }

    // -----------------------------------------------------------------------
    // RSC-3.5b: SM port-payload slots (drain 1) + __clock_time invariant
    // -----------------------------------------------------------------------

    /// Build a single-port `PortMessage`-triggered SM: `idle --(tripIn)--> hot`.
    fn payload_port_runner() -> StateMachineRunner {
        let ir = StateMachineIR::new("Breaker", "idle")
            .with_state(StateIR::new("idle"))
            .with_state(StateIR::new("hot"))
            .with_transition(TransitionIR::new("idle", "hot").with_event("tripIn"));
        let mut runner = StateMachineRunner::new(ir);
        // Re-tag transition 0's trigger as a PortMessage on `tripIn`.
        runner.set_transition_trigger(
            0,
            TriggerKind::PortMessage {
                port_name: "tripIn".to_owned(),
                payload_type: None,
                param_name: None,
            },
        );
        runner
    }

    /// Mint a `{port}.payload` (+ `_payload`) Discrete slot owned by the SM
    /// executor — the compile-static claim RSC-3.5b proves (the receiving
    /// port SET is enumerable; only the VALUE is runtime-dynamic).
    fn mint_payload_slot(
        store: &mut crate::slots::SlotStore,
        runtime: &str,
        canonical: &str,
        writer: crate::slots::WriterId,
    ) {
        use crate::slots::{RuntimeId, SlotMeta, Variability};
        store.intern(
            SlotMeta::new(
                RuntimeId::top_level(sysml_core::ElementId::from_string(format!(
                    "rsc35b-payload-test:{runtime}"
                ))),
                Variability::Discrete,
                writer,
                canonical,
                runtime,
            ),
            Value::Null,
        );
    }

    /// RSC-3.5b drain-1 (a): a SM with a payload-receiving port mints a
    /// `{port}.payload` Discrete slot and the tick-time payload write ROUTES
    /// through it — the slot carries the delivered value, and the payload key
    /// is NOT reported as a name-keyed fallback.
    #[test]
    fn rsc35b_payload_routes_through_minted_slot() {
        use crate::orchestrator::{Executor, TickContext};
        use crate::slots::{SlotStore, WriterId};

        let mut runner = payload_port_runner();
        assert_eq!(
            runner.accept_ports(),
            vec!["tripIn".to_owned()],
            "the PortMessage trigger surfaces tripIn as an accept port"
        );

        // Top-level SM: payload keys are bare. Mint both spellings, owned by
        // executor 0 (this SM).
        let mut store = SlotStore::new();
        mint_payload_slot(
            &mut store,
            "tripIn.payload",
            "tripIn.payload",
            WriterId::Executor(0),
        );
        mint_payload_slot(
            &mut store,
            "tripIn_payload",
            "tripIn_payload",
            WriterId::Executor(0),
        );

        Executor::prepare_slot_writeback(&mut runner, &store, None, None, WriterId::Executor(0));

        // Deliver a payload to tripIn this tick.
        let shared_in = EvalContext::new();
        let payloads = vec![("tripIn".to_owned(), Value::Float(42.0))];
        let ctx = TickContext {
            t: 0.0,
            dt: 0.1,
            tick: 0,
            context: &shared_in,
            event: Some("tripIn"),
            port_payloads: &payloads,
            local_clock_time: None,
        };
        let _ = Executor::tick(&mut runner, &ctx);

        // Writeback routes the payload by SlotId.
        let mut shared = EvalContext::new();
        // Attach the same store so set_slot can route.
        shared.slots = Some(std::sync::Arc::new(std::sync::RwLock::new(store)));
        assert!(Executor::sync_context_out_slots(
            &runner,
            &mut shared,
            crate::ode::SignalEvalMode::FreshState
        ));

        // The slot carries the delivered payload (both spellings).
        let slots = shared.slots.as_ref().unwrap().read().unwrap();
        let dot = slots
            .slot_by_name("tripIn.payload")
            .expect("payload slot minted");
        assert_eq!(
            slots.get(dot),
            Some(&Value::Float(42.0)),
            "delivered payload landed in the routed {{port}}.payload slot"
        );
        let under = slots
            .slot_by_name("tripIn_payload")
            .expect("underscore payload slot minted");
        assert_eq!(slots.get(under), Some(&Value::Float(42.0)));
        drop(slots);

        // And the payload keys are NOT name-keyed fallbacks (drained).
        let fallbacks = Executor::slot_write_fallbacks(&runner);
        assert!(
            fallbacks.is_empty(),
            "RSC-3.5b: routed payloads + no clock ⇒ empty fallback set: {fallbacks:?}"
        );
    }

    /// RSC-3.5b drain-1 (b): `slot_write_fallbacks()` reports `__clock_time`
    /// ONLY (never the payload keys) when a local clock is active, and is
    /// EMPTY when no clock is active. `__clock_time` is a runtime-dynamic
    /// local-clock key (a Phase-4 item, design doc §8 caveat (d)) — it must
    /// STAY in the fallback class; the payload keys must NOT.
    #[test]
    fn rsc35b_clock_time_is_the_only_remaining_fallback() {
        use crate::orchestrator::{Executor, TickContext};
        use crate::slots::{SlotStore, WriterId};

        let mut runner = payload_port_runner();
        let mut store = SlotStore::new();
        mint_payload_slot(
            &mut store,
            "tripIn.payload",
            "tripIn.payload",
            WriterId::Executor(0),
        );
        mint_payload_slot(
            &mut store,
            "tripIn_payload",
            "tripIn_payload",
            WriterId::Executor(0),
        );
        Executor::prepare_slot_writeback(&mut runner, &store, None, None, WriterId::Executor(0));

        // --- No clock active: deliver a payload only. ---
        let shared_in = EvalContext::new();
        let payloads = vec![("tripIn".to_owned(), Value::Float(7.0))];
        let ctx = TickContext {
            t: 0.0,
            dt: 0.1,
            tick: 0,
            context: &shared_in,
            event: Some("tripIn"),
            port_payloads: &payloads,
            local_clock_time: None,
        };
        let _ = Executor::tick(&mut runner, &ctx);
        assert_eq!(
            Executor::slot_write_fallbacks(&runner),
            Vec::<String>::new(),
            "no local clock + routed payloads ⇒ EMPTY fallback set"
        );

        // --- Local clock active: __clock_time enters dynamic_keys. ---
        let ctx_clk = TickContext {
            t: 0.5,
            dt: 0.1,
            tick: 1,
            context: &shared_in,
            event: None,
            port_payloads: &payloads,
            local_clock_time: Some(0.5),
        };
        let _ = Executor::tick(&mut runner, &ctx_clk);
        assert_eq!(
            Executor::slot_write_fallbacks(&runner),
            vec!["__clock_time".to_owned()],
            "local clock active ⇒ __clock_time is the ONLY fallback (payloads stay drained)"
        );
    }

    /// RSC-3.5b drain-1 (a, unrouted): without a minted payload slot the
    /// payload route degrades to the EXACT legacy name-keyed write (map
    /// parity) and is reported as a fallback under the instance prefix — the
    /// pre-3.5b behaviour, preserved when minting is absent (e.g. a hand-built
    /// runner with no compiler-minted store).
    #[test]
    fn rsc35b_payload_falls_back_when_unminted() {
        use crate::orchestrator::{Executor, TickContext};
        use crate::slots::{SlotStore, WriterId};

        let mut runner = payload_port_runner();
        let store = SlotStore::new(); // empty — nothing minted
        Executor::prepare_slot_writeback(
            &mut runner,
            &store,
            Some("u1"),
            None,
            WriterId::Executor(0),
        );

        let shared_in = EvalContext::new();
        let payloads = vec![("tripIn".to_owned(), Value::Float(3.0))];
        let ctx = TickContext {
            t: 0.0,
            dt: 0.1,
            tick: 0,
            context: &shared_in,
            event: Some("tripIn"),
            port_payloads: &payloads,
            local_clock_time: None,
        };
        let _ = Executor::tick(&mut runner, &ctx);

        let mut shared = EvalContext::new();
        assert!(Executor::sync_context_out_slots(
            &runner,
            &mut shared,
            crate::ode::SignalEvalMode::FreshState
        ));
        assert_eq!(
            shared.get("u1.tripIn.payload"),
            Some(&Value::Float(3.0)),
            "unrouted payload uses the exact legacy {{prefix}}.{{port}}.payload key"
        );
        let fallbacks = Executor::slot_write_fallbacks(&runner);
        assert!(
            fallbacks.contains(&"u1.tripIn.payload".to_owned())
                && fallbacks.contains(&"u1.tripIn_payload".to_owned()),
            "unminted payload keys still report as name-keyed fallbacks: {fallbacks:?}"
        );
    }
}
