//! Sequence trace generation from simulation runs.
//!
//! Records flow events as an interaction model with lifelines (for parts)
//! and messages (for flows). Can be generated from FlowRouter events or
//! orchestrator execution snapshots.
//!
//! ## Architecture
//!
//! ```text
//! FlowRouter events / ExecutionSnapshots
//!     │
//!     ▼
//! SequenceTraceBuilder::record_*()
//!     │
//!     ▼
//! SequenceTrace { lifelines, messages }
//!     │
//!     ▼
//! JSON output or SModel sequence diagram
//! ```

#![allow(clippy::indexing_slicing)]
use std::collections::HashMap;

use sysml_core::Value;

use crate::expressions::{EvalContext, ExprIR, ExpressionEvaluator};

// ---------------------------------------------------------------------------
// Succession constraints (Feature 10.2: HappensBefore temporal ordering)
// ---------------------------------------------------------------------------

/// A temporal ordering constraint: occurrence A must happen before occurrence B.
///
/// Optionally includes a minimum delay, a maximum delay (deadline), and a guard
/// condition. Models the SysML v2 `succession` / `HappensBefore` relationship.
#[derive(Debug, Clone)]
pub struct SuccessionConstraint {
    /// Name/ID of the predecessor occurrence.
    pub before: String,
    /// Name/ID of the successor occurrence.
    pub after: String,
    /// Minimum time delay between before and after (in seconds). None = no delay.
    pub min_delay: Option<f64>,
    /// Maximum time delay (deadline). None = no deadline.
    pub max_delay: Option<f64>,
    /// Guard expression that must be true for the succession to fire.
    pub guard: Option<ExprIR>,
}

impl SuccessionConstraint {
    /// Create a new succession constraint: `before` must happen before `after`.
    pub fn new(before: impl Into<String>, after: impl Into<String>) -> Self {
        Self {
            before: before.into(),
            after: after.into(),
            min_delay: None,
            max_delay: None,
            guard: None,
        }
    }

    /// Set a minimum delay (in seconds) between before and after.
    pub fn with_min_delay(mut self, delay: f64) -> Self {
        self.min_delay = Some(delay);
        self
    }

    /// Set a maximum delay / deadline (in seconds) between before and after.
    pub fn with_max_delay(mut self, delay: f64) -> Self {
        self.max_delay = Some(delay);
        self
    }

    /// Set a guard expression that must be true for the successor to fire.
    pub fn with_guard(mut self, guard: ExprIR) -> Self {
        self.guard = Some(guard);
        self
    }
}

/// A pending successor waiting to fire.
#[derive(Debug, Clone)]
struct PendingSuccessor {
    /// Name of the successor occurrence.
    after: String,
    /// Earliest time (in seconds) the successor can fire.
    ready_time: f64,
    /// Guard expression (cloned from the constraint).
    guard: Option<ExprIR>,
}

/// Tracks pending succession constraints during simulation.
///
/// The queue holds registered constraints and manages pending successors.
/// When a predecessor occurrence completes, `notify_completed` enqueues its
/// successors. `drain_ready` returns successors whose delay has elapsed and
/// whose guard (if any) evaluates to true.
#[derive(Debug, Default, Clone)]
pub struct SuccessionQueue {
    /// Registered succession constraints.
    constraints: Vec<SuccessionConstraint>,
    /// Pending successors waiting to fire.
    pending: Vec<PendingSuccessor>,
    /// Names of successors evicted by `drain_ready` because their
    /// model-declared `max_delay` deadline elapsed before the guard fired
    /// (or before firing at all). Accumulates until drained by
    /// [`Self::take_deadline_violations`] — surfaced, not silently dropped.
    deadline_violations: Vec<String>,
}

impl SuccessionQueue {
    /// Create a new empty succession queue.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a succession constraint.
    pub fn add_constraint(&mut self, constraint: SuccessionConstraint) {
        self.constraints.push(constraint);
    }

    /// Notify that an occurrence has completed at the given time (in seconds).
    ///
    /// Checks all registered constraints for matching predecessors and enqueues
    /// their successors as pending, respecting minimum delay.
    pub fn notify_completed(&mut self, occurrence: &str, current_time: f64) {
        let triggered: Vec<_> = self
            .constraints
            .iter()
            .filter(|c| c.before == occurrence)
            .map(|c| PendingSuccessor {
                after: c.after.clone(),
                ready_time: current_time + c.min_delay.unwrap_or(0.0),
                guard: c.guard.clone(),
            })
            .collect();
        self.pending.extend(triggered);
    }

