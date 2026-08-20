//! # Causation event recorder (R7.1)
//!
//! Records a fine-grained causal history of what happened during simulation:
//! variable writes, transition firings, action invocations, constraint evaluations,
//! event injections, and ODE integration steps. Each event carries optional
//! `caused_by` links to upstream events so that, given a "failure event" (a
//! failed constraint verdict, a breakpoint-hit transition, a violating variable
//! write), the UI can walk **backwards** through the causal graph and explain
//! "why did this happen?".
//!
//! Complements the existing [`crate::orchestrator::CausationLink`], which
//! summarises one-tick cross-subsystem variable-to-guard impacts. This recorder
//! operates on a finer granularity (per-event) and across ticks.
//!
//! ## Shape
//!
//! Each [`CausationEvent`] has:
//! - `id`: stable identifier within the session (`"ev-<tick>-<ordinal>"`)
//! - `tick`: simulation tick at which the event occurred
//! - `kind`: discriminated union of event kinds ([`CausationKind`])
//! - `actor`: the model element / subsystem that triggered it
//! - `target`: optional model element affected (variable name, state id, ...)
//! - `detail`: free-form JSON-serialisable payload (numbers, strings, booleans)
//! - `caused_by`: ids of upstream events that contributed to this one
//!
//! ## Ring buffer
//!
//! Events are kept in a bounded `VecDeque`. When the buffer is full the oldest
//! event is evicted. Default capacity is [`MAX_CAUSATION_EVENTS`] (2048); use
//! [`CausationRecorder::with_capacity`] to override.
//!
//! ## BFS walker
//!
//! [`CausationRecorder::trace`] performs a breadth-first backward walk from a
//! root event along `caused_by` edges. The walker tolerates diamonds (a visited
//! set prevents revisiting the same event), caps depth, and returns a flat
//! chain ordered by BFS traversal (root first, closest causes first).

use std::collections::{HashSet, VecDeque};

use sysml_core::Value;

/// Default maximum number of causation events retained per session.
pub const MAX_CAUSATION_EVENTS: usize = 2048;

/// Default maximum BFS depth used when a client does not specify `max_depth`.
pub const DEFAULT_TRACE_DEPTH: u8 = 5;

/// A discriminated union of causation event kinds.
///
/// Serialised as snake_case (serde default) to match the frontend type mirror.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(tag = "kind", rename_all = "snake_case"))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub enum CausationKind {
    /// A variable in the shared context changed value.
    VariableWrite {
        /// Variable name.
        var: String,
        /// Value before the write. `Value::Null` when the variable did not
        /// previously exist.
        old_value: Value,
        /// Value after the write.
        new_value: Value,
    },
    /// A state-machine transition fired.
    TransitionFire {
        /// Source state name.
        from: String,
        /// Target state name.
        to: String,
        /// Triggering event, if any.
        event: Option<String>,
    },
    /// An action (structured transition action, do-activity, entry/exit) ran.
    ActionInvoke {
        /// Action identifier / source string.
        action: String,
        /// Serialised arguments (empty list when not applicable).
        args: Vec<String>,
    },
    /// A constraint was evaluated and produced a verdict.
    ConstraintEvaluated {
        /// Constraint name or description.
        constraint: String,
        /// `true` if the constraint passed, `false` if it failed.
        verdict: bool,
        /// Computed ("actual") value, if available.
        actual: Option<Value>,
        /// Expected value, if available.
        expected: Option<Value>,
    },
    /// An external event was injected into a subsystem (scheduled or immediate).
    EventInjected {
        /// Event name.
        event: String,
    },
    /// An ODE solver advanced by `dt`, writing new state variables.
    OdeStep {
        /// Time step in seconds.
        dt: f64,
        /// Names of the variables that the solver wrote this step.
        changed_vars: Vec<String>,
    },
}

