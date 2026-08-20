//! # sysml-run
//!
//! Execution runtime traits and IR (Intermediate Representation) types for SysML v2.
//!
//! This crate defines the core abstractions for executing SysML models:
//! - Runner trait for stepping through execution
//! - CompileToIR trait for compiling ModelGraph to executable IR
//! - IR structs for state machines, constraints, etc.
//!
//! Actual implementations are in sub-crates (sysml-run-statemachine, etc.).

use std::collections::HashMap;
use sysml_core::{ModelGraph, Value};
use sysml_span::Diagnostic;

/// The result of a single execution step.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct StepResult {
    /// The current state after the step.
    pub state: String,
    /// Any outputs produced by the step.
    pub outputs: Vec<String>,
    /// Whether execution has completed.
    pub completed: bool,
    /// Events to send via FlowRouter (from structured transition actions).
    pub sends: Vec<String>,
    /// Port-addressed payload sends produced this step: `(port_name, payload)`.
    /// Evaluated from the structured action's `port_send_ops`; routed by the
    /// orchestrator as addressed MessageTransfers. Skipped when empty so the
    /// serialized form is unchanged for the (overwhelmingly common) no-payload
    /// case.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub port_sends: Vec<(String, Value)>,
    /// Available transitions from the current state as `(event, target_state)`.
    /// Populated when a step event didn't match any transition, empty otherwise.
    pub available_transitions: Vec<(String, String)>,
    /// The triggering event that caused entry into `state` this step, when a
    /// *triggered* (non-completion) transition fired. `None` for completion
    /// transitions, no-op steps, and the initial state.
    ///
    /// SPEC-SILENT: records the triggering event name; the full `MessageTransfer`
    /// identity is deferred (`StatePerformances.kerml:48`
    /// `incomingTransitionTrigger : MessageTransfer [0..1]`).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub incoming_trigger: Option<String>,
}

impl StepResult {
    /// Create a new step result.
    pub fn new(state: impl Into<String>) -> Self {
        StepResult {
            state: state.into(),
            outputs: Vec::new(),
            completed: false,
            sends: Vec::new(),
            port_sends: Vec::new(),
            available_transitions: Vec::new(),
            incoming_trigger: None,
        }
    }

    /// Record the triggering event that caused entry into the current state.
    pub fn with_incoming_trigger(mut self, event: Option<String>) -> Self {
        self.incoming_trigger = event;
        self
    }

    /// Mark this result as completed.
    pub fn completed(mut self) -> Self {
        self.completed = true;
        self
    }

    /// Add an output.
    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        self.outputs.push(output.into());
        self
    }

    /// Add multiple outputs.
    pub fn with_outputs(mut self, outputs: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.outputs.extend(outputs.into_iter().map(|o| o.into()));
        self
    }

    /// Add a send event (for FlowRouter routing).
    pub fn with_send(mut self, event: impl Into<String>) -> Self {
        self.sends.push(event.into());
        self
    }

    /// Add multiple send events.
    pub fn with_sends(mut self, events: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.sends.extend(events.into_iter().map(|e| e.into()));
        self
    }

    /// Attach the port-addressed payload sends for this step.
    pub fn with_port_sends(mut self, port_sends: Vec<(String, Value)>) -> Self {
        self.port_sends = port_sends;
        self
    }
}

/// Extended result for parallel state machine execution.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ParallelStepResult {
    /// Current state of each region (region name -> state name).
    pub region_states: HashMap<String, String>,
    /// Any outputs produced by the step.
    pub outputs: Vec<String>,
    /// Internal events generated during this step.
    pub internal_events: Vec<String>,
    /// Whether execution has completed.
    pub completed: bool,
    /// Timing and other context variables.
    pub context: HashMap<String, f64>,
}

impl ParallelStepResult {
    /// Create a new parallel step result.
    pub fn new() -> Self {
        ParallelStepResult {
            region_states: HashMap::new(),
            outputs: Vec::new(),
            internal_events: Vec::new(),
            completed: false,
            context: HashMap::new(),
        }
    }

    /// Set the state for a region.
    pub fn with_region_state(
        mut self,
        region: impl Into<String>,
        state: impl Into<String>,
    ) -> Self {
        self.region_states.insert(region.into(), state.into());
        self
    }