    /// Drain successors that are ready to fire at the given time (in seconds).
    ///
    /// A successor is ready when:
    /// - `current_time >= ready_time` (minimum delay has elapsed)
    /// - The guard (if any) evaluates to `true` in the given context
    ///
    /// Ready successors are removed from the pending queue and their names returned.
    /// Successors that are not yet ready (time or guard) remain pending — UNLESS
    /// the constraint that produced them declared a `max_delay` and that deadline
    /// has now elapsed, in which case they are evicted as a deadline violation
    /// (reusing [`Self::check_deadlines`]'s condition) rather than retained
    /// forever. A successor whose constraint declared no `max_delay` made no
    /// deadline promise, so it is retained indefinitely regardless of how long
    /// it has been pending. Evicted names are recorded in `deadline_violations`,
    /// retrievable via [`Self::take_deadline_violations`] — surfaced, not
    /// silently dropped.
    pub fn drain_ready(&mut self, current_time: f64, ctx: &EvalContext) -> Vec<String> {
        let evaluator = ExpressionEvaluator::new();
        let mut ready = Vec::new();
        let constraints = &self.constraints;
        let mut newly_violated = Vec::new();
        self.pending.retain(|p| {
            if current_time >= p.ready_time {
                // Check guard if present
                let guard_ok = match &p.guard {
                    Some(expr) => evaluator
                        .eval(expr, ctx)
                        .map(|v| match v {
                            Value::Bool(b) => b,
                            _ => false,
                        })
                        .unwrap_or(false),
                    None => true,
                };
                if guard_ok {
                    ready.push(p.after.clone());
                    return false; // remove from pending
                }
            }
            // Not firing this tick — check whether its deadline has elapsed.
            // Same lookup/condition as `check_deadlines`: find the constraint
            // that produced this pending successor and, if it declared a
            // `max_delay`, compare against the origin time.
            if let Some(constraint) = constraints
                .iter()
                .find(|c| c.after == p.after && c.before != p.after)
            {
                if let Some(max_delay) = constraint.max_delay {
                    let origin_time = p.ready_time - constraint.min_delay.unwrap_or(0.0);
                    let deadline = origin_time + max_delay;
                    if current_time > deadline {
                        newly_violated.push(p.after.clone());
                        return false; // evict — deadline promise broken
                    }
                }
            }
            true // keep in pending — no deadline declared, or not yet elapsed
        });
        self.deadline_violations.extend(newly_violated);
        ready
    }

    /// Drain and return the names of successors evicted so far by
    /// [`Self::drain_ready`] for missing their declared `max_delay` deadline.
    ///
    /// Call this after `drain_ready` to surface deadline violations (e.g. for
    /// logging or diagnostics) instead of letting them disappear silently.
    pub fn take_deadline_violations(&mut self) -> Vec<String> {
        std::mem::take(&mut self.deadline_violations)
    }

    /// Check for deadline violations: constraints where `before` completed but
    /// `after` has not fired within `max_delay`.
    ///
    /// Returns the names of successor occurrences that have violated their deadline.
    /// This is a diagnostic check, not enforcement.
    pub fn check_deadlines(&self, current_time: f64) -> Vec<String> {
        // Find pending successors whose deadline has passed.
        // A deadline violation means: ready_time was set (from min_delay), but
        // the constraint also has max_delay, and current_time exceeds
        // (ready_time - min_delay + max_delay).
        let mut violations = Vec::new();
        for pending in &self.pending {
            // Find the constraint that produced this pending successor
            if let Some(constraint) = self
                .constraints
                .iter()
                .find(|c| c.after == pending.after && c.before != pending.after)
            {
                if let Some(max_delay) = constraint.max_delay {
                    let origin_time = pending.ready_time - constraint.min_delay.unwrap_or(0.0);
                    let deadline = origin_time + max_delay;
                    if current_time > deadline {
                        violations.push(pending.after.clone());
                    }
                }
            }
        }
        violations
    }

    /// Reset the queue, clearing all pending successors.
    ///
    /// Registered constraints are preserved.
    pub fn reset(&mut self) {
        self.pending.clear();
        self.deadline_violations.clear();
    }

    /// Return the number of pending successors.
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }
}

// ---------------------------------------------------------------------------
// Sequence trace types
// ---------------------------------------------------------------------------

/// A complete sequence trace from a simulation run.
#[derive(Debug, Clone)]
pub struct SequenceTrace {
    /// Lifelines (participants) in the sequence diagram.
    pub lifelines: Vec<Lifeline>,
    /// Messages exchanged between lifelines.
    pub messages: Vec<SequenceMessage>,
    /// Combined interaction fragments grouping messages.
    pub combined_fragments: Vec<InteractionFragment>,
}