/// A single recorded causation event.
///
/// Events form a directed acyclic graph via `caused_by` links. The recorder
/// does **not** enforce acyclicity — downstream callers that walk the graph
/// must protect against cycles (the built-in [`CausationRecorder::trace`]
/// walker does so via a visited set).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CausationEvent {
    /// Opaque identifier unique within the recorder's ring buffer.
    /// Shape: `"ev-<tick>-<ordinal>"`.
    pub id: String,
    /// Simulation tick at which the event occurred.
    pub tick: u64,
    /// Event kind + kind-specific payload.
    #[cfg_attr(feature = "serde", serde(flatten))]
    pub kind: CausationKind,
    /// The subsystem / element that produced the event (e.g. state-machine
    /// name, action name, `"orchestrator"`). Empty string if unknown.
    pub actor: String,
    /// Optional target element (variable name, state id, constraint id).
    /// `None` when the event kind has no meaningful target (e.g. an ODE step
    /// writing multiple variables — the target list lives inside the kind).
    pub target: Option<String>,
    /// Human-readable summary for the row label. Intentionally free-form so
    /// the UI doesn't need to re-materialise from `kind`.
    pub detail: String,
    /// Ids of upstream events that contributed to this one.
    pub caused_by: Vec<String>,
}

impl CausationEvent {
    /// Convenience constructor that populates the id lazily (callers that
    /// insert directly into [`CausationRecorder`] should prefer
    /// [`CausationRecorder::record`] which assigns the id itself).
    pub fn new(
        id: impl Into<String>,
        tick: u64,
        kind: CausationKind,
        actor: impl Into<String>,
        target: Option<String>,
        detail: impl Into<String>,
        caused_by: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            tick,
            kind,
            actor: actor.into(),
            target,
            detail: detail.into(),
            caused_by,
        }
    }
}

/// Ring-buffered store of [`CausationEvent`]s plus a backward BFS walker.
///
/// Cheap to clone (`VecDeque` is `Clone`) — needed because
/// `Orchestrator::fork` deep-clones the entire orchestrator, including any
/// owned recorder. The walker methods are non-consuming and work on a borrow.
#[derive(Debug, Clone)]
pub struct CausationRecorder {
    /// Event history, oldest-first.
    events: VecDeque<CausationEvent>,
    /// Maximum retained event count. Oldest entries are evicted on overflow.
    capacity: usize,
    /// Monotonic counter used to disambiguate events within a single tick.
    /// Resets to 0 on [`CausationRecorder::clear`].
    next_ordinal: u64,
}

impl Default for CausationRecorder {
    fn default() -> Self {
        Self::with_capacity(MAX_CAUSATION_EVENTS)
    }
}