    /// Add an output.
    pub fn with_output(mut self, output: impl Into<String>) -> Self {
        self.outputs.push(output.into());
        self
    }

    /// Add an internal event.
    pub fn with_internal_event(mut self, event: impl Into<String>) -> Self {
        self.internal_events.push(event.into());
        self
    }

    /// Mark as completed.
    pub fn completed(mut self) -> Self {
        self.completed = true;
        self
    }
}

impl Default for ParallelStepResult {
    fn default() -> Self {
        Self::new()
    }
}

/// History pseudo-state kind (SysML v2 spec).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryKind {
    Shallow,
    Deep,
}

/// Assignment operator for structured actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignmentOp {
    /// Direct assignment (=)
    Set,
    /// Addition assignment (+=)
    Add,
    /// Subtraction assignment (-=)
    Subtract,
}

/// A variable assignment in a structured action.
#[derive(Debug, Clone, PartialEq)]
pub struct AssignmentIR {
    /// The variable name being assigned.
    pub variable: String,
    /// The assignment operator.
    pub operator: AssignmentOp,
    /// The literal value being assigned when no expression is present.
    pub value: sysml_core::Value,
    /// Optional compiled RHS expression evaluated at action execution time.
    pub value_expr: Option<crate::expressions::ExprIR>,
    /// Original RHS source string for diagnostics and formatting.
    pub value_source: Option<String>,
}

impl AssignmentIR {
    /// Create a new assignment.
    pub fn new(variable: impl Into<String>, operator: AssignmentOp, value: impl Into<sysml_core::Value>) -> Self {
        AssignmentIR {
            variable: variable.into(),
            operator,
            value: value.into(),
            value_expr: None,
            value_source: None,
        }
    }

    /// Create an assignment whose RHS is evaluated as an expression at runtime.
    pub fn from_expr(
        variable: impl Into<String>,
        operator: AssignmentOp,
        source: impl Into<String>,
        expr: crate::expressions::ExprIR,
    ) -> Self {
        let source = source.into();
        AssignmentIR {
            variable: variable.into(),
            operator,
            value: sysml_core::Value::Null,
            value_expr: Some(expr),
            value_source: Some(source),
        }
    }

    /// Create a set assignment (x = value).
    pub fn set(variable: impl Into<String>, value: impl Into<sysml_core::Value>) -> Self {
        Self::new(variable, AssignmentOp::Set, value)
    }

    /// Create an add assignment (x += value).
    pub fn add(variable: impl Into<String>, value: impl Into<sysml_core::Value>) -> Self {
        Self::new(variable, AssignmentOp::Add, value)
    }

    /// Create a subtract assignment (x -= value).
    pub fn subtract(variable: impl Into<String>, value: impl Into<sysml_core::Value>) -> Self {
        Self::new(variable, AssignmentOp::Subtract, value)
    }