/// A participant lifeline in the sequence diagram.
#[derive(Debug, Clone)]
pub struct Lifeline {
    /// Lifeline index (for message references).
    pub index: usize,
    /// Display name (part name or subsystem name).
    pub name: String,
    /// Element kind (e.g., "part", "stateMachine", "action").
    pub kind: String,
}

// ---------------------------------------------------------------------------
// Combined interaction fragments (Feature 9.2: UML combined fragments)
// ---------------------------------------------------------------------------

/// Interaction fragment operator (UML/SysML combined fragment types).
#[derive(Debug, Clone, PartialEq)]
pub enum FragmentOperator {
    /// Alternative: choose one operand based on guards.
    Alt,
    /// Optional: execute operand if guard is true.
    Opt,
    /// Loop: repeat operand while guard is true.
    Loop,
    /// Parallel: execute all operands concurrently.
    Par,
    /// Sequential: execute operands in order (default composition).
    Seq,
    /// Break: exit enclosing fragment if guard is true.
    Break,
}

/// One operand within a combined fragment.
#[derive(Debug, Clone)]
pub struct FragmentOperand {
    /// Guard condition for this operand (e.g., "[success]", "[failure]").
    pub guard: Option<String>,
    /// Messages within this operand.
    pub messages: Vec<SequenceMessage>,
    /// Nested sub-fragments within this operand.
    pub sub_fragments: Vec<InteractionFragment>,
}

/// A combined interaction fragment grouping messages with control semantics.
#[derive(Debug, Clone)]
pub struct InteractionFragment {
    /// Fragment operator (alt, loop, par, etc.).
    pub operator: FragmentOperator,
    /// Operands within this fragment.
    pub operands: Vec<FragmentOperand>,
}

impl InteractionFragment {
    /// Create a new fragment with the given operator and one empty operand.
    pub fn new(operator: FragmentOperator) -> Self {
        Self {
            operator,
            operands: vec![FragmentOperand {
                guard: None,
                messages: Vec::new(),
                sub_fragments: Vec::new(),
            }],
        }
    }

    /// Add an operand with an optional guard.
    pub fn with_operand(mut self, guard: Option<String>) -> Self {
        self.operands.push(FragmentOperand {
            guard,
            messages: Vec::new(),
            sub_fragments: Vec::new(),
        });
        self
    }

    /// Total message count across all operands (recursive).
    pub fn message_count(&self) -> usize {
        self.operands
            .iter()
            .map(|op| {
                op.messages.len()
                    + op.sub_fragments
                        .iter()
                        .map(|f| f.message_count())
                        .sum::<usize>()
            })
            .sum()
    }
}

// ---------------------------------------------------------------------------
// Sequence trace types
// ---------------------------------------------------------------------------

/// A message between lifelines in the sequence diagram.
#[derive(Debug, Clone)]
pub struct SequenceMessage {
    /// Monotonic sequence number.
    pub sequence: u64,
    /// Source lifeline index.
    pub from_lifeline: usize,
    /// Target lifeline index.
    pub to_lifeline: usize,
    /// Message label (flow name, event name, etc.).
    pub label: String,
    /// Timestamp in milliseconds.
    pub timestamp_ms: f64,
    /// Optional payload value.
    pub payload: Option<Value>,
}

/// Builder for constructing sequence traces from events.
pub struct SequenceTraceBuilder {
    lifelines: Vec<Lifeline>,
    messages: Vec<SequenceMessage>,
    /// Maps participant name → lifeline index for O(1) lookup.
    name_to_index: HashMap<String, usize>,
    /// Monotonic sequence counter.
    sequence: u64,
    /// Stack of active fragments being built.
    fragment_stack: Vec<InteractionFragment>,
    /// Completed fragments.
    completed_fragments: Vec<InteractionFragment>,
}

impl SequenceTraceBuilder {
    pub fn new() -> Self {
        Self {
            lifelines: Vec::new(),
            messages: Vec::new(),
            name_to_index: HashMap::new(),
            sequence: 0,
            fragment_stack: Vec::new(),
            completed_fragments: Vec::new(),
        }
    }

    /// Get or create a lifeline for the given participant name.
    fn ensure_lifeline(&mut self, name: &str, kind: &str) -> usize {
        if let Some(&idx) = self.name_to_index.get(name) {
            return idx;
        }
        let idx = self.lifelines.len();
        self.lifelines.push(Lifeline {
            index: idx,
            name: name.to_owned(),
            kind: kind.to_owned(),
        });
        self.name_to_index.insert(name.to_owned(), idx);
        idx
    }