impl CausationRecorder {
    /// Create a recorder with the default capacity.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a recorder with a specified ring-buffer capacity.
    ///
    /// A capacity of zero disables recording (all [`CausationRecorder::record`]
    /// calls are dropped silently, and [`CausationRecorder::trace`] returns an
    /// empty chain).
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            events: VecDeque::with_capacity(capacity.min(1024)),
            capacity,
            next_ordinal: 0,
        }
    }

    /// Returns the configured capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Current number of retained events.
    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// True when no events are currently retained.
    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    /// Drop every recorded event and reset the ordinal counter.
    pub fn clear(&mut self) {
        self.events.clear();
        self.next_ordinal = 0;
    }

    /// Append a new event to the ring buffer. Returns the assigned id.
    ///
    /// When `capacity` is zero the event is dropped; the function still
    /// returns the id that *would have been* assigned so callers can
    /// unconditionally thread it into `caused_by` fields of later events
    /// without special-casing the zero-capacity path.
    pub fn record(
        &mut self,
        tick: u64,
        kind: CausationKind,
        actor: impl Into<String>,
        target: Option<String>,
        detail: impl Into<String>,
        caused_by: Vec<String>,
    ) -> String {
        let ordinal = self.next_ordinal;
        self.next_ordinal = self.next_ordinal.wrapping_add(1);
        let id = format!("ev-{tick}-{ordinal}");
        if self.capacity == 0 {
            return id;
        }
        let event = CausationEvent {
            id: id.clone(),
            tick,
            kind,
            actor: actor.into(),
            target,
            detail: detail.into(),
            caused_by,
        };
        self.events.push_back(event);
        while self.events.len() > self.capacity {
            self.events.pop_front();
        }
        id
    }

    /// Iterate over retained events, oldest first.
    pub fn iter(&self) -> std::collections::vec_deque::Iter<'_, CausationEvent> {
        self.events.iter()
    }

    /// Lookup an event by id.
    pub fn find(&self, id: &str) -> Option<&CausationEvent> {
        self.events.iter().find(|e| e.id == id)
    }

    /// Find the most recent event matching a tick and target.
    ///
    /// Iterates from newest to oldest to prefer the latest match. Returns
    /// `None` if nothing matches.
    pub fn find_by_tick_target(&self, tick: u64, target: &str) -> Option<&CausationEvent> {
        self.events
            .iter()
            .rev()
            .find(|e| e.tick == tick && e.target.as_deref() == Some(target))
    }

    /// Breadth-first backward walk from `root_id`, capped at `max_depth`.
    ///
    /// Returns a flat `Vec<CausationEvent>` beginning with the root (at index
    /// 0), followed by its upstream causes in BFS order. Each event is cloned
    /// — the recorder keeps its own copy. The visited set prevents revisiting
    /// the same event twice, so diamond-shaped graphs terminate cleanly.
    ///
    /// `max_depth = 0` returns just the root (or empty if the root is
    /// missing). `max_depth >= 1` includes upstream edges one hop away, and
    /// so on.
    pub fn trace(&self, root_id: &str, max_depth: u8) -> Vec<CausationEvent> {
        let Some(root) = self.find(root_id) else {
            return Vec::new();
        };
        let mut chain: Vec<CausationEvent> = Vec::new();
        let mut visited: HashSet<String> = HashSet::new();
        let mut queue: VecDeque<(String, u8)> = VecDeque::new();

        visited.insert(root.id.clone());
        chain.push(root.clone());
        if max_depth >= 1 {
            for parent_id in &root.caused_by {
                if !visited.contains(parent_id) {
                    queue.push_back((parent_id.clone(), 1));
                }
            }
        }

        while let Some((id, depth)) = queue.pop_front() {
            if visited.contains(&id) {
                continue;
            }
            visited.insert(id.clone());
            let Some(event) = self.find(&id) else {
                continue;
            };
            chain.push(event.clone());
            if depth < max_depth {
                for parent_id in &event.caused_by {
                    if !visited.contains(parent_id) {
                        queue.push_back((parent_id.clone(), depth + 1));
                    }
                }
            }
        }

        chain
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn ev(
        rec: &mut CausationRecorder,
        tick: u64,
        actor: &str,
        target: Option<&str>,
        caused_by: Vec<String>,
    ) -> String {
        rec.record(
            tick,
            CausationKind::VariableWrite {
                var: target.unwrap_or("x").to_owned(),
                old_value: Value::Null,
                new_value: Value::Int(1),
            },
            actor,
            target.map(str::to_owned),
            format!("set {}", target.unwrap_or("x")),
            caused_by,
        )
    }

    #[test]
    fn record_and_find() {
        let mut rec = CausationRecorder::new();
        let id = ev(&mut rec, 3, "sm1", Some("speed"), vec![]);
        assert_eq!(rec.len(), 1);
        let got = rec.find(&id).unwrap();
        assert_eq!(got.tick, 3);
        assert_eq!(got.actor, "sm1");
        assert_eq!(got.target.as_deref(), Some("speed"));
    }

    #[test]
    fn ring_buffer_evicts_oldest() {
        let mut rec = CausationRecorder::with_capacity(3);
        let a = ev(&mut rec, 0, "a", Some("v"), vec![]);
        let b = ev(&mut rec, 1, "b", Some("v"), vec![]);
        let c = ev(&mut rec, 2, "c", Some("v"), vec![]);
        let d = ev(&mut rec, 3, "d", Some("v"), vec![]);
        assert_eq!(rec.len(), 3);
        assert!(rec.find(&a).is_none(), "oldest event should be evicted");
        assert!(rec.find(&b).is_some());
        assert!(rec.find(&c).is_some());
        assert!(rec.find(&d).is_some());
    }

    #[test]
    fn zero_capacity_drops_events_but_returns_ids() {
        let mut rec = CausationRecorder::with_capacity(0);
        let id = ev(&mut rec, 0, "a", Some("v"), vec![]);
        assert!(!id.is_empty(), "id is assigned even when capacity is 0");
        assert_eq!(rec.len(), 0);
        assert!(rec.find(&id).is_none());
        assert_eq!(rec.trace(&id, 5).len(), 0);
    }

    #[test]
    fn trace_with_no_causes_returns_root_only() {
        let mut rec = CausationRecorder::new();
        let id = ev(&mut rec, 1, "a", Some("v"), vec![]);
        let chain = rec.trace(&id, 5);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].id, id);
    }

    #[test]
    fn trace_depth_cap() {
        // Chain: a <- b <- c <- d <- e
        let mut rec = CausationRecorder::new();
        let a = ev(&mut rec, 0, "a", Some("v"), vec![]);
        let b = ev(&mut rec, 1, "b", Some("v"), vec![a.clone()]);
        let c = ev(&mut rec, 2, "c", Some("v"), vec![b.clone()]);
        let d = ev(&mut rec, 3, "d", Some("v"), vec![c.clone()]);
        let e = ev(&mut rec, 4, "e", Some("v"), vec![d.clone()]);

        // Depth 0 = root only.
        let chain = rec.trace(&e, 0);
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].id, e);

        // Depth 2 = e, d, c (3 events).
        let chain = rec.trace(&e, 2);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].id, e);
        assert_eq!(chain[1].id, d);
        assert_eq!(chain[2].id, c);

        // Depth 10 = everything (5 events) without exceeding the graph.
        let chain = rec.trace(&e, 10);
        assert_eq!(chain.len(), 5);
    }

    #[test]
    fn trace_diamond_is_visited_safely() {
        // Diamond:       a
        //              /   \
        //             b     c
        //              \   /
        //                d
        // Walking backwards from d must visit a at most once.
        let mut rec = CausationRecorder::new();
        let a = ev(&mut rec, 0, "a", Some("v"), vec![]);
        let b = ev(&mut rec, 1, "b", Some("v"), vec![a.clone()]);
        let c = ev(&mut rec, 1, "c", Some("v"), vec![a.clone()]);
        let d = ev(&mut rec, 2, "d", Some("v"), vec![b.clone(), c.clone()]);

        let chain = rec.trace(&d, 5);
        // root d + {b, c} at depth 1 + a at depth 2 = 4 events
        assert_eq!(chain.len(), 4);
        let ids: HashSet<&str> = chain.iter().map(|e| e.id.as_str()).collect();
        assert!(ids.contains(a.as_str()));
        assert!(ids.contains(b.as_str()));
        assert!(ids.contains(c.as_str()));
        assert!(ids.contains(d.as_str()));
    }

    #[test]
    fn trace_handles_cycles_without_infinite_loop() {
        // Cyclic (pathological) graph: a -> b -> a. Walker must terminate.
        let mut rec = CausationRecorder::new();
        let a_id = "ev-0-0".to_owned(); // predict the id the recorder will assign
        let _a = rec.record(
            0,
            CausationKind::VariableWrite {
                var: "v".to_owned(),
                old_value: Value::Null,
                new_value: Value::Int(0),
            },
            "a",
            Some("v".to_owned()),
            "a",
            vec![],
        );
        let b = rec.record(
            1,
            CausationKind::VariableWrite {
                var: "v".to_owned(),
                old_value: Value::Int(0),
                new_value: Value::Int(1),
            },
            "b",
            Some("v".to_owned()),
            "b",
            vec![a_id.clone()],
        );
        // Surgically patch `a`'s caused_by to create the cycle.
        let a_mut = rec.events.iter_mut().find(|e| e.id == a_id).unwrap();
        a_mut.caused_by = vec![b.clone()];

        let chain = rec.trace(&b, 10);
        // Two distinct events, each visited once.
        assert_eq!(chain.len(), 2);
    }

    #[test]
    fn find_by_tick_target_prefers_latest() {
        let mut rec = CausationRecorder::new();
        let _first = ev(&mut rec, 5, "sm1", Some("speed"), vec![]);
        let second = ev(&mut rec, 5, "sm2", Some("speed"), vec![]);
        let found = rec.find_by_tick_target(5, "speed").unwrap();
        assert_eq!(found.id, second);
    }

    #[test]
    fn clear_resets_recorder() {
        let mut rec = CausationRecorder::new();
        let _ = ev(&mut rec, 0, "a", Some("v"), vec![]);
        rec.clear();
        assert_eq!(rec.len(), 0);
        assert!(rec.is_empty());
    }
}