    /// Extract the f64 from the value, if it is numeric (Float or Int).
    /// Returns `None` for non-numeric values (e.g., Bool, String).
    pub fn as_f64(&self) -> Option<f64> {
        if self.value_expr.is_some() {
            return None;
        }
        match &self.value {
            sysml_core::Value::Float(f) => Some(*f),
            sysml_core::Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
}

/// Action IR for state machine transitions: entry/exit/transition actions.
///
/// This represents simple action effects within state machine transitions,
/// NOT the full control-flow graph (see `ActionGraphIR` in `sysml-run-actions`).
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionActionIR {
    /// Simple action as a string (backward compatible).
    Simple(String),
    /// Structured action with variable assignments and send events.
    Structured {
        /// Variable assignments (e.g., t += 10).
        assignments: Vec<AssignmentIR>,
        /// Events to send to the event queue.
        sends: Vec<String>,
        /// Port-addressed sends carrying a payload expression:
        /// `(port_name, payload_expr)`. Evaluated at execution time into
        /// `(port_name, Value)` and routed as an addressed MessageTransfer
        /// (`{owner}.{port}`). Distinct from `sends`, which keeps the string
        /// trace surface (snapshot-asserted via `SubsystemState.sends`) that
        /// structurally cannot carry a payload Value. Populated only by the SM
        /// compiler's `send <payload> via <port>` lowering.
        port_send_ops: Vec<(String, crate::expressions::ExprIR)>,
    },
}

impl TransitionActionIR {
    /// Create a simple action from a string.
    pub fn simple(action: impl Into<String>) -> Self {
        TransitionActionIR::Simple(action.into())
    }

    /// Create a structured action (no port-addressed payload sends).
    pub fn structured(assignments: Vec<AssignmentIR>, sends: Vec<String>) -> Self {
        TransitionActionIR::Structured {
            assignments,
            sends,
            port_send_ops: Vec::new(),
        }
    }

    /// Create a structured action carrying port-addressed payload sends.
    pub fn structured_with_ports(
        assignments: Vec<AssignmentIR>,
        sends: Vec<String>,
        port_send_ops: Vec<(String, crate::expressions::ExprIR)>,
    ) -> Self {
        TransitionActionIR::Structured {
            assignments,
            sends,
            port_send_ops,
        }
    }

    /// Check if this is a simple action.
    pub fn is_simple(&self) -> bool {
        matches!(self, TransitionActionIR::Simple(_))
    }

    /// Get the simple action string if this is a simple action.
    pub fn as_simple(&self) -> Option<&str> {
        match self {
            TransitionActionIR::Simple(s) => Some(s),
            TransitionActionIR::Structured { .. } => None,
        }
    }
}

impl From<String> for TransitionActionIR {
    fn from(s: String) -> Self {
        TransitionActionIR::Simple(s)
    }
}

impl From<&str> for TransitionActionIR {
    fn from(s: &str) -> Self {
        TransitionActionIR::Simple(s.to_owned())
    }
}

/// Parallel region within a composite state machine.
#[derive(Debug, Clone)]
pub struct RegionIR {
    /// The region name.
    pub name: String,
    /// All states in this region.
    pub states: Vec<StateIR>,
    /// All transitions in this region.
    pub transitions: Vec<TransitionIR>,
    /// The initial state name for this region.
    pub initial: String,
}

impl RegionIR {
    /// Create a new region.
    pub fn new(name: impl Into<String>, initial: impl Into<String>) -> Self {
        RegionIR {
            name: name.into(),
            states: Vec::new(),
            transitions: Vec::new(),
            initial: initial.into(),
        }
    }

    /// Add a state to this region.
    pub fn with_state(mut self, state: StateIR) -> Self {
        self.states.push(state);
        self
    }

    /// Add a transition to this region.
    pub fn with_transition(mut self, transition: TransitionIR) -> Self {
        self.transitions.push(transition);
        self
    }

    /// Find a state by name.
    pub fn find_state(&self, name: &str) -> Option<&StateIR> {
        self.states.iter().find(|s| s.name == name)
    }

    /// Find states that have no outgoing transitions (terminal/sink states).
    ///
    /// Terminal states represent endpoints like trip, lockout, or disconnected.
    /// When a state machine reaches a terminal state, it cannot leave without
    /// external reset. Used by flow gating to detect when an owning part's
    /// flows should be blocked.
    pub fn terminal_states(&self) -> Vec<&str> {
        self.states
            .iter()
            .filter(|s| {
                !self.transitions.iter().any(|t| t.from == s.name)
            })
            .map(|s| s.name.as_str())
            .collect()
    }

    /// Get all transitions from a given state.
    pub fn transitions_from(&self, state: &str) -> Vec<&TransitionIR> {
        self.transitions
            .iter()
            .filter(|t| t.from == state)
            .collect()
    }
}

/// Trait for executable runners.
///
/// A runner maintains state and can be stepped through execution
/// by providing optional events.
pub trait Runner {
    /// Reset the runner to its initial state.
    fn reset(&mut self);

    /// Execute a single step, optionally triggered by an event.
    ///
    /// # Arguments
    ///
    /// * `event` - An optional event name that triggers the step
    ///
    /// # Returns
    ///
    /// The result of the step including the new state and any outputs.
    fn step(&mut self, event: Option<&str>) -> StepResult;

    /// Get the current state name.
    fn current_state(&self) -> &str;

    /// Check if execution is complete.
    fn is_completed(&self) -> bool;
}

/// Trait for compiling a ModelGraph to an IR type.
pub trait CompileToIR<T> {
    /// Compile a model graph to the target IR.
    ///
    /// # Arguments
    ///
    /// * `graph` - The model graph to compile
    ///
    /// # Returns
    ///
    /// The compiled IR on success, or diagnostics on failure.
    fn compile(graph: &ModelGraph) -> Result<T, Vec<Diagnostic>>;
}

/// IR for a state machine.
#[derive(Debug, Clone)]
pub struct StateMachineIR {
    /// The name of this state machine.
    pub name: String,
    /// All states in the machine (for simple, non-parallel state machines).
    pub states: Vec<StateIR>,
    /// All transitions in the machine (for simple, non-parallel state machines).
    pub transitions: Vec<TransitionIR>,
    /// The initial state name (for simple, non-parallel state machines).
    pub initial: String,
    /// Parallel regions (for composite state machines with concurrent regions).
    pub regions: Vec<RegionIR>,
}

impl StateMachineIR {
    /// Create a new state machine IR.
    pub fn new(name: impl Into<String>, initial: impl Into<String>) -> Self {
        StateMachineIR {
            name: name.into(),
            states: Vec::new(),
            transitions: Vec::new(),
            initial: initial.into(),
            regions: Vec::new(),
        }
    }

    /// Create a parallel state machine with regions.
    pub fn parallel(name: impl Into<String>) -> Self {
        StateMachineIR {
            name: name.into(),
            states: Vec::new(),
            transitions: Vec::new(),
            initial: String::new(),
            regions: Vec::new(),
        }
    }

    /// Add a region to this state machine.
    pub fn with_region(mut self, region: RegionIR) -> Self {
        self.regions.push(region);
        self
    }

    /// Check if this is a parallel state machine (has regions).
    pub fn is_parallel(&self) -> bool {
        !self.regions.is_empty()
    }

    /// Get a region by name.
    pub fn find_region(&self, name: &str) -> Option<&RegionIR> {
        self.regions.iter().find(|r| r.name == name)
    }

    /// Add a state.
    pub fn with_state(mut self, state: StateIR) -> Self {
        self.states.push(state);
        self
    }

    /// Add a transition.
    pub fn with_transition(mut self, transition: TransitionIR) -> Self {
        self.transitions.push(transition);
        self
    }

    /// Find a state by name.
    pub fn find_state(&self, name: &str) -> Option<&StateIR> {
        self.states.iter().find(|s| s.name == name)
    }

    /// Find states that have no outgoing transitions (terminal/sink states).
    ///
    /// Terminal states represent endpoints like trip, lockout, or disconnected.
    /// When a state machine reaches a terminal state, it cannot leave without
    /// external reset. Used by flow gating to detect when an owning part's
    /// flows should be blocked.
    pub fn terminal_states(&self) -> Vec<&str> {
        self.states
            .iter()
            .filter(|s| {
                !self.transitions.iter().any(|t| t.from == s.name)
            })
            .map(|s| s.name.as_str())
            .collect()
    }

    /// Get all transitions from a given state.
    pub fn transitions_from(&self, state: &str) -> Vec<&TransitionIR> {
        self.transitions
            .iter()
            .filter(|t| t.from == state)
            .collect()
    }
}

/// IR for a state within a state machine.
#[derive(Debug, Clone)]
pub struct StateIR {
    /// The state name.
    pub name: String,
    /// Entry action (optional).
    pub entry_action: Option<TransitionActionIR>,
    /// Do action (optional) - executed each step while in the state.
    pub do_action: Option<TransitionActionIR>,
    /// Exit action (optional).
    pub exit_action: Option<TransitionActionIR>,
    /// Whether this is a final state.
    pub is_final: bool,
    /// Nested state machine for composite states (states containing child state machines).
    pub sub_machine: Option<Box<StateMachineIR>>,
    /// History pseudo-state kind (shallow or deep), if any.
    pub history: Option<HistoryKind>,
    /// Events deferred while in this state (replayed on exit).
    pub deferred_events: Vec<String>,
}

impl StateIR {
    /// Create a new state IR.
    pub fn new(name: impl Into<String>) -> Self {
        StateIR {
            name: name.into(),
            entry_action: None,
            do_action: None,
            exit_action: None,
            is_final: false,
            sub_machine: None,
            history: None,
            deferred_events: Vec::new(),
        }
    }

    /// Set entry action (accepts string or TransitionActionIR).
    pub fn with_entry(mut self, action: impl Into<TransitionActionIR>) -> Self {
        self.entry_action = Some(action.into());
        self
    }

    /// Set exit action (accepts string or TransitionActionIR).
    pub fn with_exit(mut self, action: impl Into<TransitionActionIR>) -> Self {
        self.exit_action = Some(action.into());
        self
    }

    /// Set do action (accepts string or TransitionActionIR).
    pub fn with_do_action(mut self, action: impl Into<TransitionActionIR>) -> Self {
        self.do_action = Some(action.into());
        self
    }

    /// Set a structured entry action.
    pub fn with_entry_action(mut self, action: TransitionActionIR) -> Self {
        self.entry_action = Some(action);
        self
    }

    /// Set a structured exit action.
    pub fn with_exit_action(mut self, action: TransitionActionIR) -> Self {
        self.exit_action = Some(action);
        self
    }

    /// Mark as final state.
    pub fn final_state(mut self) -> Self {
        self.is_final = true;
        self
    }

    /// Set a nested sub-machine for composite states.
    pub fn with_sub_machine(mut self, sm: StateMachineIR) -> Self {
        self.sub_machine = Some(Box::new(sm));
        self
    }

    /// Set the history pseudo-state kind (shallow or deep).
    pub fn with_history(mut self, kind: HistoryKind) -> Self {
        self.history = Some(kind);
        self
    }

    /// Set events deferred while in this state (replayed on exit).
    pub fn with_deferred(mut self, events: Vec<String>) -> Self {
        self.deferred_events = events;
        self
    }
}

/// IR for a transition between states.
#[derive(Debug, Clone)]
pub struct TransitionIR {
    /// The source state name.
    pub from: String,
    /// The target state name.
    pub to: String,
    /// The triggering event (optional).
    pub event: Option<String>,
    /// The guard condition (optional, as string expression).
    pub guard: Option<String>,
    /// The action to execute (optional).
    pub action: Option<TransitionActionIR>,
    /// Whether this is a completion transition (fires only when the source state's
    /// do-activity has completed). Set automatically by the compiler for null-event
    /// transitions whose source state has a do-action.
    pub is_completion: bool,
    /// Whether this transition has a guard but no real event trigger.
    /// Guard-only transitions fire automatically when their guard condition becomes
    /// true, without requiring an explicit event. Set by the compiler when the
    /// transition has a guard but the "event" field is just a label/name (not from
    /// an `accept` trigger in the SysML source).
    pub is_guard_only: bool,
    /// The accept parameter name for an `accept <name> via <port>` trigger, if
    /// declared. The canonical trigger string (`event`) cannot carry it, so it
    /// is threaded separately into the `PortMessage` trigger's `param_name` and
    /// bound to the delivered payload at tick time.
    pub accept_param: Option<String>,
}

impl TransitionIR {
    /// Create a new transition IR.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        TransitionIR {
            from: from.into(),
            to: to.into(),
            event: None,
            guard: None,
            action: None,
            is_completion: false,
            is_guard_only: false,
            accept_param: None,
        }
    }