    /// Record a flow message delivery.
    pub fn record_flow_delivery(
        &mut self,
        source_key: &str,
        target_key: &str,
        flow_id: &str,
        payload: Option<Value>,
        timestamp_ms: f64,
    ) {
        // Parse "owner.port" → owner
        let source_name = source_key.split('.').next().unwrap_or(source_key);
        let target_name = target_key.split('.').next().unwrap_or(target_key);

        let from = self.ensure_lifeline(source_name, "part");
        let to = self.ensure_lifeline(target_name, "part");

        self.sequence += 1;
        let msg = SequenceMessage {
            sequence: self.sequence,
            from_lifeline: from,
            to_lifeline: to,
            label: flow_id.to_owned(),
            timestamp_ms,
            payload,
        };
        if let Some(frag) = self.fragment_stack.last_mut() {
            if let Some(operand) = frag.operands.last_mut() {
                operand.messages.push(msg.clone());
            }
        }
        self.messages.push(msg);
    }

    /// Record a state transition event.
    pub fn record_state_transition(
        &mut self,
        subsystem: &str,
        from_state: &str,
        to_state: &str,
        trigger: Option<&str>,
        timestamp_ms: f64,
    ) {
        let lifeline = self.ensure_lifeline(subsystem, "stateMachine");

        self.sequence += 1;
        let label = if let Some(trig) = trigger {
            format!("{from_state} →[{trig}] {to_state}")
        } else {
            format!("{from_state} → {to_state}")
        };

        // Self-message (state transition is internal)
        let msg = SequenceMessage {
            sequence: self.sequence,
            from_lifeline: lifeline,
            to_lifeline: lifeline,
            label,
            timestamp_ms,
            payload: None,
        };
        if let Some(frag) = self.fragment_stack.last_mut() {
            if let Some(operand) = frag.operands.last_mut() {
                operand.messages.push(msg.clone());
            }
        }
        self.messages.push(msg);
    }

    /// Add a lifeline by name with a default "part" kind.
    ///
    /// This is a convenience method for tests and manual trace construction.
    /// The `record_*` methods automatically create lifelines on demand; this
    /// method is useful when you want to pre-register lifelines before
    /// recording any messages.
    pub fn add_lifeline(&mut self, name: &str) -> usize {
        self.ensure_lifeline(name, "part")
    }

    /// Record a subsystem event injection.
    pub fn record_event_injection(
        &mut self,
        source: &str,
        target: &str,
        event: &str,
        timestamp_ms: f64,
    ) {
        let from = self.ensure_lifeline(source, "external");
        let to = self.ensure_lifeline(target, "subsystem");

        self.sequence += 1;
        let msg = SequenceMessage {
            sequence: self.sequence,
            from_lifeline: from,
            to_lifeline: to,
            label: event.to_owned(),
            timestamp_ms,
            payload: None,
        };
        if let Some(frag) = self.fragment_stack.last_mut() {
            if let Some(operand) = frag.operands.last_mut() {
                operand.messages.push(msg.clone());
            }
        }
        self.messages.push(msg);
    }

    // -----------------------------------------------------------------------
    // Fragment building methods (Feature 9.2)
    // -----------------------------------------------------------------------

    /// Begin a new combined fragment.
    ///
    /// Messages recorded after this call will be added to the fragment's
    /// current operand. Use `add_operand` to start a new branch (for alt/par),
    /// and `end_fragment` to close the fragment.
    pub fn begin_fragment(&mut self, operator: FragmentOperator, guard: Option<String>) {
        let mut frag = InteractionFragment::new(operator);
        if let Some(g) = guard {
            frag.operands[0].guard = Some(g);
        }
        self.fragment_stack.push(frag);
    }

    /// Add a new operand to the current fragment (for alt branches, par regions).
    ///
    /// Subsequent messages will be recorded into this new operand until the
    /// next `add_operand` or `end_fragment` call.
    pub fn add_operand(&mut self, guard: Option<String>) {
        if let Some(frag) = self.fragment_stack.last_mut() {
            frag.operands.push(FragmentOperand {
                guard,
                messages: Vec::new(),
                sub_fragments: Vec::new(),
            });
        }
    }