    /// Set the triggering event.
    pub fn with_event(mut self, event: impl Into<String>) -> Self {
        self.event = Some(event.into());
        self
    }

    /// Set the accept parameter name (`accept <name> via <port>`).
    pub fn with_accept_param(mut self, name: impl Into<String>) -> Self {
        self.accept_param = Some(name.into());
        self
    }

    /// Set the guard condition.
    pub fn with_guard(mut self, guard: impl Into<String>) -> Self {
        self.guard = Some(guard.into());
        self
    }

    /// Set the action (accepts string or TransitionActionIR).
    pub fn with_action(mut self, action: impl Into<TransitionActionIR>) -> Self {
        self.action = Some(action.into());
        self
    }

    /// Set a structured action.
    pub fn with_action_ir(mut self, action: TransitionActionIR) -> Self {
        self.action = Some(action);
        self
    }

    /// Mark this transition as a completion transition.
    ///
    /// Completion transitions only fire when the source state's do-activity
    /// has completed, rather than on every null-event step.
    pub fn completion(mut self) -> Self {
        self.is_completion = true;
        self
    }

    /// Mark this transition as guard-only (no real event trigger).
    ///
    /// Guard-only transitions fire automatically when their guard condition
    /// becomes true, without requiring an explicit event injection.
    pub fn guard_only(mut self) -> Self {
        self.is_guard_only = true;
        self
    }

    /// Check if this transition matches an event.
    ///
    /// Completion transitions (no event, source state has do-activity) return `false`
    /// here — they are handled separately by the runner after the do-activity completes.
    /// Auto-transitions (no event, no do-activity) still fire immediately.
    pub fn matches(&self, event: Option<&str>) -> bool {
        match (&self.event, event) {
            (None, _) if self.is_completion => false, // Completion: runner handles separately
            (None, _) => true,                         // Auto: always fire
            (Some(e), Some(ev)) => e == ev,
            (Some(_), None) => false,
        }
    }
}

/// IR for a constraint.
#[derive(Debug, Clone, Default)]
pub struct ConstraintIR {
    /// The constraint expression as a string.
    pub expr: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Owner element ID (for scope-aware evaluation).
    pub owner_id: Option<sysml_core::ElementId>,
    /// When true, this constraint comes from a negated `assert not constraint`
    /// usage: per SysML §7.20 the inner constraint is asserted to be *false*,
    /// so a decided verdict is inverted (inner-true ⇒ violated, inner-false ⇒
    /// satisfied). Read from the `AssertConstraintUsage` element's `isNegated`
    /// property at extraction. Mirrors `RequirementConstraintIR::is_negated`.
    pub is_negated: bool,
}

impl ConstraintIR {
    /// Create a new constraint IR.
    pub fn new(expr: impl Into<String>) -> Self {
        ConstraintIR {
            expr: expr.into(),
            description: None,
            owner_id: None,
            is_negated: false,
        }
    }