    /// End the current fragment and either nest it in the parent or add to completed.
    ///
    /// If there is a parent fragment on the stack, the closed fragment is nested
    /// inside the parent's last operand as a sub-fragment. Otherwise, it is added
    /// to the completed fragments list.
    pub fn end_fragment(&mut self) {
        if let Some(frag) = self.fragment_stack.pop() {
            if let Some(parent) = self.fragment_stack.last_mut() {
                // Nest inside parent's last operand
                if let Some(operand) = parent.operands.last_mut() {
                    operand.sub_fragments.push(frag);
                }
            } else {
                self.completed_fragments.push(frag);
            }
        }
    }

    /// Build the final trace.
    pub fn build(self) -> SequenceTrace {
        SequenceTrace {
            lifelines: self.lifelines,
            messages: self.messages,
            combined_fragments: self.completed_fragments,
        }
    }
}

impl Default for SequenceTraceBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Build a sequence trace from orchestrator execution snapshots.
///
/// Extracts flow deliveries and state transitions from the snapshot trace.
pub fn trace_from_snapshots(snapshots: &[crate::orchestrator::ExecutionSnapshot]) -> SequenceTrace {
    let mut builder = SequenceTraceBuilder::new();

    for snap in snapshots {
        // Record flow message deliveries
        for msg in &snap.messages {
            let source_name = msg.source.split('.').next().unwrap_or(&msg.source);
            let target_name = msg.target.split('.').next().unwrap_or(&msg.target);
            builder.record_flow_delivery(
                source_name,
                target_name,
                &msg.flow_id,
                Some(msg.payload.clone()),
                snap.time_ms,
            );
        }

        // Record state transitions (compare with previous snapshot)
        // We detect transitions by checking if subsystem state changed
    }

    builder.build()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn trace_builder_basic() {
        let mut builder = SequenceTraceBuilder::new();
        builder.record_flow_delivery(
            "sensor.tempOut",
            "controller.tempIn",
            "tempFlow",
            Some(Value::Float(92.0)),
            100.0,
        );
        builder.record_state_transition("controller", "idle", "heating", Some("tempHigh"), 100.0);
        builder.record_flow_delivery(
            "controller.heaterOut",
            "heater.powerIn",
            "powerFlow",
            Some(Value::Float(1500.0)),
            200.0,
        );

        let trace = builder.build();
        assert_eq!(trace.lifelines.len(), 3); // sensor, controller, heater
        assert_eq!(trace.messages.len(), 3);

        // First message: sensor → controller
        assert_eq!(trace.messages[0].from_lifeline, 0); // sensor
        assert_eq!(trace.messages[0].to_lifeline, 1); // controller
        assert_eq!(trace.messages[0].label, "tempFlow");

        // Second: controller self-transition
        assert_eq!(trace.messages[1].from_lifeline, 1);
        assert_eq!(trace.messages[1].to_lifeline, 1);
        assert!(trace.messages[1].label.contains("heating"));

        // Third: controller → heater
        assert_eq!(trace.messages[2].from_lifeline, 1);
        assert_eq!(trace.messages[2].to_lifeline, 2);
    }

    #[test]
    fn trace_builder_lifeline_dedup() {
        let mut builder = SequenceTraceBuilder::new();
        builder.record_flow_delivery("a.out", "b.in", "f1", None, 0.0);
        builder.record_flow_delivery("a.out", "b.in", "f2", None, 10.0);
        builder.record_flow_delivery("b.out", "a.in", "f3", None, 20.0);

        let trace = builder.build();
        assert_eq!(trace.lifelines.len(), 2); // only a and b
        assert_eq!(trace.messages.len(), 3);
    }

    #[test]
    fn trace_builder_event_injection() {
        let mut builder = SequenceTraceBuilder::new();
        builder.record_event_injection("user", "controller", "startBrew", 0.0);

        let trace = builder.build();
        assert_eq!(trace.lifelines.len(), 2);
        assert_eq!(trace.messages[0].label, "startBrew");
    }

    #[test]
    fn trace_sequence_numbers_monotonic() {
        let mut builder = SequenceTraceBuilder::new();
        builder.record_flow_delivery("a.x", "b.y", "f1", None, 0.0);
        builder.record_flow_delivery("b.y", "c.z", "f2", None, 1.0);
        builder.record_flow_delivery("c.z", "a.x", "f3", None, 2.0);

        let trace = builder.build();
        for i in 1..trace.messages.len() {
            assert!(trace.messages[i].sequence > trace.messages[i - 1].sequence);
        }
    }

    // -----------------------------------------------------------------------
    // Succession constraint tests (Feature 10.2)
    // -----------------------------------------------------------------------

    #[test]
    fn test_succession_simple_ordering() {
        let mut queue = SuccessionQueue::new();
        queue.add_constraint(SuccessionConstraint::new("A", "B"));

        // Notify A completed at t=0
        queue.notify_completed("A", 0.0);

        // B should be ready immediately
        let ctx = EvalContext::new();
        let ready = queue.drain_ready(0.0, &ctx);
        assert_eq!(ready, vec!["B"]);
    }

    #[test]
    fn test_succession_with_delay() {
        let mut queue = SuccessionQueue::new();
        queue.add_constraint(SuccessionConstraint::new("A", "B").with_min_delay(5.0));

        queue.notify_completed("A", 10.0); // A completes at t=10

        let ctx = EvalContext::new();
        // At t=12, B is not ready (needs t>=15)
        let ready = queue.drain_ready(12.0, &ctx);
        assert!(ready.is_empty());

        // At t=15, B is ready
        let ready = queue.drain_ready(15.0, &ctx);
        assert_eq!(ready, vec!["B"]);
    }

    #[test]
    fn test_succession_with_guard() {
        use crate::expressions::compile_simple_expression;

        let mut queue = SuccessionQueue::new();
        let guard = compile_simple_expression("temperature > 90").unwrap();
        queue.add_constraint(SuccessionConstraint::new("heat", "brew").with_guard(guard));

        queue.notify_completed("heat", 0.0);

        // Guard not met
        let mut ctx = EvalContext::new();
        ctx.set("temperature", Value::Float(50.0));
        let ready = queue.drain_ready(0.0, &ctx);
        assert!(ready.is_empty());

        // Guard met
        ctx.set("temperature", Value::Float(95.0));
        let ready = queue.drain_ready(0.0, &ctx);
        assert_eq!(ready, vec!["brew"]);
    }

    #[test]
    fn test_succession_chain() {
        let mut queue = SuccessionQueue::new();
        queue.add_constraint(SuccessionConstraint::new("A", "B"));
        queue.add_constraint(SuccessionConstraint::new("B", "C"));

        let ctx = EvalContext::new();
        queue.notify_completed("A", 0.0);
        let ready = queue.drain_ready(0.0, &ctx);
        assert_eq!(ready, vec!["B"]);

        // Now B completes, C should be ready
        queue.notify_completed("B", 1.0);
        let ready = queue.drain_ready(1.0, &ctx);
        assert_eq!(ready, vec!["C"]);
    }

    #[test]
    fn test_succession_reset() {
        let mut queue = SuccessionQueue::new();
        queue.add_constraint(SuccessionConstraint::new("A", "B"));
        queue.notify_completed("A", 0.0);
        queue.reset();
        let ctx = EvalContext::new();
        let ready = queue.drain_ready(0.0, &ctx);
        assert!(ready.is_empty());
    }

    #[test]
    fn test_succession_no_match() {
        let mut queue = SuccessionQueue::new();
        queue.add_constraint(SuccessionConstraint::new("A", "B"));

        // Notify a different occurrence — should not trigger anything
        queue.notify_completed("X", 0.0);

        let ctx = EvalContext::new();
        let ready = queue.drain_ready(0.0, &ctx);
        assert!(ready.is_empty());
    }

    #[test]
    fn test_succession_multiple_successors() {
        let mut queue = SuccessionQueue::new();
        queue.add_constraint(SuccessionConstraint::new("A", "B"));
        queue.add_constraint(SuccessionConstraint::new("A", "C"));

        queue.notify_completed("A", 0.0);

        let ctx = EvalContext::new();
        let ready = queue.drain_ready(0.0, &ctx);
        assert_eq!(ready.len(), 2);
        assert!(ready.contains(&"B".to_string()));
        assert!(ready.contains(&"C".to_string()));
    }

    #[test]
    fn test_succession_delay_and_guard_combined() {
        use crate::expressions::compile_simple_expression;

        let mut queue = SuccessionQueue::new();
        let guard = compile_simple_expression("ready == true").unwrap();
        queue.add_constraint(
            SuccessionConstraint::new("prep", "launch")
                .with_min_delay(2.0)
                .with_guard(guard),
        );

        queue.notify_completed("prep", 0.0);

        let mut ctx = EvalContext::new();
        ctx.set("ready", Value::Bool(false));

        // At t=3 (past delay) but guard false — not ready
        let ready = queue.drain_ready(3.0, &ctx);
        assert!(ready.is_empty());

        // Guard met but time not yet — should still be pending from earlier
        // (already past delay, so just guard is blocking)
        ctx.set("ready", Value::Bool(true));
        let ready = queue.drain_ready(3.0, &ctx);
        assert_eq!(ready, vec!["launch"]);
    }

    #[test]
    fn test_succession_pending_count() {
        let mut queue = SuccessionQueue::new();
        queue.add_constraint(SuccessionConstraint::new("A", "B"));
        queue.add_constraint(SuccessionConstraint::new("A", "C"));
        assert_eq!(queue.pending_count(), 0);

        queue.notify_completed("A", 0.0);
        assert_eq!(queue.pending_count(), 2);

        let ctx = EvalContext::new();
        queue.drain_ready(0.0, &ctx);
        assert_eq!(queue.pending_count(), 0);
    }

    #[test]
    fn test_succession_deadline_violation() {
        let mut queue = SuccessionQueue::new();
        queue.add_constraint(SuccessionConstraint::new("A", "B").with_max_delay(5.0));

        queue.notify_completed("A", 10.0);

        // At t=14, no violation yet (within 5s deadline)
        let violations = queue.check_deadlines(14.0);
        assert!(violations.is_empty());

        // At t=16, deadline violated (10 + 5 = 15, and 16 > 15)
        let violations = queue.check_deadlines(16.0);
        assert_eq!(violations, vec!["B"]);
    }

    #[test]
    fn test_drain_ready_evicts_on_elapsed_deadline_with_unmet_guard() {
        use crate::expressions::compile_simple_expression;

        let mut queue = SuccessionQueue::new();
        let guard = compile_simple_expression("ready == true").unwrap();
        queue.add_constraint(
            SuccessionConstraint::new("A", "B")
                .with_max_delay(5.0)
                .with_guard(guard),
        );

        // A completes at t=0 -> B pending, ready_time = 0.0 (no min_delay).
        queue.notify_completed("A", 0.0);

        let mut ctx = EvalContext::new();
        ctx.set("ready", Value::Bool(false)); // guard never fires

        // Before the deadline (origin 0 + max_delay 5 = 5): guard is false,
        // so B doesn't fire, but the deadline hasn't elapsed yet -> retained.
        let ready = queue.drain_ready(3.0, &ctx);
        assert!(ready.is_empty());
        assert_eq!(queue.pending_count(), 1);
        assert!(queue.take_deadline_violations().is_empty());

        // Past the deadline: B is evicted (not fired) and surfaced as a
        // deadline violation instead of being retained forever.
        let ready = queue.drain_ready(6.0, &ctx);
        assert!(ready.is_empty());
        assert_eq!(queue.pending_count(), 0);
        let violations = queue.take_deadline_violations();
        assert_eq!(violations, vec!["B".to_string()]);
    }

    #[test]
    fn test_drain_ready_retains_pending_with_no_max_delay_declared() {
        use crate::expressions::compile_simple_expression;

        let mut queue = SuccessionQueue::new();
        let guard = compile_simple_expression("ready == true").unwrap();
        // No `max_delay` declared -> no deadline promise was made, so this
        // successor must be retained indefinitely, not auto-evicted.
        queue.add_constraint(SuccessionConstraint::new("A", "B").with_guard(guard));

        queue.notify_completed("A", 0.0);

        let mut ctx = EvalContext::new();
        ctx.set("ready", Value::Bool(false));

        // Advance far past any deadline that WOULD have applied if one had
        // been declared -- still retained, and no violation is recorded.
        let ready = queue.drain_ready(1_000.0, &ctx);
        assert!(ready.is_empty());
        assert_eq!(queue.pending_count(), 1);
        assert!(queue.take_deadline_violations().is_empty());
    }

    // -----------------------------------------------------------------------
    // Combined interaction fragment tests (Feature 9.2)
    // -----------------------------------------------------------------------

    #[test]
    fn test_fragment_alt_two_branches() {
        let mut builder = SequenceTraceBuilder::new();
        builder.add_lifeline("client");
        builder.add_lifeline("server");

        builder.begin_fragment(FragmentOperator::Alt, Some("[authenticated]".into()));
        builder.record_event_injection("client", "server", "grant_access", 1.0);
        builder.add_operand(Some("[not authenticated]".into()));
        builder.record_event_injection("client", "server", "deny_access", 2.0);
        builder.end_fragment();

        let trace = builder.build();
        assert_eq!(trace.combined_fragments.len(), 1);
        let frag = &trace.combined_fragments[0];
        assert_eq!(frag.operator, FragmentOperator::Alt);
        assert_eq!(frag.operands.len(), 2);
        assert_eq!(frag.operands[0].guard.as_deref(), Some("[authenticated]"));
        assert_eq!(frag.operands[0].messages.len(), 1);
        assert_eq!(
            frag.operands[1].guard.as_deref(),
            Some("[not authenticated]")
        );
        assert_eq!(frag.operands[1].messages.len(), 1);
    }

    #[test]
    fn test_fragment_loop() {
        let mut builder = SequenceTraceBuilder::new();
        builder.add_lifeline("sensor");
        builder.add_lifeline("controller");

        builder.begin_fragment(FragmentOperator::Loop, Some("[count < 10]".into()));
        builder.record_event_injection("sensor", "controller", "read_temperature", 1.0);
        builder.record_event_injection("controller", "sensor", "process_reading", 2.0);
        builder.end_fragment();

        let trace = builder.build();
        assert_eq!(trace.combined_fragments.len(), 1);
        assert_eq!(trace.combined_fragments[0].operator, FragmentOperator::Loop);
        assert_eq!(trace.combined_fragments[0].message_count(), 2);
    }

    #[test]
    fn test_fragment_par() {
        let mut builder = SequenceTraceBuilder::new();
        builder.add_lifeline("a");
        builder.add_lifeline("b");
        builder.add_lifeline("c");

        builder.begin_fragment(FragmentOperator::Par, None);
        builder.record_event_injection("a", "b", "task_a", 1.0);
        builder.add_operand(None);
        builder.record_event_injection("b", "c", "task_b", 1.0);
        builder.add_operand(None);
        builder.record_event_injection("c", "a", "task_c", 1.0);
        builder.end_fragment();

        let trace = builder.build();
        assert_eq!(trace.combined_fragments[0].operands.len(), 3);
    }

    #[test]
    fn test_fragment_nested() {
        let mut builder = SequenceTraceBuilder::new();
        builder.add_lifeline("x");
        builder.add_lifeline("env");

        builder.begin_fragment(FragmentOperator::Seq, None);
        builder.record_event_injection("env", "x", "step1", 1.0);
        builder.begin_fragment(FragmentOperator::Alt, Some("[hot]".into()));
        builder.record_event_injection("env", "x", "cool_down", 2.0);
        builder.add_operand(Some("[cold]".into()));
        builder.record_event_injection("env", "x", "heat_up", 3.0);
        builder.end_fragment(); // closes alt
        builder.record_event_injection("env", "x", "step2", 4.0);
        builder.end_fragment(); // closes seq

        let trace = builder.build();
        assert_eq!(trace.combined_fragments.len(), 1);
        let seq = &trace.combined_fragments[0];
        assert_eq!(seq.operator, FragmentOperator::Seq);
        assert_eq!(seq.operands[0].sub_fragments.len(), 1); // nested alt
        assert_eq!(
            seq.operands[0].sub_fragments[0].operator,
            FragmentOperator::Alt
        );
    }

    #[test]
    fn test_fragment_message_count() {
        let frag = InteractionFragment::new(FragmentOperator::Alt).with_operand(Some("[b]".into()));
        assert_eq!(frag.message_count(), 0);
    }

    #[test]
    fn test_empty_fragment() {
        let mut builder = SequenceTraceBuilder::new();
        builder.begin_fragment(FragmentOperator::Opt, Some("[debug]".into()));
        builder.end_fragment();

        let trace = builder.build();
        assert_eq!(trace.combined_fragments.len(), 1);
        assert_eq!(trace.combined_fragments[0].operator, FragmentOperator::Opt);
    }

    #[test]
    fn test_fragment_messages_also_in_flat_list() {
        // Messages inside fragments should also appear in the flat messages list
        let mut builder = SequenceTraceBuilder::new();
        builder.add_lifeline("a");
        builder.add_lifeline("b");

        builder.begin_fragment(FragmentOperator::Opt, Some("[flag]".into()));
        builder.record_event_injection("a", "b", "msg1", 1.0);
        builder.end_fragment();

        let trace = builder.build();
        assert_eq!(trace.messages.len(), 1);
        assert_eq!(trace.combined_fragments[0].operands[0].messages.len(), 1);
    }

    #[test]
    fn test_add_lifeline() {
        let mut builder = SequenceTraceBuilder::new();
        let idx0 = builder.add_lifeline("alpha");
        let idx1 = builder.add_lifeline("beta");
        // Adding same name again returns same index
        let idx0_again = builder.add_lifeline("alpha");

        assert_eq!(idx0, 0);
        assert_eq!(idx1, 1);
        assert_eq!(idx0_again, 0);

        let trace = builder.build();
        assert_eq!(trace.lifelines.len(), 2);
    }
}