    /// Set the description.
    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn step_result_creation() {
        let result = StepResult::new("initial")
            .with_output("started")
            .with_output("ready");

        assert_eq!(result.state, "initial");
        assert_eq!(result.outputs.len(), 2);
        assert!(!result.completed);
    }

    #[test]
    fn step_result_completed() {
        let result = StepResult::new("final").completed();
        assert!(result.completed);
    }

    #[test]
    fn state_machine_ir_creation() {
        let ir = StateMachineIR::new("TestMachine", "initial")
            .with_state(StateIR::new("initial"))
            .with_state(StateIR::new("running"))
            .with_state(StateIR::new("final").final_state())
            .with_transition(TransitionIR::new("initial", "running").with_event("start"))
            .with_transition(TransitionIR::new("running", "final").with_event("stop"));

        assert_eq!(ir.name, "TestMachine");
        assert_eq!(ir.states.len(), 3);
        assert_eq!(ir.transitions.len(), 2);
        assert_eq!(ir.initial, "initial");
    }

    #[test]
    fn find_state() {
        let ir = StateMachineIR::new("Test", "s1")
            .with_state(StateIR::new("s1"))
            .with_state(StateIR::new("s2"));

        assert!(ir.find_state("s1").is_some());
        assert!(ir.find_state("s3").is_none());
    }

    #[test]
    fn transitions_from() {
        let ir = StateMachineIR::new("Test", "s1")
            .with_transition(TransitionIR::new("s1", "s2").with_event("e1"))
            .with_transition(TransitionIR::new("s1", "s3").with_event("e2"))
            .with_transition(TransitionIR::new("s2", "s3").with_event("e3"));

        let from_s1 = ir.transitions_from("s1");
        assert_eq!(from_s1.len(), 2);
    }

    #[test]
    fn transition_matching() {
        let t1 = TransitionIR::new("s1", "s2").with_event("click");
        let t2 = TransitionIR::new("s1", "s2"); // Auto-transition

        assert!(t1.matches(Some("click")));
        assert!(!t1.matches(Some("hover")));
        assert!(!t1.matches(None));

        assert!(t2.matches(Some("anything")));
        assert!(t2.matches(None));

        // Completion transition: matches() returns false (runner handles separately)
        let t3 = TransitionIR::new("s1", "s2").completion();
        assert!(!t3.matches(None), "completion transition should not match via matches()");
        assert!(!t3.matches(Some("anything")), "completion transition should not match any event");
    }

    #[test]
    fn state_with_actions() {
        let state = StateIR::new("running")
            .with_entry("onEnter()")
            .with_exit("onExit()");

        assert_eq!(
            state.entry_action.as_ref().and_then(|a| a.as_simple()),
            Some("onEnter()")
        );
        assert_eq!(
            state.exit_action.as_ref().and_then(|a| a.as_simple()),
            Some("onExit()")
        );
    }

    #[test]
    fn parallel_step_result() {
        let result = ParallelStepResult::new()
            .with_region_state("grid", "energized")
            .with_region_state("relay", "closed")
            .with_output("initialized")
            .with_internal_event("gridReady");

        assert_eq!(
            result.region_states.get("grid"),
            Some(&"energized".to_string())
        );
        assert_eq!(
            result.region_states.get("relay"),
            Some(&"closed".to_string())
        );
        assert_eq!(result.outputs.len(), 1);
        assert_eq!(result.internal_events.len(), 1);
    }

    #[test]
    fn region_ir() {
        let region = RegionIR::new("grid", "energized")
            .with_state(StateIR::new("energized"))
            .with_state(StateIR::new("deEnergized"))
            .with_transition(TransitionIR::new("energized", "deEnergized").with_event("gridFail"));

        assert_eq!(region.name, "grid");
        assert_eq!(region.initial, "energized");
        assert_eq!(region.states.len(), 2);
        assert_eq!(region.transitions.len(), 1);
        assert!(region.find_state("energized").is_some());
        assert_eq!(region.transitions_from("energized").len(), 1);
    }

    #[test]
    fn action_ir_types() {
        let simple = TransitionActionIR::simple("doSomething()");
        assert!(simple.is_simple());
        assert_eq!(simple.as_simple(), Some("doSomething()"));

        let structured = TransitionActionIR::structured(
            vec![AssignmentIR::add("t", 10.0)],
            vec!["eventA".to_string()],
        );
        assert!(!structured.is_simple());
        assert_eq!(structured.as_simple(), None);
    }

    #[test]
    fn assignment_ir() {
        let set = AssignmentIR::set("x", 5.0);
        assert_eq!(set.variable, "x");
        assert_eq!(set.operator, AssignmentOp::Set);
        assert_eq!(set.value, sysml_core::Value::Float(5.0));

        let add = AssignmentIR::add("t", 10.0);
        assert_eq!(add.operator, AssignmentOp::Add);
    }

    #[test]
    fn parallel_state_machine_ir() {
        let ir = StateMachineIR::parallel("HybridSystem")
            .with_region(RegionIR::new("grid", "energized"))
            .with_region(RegionIR::new("relay", "closed"));

        assert!(ir.is_parallel());
        assert_eq!(ir.regions.len(), 2);
        assert!(ir.find_region("grid").is_some());
        assert!(ir.find_region("unknown").is_none());
    }

    #[test]
    fn constraint_ir() {
        let constraint =
            ConstraintIR::new("speed < 100").with_description("Speed limit constraint");

        assert_eq!(constraint.expr, "speed < 100");
        assert!(constraint.description.is_some());
    }
}

/// State of a single subsystem at a tick.
/// Defined here (not in orchestrator) to avoid type cycles.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct SubsystemState {
    /// Subsystem name.
    pub name: String,
    /// Kind label ("stateMachine", "action", "ode", etc.).
    #[cfg_attr(feature = "serde", serde(skip))]
    #[cfg_attr(feature = "schemars", schemars(skip))]
    pub kind: &'static str,
    /// Current state name (for SM) or current node ID (for action).
    pub current_state: String,
    /// Whether this subsystem has completed.
    pub completed: bool,
    /// Available transitions from current state: `(event_name, target_state)`.
    pub available_transitions: Vec<(String, String)>,
    /// Trace outputs from this tick.
    pub outputs: Vec<String>,
    /// Events sent by this subsystem during this tick.
    pub sends: Vec<String>,
    /// The triggering event that caused entry into `current_state` this tick,
    /// when a *triggered* (message/event) transition fired; `None` otherwise.
    /// Mirrors `StatePerformances.kerml:48` `incomingTransitionTrigger`.
    ///
    /// SPEC-SILENT: records the triggering event name; the full `MessageTransfer`
    /// identity is deferred.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub incoming_transition_trigger: Option<String>,
    /// Number of deferred events currently queued for this subsystem.
    #[cfg_attr(feature = "serde", serde(default))]
    pub deferred_event_count: usize,
    /// `ElementId` of the source model element this subsystem was
    /// compiled from (e.g., the `StateUsage` / `StateDefinition` /
    /// ODE owner). Copied from `Subsystem.source_element_id` at
    /// `subsystem_states.insert` time so the projection layer
    /// (`SubsystemView`) can forward an authoritative element id to
    /// the frontend without needing access to the orchestrator state.
    /// `None` when the source subsystem has no element id (legacy /
    /// test-only `add_*` paths).
    #[cfg_attr(feature = "serde", serde(default, skip_serializing_if = "Option::is_none"))]
    pub source_element_id: Option<sysml_core::ElementId>,
}

/// Captured variable state at one orchestrator tick.
/// Cycle-free: does not reference EvalContext or ExecutionSnapshot.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct TickSnapshot {
    /// Tick number.
    pub tick: u64,
    /// Simulation time in milliseconds.
    pub time_ms: f64,
    /// Variable bindings at this tick.
    pub variables: HashMap<String, Value>,
    /// State of each subsystem at this tick.
    pub subsystem_states: HashMap<String, SubsystemState>,
}

pub mod breakpoint;
pub mod causation;
pub mod compiler;
pub mod expressions;
pub mod actions;
pub mod flows;
pub mod links;
pub mod exchange;
pub mod calculations;
pub mod cases;
pub mod statemachine;
pub mod constraints;
pub mod orchestrator;
pub mod scheduler;
pub mod hybrid;
pub mod health;
pub mod solver;
pub mod solver_builtins;
pub mod solver_external;
pub mod solver_plugin;
pub mod solver_registry;
pub mod sequence;
pub mod slots;
pub mod quantity_health;
pub mod view_condition;
#[cfg(feature = "montecarlo")]
pub mod montecarlo;
pub mod ode;
pub mod ode45;
pub mod ode_builder;
pub mod ode_events;
pub mod solvers;
pub mod clock;
pub mod occurrence;
pub mod observables;
pub mod physics;
pub mod snapshot_view;
pub mod snapshot_diff;
pub mod timeseries;
pub mod aggregates;
pub mod session_events;
pub mod step_size_advisory;

pub use breakpoint::{Breakpoint, BreakpointId, CompareOp, new_breakpoint_id};
pub use causation::{
    CausationEvent, CausationKind, CausationRecorder, DEFAULT_TRACE_DEPTH, MAX_CAUSATION_EVENTS,
};
pub use cases::{EvidenceRef, Verdict, VerdictKind};
pub use observables::{
    measure_observable, measure_observables, ObservableKind, ObservableSpec, OutputBundle,
    OutputEvidence, OutputTarget, OutputValue, Window,
};
pub use occurrence::{Occurrence, OccurrenceKind, OccurrenceTracker};
pub use solver_plugin::{
    ParamDirection, SolverCapabilities, SolverError, SolverParam, SolverPlugin, SolverResult,
};
pub use solver_registry::SolverRegistry;
pub use slots::{
    RuntimeId, SharedSlotStore, SlotId, SlotMeta, SlotStore, SlotWriteError, Variability, WriterId,
};
pub use clock::{LocalClock, ClockRegistry};
pub use links::{
    classify_links, classify_links_from_graph, ClassDistribution, LinkClass, LinkEndpoint,
    LinkGraph, LinkId, LinkIR, LinkSourceKind,
};
pub use sequence::{
    FragmentOperand, FragmentOperator, InteractionFragment, SuccessionConstraint, SuccessionQueue,
};
