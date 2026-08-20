//! # sysml-run-actions
//!
//! Action execution engine for the SysML v2 runtime.
//!
//! This crate implements the full SysML v2 action type hierarchy using a
//! token-flow execution model. Actions are compiled from [`ModelGraph`] to
//! [`ActionNodeIR`] control-flow graphs, then executed by [`ActionRunner`].
//!
//! ## Action Type Hierarchy (from SysML v2 spec)
//!
//! ```text
//! ActionUsage (base)
//! ├── PerformActionUsage    — execute a referenced action
//! ├── SendActionUsage       — send a message
//! ├── AcceptActionUsage     — receive a message
//! ├── AssignmentActionUsage — assign a value to a feature
//! ├── IfActionUsage         — conditional branching
//! ├── WhileLoopActionUsage  — while loop
//! ├── ForLoopActionUsage    — for-each loop
//! ├── TerminateActionUsage  — terminate execution
//! └── ControlNode (abstract)
//!     ├── DecisionNode      — branch on guards
//!     ├── MergeNode         — converge branches
//!     ├── ForkNode          — split into parallel
//!     └── JoinNode          — synchronize parallel
//! ```
//!
//! ## Execution Model
//!
//! Actions are executed as a **token-flow** through a control-flow graph:
//! 1. A token starts at the initial node
//! 2. Each node processes the token and advances it to successors
//! 3. Fork nodes create multiple tokens (parallel execution)
//! 4. Join nodes wait for all incoming tokens before proceeding
//! 5. When all tokens reach final nodes, the action completes
//!
//! ## Spec References
//!
//! - `SysML.xtext:1352-1703` — Action grammar rules
//! - `library.systems/Actions.sysml` — Standard action types
//! - `SysML-vocab.ttl` — ActionUsage subtypes, ControlNode subtypes

#![allow(clippy::indexing_slicing)]
mod health;
pub use health::action_health_diagnostics;

use crate::expressions::{EvalContext, ExprIR, ExpressionEvaluator};
use std::collections::{HashMap, VecDeque};
use sysml_core::{Element, ElementId, ElementKind, ModelGraph, RelationshipKind, Value};
use sysml_span::Diagnostic;

// ---------------------------------------------------------------------------
// Action IR (control-flow graph)
// ---------------------------------------------------------------------------

/// Compiled action representation as a control-flow graph.
///
/// An action consists of nodes connected by edges. Tokens flow through
/// the graph according to succession ordering and control node semantics.
#[derive(Debug, Clone)]
pub struct ActionGraphIR {
    /// Unique identifier for this action.
    pub id: String,
    /// Human-readable name.
    pub name: String,
    /// All nodes in the graph.
    pub nodes: Vec<ActionNodeIR>,
    /// Directed edges connecting nodes.
    pub edges: Vec<ActionEdgeIR>,
    /// ID of the initial node.
    pub initial_node_id: String,
    /// IDs of final nodes.
    pub final_node_ids: Vec<String>,
    /// Parameter declarations (inputs/outputs).
    pub parameters: Vec<ActionParameter>,
}

impl ActionGraphIR {
    /// Create a new empty action graph.
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        let id = id.into();
        let initial_id = format!("{}_initial", &id);
        let final_id = format!("{}_final", &id);
        Self {
            id,
            name: name.into(),
            nodes: vec![
                ActionNodeIR::initial(initial_id.clone()),
                ActionNodeIR::final_node(final_id.clone()),
            ],
            edges: Vec::new(),
            initial_node_id: initial_id,
            final_node_ids: vec![final_id],
            parameters: Vec::new(),
        }
    }

    /// Find a node by ID.
    pub fn find_node(&self, id: &str) -> Option<&ActionNodeIR> {
        self.nodes.iter().find(|n| n.id() == id)
    }

    /// Get all outgoing edges from a node.
    pub fn outgoing_edges(&self, node_id: &str) -> Vec<&ActionEdgeIR> {
        self.edges.iter().filter(|e| e.from == node_id).collect()
    }

    /// Add a node and return its ID.
    pub fn add_node(&mut self, node: ActionNodeIR) -> String {
        let id = node.id().to_owned();
        self.nodes.push(node);
        id
    }

    /// Add an edge between two nodes.
    pub fn add_edge(&mut self, from: impl Into<String>, to: impl Into<String>) {
        self.edges.push(ActionEdgeIR {
            from: from.into(),
            to: to.into(),
            guard: None,
        });
    }

    /// Attach an inline sub-action graph to the Perform node with the given ID.
    ///
    /// Returns `self` for builder-style chaining. If no Perform node with
    /// `node_id` exists, the graph is returned unchanged.
    pub fn with_sub_action(mut self, node_id: &str, sub_graph: ActionGraphIR) -> Self {
        if let Some(ActionNodeIR::Perform { sub_action, .. }) =
            self.nodes.iter_mut().find(|n| n.id() == node_id)
        {
            *sub_action = Some(Box::new(sub_graph));
        }
        self
    }

    /// Add a guarded edge between two nodes.
    pub fn add_guarded_edge(
        &mut self,
        from: impl Into<String>,
        to: impl Into<String>,
        guard: ExprIR,
    ) {
        self.edges.push(ActionEdgeIR {
            from: from.into(),
            to: to.into(),
            guard: Some(guard),
        });
    }
}

/// A node in the action control-flow graph.
#[derive(Debug, Clone)]
pub enum ActionNodeIR {
    /// Initial node — execution starts here.
    Initial { id: String },

    /// Final node — execution ends here.
    Final { id: String },

    /// Perform a named action (subaction invocation).
    ///
    /// The sub-action can be resolved in two ways:
    /// 1. **Inline**: If `sub_action` is `Some`, execute that graph directly.
    /// 2. **Library lookup**: Otherwise, look up `action_ref` in the runner's
    ///    action library.
    Perform {
        id: String,
        action_ref: String,
        inputs: Vec<(String, ExprIR)>,
        output_binding: Option<String>,
        /// Optional inline sub-action graph. When present, this graph is
        /// executed directly instead of looking up `action_ref` in the library.
        sub_action: Option<Box<ActionGraphIR>>,
    },

    /// Send a message to a target.
    Send {
        id: String,
        payload: ExprIR,
        target: String,
        /// Optional port-level target (e.g., "brewer.waterIn").
        /// When set, the message is routed through FlowRouter to this port
        /// instead of being sent to the target action directly.
        port_target: Option<String>,
    },

    /// Accept (receive) a message.
    Accept {
        id: String,
        source: Option<String>,
        payload_binding: String,
        /// Optional port-level source filter.
        /// When set, only accepts messages originating from this port.
        port_source: Option<String>,
    },

    /// Assign a value to a feature.
    Assign {
        id: String,
        target: String,
        value: ExprIR,
    },

    /// Conditional branch.
    If {
        id: String,
        condition: ExprIR,
        then_branch: String,
        else_branch: Option<String>,
    },

    /// While loop.
    WhileLoop {
        id: String,
        condition: ExprIR,
        body_entry: String,
        exit_node: String,
    },

    /// For-each loop.
    ForLoop {
        id: String,
        variable: String,
        sequence: ExprIR,
        body_entry: String,
        exit_node: String,
    },

    /// Terminate execution.
    Terminate { id: String },

    /// Decision node — evaluate guards on outgoing edges to choose a branch.
    Decision { id: String },

    /// Merge node — converge control from multiple incoming branches.
    Merge { id: String },

    /// Fork node — split execution into parallel branches.
    Fork { id: String },

    /// Join node — wait for all parallel branches to complete.
    Join { id: String },

    /// A continuously-emitting stream source. Evaluates `value_expr` periodically
    /// and sends the result to `target`. Unlike other nodes, a stream source
    /// does NOT advance the token -- it keeps emitting until the flow terminates.
    StreamSource {
        id: String,
        /// Expression evaluated each emission to produce the stream value.
        value_expr: ExprIR,
        /// Target node or port to send streamed values to.
        target: String,
        /// Optional port target for port-based streaming.
        port_target: Option<String>,
        /// Emit every N steps (1 = every step, 5 = every 5th step).
        emit_interval: u32,
    },
}

impl ActionNodeIR {
    /// Get the node ID.
    pub fn id(&self) -> &str {
        match self {
            Self::Initial { id }
            | Self::Final { id }
            | Self::Perform { id, .. }
            | Self::Send { id, .. }
            | Self::Accept { id, .. }
            | Self::Assign { id, .. }
            | Self::If { id, .. }
            | Self::WhileLoop { id, .. }
            | Self::ForLoop { id, .. }
            | Self::Terminate { id }
            | Self::Decision { id }
            | Self::Merge { id }
            | Self::Fork { id }
            | Self::Join { id }
            | Self::StreamSource { id, .. } => id,
        }
    }

    /// Create an initial node.
    pub fn initial(id: impl Into<String>) -> Self {
        Self::Initial { id: id.into() }
    }

    /// Create a final node.
    pub fn final_node(id: impl Into<String>) -> Self {
        Self::Final { id: id.into() }
    }
}

/// An edge in the action control-flow graph.
#[derive(Debug, Clone)]
pub struct ActionEdgeIR {
    /// Source node ID.
    pub from: String,
    /// Target node ID.
    pub to: String,
    /// Optional guard expression (for decision nodes).
    pub guard: Option<ExprIR>,
}

/// An action parameter (input or output).
#[derive(Debug, Clone)]
pub struct ActionParameter {
    /// Parameter name.
    pub name: String,
    /// Direction: "in", "out", or "inout".
    pub direction: ParameterDirection,
    /// Default value expression (optional).
    pub default_value: Option<ExprIR>,
}

/// Parameter direction per SysML v2 spec.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParameterDirection {
    In,
    Out,
    InOut,
}

// ---------------------------------------------------------------------------
// Messages (for send/accept integration with flows)
// ---------------------------------------------------------------------------

/// A message produced by a send action.
#[derive(Debug, Clone)]
pub struct ActionMessage {
    /// Target port or participant name.
    pub target: String,
    /// The payload value.
    pub payload: Value,
    /// The source action that produced this message.
    pub source_action: String,
}

// ---------------------------------------------------------------------------
// Action execution token
// ---------------------------------------------------------------------------

/// A token flowing through the action graph.
#[derive(Debug, Clone)]
struct ActionToken {
    /// Current node ID where the token resides.
    current_node: String,
    /// Local variable bindings for this token.
    bindings: HashMap<String, Value>,
}

// ---------------------------------------------------------------------------
// Action step result
// ---------------------------------------------------------------------------

/// Result of stepping the action runner.
#[derive(Debug, Clone)]
pub struct ActionStepResult {
    /// Whether the action has completed.
    pub completed: bool,
    /// Trace outputs for logging/debugging.
    pub outputs: Vec<String>,
    /// Messages produced by send actions (to be routed by flow layer).
    pub messages: Vec<ActionMessage>,
    /// Diagnostics (errors/warnings during execution).
    pub diagnostics: Vec<Diagnostic>,
}

impl ActionStepResult {
    fn new() -> Self {
        Self {
            completed: false,
            outputs: Vec::new(),
            messages: Vec::new(),
            diagnostics: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// RSC-2.4c — compiled write-set of an action graph
// ---------------------------------------------------------------------------

/// Statically-known write targets of a compiled action graph (RSC-2.4c):
/// `Assign` targets, `Accept` payload bindings, and `Perform` output
/// bindings, recursively through inline sub-action graphs. This is the ONE
/// home for the action write-set — the spec-level baseline inventory
/// (`rsc2_behavioural_baseline.rs::action_graph_write_set`) enumerates the
/// same three classes independently as its oracle, and any future compiler
/// slot-claim pass for action subsystems must delegate here (the RSC-2.4b
/// `collect_assignment_targets` rule: claim collection and runner write-set
/// may never drift).
///
/// Deliberately **excluded** (token/loop bookkeeping, never published):
/// - `ForLoop` loop variables — per-iteration token bindings;
/// - `Perform` *input* names — reads seeded into the sub-action context;
/// - token positions, join-pending sets, streaming emission state —
///   runner-internal sequencing, not named values.
///
/// Note the action executor's write discipline differs from SM/ODE at the
/// root: every one of these targets is written into **token-local
/// bindings** (`ActionToken::bindings` / `final_bindings`), never into the
/// shared `EvalContext`. See `Executor::sync_context_out_slots` on
/// [`ActionRunner`] for what that means for the slot cutover.
pub(crate) fn collect_write_targets(ir: &ActionGraphIR) -> Vec<String> {
    fn walk(ir: &ActionGraphIR, out: &mut Vec<String>) {
        let push = |name: &str, out: &mut Vec<String>| {
            if !name.is_empty() && !out.iter().any(|n| n == name) {
                out.push(name.to_owned());
            }
        };
        for node in &ir.nodes {
            match node {
                ActionNodeIR::Assign { target, .. } => push(target, out),
                ActionNodeIR::Accept {
                    payload_binding, ..
                } => push(payload_binding, out),
                ActionNodeIR::Perform {
                    output_binding,
                    sub_action,
                    ..
                } => {
                    if let Some(b) = output_binding {
                        push(b, out);
                    }
                    if let Some(sub) = sub_action {
                        walk(sub, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(ir, &mut out);
    out
}

/// Names that resolve from **token bindings** at eval time (RSC-2.4c):
/// the write-set ([`collect_write_targets`]) plus `ForLoop` loop variables,
/// `Perform` input parameter names, and declared action parameters,
/// recursively through inline sub-action graphs. Declared to the
/// [`SlotBinder`](crate::expressions::SlotBinder) as locals so the bind
/// pass neither rewrites them to slots nor reports them as RS003
/// candidates — they are legal runtime-dynamic names, exactly like SM
/// guard event-string operands.
pub(crate) fn collect_token_local_names(ir: &ActionGraphIR) -> Vec<String> {
    fn walk(ir: &ActionGraphIR, out: &mut Vec<String>) {
        let push = |name: &str, out: &mut Vec<String>| {
            if !name.is_empty() && !out.iter().any(|n| n == name) {
                out.push(name.to_owned());
            }
        };
        for name in collect_write_targets(ir) {
            push(&name, out);
        }
        for p in &ir.parameters {
            push(&p.name, out);
        }
        for node in &ir.nodes {
            match node {
                ActionNodeIR::ForLoop { variable, .. } => push(variable, out),
                ActionNodeIR::Perform {
                    inputs, sub_action, ..
                } => {
                    for (param, _) in inputs {
                        push(param, out);
                    }
                    if let Some(sub) = sub_action {
                        walk(sub, out);
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(ir, &mut out);
    out
}

/// RSC-2.4c: precomputed slot write-set of one action executor — one
/// [`WriteRoute`](crate::slots::WriteRoute) per compiled write target.
/// Built by `Executor::prepare_slot_writeback`; mirrors the SM's
/// `SmWriteSet`. Today the routes carry **observability only** (mint
/// coverage via `slot_write_fallbacks`) because the action executor
/// publishes nothing into the shared context — see
/// `Executor::sync_context_out_slots` on [`ActionRunner`].
#[derive(Debug, Clone)]
struct ActionWriteSet {
    /// `(bare target name, route)` per compiled write target
    /// (collection order = [`collect_write_targets`] order).
    targets: Vec<(String, crate::slots::WriteRoute)>,
}

// ---------------------------------------------------------------------------
// Action runner
// ---------------------------------------------------------------------------

/// Per-stream-source node state for continuous emission.
#[derive(Debug, Clone)]
struct StreamingNodeState {
    /// Steps since last emission.
    ticks_since_emit: u32,
    /// Total emissions so far.
    total_emissions: u64,
    /// Last emitted value.
    last_value: Option<Value>,
}

/// Executes an action graph using token-flow semantics.
///
/// # Example
///
/// ```ignore
/// let graph = ActionGraphIR::new("action1", "MyAction");
/// let mut runner = ActionRunner::new(graph);
/// let ctx = EvalContext::new();
///
/// loop {
///     let result = runner.step(&ctx);
///     for output in &result.outputs {
///         println!("{}", output);
///     }
///     if result.completed {
///         break;
///     }
/// }
/// ```
#[derive(Clone)]
pub struct ActionRunner {
    ir: ActionGraphIR,
    tokens: Vec<ActionToken>,
    evaluator: ExpressionEvaluator,
    message_inbox: VecDeque<ActionMessage>,
    /// RSC port-flow Wave B-inc-2: per-port delivery buffer for `accept …
    /// via <port>` nodes, fed from the orchestrator's unified
    /// `TickContext::port_payloads` (the same channel the SM reads). Keyed by
    /// the Accept node's `port_source`. Distinct from `message_inbox`, which
    /// stays the intra-action / `deliver_message` path for port-less accepts.
    port_inbox: HashMap<String, VecDeque<Value>>,
    completed: bool,
    /// Maximum number of loop iterations before aborting (default 10000).
    pub max_iterations: usize,
    /// Tracks how many tokens have arrived at each join node.
    join_pending: HashMap<String, Vec<ActionToken>>,
    /// Library of compiled sub-action graphs, keyed by action name.
    /// Used by Perform nodes to look up and execute referenced sub-actions.
    action_library: HashMap<String, ActionGraphIR>,
    /// Bindings captured when a token reaches a final node.
    /// Used to extract output values from sub-action execution.
    final_bindings: HashMap<String, Value>,
    /// Per-stream-source node state for continuous emission.
    streaming_state: HashMap<String, StreamingNodeState>,
    /// RSC-2.4c: precomputed slot write-set (compiled write targets →
    /// routes). `None` until `Executor::prepare_slot_writeback` runs —
    /// hand-built runners and the `action.start`/`action.run` service
    /// paths (no slot table) stay `None`. Compile-time state: survives
    /// [`reset`](Self::reset), like the SM's write-set.
    write_set: Option<ActionWriteSet>,
    /// RSC-2.4c: unfiltered outcome of the last `bind_expression_slots`
    /// pass over this runner's retained node expressions (the public
    /// report clears `unresolved` — token-local names are legal).
    bind_report: crate::expressions::BindReport,
}

impl ActionRunner {
    /// Create a new action runner from a compiled action graph.
    pub fn new(ir: ActionGraphIR) -> Self {
        let initial_node = ir.initial_node_id.clone();
        #[cfg(feature = "tracing")]
        tracing::trace!(
            action = %ir.name,
            nodes = ir.nodes.len(),
            edges = ir.edges.len(),
            "creating action runner"
        );
        Self {
            ir,
            tokens: vec![ActionToken {
                current_node: initial_node,
                bindings: HashMap::new(),
            }],
            evaluator: ExpressionEvaluator::new(),
            message_inbox: VecDeque::new(),
            port_inbox: HashMap::new(),
            completed: false,
            max_iterations: 10_000,
            join_pending: HashMap::new(),
            action_library: HashMap::new(),
            final_bindings: HashMap::new(),
            streaming_state: HashMap::new(),
            write_set: None,
            bind_report: crate::expressions::BindReport::default(),
        }
    }

    /// Create a new action runner with a library of sub-action graphs.
    ///
    /// The library maps action names to compiled graphs. When a Perform node
    /// references an action by name, the runner looks it up in this library,
    /// creates a sub-runner, and executes it inline.
    pub fn with_library(ir: ActionGraphIR, library: HashMap<String, ActionGraphIR>) -> Self {
        let initial_node = ir.initial_node_id.clone();
        Self {
            ir,
            tokens: vec![ActionToken {
                current_node: initial_node,
                bindings: HashMap::new(),
            }],
            evaluator: ExpressionEvaluator::new(),
            message_inbox: VecDeque::new(),
            port_inbox: HashMap::new(),
            completed: false,
            max_iterations: 10_000,
            join_pending: HashMap::new(),
            action_library: library,
            final_bindings: HashMap::new(),
            streaming_state: HashMap::new(),
            write_set: None,
            bind_report: crate::expressions::BindReport::default(),
        }
    }

    /// Reset the runner to its initial state.
    ///
    /// The action library is preserved across resets.
    pub fn reset(&mut self) {
        self.tokens = vec![ActionToken {
            current_node: self.ir.initial_node_id.clone(),
            bindings: HashMap::new(),
        }];
        self.message_inbox.clear();
        self.port_inbox.clear();
        self.completed = false;
        self.join_pending.clear();
        self.final_bindings.clear();
        self.streaming_state.clear();
    }

    /// Get the bindings captured when the action reached a final node.
    ///
    /// These contain all variable values at the point of completion,
    /// useful for extracting output values from sub-action execution.
    pub fn final_bindings(&self) -> &HashMap<String, Value> {
        &self.final_bindings
    }

    /// Check if execution has completed.
    pub fn is_completed(&self) -> bool {
        self.completed
    }

    /// RSC-2.4c: unfiltered outcome of the last slot-binding pass over
    /// this runner's retained node expressions (`unresolved` included —
    /// the public `Executor::bind_expression_slots` report clears it
    /// because token-local names are legal runtime input, not RS003
    /// candidates). Mirrors the SM's `guard_bind_report`.
    pub fn bind_report(&self) -> &crate::expressions::BindReport {
        &self.bind_report
    }

    /// Returns true if all remaining tokens are blocked at Accept nodes
    /// waiting for messages. This indicates a potential deadlock if no
    /// external messages will be delivered.
    pub fn is_blocked(&self) -> bool {
        if self.tokens.is_empty() {
            return false;
        }
        self.tokens.iter().all(|t| {
            matches!(
                self.ir.find_node(&t.current_node),
                Some(ActionNodeIR::Accept { .. })
            )
        })
    }

    /// Deliver a message to this action's inbox (for accept nodes).
    pub fn deliver_message(&mut self, msg: ActionMessage) {
        self.message_inbox.push_back(msg);
    }

    /// RSC port-flow Wave B-inc-2: the ports this action accepts on — the
    /// `port_source` of every `accept … via <port>` node. The orchestrator
    /// registers these as acceptors (mirrors `StateMachineRunner::accept_ports`)
    /// so a routed `accept … via <port>` transfer reaches this subsystem.
    pub fn accept_ports(&self) -> Vec<String> {
        let mut ports = Vec::new();
        for node in &self.ir.nodes {
            if let ActionNodeIR::Accept {
                port_source: Some(port),
                ..
            } = node
            {
                if !ports.contains(port) {
                    ports.push(port.clone());
                }
            }
        }
        ports
    }

    /// The accept ports ARMED right now: ports of `accept … via <port>` nodes
    /// a token is currently parked on (mirrors
    /// `StateMachineRunner::armed_accept_ports`).
    pub fn armed_accept_ports(&self) -> Vec<String> {
        let mut ports = Vec::new();
        for token in &self.tokens {
            if let Some(ActionNodeIR::Accept {
                port_source: Some(port),
                ..
            }) = self.ir.find_node(&token.current_node)
            {
                if !ports.contains(port) {
                    ports.push(port.clone());
                }
            }
        }
        ports
    }

    /// Get the total emissions from a streaming source node.
    pub fn stream_emissions(&self, node_id: &str) -> u64 {
        self.streaming_state
            .get(node_id)
            .map(|s| s.total_emissions)
            .unwrap_or(0)
    }

    /// Get the last emitted value from a streaming source.
    pub fn stream_last_value(&self, node_id: &str) -> Option<&Value> {
        self.streaming_state
            .get(node_id)
            .and_then(|s| s.last_value.as_ref())
    }

    /// Get the current node ID (for trace recording).
    ///
    /// Returns the node ID of the first active token, or the initial node if no tokens exist.
    /// When multiple tokens are active (parallel execution), returns the first one.
    pub fn current_node_id(&self) -> &str {
        self.tokens
            .first()
            .map(|t| t.current_node.as_str())
            .unwrap_or(&self.ir.initial_node_id)
    }

    #[cfg(feature = "tracing")]
    fn current_node_kind(&self) -> &'static str {
        match self.ir.find_node(self.current_node_id()) {
            Some(ActionNodeIR::Initial { .. }) => "Initial",
            Some(ActionNodeIR::Final { .. }) => "Final",
            Some(ActionNodeIR::Perform { .. }) => "Perform",
            Some(ActionNodeIR::Send { .. }) => "Send",
            Some(ActionNodeIR::Accept { .. }) => "Accept",
            Some(ActionNodeIR::Assign { .. }) => "Assign",
            Some(ActionNodeIR::If { .. }) => "If",
            Some(ActionNodeIR::WhileLoop { .. }) => "WhileLoop",
            Some(ActionNodeIR::ForLoop { .. }) => "ForLoop",
            Some(ActionNodeIR::Terminate { .. }) => "Terminate",
            Some(ActionNodeIR::Decision { .. }) => "Decision",
            Some(ActionNodeIR::Merge { .. }) => "Merge",
            Some(ActionNodeIR::Fork { .. }) => "Fork",
            Some(ActionNodeIR::Join { .. }) => "Join",
            Some(ActionNodeIR::StreamSource { .. }) => "StreamSource",
            None => "Unknown",
        }
    }

    /// Get the current variable bindings (for trace recording).
    ///
    /// Returns the bindings from the first active token.
    /// When multiple tokens are active (parallel execution), returns the first one's bindings.
    pub fn current_bindings(&self) -> &HashMap<String, Value> {
        self.tokens
            .first()
            .map(|t| &t.bindings)
            .unwrap_or(&self.final_bindings)
    }

    /// Step the execution forward.
    pub fn step(&mut self, ctx: &EvalContext) -> ActionStepResult {
        #[cfg(feature = "tracing")]
        let current_node = self.current_node_id();
        #[cfg(feature = "tracing")]
        let current_node_kind = self.current_node_kind();
        #[cfg(feature = "tracing")]
        tracing::trace!(
            action = %self.ir.name,
            current_node,
            current_node_kind,
            active_tokens = self.tokens.len(),
            join_waiting = self.join_pending.len(),
            binding_count = ctx.variables.len(),
            "action runner step start"
        );

        let mut result = ActionStepResult::new();

        if self.completed {
            result.completed = true;
            return result;
        }

        let tokens = std::mem::take(&mut self.tokens);
        for token in tokens {
            self.process_token(token, ctx, &mut result);
        }

        // Process streaming nodes: these emit values independently of token flow.
        let nodes: Vec<_> = self
            .ir
            .nodes
            .iter()
            .filter_map(|node| {
                if let ActionNodeIR::StreamSource {
                    id,
                    value_expr,
                    target,
                    port_target,
                    emit_interval,
                } = node
                {
                    Some((
                        id.clone(),
                        value_expr.clone(),
                        target.clone(),
                        port_target.clone(),
                        *emit_interval,
                    ))
                } else {
                    None
                }
            })
            .collect();

        for (id, value_expr, target, port_target, emit_interval) in nodes {
            let state = self
                .streaming_state
                .entry(id.clone())
                .or_insert(StreamingNodeState {
                    ticks_since_emit: 0,
                    total_emissions: 0,
                    last_value: None,
                });

            state.ticks_since_emit += 1;
            if state.ticks_since_emit >= emit_interval {
                match self.evaluator.eval(&value_expr, ctx) {
                    Ok(val) => {
                        state.last_value = Some(val.clone());
                        state.total_emissions += 1;
                        state.ticks_since_emit = 0;

                        let effective_target = port_target.as_deref().unwrap_or(target.as_str());
                        result.messages.push(ActionMessage {
                            source_action: id.clone(),
                            target: effective_target.to_owned(),
                            payload: val,
                        });
                    }
                    Err(_) => {
                        // Expression evaluation failed -- skip this emission
                    }
                }
            }
        }

        // Check if there are active stream sources (background emitters).
        let has_stream_sources = self
            .ir
            .nodes
            .iter()
            .any(|n| matches!(n, ActionNodeIR::StreamSource { .. }));

        // Check completion: all tokens have reached final nodes (and been removed)
        // Tokens waiting in join_pending are still in-flight, not complete.
        // An action with ONLY stream sources (no other flow tokens) stays active.
        if self.tokens.is_empty() && self.join_pending.is_empty() && !has_stream_sources {
            self.completed = true;
            result.completed = true;
        }

        #[cfg(feature = "tracing")]
        let next_node = self.current_node_id();
        #[cfg(feature = "tracing")]
        let next_node_kind = self.current_node_kind();
        #[cfg(feature = "tracing")]
        tracing::trace!(
            action = %self.ir.name,
            current_node = next_node,
            current_node_kind = next_node_kind,
            active_tokens = self.tokens.len(),
            join_waiting = self.join_pending.len(),
            outputs = result.outputs.len(),
            messages = result.messages.len(),
            diagnostics = result.diagnostics.len(),
            completed = result.completed,
            "action runner step complete"
        );

        result
    }

    fn process_token(
        &mut self,
        mut token: ActionToken,
        ctx: &EvalContext,
        result: &mut ActionStepResult,
    ) {
        let node = self.ir.find_node(&token.current_node).cloned();

        match node {
            Some(ActionNodeIR::Final { .. }) => {
                // Token completes — capture bindings and don't re-add
                for (k, v) in &token.bindings {
                    self.final_bindings.insert(k.clone(), v.clone());
                }
                result
                    .outputs
                    .push(format!("Action {} reached final node", self.ir.name));
            }

            Some(ActionNodeIR::Initial { id }) => {
                // Advance to first successor
                if let Some(next) = self.next_node(&id) {
                    token.current_node = next;
                    self.tokens.push(token);
                }
            }

            Some(ActionNodeIR::Assign { id, target, value }) => {
                // Evaluate expression and bind to target
                let merged_ctx = self.merge_context(ctx, &token);
                match self.evaluator.eval(&value, &merged_ctx) {
                    Ok(val) => {
                        result.outputs.push(format!("{} = {:?}", target, val));
                        token.bindings.insert(target, val);
                    }
                    Err(e) => {
                        result
                            .diagnostics
                            .push(Diagnostic::error(format!("assignment error: {}", e)));
                    }
                }
                if let Some(next) = self.next_node(&id) {
                    token.current_node = next;
                    self.tokens.push(token);
                }
            }

            Some(ActionNodeIR::Send {
                id,
                payload,
                target,
                port_target,
            }) => {
                let merged_ctx = self.merge_context(ctx, &token);
                match self.evaluator.eval(&payload, &merged_ctx) {
                    Ok(val) => {
                        // Use port_target if available, otherwise fall back to target
                        let effective_target = port_target.as_deref().unwrap_or(target.as_str());
                        result.messages.push(ActionMessage {
                            target: effective_target.to_owned(),
                            payload: val,
                            source_action: self.ir.id.clone(),
                        });
                        result.outputs.push(format!("send to {}", target));
                    }
                    Err(e) => {
                        result
                            .diagnostics
                            .push(Diagnostic::error(format!("send error: {}", e)));
                    }
                }
                if let Some(next) = self.next_node(&id) {
                    token.current_node = next;
                    self.tokens.push(token);
                }
            }

            Some(ActionNodeIR::Accept {
                id,
                source: _,
                payload_binding,
                port_source,
            }) => {
                // Wave B-inc-2: an `accept … via <port>` node consumes from the
                // per-port buffer fed by `TickContext::port_payloads` (the
                // unified orchestrator delivery channel). A port-less accept
                // keeps the `message_inbox` path (`deliver_message` /
                // intra-action wiring). One AcceptPerformance → one source.
                let delivered = match &port_source {
                    Some(port) => self.port_inbox.get_mut(port).and_then(|q| q.pop_front()),
                    None => self.message_inbox.pop_front().map(|m| m.payload),
                };
                if let Some(payload) = delivered {
                    // Message available — accept it and advance
                    token.bindings.insert(payload_binding.clone(), payload);
                    result
                        .outputs
                        .push(format!("accepted message into {}", payload_binding));
                    if let Some(next) = self.next_node(&id) {
                        token.current_node = next;
                    }
                    self.tokens.push(token);
                } else {
                    // No message — BLOCK: keep token at Accept node, don't advance.
                    // The token will retry on the next step() call.
                    self.tokens.push(token);
                }
            }

            Some(ActionNodeIR::Decision { id }) => {
                // Evaluate guards on outgoing edges
                let edges = self.ir.outgoing_edges(&id);
                let merged_ctx = self.merge_context(ctx, &token);
                let mut matched = false;
                for edge in &edges {
                    if let Some(guard) = &edge.guard {
                        if let Ok(Value::Bool(true)) = self.evaluator.eval(guard, &merged_ctx) {
                            token.current_node = edge.to.clone();
                            self.tokens.push(token);
                            matched = true;
                            break;
                        }
                    } else {
                        // Unguarded edge = default/else branch
                        token.current_node = edge.to.clone();
                        self.tokens.push(token);
                        matched = true;
                        break;
                    }
                }
                if !matched {
                    result
                        .diagnostics
                        .push(Diagnostic::error("no matching guard in decision node"));
                }
            }

            Some(ActionNodeIR::Fork { id }) => {
                // Create a token for each outgoing edge
                let edges = self.ir.outgoing_edges(&id);
                for edge in edges {
                    let forked = ActionToken {
                        current_node: edge.to.clone(),
                        bindings: token.bindings.clone(),
                    };
                    self.tokens.push(forked);
                }
            }

            Some(ActionNodeIR::Merge { id }) => {
                if let Some(next) = self.next_node(&id) {
                    token.current_node = next;
                    self.tokens.push(token);
                }
            }

            Some(ActionNodeIR::Join { id }) => {
                let expected = self.incoming_edge_count(&id);
                let pending = self.join_pending.entry(id.clone()).or_default();
                pending.push(token);
                if pending.len() >= expected {
                    // All branches arrived — merge bindings and continue
                    // SAFETY: join_pending entry verified present above via `.entry().or_default()`
                    #[allow(clippy::expect_used)]
                    let arrived = self
                        .join_pending
                        .remove(&id)
                        .expect("invariant: join_pending entry verified present above");
                    let mut merged_bindings = HashMap::new();
                    for t in &arrived {
                        for (k, v) in &t.bindings {
                            merged_bindings.insert(k.clone(), v.clone());
                        }
                    }
                    if let Some(next) = self.next_node(&id) {
                        self.tokens.push(ActionToken {
                            current_node: next,
                            bindings: merged_bindings,
                        });
                    }
                }
                // Otherwise: token stays in pending, waiting for other branches
            }

            Some(ActionNodeIR::Terminate { .. }) => {
                // Drop the token — terminate this execution path
                result.outputs.push("terminate".to_owned());
                self.completed = true;
            }

            Some(ActionNodeIR::Perform {
                id,
                action_ref,
                inputs,
                output_binding,
                sub_action,
            }) => {
                // Resolve the sub-action graph: prefer inline, then library
                let resolved_graph = sub_action
                    .map(|g| *g)
                    .or_else(|| self.action_library.get(&action_ref).cloned());

                if let Some(sub_graph) = resolved_graph {
                    // Build sub-action context: start with parent bindings, then overlay inputs
                    let merged_ctx = self.merge_context(ctx, &token);
                    let mut sub_ctx = merged_ctx.alias_live();
                    for (param_name, expr) in &inputs {
                        match self.evaluator.eval(expr, &merged_ctx) {
                            Ok(val) => {
                                sub_ctx.set(param_name.clone(), val);
                            }
                            Err(e) => {
                                result.diagnostics.push(Diagnostic::error(format!(
                                    "perform input binding error for '{}': {}",
                                    param_name, e
                                )));
                            }
                        }
                    }

                    // Run sub-action to completion (bounded to prevent infinite loops)
                    let max_sub_steps = self.max_iterations.min(1000);
                    let mut sub_runner =
                        ActionRunner::with_library(sub_graph, self.action_library.clone());
                    sub_runner.max_iterations = self.max_iterations;
                    for _ in 0..max_sub_steps {
                        let sub_result = sub_runner.step(&sub_ctx);
                        result.outputs.extend(sub_result.outputs);
                        result.messages.extend(sub_result.messages);
                        result.diagnostics.extend(sub_result.diagnostics);
                        if sub_result.completed {
                            break;
                        }
                    }

                    // Merge sub-action's final bindings into the parent token
                    for (k, v) in sub_runner.final_bindings() {
                        token.bindings.insert(k.clone(), v.clone());
                    }

                    // Extract output values from sub-runner's final bindings
                    if let Some(out_name) = &output_binding {
                        let final_bindings = sub_runner.final_bindings();
                        // First try declared output parameters
                        let mut found = false;
                        for sub_param in &sub_runner.ir.parameters {
                            if sub_param.direction == ParameterDirection::Out
                                || sub_param.direction == ParameterDirection::InOut
                            {
                                if let Some(val) = final_bindings.get(&sub_param.name) {
                                    token.bindings.insert(out_name.clone(), val.clone());
                                    found = true;
                                    break;
                                }
                            }
                        }
                        // Fallback: try 'result' convention
                        if !found {
                            if let Some(val) = final_bindings.get("result") {
                                token.bindings.insert(out_name.clone(), val.clone());
                            }
                        }
                    }

                    result
                        .outputs
                        .push(format!("perform {} completed", action_ref));
                } else {
                    result
                        .outputs
                        .push(format!("perform {} (no library entry)", action_ref));
                }
                if let Some(next) = self.next_node(&id) {
                    token.current_node = next;
                    self.tokens.push(token);
                }
            }

            Some(ActionNodeIR::If {
                id,
                condition,
                then_branch,
                else_branch,
            }) => {
                let merged_ctx = self.merge_context(ctx, &token);
                match self.evaluator.eval(&condition, &merged_ctx) {
                    Ok(Value::Bool(true)) => {
                        token.current_node = then_branch;
                        self.tokens.push(token);
                    }
                    Ok(Value::Bool(false)) => {
                        if let Some(else_id) = else_branch {
                            token.current_node = else_id;
                        } else if let Some(next) = self.next_node(&id) {
                            token.current_node = next;
                        }
                        self.tokens.push(token);
                    }
                    Ok(_) => {
                        result
                            .diagnostics
                            .push(Diagnostic::error("if condition must be boolean"));
                    }
                    Err(e) => {
                        result
                            .diagnostics
                            .push(Diagnostic::error(format!("if condition error: {}", e)));
                    }
                }
            }

            Some(ActionNodeIR::WhileLoop {
                id,
                condition,
                body_entry,
                exit_node,
            }) => {
                let mut iteration_count = 0usize;
                let mut loop_token = token;
                loop {
                    if iteration_count >= self.max_iterations {
                        result.diagnostics.push(Diagnostic::error(format!(
                            "while loop exceeded max iterations ({})",
                            self.max_iterations
                        )));
                        break;
                    }
                    let merged_ctx = self.merge_context(ctx, &loop_token);
                    match self.evaluator.eval(&condition, &merged_ctx) {
                        Ok(Value::Bool(true)) => {
                            // Execute body nodes inline
                            self.execute_body(&body_entry, &id, &mut loop_token, ctx, result);
                            iteration_count += 1;
                        }
                        Ok(Value::Bool(false)) => {
                            // Condition false — exit loop
                            loop_token.current_node = exit_node;
                            self.tokens.push(loop_token);
                            break;
                        }
                        Ok(_) => {
                            result
                                .diagnostics
                                .push(Diagnostic::error("while loop condition must be boolean"));
                            break;
                        }
                        Err(e) => {
                            result.diagnostics.push(Diagnostic::error(format!(
                                "while loop condition error: {}",
                                e
                            )));
                            break;
                        }
                    }
                }
            }

            Some(ActionNodeIR::ForLoop {
                id,
                variable,
                sequence,
                body_entry,
                exit_node,
            }) => {
                let merged_ctx = self.merge_context(ctx, &token);
                match self.evaluator.eval(&sequence, &merged_ctx) {
                    Ok(Value::List(items)) => {
                        let mut loop_token = token;
                        for item in items {
                            loop_token.bindings.insert(variable.clone(), item);
                            self.execute_body(&body_entry, &id, &mut loop_token, ctx, result);
                        }
                        loop_token.current_node = exit_node;
                        self.tokens.push(loop_token);
                    }
                    Ok(_) => {
                        result
                            .diagnostics
                            .push(Diagnostic::error("for loop sequence must be a list"));
                    }
                    Err(e) => {
                        result
                            .diagnostics
                            .push(Diagnostic::error(format!("for loop sequence error: {}", e)));
                    }
                }
            }

            Some(ActionNodeIR::StreamSource { id, .. }) => {
                // Stream sources are processed in the streaming emission pass,
                // not via token flow. If a token arrives here, just advance it.
                if let Some(next) = self.next_node(&id) {
                    token.current_node = next;
                    self.tokens.push(token);
                }
                // If no successor, the token is consumed (stream runs in background).
            }

            None => {
                result.diagnostics.push(Diagnostic::error(format!(
                    "unknown node: {}",
                    token.current_node
                )));
            }
        }
    }

    fn next_node(&self, from: &str) -> Option<String> {
        self.ir.outgoing_edges(from).first().map(|e| e.to.clone())
    }

    /// Count the number of incoming edges to a node.
    fn incoming_edge_count(&self, node_id: &str) -> usize {
        self.ir.edges.iter().filter(|e| e.to == node_id).count()
    }

    fn merge_context(&self, base: &EvalContext, token: &ActionToken) -> EvalContext {
        // Cull-arc W3: alias_live — this builds the context actions EXECUTE
        // against (assignments must reach production state), so it is a genuine
        // live copy, not a speculative snapshot.
        let mut merged = base.alias_live();
        for (k, v) in &token.bindings {
            merged.set(k.clone(), v.clone());
        }
        merged
    }

    /// Execute body nodes inline for loops.
    ///
    /// Walks from `body_entry` through successor nodes, stopping when it
    /// reaches `loop_node_id` (back edge) or a node with no outgoing edges.
    /// Assignments along the way update the token bindings.
    fn execute_body(
        &self,
        body_entry: &str,
        loop_node_id: &str,
        token: &mut ActionToken,
        ctx: &EvalContext,
        result: &mut ActionStepResult,
    ) {
        let mut current_id = body_entry.to_owned();
        loop {
            if current_id == loop_node_id {
                // Back edge to the loop header — stop body execution
                break;
            }
            let node = self.ir.find_node(&current_id).cloned();
            match node {
                Some(ActionNodeIR::Assign {
                    id, target, value, ..
                }) => {
                    let merged_ctx = self.merge_context(ctx, token);
                    match self.evaluator.eval(&value, &merged_ctx) {
                        Ok(val) => {
                            result.outputs.push(format!("{} = {:?}", target, val));
                            token.bindings.insert(target.clone(), val);
                        }
                        Err(e) => {
                            result.diagnostics.push(Diagnostic::error(format!(
                                "assignment error in loop body: {}",
                                e
                            )));
                            break;
                        }
                    }
                    match self.next_node(&id) {
                        Some(next) => current_id = next,
                        None => break,
                    }
                }
                Some(ActionNodeIR::Send {
                    id,
                    payload,
                    target,
                    port_target,
                }) => {
                    let merged_ctx = self.merge_context(ctx, token);
                    if let Ok(val) = self.evaluator.eval(&payload, &merged_ctx) {
                        let effective_target = port_target.as_deref().unwrap_or(target.as_str());
                        result.messages.push(ActionMessage {
                            target: effective_target.to_owned(),
                            payload: val,
                            source_action: self.ir.id.clone(),
                        });
                    }
                    match self.next_node(&id) {
                        Some(next) => current_id = next,
                        None => break,
                    }
                }
                Some(ActionNodeIR::Perform {
                    id,
                    action_ref,
                    inputs,
                    output_binding,
                    sub_action,
                }) => {
                    // Resolve: prefer inline sub_action, then library
                    let resolved_graph = sub_action
                        .map(|g| *g)
                        .or_else(|| self.action_library.get(&action_ref).cloned());

                    if let Some(sub_graph) = resolved_graph {
                        let merged_ctx = self.merge_context(ctx, token);
                        let mut sub_ctx = merged_ctx.alias_live();
                        for (param_name, expr) in &inputs {
                            if let Ok(val) = self.evaluator.eval(expr, &merged_ctx) {
                                sub_ctx.set(param_name.clone(), val);
                            }
                        }
                        let max_sub_steps = self.max_iterations.min(1000);
                        let mut sub_runner =
                            ActionRunner::with_library(sub_graph, self.action_library.clone());
                        sub_runner.max_iterations = self.max_iterations;
                        for _ in 0..max_sub_steps {
                            let sub_result = sub_runner.step(&sub_ctx);
                            result.outputs.extend(sub_result.outputs);
                            result.messages.extend(sub_result.messages);
                            result.diagnostics.extend(sub_result.diagnostics);
                            if sub_result.completed {
                                break;
                            }
                        }
                        // Merge sub-action's final bindings into the parent token
                        for (k, v) in sub_runner.final_bindings() {
                            token.bindings.insert(k.clone(), v.clone());
                        }
                        if let Some(out_name) = &output_binding {
                            let final_bindings = sub_runner.final_bindings();
                            if let Some(val) = final_bindings.get("result") {
                                token.bindings.insert(out_name.clone(), val.clone());
                            }
                        }
                        result
                            .outputs
                            .push(format!("perform {} completed", action_ref));
                    } else {
                        result
                            .outputs
                            .push(format!("perform {} (no library entry)", action_ref));
                    }
                    match self.next_node(&id) {
                        Some(next) => current_id = next,
                        None => break,
                    }
                }
                Some(other) => {
                    // For any other node type (merge, final, etc.) just advance
                    let id = other.id().to_owned();
                    match self.next_node(&id) {
                        Some(next) => current_id = next,
                        None => break,
                    }
                }
                None => break,
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Action compiler (from ModelGraph)
// ---------------------------------------------------------------------------

/// Compile an action definition or usage from a [`ModelGraph`] into an [`ActionGraphIR`].
///
/// Walks the model graph to find an action element with the given name,
/// then extracts its child action steps, succession ordering, control nodes,
/// and parameters to build a complete control-flow graph.
///
/// # Errors
///
/// Returns diagnostics if the named action is not found or has structural problems.
pub fn compile_action(
    action_name: &str,
    graph: &ModelGraph,
) -> Result<ActionGraphIR, Vec<Diagnostic>> {
    #[cfg(feature = "tracing")]
    tracing::trace!(
        action = action_name,
        element_count = graph.element_count(),
        relationship_count = graph.relationship_count(),
        "compiling action graph"
    );

    // Find the action definition or usage by name
    let action_elem = graph
        .elements
        .values()
        .find(|e| {
            e.name.as_deref() == Some(action_name)
                && (e.kind == ElementKind::ActionDefinition
                    || e.kind == ElementKind::ActionUsage
                    || e.kind == ElementKind::PerformActionUsage)
        })
        .ok_or_else(|| {
            #[cfg(feature = "tracing")]
            tracing::debug!(action = action_name, "action not found in model graph");
            vec![Diagnostic::error(format!(
                "action '{}' not found in model graph",
                action_name
            ))]
        })?;

    let action_id = action_elem.id.clone();
    let mut ir = ActionGraphIR::new(action_id.to_string(), action_name);

    // Collect child elements that are action steps
    let children: Vec<&Element> = graph.children_of(&action_id).collect();

    // Map element IDs to IR node IDs for edge generation
    let mut elem_to_node: HashMap<ElementId, String> = HashMap::new();

    // Extract parameters from children
    for child in &children {
        if let Some(direction_val) = child.props.get("direction") {
            if let Some(dir_str) = Value::as_str(direction_val) {
                let direction = match dir_str {
                    "in" => ParameterDirection::In,
                    "out" => ParameterDirection::Out,
                    "inout" => ParameterDirection::InOut,
                    _ => continue,
                };
                let param_name = child.name.clone().unwrap_or_default();
                ir.parameters.push(ActionParameter {
                    name: param_name,
                    direction,
                    default_value: None,
                });
            }
        }
    }

    // Create IR nodes for each action step child
    for child in &children {
        // Skip parameter-like elements (already handled above)
        if child.props.contains_key("direction") {
            continue;
        }

        let node_id = child.id.to_string();
        let node = match child.kind {
            ElementKind::ActionUsage => {
                // Generic action step — treat as a perform if it has a type reference
                let child_name = child.name.clone().unwrap_or_else(|| node_id.clone());
                ActionNodeIR::Perform {
                    id: node_id.clone(),
                    action_ref: child_name,
                    inputs: Vec::new(),
                    output_binding: None,
                    sub_action: None,
                }
            }
            ElementKind::PerformActionUsage => {
                let ref_name = child.name.clone().unwrap_or_else(|| "unknown".to_owned());
                ActionNodeIR::Perform {
                    id: node_id.clone(),
                    action_ref: ref_name,
                    inputs: Vec::new(),
                    output_binding: None,
                    sub_action: None,
                }
            }
            ElementKind::SendActionUsage => {
                let target = child
                    .props
                    .get("target")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned();
                let port = child
                    .props
                    .get("portTarget")
                    .and_then(Value::as_str)
                    .map(String::from);
                ActionNodeIR::Send {
                    id: node_id.clone(),
                    payload: ExprIR::LiteralString(
                        child
                            .props
                            .get("payload")
                            .and_then(Value::as_str)
                            .unwrap_or("message")
                            .to_owned(),
                    ),
                    target,
                    port_target: port,
                }
            }
            ElementKind::AcceptActionUsage => {
                // RSC port-flow Wave B-inc-2: lower `accept <name> via <port>`
                // to an Accept token-flow node. The parser keeps the accept
                // parameter name on the element itself (`test_accept_action_
                // dispatch` invariant) and stamps the `via <port>` target as the
                // `via_port` prop (dispatch.rs, mirror of the SM/L38 path). The
                // port becomes `port_source` so the runner consumes the matching
                // payload from `port_inbox` (fed by `TickContext::port_payloads`).
                // Port-less accepts keep `port_source = None` (the message_inbox
                // / `deliver_message` path).
                let payload_binding = child.name.clone().unwrap_or_default();
                let port_source = child
                    .props
                    .get("via_port")
                    .and_then(Value::as_str)
                    .map(String::from);
                ActionNodeIR::Accept {
                    id: node_id.clone(),
                    source: None,
                    payload_binding,
                    port_source,
                }
            }
            ElementKind::AssignmentActionUsage => {
                let target_name = child
                    .props
                    .get("targetFeature")
                    .and_then(Value::as_str)
                    .unwrap_or("x")
                    .to_owned();
                ActionNodeIR::Assign {
                    id: node_id.clone(),
                    target: target_name,
                    value: ExprIR::LiteralInt(0), // placeholder
                }
            }
            ElementKind::TerminateActionUsage => ActionNodeIR::Terminate {
                id: node_id.clone(),
            },
            ElementKind::DecisionNode => ActionNodeIR::Decision {
                id: node_id.clone(),
            },
            ElementKind::ForkNode => ActionNodeIR::Fork {
                id: node_id.clone(),
            },
            ElementKind::JoinNode => ActionNodeIR::Join {
                id: node_id.clone(),
            },
            ElementKind::MergeNode => ActionNodeIR::Merge {
                id: node_id.clone(),
            },
            _ => continue, // Skip non-action children
        };

        elem_to_node.insert(child.id.clone(), node_id.clone());
        ir.add_node(node);
    }

    // Convert succession relationships into edges.
    // Use outgoing() index per source node instead of scanning all relationships.
    for source_id in elem_to_node.keys() {
        for rel in graph.outgoing(source_id) {
            if rel.kind != RelationshipKind::Transition {
                continue;
            }
            if let (Some(from), Some(to)) =
                (elem_to_node.get(&rel.source), elem_to_node.get(&rel.target))
            {
                ir.add_edge(from.clone(), to.clone());
            }
        }
    }

    // If there are nodes but no edges from initial, connect initial to the first step
    let node_ids: Vec<String> = elem_to_node.values().cloned().collect();
    if !node_ids.is_empty() {
        // Find nodes with no incoming edges to use as entry points
        let has_incoming: std::collections::HashSet<&str> =
            ir.edges.iter().map(|e| e.to.as_str()).collect();
        let entry_nodes: Vec<&str> = node_ids
            .iter()
            .filter(|id| !has_incoming.contains(id.as_str()))
            .map(|s| s.as_str())
            .collect();

        // Connect initial to entry nodes
        if let Some(&first) = entry_nodes.first() {
            if !ir.edges.iter().any(|e| e.from == ir.initial_node_id) {
                ir.add_edge(ir.initial_node_id.clone(), first.to_owned());
            }
        }

        // Find nodes with no outgoing edges and connect them to final
        let has_outgoing: std::collections::HashSet<&str> =
            ir.edges.iter().map(|e| e.from.as_str()).collect();
        let exit_nodes: Vec<&str> = node_ids
            .iter()
            .filter(|id| !has_outgoing.contains(id.as_str()))
            .map(|s| s.as_str())
            .collect();

        let final_id = ir.final_node_ids[0].clone();
        for exit in exit_nodes {
            ir.add_edge(exit.to_owned(), final_id.clone());
        }
    } else {
        // No child steps — connect initial directly to final
        let final_id = ir.final_node_ids[0].clone();
        ir.add_edge(ir.initial_node_id.clone(), final_id);
    }

    #[cfg(feature = "tracing")]
    tracing::debug!(
        action = action_name,
        nodes = ir.nodes.len(),
        edges = ir.edges.len(),
        parameters = ir.parameters.len(),
        "compiled action graph"
    );

    Ok(ir)
}

// ---------------------------------------------------------------------------
// Executor trait implementation (Phase 3)
// ---------------------------------------------------------------------------

/// RSC-2.4c: bind every retained node expression of an action graph
/// (`Assign` values, `Send`/`StreamSource` payloads, `If`/`WhileLoop`
/// conditions, `ForLoop` sequences, `Perform` inputs, decision-edge
/// guards), recursively through inline sub-action graphs. Library graphs
/// are NOT bound — sub-runners minted from the library evaluate against a
/// sub-context seeded from `Perform` inputs, exactly like SM sub-runners
/// keep compiled-but-unbound caches (identical results through the
/// context-name-first path).
fn bind_action_graph_exprs(
    ir: &mut ActionGraphIR,
    binder: &crate::expressions::SlotBinder<'_>,
    report: &mut crate::expressions::BindReport,
) {
    use crate::expressions::bind_slots;
    for node in &mut ir.nodes {
        match node {
            ActionNodeIR::Assign { value, .. } => bind_slots(value, binder, report),
            ActionNodeIR::Send { payload, .. } => bind_slots(payload, binder, report),
            ActionNodeIR::If { condition, .. } | ActionNodeIR::WhileLoop { condition, .. } => {
                bind_slots(condition, binder, report)
            }
            ActionNodeIR::ForLoop { sequence, .. } => bind_slots(sequence, binder, report),
            ActionNodeIR::StreamSource { value_expr, .. } => bind_slots(value_expr, binder, report),
            ActionNodeIR::Perform {
                inputs, sub_action, ..
            } => {
                for (_, expr) in inputs.iter_mut() {
                    bind_slots(expr, binder, report);
                }
                if let Some(sub) = sub_action {
                    bind_action_graph_exprs(sub, binder, report);
                }
            }
            _ => {}
        }
    }
    for edge in &mut ir.edges {
        if let Some(guard) = &mut edge.guard {
            bind_slots(guard, binder, report);
        }
    }
}

/// RSC-4.1: collect the compiler-resolved `SlotId`s read by every retained
/// node/edge expression of an action graph — the read-set dual of
/// [`bind_action_graph_exprs`], walking the *same* sites (so the read-set is
/// complete for whatever was bound). Recurses through inline sub-action
/// graphs; library graphs stay unbound (their reads resolve through the
/// `Perform`-seeded sub-context) so they contribute no slots, exactly like
/// the binding pass skips them.
fn collect_action_graph_slot_reads(ir: &ActionGraphIR, out: &mut Vec<crate::slots::SlotId>) {
    for node in &ir.nodes {
        match node {
            ActionNodeIR::Assign { value, .. } => out.extend(value.slot_reads()),
            ActionNodeIR::Send { payload, .. } => out.extend(payload.slot_reads()),
            ActionNodeIR::If { condition, .. } | ActionNodeIR::WhileLoop { condition, .. } => {
                out.extend(condition.slot_reads())
            }
            ActionNodeIR::ForLoop { sequence, .. } => out.extend(sequence.slot_reads()),
            ActionNodeIR::StreamSource { value_expr, .. } => out.extend(value_expr.slot_reads()),
            ActionNodeIR::Perform {
                inputs, sub_action, ..
            } => {
                for (_, expr) in inputs.iter() {
                    out.extend(expr.slot_reads());
                }
                if let Some(sub) = sub_action {
                    collect_action_graph_slot_reads(sub, out);
                }
            }
            _ => {}
        }
    }
    for edge in &ir.edges {
        if let Some(guard) = &edge.guard {
            out.extend(guard.slot_reads());
        }
    }
}

impl crate::orchestrator::Executor for ActionRunner {
    fn phase(&self) -> crate::orchestrator::ExecutionPhase {
        crate::orchestrator::ExecutionPhase::Action
    }

    fn kind_label(&self) -> &'static str {
        "action"
    }

    fn tick(
        &mut self,
        ctx: &crate::orchestrator::TickContext<'_>,
    ) -> crate::orchestrator::TickOutput {
        // Wave B-inc-2: feed port-addressed payloads to `accept … via <port>`
        // nodes through the unified `ctx.port_payloads` channel (the same the
        // SM reads). `port_events` are already scoped to this subsystem by
        // name, so every payload here is addressed to one of this action's
        // accept ports — buffer it for the Accept arm to consume. The
        // orchestrator no longer needs to feed action accepts via
        // `deliver_message`; `convert_deliveries_to_port_events` delivers.
        //
        // Steward-blessed fix (option iv): a COMPLETED action has no tokens
        // left to reach an Accept node — buffering here would grow
        // `port_inbox` forever with nothing left to drain it (only
        // `reset()` clears it, at ~line 634). Once completed, stop
        // buffering; this is a zero-heuristic gate, not a cap.
        if !self.completed {
            for (port, payload) in ctx.port_payloads {
                self.port_inbox
                    .entry(port.clone())
                    .or_default()
                    .push_back(payload.clone());
            }
        }
        let result = self.step(ctx.context);
        let sends: Vec<String> = result.messages.iter().map(|m| m.target.clone()).collect();
        // RSC-3.3c D4: action sends carry their named receiver
        // (`ActionMessage::target` = the Send node's port_target/target) to
        // the router as occurrence-addressed messages instead of dropping
        // the target at this seam. The orchestrator routes them via
        // `FlowRouter::send_message`: declared flows on the source action
        // keep winning; otherwise the named receiver resolves against the
        // registered accepting surfaces.
        let addressed_messages: Vec<(String, String, Value)> = result
            .messages
            .iter()
            .map(|m| (m.source_action.clone(), m.target.clone(), m.payload.clone()))
            .collect();

        crate::orchestrator::TickOutput {
            current_state: self.current_node_id().to_owned(),
            completed: result.completed,
            available_transitions: Vec::new(),
            outputs: result.outputs,
            sends,
            port_sends: Vec::new(),
            messages: Vec::new(),
            addressed_messages,
            incoming_trigger: None,
        }
    }

    fn reset_executor(&mut self) {
        self.reset();
    }

    fn accept_ports(&self) -> Vec<String> {
        ActionRunner::accept_ports(self)
    }

    fn armed_accept_ports(&self) -> Vec<String> {
        ActionRunner::armed_accept_ports(self)
    }

    fn is_completed(&self) -> bool {
        self.completed
    }

    fn clone_boxed(&self) -> Box<dyn crate::orchestrator::Executor> {
        Box::new(self.clone())
    }

    // `sync_context_in` / `sync_context_out` deliberately stay the trait
    // defaults (no-ops). The action executor has NEVER synchronized state
    // through the context seam: `step()` takes the shared context
    // read-only, and every write target lands in token-local bindings
    // (`ActionToken::bindings` → `final_bindings`). Nothing in the
    // runtime or service layer reads those bindings back out of a shared
    // context — trace output travels `TickOutput::outputs`, messages
    // travel `TickOutput::messages` into the FlowRouter (Phase 3
    // exchange plane).

    /// RSC-2.4c: slot-seam writeback for the action executor.
    ///
    /// Established at the cutover survey (2026-06-11): unlike SM/ODE,
    /// the legacy action writeback published **nothing** — both sync
    /// hooks are no-ops, all writes are token-local by design. The
    /// migrated path preserves that byte-for-byte: with a prepared
    /// write-set this returns `true` (the action kind formally leaves
    /// the legacy seam) and publishes nothing. Promoting token bindings
    /// to context publication would be a *behavioural* change to every
    /// snapshot (new keys in the variables map) — that is an exchange-
    /// plane/scheduler decision (Phase 3/4), not a storage cutover, and
    /// the compiled write-set + routes built here are exactly what that
    /// decision will need.
    fn sync_context_out_slots(
        &self,
        _shared: &mut EvalContext,
        _mode: crate::ode::SignalEvalMode,
    ) -> bool {
        self.write_set.is_some()
    }

    /// RSC-2.4c: build the precomputed slot write-set from the compiled
    /// write targets. Routes are resolved with the same single-writer
    /// check as SM/ODE (`WriteRoute::resolve`); today they only feed
    /// [`slot_write_fallbacks`](Self::slot_write_fallbacks) since nothing
    /// is published (see `sync_context_out_slots`). Until the compiler
    /// registers action subsystems and mints their write-target claims,
    /// every target resolves unrouted — coverage 0 by construction,
    /// pinned by test.
    fn prepare_slot_writeback(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
        canonical_prefix: Option<&str>,
        writer: crate::slots::WriterId,
    ) {
        let targets: Vec<(String, crate::slots::WriteRoute)> = collect_write_targets(&self.ir)
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
        self.write_set = Some(ActionWriteSet { targets });
    }

    /// RSC-2.4c: compiled write targets without a claimed slot route —
    /// for actions this is a **mint-coverage report** (the targets are
    /// token-local and published nowhere; nothing actually takes a
    /// name-keyed path). Surfaced through
    /// `Orchestrator::action_slot_fallbacks`, mirroring
    /// `sm_slot_fallbacks` / `ode_scoped_fallbacks` as the RSC-2.5
    /// observability hook.
    fn slot_write_fallbacks(&self) -> Vec<String> {
        let Some(ws) = &self.write_set else {
            return Vec::new();
        };
        ws.targets
            .iter()
            .filter(|(_, route)| !route.is_routed())
            .map(|(_, route)| route.runtime_key().to_owned())
            .collect()
    }

    /// RSC-2.4c (closes the action half of the RSC-2.3 deferred binding
    /// gap): bind the retained node expressions to slots in the
    /// subsystem-local scope. Evaluation stays context-name-first, so
    /// every node result is unchanged wherever the name is present in
    /// the merged context; the slot serves the read when it is not.
    /// Token-binding names (write targets, loop variables, perform
    /// inputs, declared parameters) are declared as binder locals: they
    /// resolve from token bindings at eval time and must neither rewrite
    /// to slots nor count as RS003 candidates — the public report clears
    /// `unresolved` accordingly (full report:
    /// [`ActionRunner::bind_report`]).
    fn bind_expression_slots(
        &mut self,
        store: &crate::slots::SlotStore,
        var_prefix: Option<&str>,
    ) -> crate::expressions::BindReport {
        use crate::expressions::{BindReport, SlotBinder};

        let locals = collect_token_local_names(&self.ir);
        let binder = SlotBinder::for_subsystem(store, var_prefix).with_locals(locals);
        let mut report = BindReport::default();
        bind_action_graph_exprs(&mut self.ir, &binder, &mut report);
        self.bind_report = report.clone();
        let mut public = report;
        public.unresolved.clear();
        public
    }

    /// RSC-4.1: read-set = the compiler-resolved slots read by every bound
    /// node/edge expression of the action graph (assignment values, send /
    /// stream payloads, if/while conditions, for-loop sequences, perform
    /// inputs, decision-edge guards), recursing inline sub-actions. Most
    /// action reads are token-local (`with_locals`) and never resolve to a
    /// slot — those contribute nothing, so a fully token-local action returns
    /// empty by construction. Only reads that bound to an instance/global slot
    /// surface here.
    fn read_slots(&self) -> Vec<crate::slots::SlotId> {
        let mut v = Vec::new();
        collect_action_graph_slot_reads(&self.ir, &mut v);
        v.sort();
        v.dedup();
        v
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::expressions::ExprIR;
    use std::borrow::Cow;
    use sysml_core::Relationship;

    #[test]
    fn empty_action_completes() {
        let mut graph = ActionGraphIR::new("test", "TestAction");
        // Connect initial directly to final
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        graph.add_edge(initial, final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        // First step: initial -> final
        let _r1 = runner.step(&ctx);
        // Second step: processes final node
        let result = runner.step(&ctx);
        assert!(result.completed);
    }

    #[test]
    fn assignment_action() {
        let mut graph = ActionGraphIR::new("test", "AssignAction");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let assign_id = graph.add_node(ActionNodeIR::Assign {
            id: "assign1".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(42),
        });

        graph.add_edge(initial, &assign_id);
        graph.add_edge(&assign_id, final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let _r1 = runner.step(&ctx); // initial -> assign
        let result = runner.step(&ctx); // assign -> final
        assert!(result
            .outputs
            .iter()
            .any(|o| o.contains("x") && o.contains("42")));
    }

    #[test]
    fn send_action_produces_message() {
        let mut graph = ActionGraphIR::new("test", "SendAction");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let send_id = graph.add_node(ActionNodeIR::Send {
            id: "send1".into(),
            payload: ExprIR::LiteralString("alert".into()),
            target: "operator".into(),
            port_target: None,
        });

        graph.add_edge(initial, &send_id);
        graph.add_edge(&send_id, final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let _r1 = runner.step(&ctx);
        let r2 = runner.step(&ctx);
        assert_eq!(r2.messages.len(), 1);
        assert_eq!(r2.messages[0].target, "operator");
    }

    #[test]
    fn fork_creates_parallel_tokens() {
        let mut graph = ActionGraphIR::new("test", "ForkAction");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let fork_id = graph.add_node(ActionNodeIR::Fork { id: "fork1".into() });
        let a = graph.add_node(ActionNodeIR::Assign {
            id: "a".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(1),
        });
        let b = graph.add_node(ActionNodeIR::Assign {
            id: "b".into(),
            target: "y".into(),
            value: ExprIR::LiteralInt(2),
        });

        graph.add_edge(initial, &fork_id);
        graph.add_edge(&fork_id, &a);
        graph.add_edge(&fork_id, &b);
        graph.add_edge(&a, &final_id);
        graph.add_edge(&b, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let _r1 = runner.step(&ctx); // initial -> fork
        let _r2 = runner.step(&ctx); // fork -> creates parallel tokens for a, b
        let r3 = runner.step(&ctx); // a and b execute in parallel
                                    // Both assignments should have produced outputs
        assert!(r3.outputs.len() >= 2);
    }

    #[test]
    fn terminate_stops_execution() {
        let mut graph = ActionGraphIR::new("test", "TermAction");
        let initial = graph.initial_node_id.clone();

        let term_id = graph.add_node(ActionNodeIR::Terminate { id: "term1".into() });

        graph.add_edge(initial, &term_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let _r1 = runner.step(&ctx);
        let _r2 = runner.step(&ctx);
        assert!(runner.is_completed());
    }

    #[test]
    fn reset_restarts_execution() {
        let mut graph = ActionGraphIR::new("test", "ResetAction");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        graph.add_edge(initial, final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        runner.step(&ctx);
        runner.step(&ctx);
        assert!(runner.is_completed());

        runner.reset();
        assert!(!runner.is_completed());
    }

    #[test]
    fn completed_runner_stops_buffering_port_inbox() {
        // Regression guard: once an ActionRunner is completed, tick() must
        // stop pushing routed (port, payload) pairs into port_inbox forever
        // — a completed action has no tokens left to reach an Accept node,
        // so nothing ever drains it except reset() (which also clears it).
        use crate::orchestrator::{Executor, TickContext};

        let mut graph = ActionGraphIR::new("test", "PortInboxAction");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        graph.add_edge(initial, final_id);

        let mut runner = ActionRunner::new(graph);
        let eval_ctx = EvalContext::new();
        let no_payloads: Vec<(String, Value)> = Vec::new();
        let empty_ctx = TickContext {
            t: 0.0,
            dt: 1.0,
            tick: 0,
            context: &eval_ctx,
            event: None,
            port_payloads: &no_payloads,
            local_clock_time: None,
        };

        // Drive to completion (Initial -> Final needs two ticks, matching
        // `reset_restarts_execution` above) with no port payloads in play,
        // so port_inbox starts out empty once completed.
        Executor::tick(&mut runner, &empty_ctx);
        let out = Executor::tick(&mut runner, &empty_ctx);
        assert!(out.completed);
        assert!(runner.is_completed());
        assert!(runner.port_inbox.is_empty());

        // Now tick several more times with non-empty port_payloads — the
        // buffering loop must be gated on `!self.completed`, so port_inbox
        // stays empty instead of growing without bound.
        let payloads = vec![("somePort".to_string(), Value::Int(1))];
        let payload_ctx = TickContext {
            t: 0.0,
            dt: 1.0,
            tick: 0,
            context: &eval_ctx,
            event: None,
            port_payloads: &payloads,
            local_clock_time: None,
        };
        for _ in 0..5 {
            Executor::tick(&mut runner, &payload_ctx);
        }
        assert!(
            runner.port_inbox.is_empty(),
            "port_inbox must not buffer payloads once the action is completed"
        );
    }

    // =====================================================================
    // S2a: While loop tests
    // =====================================================================

    /// Helper: run an action to completion, collecting all step results.
    fn run_to_completion(runner: &mut ActionRunner, ctx: &EvalContext) -> Vec<ActionStepResult> {
        let mut results = Vec::new();
        for _ in 0..200 {
            let r = runner.step(ctx);
            let done = r.completed;
            results.push(r);
            if done {
                break;
            }
        }
        results
    }

    #[test]
    fn while_loop_three_iterations() {
        // while (counter < 3) { counter = counter + 1 }
        let mut graph = ActionGraphIR::new("test", "WhileLoop3");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        // Set counter = 0
        let init_assign = graph.add_node(ActionNodeIR::Assign {
            id: "init_counter".into(),
            target: "counter".into(),
            value: ExprIR::LiteralInt(0),
        });

        // Body: counter = counter + 1
        let body_assign = graph.add_node(ActionNodeIR::Assign {
            id: "body_inc".into(),
            target: "counter".into(),
            value: ExprIR::BinaryOp {
                op: crate::expressions::BinOp::Add,
                left: Box::new(ExprIR::FeatureRef("counter".into())),
                right: Box::new(ExprIR::LiteralInt(1)),
            },
        });

        let while_node = graph.add_node(ActionNodeIR::WhileLoop {
            id: "while1".into(),
            condition: ExprIR::BinaryOp {
                op: crate::expressions::BinOp::LessThan,
                left: Box::new(ExprIR::FeatureRef("counter".into())),
                right: Box::new(ExprIR::LiteralInt(3)),
            },
            body_entry: "body_inc".into(),
            exit_node: final_id.clone(),
        });

        // body_inc -> while1 (back edge)
        graph.add_edge(&initial, &init_assign);
        graph.add_edge(&init_assign, &while_node);
        graph.add_edge(&body_assign, &while_node);
        graph.add_edge(&while_node, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // Should have 3 assignment outputs for "counter = ..."
        let assign_outputs: Vec<&String> = results
            .iter()
            .flat_map(|r| &r.outputs)
            .filter(|o| o.contains("counter"))
            .collect();
        // init (0) + 3 body increments (1, 2, 3) = 4 assignments
        assert_eq!(assign_outputs.len(), 4);
    }

    #[test]
    fn while_loop_zero_iterations() {
        // counter starts at 5, condition counter < 3 is false initially
        let mut graph = ActionGraphIR::new("test", "WhileLoop0");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let init_assign = graph.add_node(ActionNodeIR::Assign {
            id: "init_counter".into(),
            target: "counter".into(),
            value: ExprIR::LiteralInt(5),
        });

        let body_assign = graph.add_node(ActionNodeIR::Assign {
            id: "body_inc".into(),
            target: "counter".into(),
            value: ExprIR::BinaryOp {
                op: crate::expressions::BinOp::Add,
                left: Box::new(ExprIR::FeatureRef("counter".into())),
                right: Box::new(ExprIR::LiteralInt(1)),
            },
        });

        let while_node = graph.add_node(ActionNodeIR::WhileLoop {
            id: "while1".into(),
            condition: ExprIR::BinaryOp {
                op: crate::expressions::BinOp::LessThan,
                left: Box::new(ExprIR::FeatureRef("counter".into())),
                right: Box::new(ExprIR::LiteralInt(3)),
            },
            body_entry: "body_inc".into(),
            exit_node: final_id.clone(),
        });

        graph.add_edge(&initial, &init_assign);
        graph.add_edge(&init_assign, &while_node);
        graph.add_edge(&body_assign, &while_node);
        graph.add_edge(&while_node, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // Only 1 output: the init assignment (no body executions)
        let body_outputs: Vec<&String> = results
            .iter()
            .flat_map(|r| &r.outputs)
            .filter(|o| o.contains("counter"))
            .collect();
        assert_eq!(body_outputs.len(), 1); // just the init
    }

    #[test]
    fn while_loop_condition_false_initially() {
        // condition = false literal
        let mut graph = ActionGraphIR::new("test", "WhileLoopFalse");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let body = graph.add_node(ActionNodeIR::Assign {
            id: "body".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(99),
        });

        let while_node = graph.add_node(ActionNodeIR::WhileLoop {
            id: "while1".into(),
            condition: ExprIR::LiteralBool(false),
            body_entry: "body".into(),
            exit_node: final_id.clone(),
        });

        graph.add_edge(&initial, &while_node);
        graph.add_edge(&body, &while_node);
        graph.add_edge(&while_node, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // No body assignments should have occurred
        let body_outputs: Vec<&String> = results
            .iter()
            .flat_map(|r| &r.outputs)
            .filter(|o| o.contains("x"))
            .collect();
        assert!(body_outputs.is_empty());
    }

    #[test]
    fn while_loop_max_iterations_exceeded() {
        // condition = true always (infinite loop)
        let mut graph = ActionGraphIR::new("test", "WhileLoopInfinite");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let body = graph.add_node(ActionNodeIR::Assign {
            id: "body".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(1),
        });

        let while_node = graph.add_node(ActionNodeIR::WhileLoop {
            id: "while1".into(),
            condition: ExprIR::LiteralBool(true),
            body_entry: "body".into(),
            exit_node: final_id.clone(),
        });

        graph.add_edge(&initial, &while_node);
        graph.add_edge(&body, &while_node);
        graph.add_edge(&while_node, &final_id);

        let mut runner = ActionRunner::new(graph);
        runner.max_iterations = 5; // Low limit for testing
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        // Should have a diagnostic about exceeding max iterations
        let has_max_iter_error = results.iter().any(|r| {
            r.diagnostics
                .iter()
                .any(|d| format!("{:?}", d).contains("max iterations"))
        });
        assert!(has_max_iter_error);
    }

    // =====================================================================
    // S2a: For loop tests
    // =====================================================================

    #[test]
    fn for_loop_over_list() {
        // for item in [1, 2, 3] { x = item }
        let mut graph = ActionGraphIR::new("test", "ForLoop");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let body = graph.add_node(ActionNodeIR::Assign {
            id: "body_assign".into(),
            target: "x".into(),
            value: ExprIR::FeatureRef("item".into()),
        });

        let for_node = graph.add_node(ActionNodeIR::ForLoop {
            id: "for1".into(),
            variable: "item".into(),
            sequence: ExprIR::Sequence(vec![
                ExprIR::LiteralInt(1),
                ExprIR::LiteralInt(2),
                ExprIR::LiteralInt(3),
            ]),
            body_entry: "body_assign".into(),
            exit_node: final_id.clone(),
        });

        graph.add_edge(&initial, &for_node);
        graph.add_edge(&body, &for_node);
        graph.add_edge(&for_node, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // 3 assignments should have occurred
        let assign_outputs: Vec<&String> = results
            .iter()
            .flat_map(|r| &r.outputs)
            .filter(|o| o.contains("x ="))
            .collect();
        assert_eq!(assign_outputs.len(), 3);
    }

    #[test]
    fn for_loop_empty_sequence() {
        let mut graph = ActionGraphIR::new("test", "ForLoopEmpty");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let body = graph.add_node(ActionNodeIR::Assign {
            id: "body_assign".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(99),
        });

        let for_node = graph.add_node(ActionNodeIR::ForLoop {
            id: "for1".into(),
            variable: "item".into(),
            sequence: ExprIR::Sequence(vec![]),
            body_entry: "body_assign".into(),
            exit_node: final_id.clone(),
        });

        graph.add_edge(&initial, &for_node);
        graph.add_edge(&body, &for_node);
        graph.add_edge(&for_node, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // No body should have executed
        let body_outputs: Vec<&String> = results
            .iter()
            .flat_map(|r| &r.outputs)
            .filter(|o| o.contains("x ="))
            .collect();
        assert!(body_outputs.is_empty());
    }

    #[test]
    fn for_loop_binds_variable() {
        // Verify the loop variable is bound correctly in each iteration
        let mut graph = ActionGraphIR::new("test", "ForLoopBind");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        // Body copies item into 'last_seen'
        let body = graph.add_node(ActionNodeIR::Assign {
            id: "body_assign".into(),
            target: "last_seen".into(),
            value: ExprIR::FeatureRef("item".into()),
        });

        let for_node = graph.add_node(ActionNodeIR::ForLoop {
            id: "for1".into(),
            variable: "item".into(),
            sequence: ExprIR::Sequence(vec![
                ExprIR::LiteralInt(10),
                ExprIR::LiteralInt(20),
                ExprIR::LiteralInt(30),
            ]),
            body_entry: "body_assign".into(),
            exit_node: final_id.clone(),
        });

        graph.add_edge(&initial, &for_node);
        graph.add_edge(&body, &for_node);
        graph.add_edge(&for_node, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // The last assignment should be last_seen = Int(30)
        let last_output = results
            .iter()
            .flat_map(|r| &r.outputs)
            .filter(|o| o.contains("last_seen"))
            .last()
            .unwrap()
            .clone();
        assert!(last_output.contains("30"));
    }

    #[test]
    fn loop_accumulates_values() {
        // sum = 0; while (sum < 10) { sum = sum + 3 }
        // Expected: sum goes 0 -> 3 -> 6 -> 9 -> 12 (exits at 12)
        let mut graph = ActionGraphIR::new("test", "LoopAccumulate");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let init_sum = graph.add_node(ActionNodeIR::Assign {
            id: "init_sum".into(),
            target: "sum".into(),
            value: ExprIR::LiteralInt(0),
        });

        let body_add = graph.add_node(ActionNodeIR::Assign {
            id: "body_add".into(),
            target: "sum".into(),
            value: ExprIR::BinaryOp {
                op: crate::expressions::BinOp::Add,
                left: Box::new(ExprIR::FeatureRef("sum".into())),
                right: Box::new(ExprIR::LiteralInt(3)),
            },
        });

        let while_node = graph.add_node(ActionNodeIR::WhileLoop {
            id: "while1".into(),
            condition: ExprIR::BinaryOp {
                op: crate::expressions::BinOp::LessThan,
                left: Box::new(ExprIR::FeatureRef("sum".into())),
                right: Box::new(ExprIR::LiteralInt(10)),
            },
            body_entry: "body_add".into(),
            exit_node: final_id.clone(),
        });

        graph.add_edge(&initial, &init_sum);
        graph.add_edge(&init_sum, &while_node);
        graph.add_edge(&body_add, &while_node);
        graph.add_edge(&while_node, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // Collect sum assignments
        let sum_outputs: Vec<String> = results
            .iter()
            .flat_map(|r| r.outputs.clone())
            .filter(|o| o.contains("sum"))
            .collect();
        // init (0), +3 (3), +3 (6), +3 (9), +3 (12) => 5 outputs
        assert_eq!(sum_outputs.len(), 5);
        // Last should be 12
        assert!(sum_outputs.last().unwrap().contains("12"));
    }

    // =====================================================================
    // S2b: Join semantics tests
    // =====================================================================

    #[test]
    fn join_waits_for_all_branches() {
        // fork -> (a, b) -> join -> final
        let mut graph = ActionGraphIR::new("test", "JoinWaits");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let fork_id = graph.add_node(ActionNodeIR::Fork { id: "fork1".into() });
        let a = graph.add_node(ActionNodeIR::Assign {
            id: "a".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(1),
        });
        let b = graph.add_node(ActionNodeIR::Assign {
            id: "b".into(),
            target: "y".into(),
            value: ExprIR::LiteralInt(2),
        });
        let join_id = graph.add_node(ActionNodeIR::Join { id: "join1".into() });

        graph.add_edge(&initial, &fork_id);
        graph.add_edge(&fork_id, &a);
        graph.add_edge(&fork_id, &b);
        graph.add_edge(&a, &join_id);
        graph.add_edge(&b, &join_id);
        graph.add_edge(&join_id, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // Both assignments should have happened
        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        assert!(all_outputs.iter().any(|o| o.contains("x")));
        assert!(all_outputs.iter().any(|o| o.contains("y")));
    }

    #[test]
    fn join_partial_arrival_blocks() {
        // Same graph as above, but we step manually to verify blocking
        let mut graph = ActionGraphIR::new("test", "JoinBlocks");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let fork_id = graph.add_node(ActionNodeIR::Fork { id: "fork1".into() });
        let a = graph.add_node(ActionNodeIR::Assign {
            id: "a".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(1),
        });
        let b = graph.add_node(ActionNodeIR::Assign {
            id: "b".into(),
            target: "y".into(),
            value: ExprIR::LiteralInt(2),
        });
        let join_id = graph.add_node(ActionNodeIR::Join { id: "join1".into() });

        graph.add_edge(&initial, &fork_id);
        graph.add_edge(&fork_id, &a);
        graph.add_edge(&fork_id, &b);
        graph.add_edge(&a, &join_id);
        graph.add_edge(&b, &join_id);
        graph.add_edge(&join_id, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        // Step 1: initial -> fork
        let r1 = runner.step(&ctx);
        assert!(!r1.completed);

        // Step 2: fork -> spawns tokens at a, b
        let r2 = runner.step(&ctx);
        assert!(!r2.completed);

        // Step 3: a and b execute, tokens arrive at join
        // At this point both should arrive at join in the same step
        let r3 = runner.step(&ctx);
        assert!(!r3.completed);

        // Step 4: join has both tokens -> forwards to final
        let _r4 = runner.step(&ctx);
        // Final node is processed
        let r5 = runner.step(&ctx);
        assert!(r5.completed);
    }

    #[test]
    fn fork_join_parallel_assignment() {
        // fork -> assign x=10 and assign y=20 -> join -> verify both exist -> final
        let mut graph = ActionGraphIR::new("test", "ForkJoinAssign");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let fork_id = graph.add_node(ActionNodeIR::Fork { id: "fork1".into() });
        let assign_x = graph.add_node(ActionNodeIR::Assign {
            id: "assign_x".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(10),
        });
        let assign_y = graph.add_node(ActionNodeIR::Assign {
            id: "assign_y".into(),
            target: "y".into(),
            value: ExprIR::LiteralInt(20),
        });
        let join_id = graph.add_node(ActionNodeIR::Join { id: "join1".into() });
        // After join, assign z = x + y to prove both are available
        let assign_z = graph.add_node(ActionNodeIR::Assign {
            id: "assign_z".into(),
            target: "z".into(),
            value: ExprIR::BinaryOp {
                op: crate::expressions::BinOp::Add,
                left: Box::new(ExprIR::FeatureRef("x".into())),
                right: Box::new(ExprIR::FeatureRef("y".into())),
            },
        });

        graph.add_edge(&initial, &fork_id);
        graph.add_edge(&fork_id, &assign_x);
        graph.add_edge(&fork_id, &assign_y);
        graph.add_edge(&assign_x, &join_id);
        graph.add_edge(&assign_y, &join_id);
        graph.add_edge(&join_id, &assign_z);
        graph.add_edge(&assign_z, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // z should be x + y = 30
        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        assert!(all_outputs
            .iter()
            .any(|o| o.contains("z") && o.contains("30")));
    }

    // =====================================================================
    // S2c: Subaction composition (Perform) tests
    // =====================================================================

    /// Helper: build a sub-action that assigns result = input + 10.
    fn build_add_ten_action() -> ActionGraphIR {
        let mut sub = ActionGraphIR::new("add_ten", "AddTen");
        sub.parameters.push(ActionParameter {
            name: "input".into(),
            direction: ParameterDirection::In,
            default_value: None,
        });
        sub.parameters.push(ActionParameter {
            name: "result".into(),
            direction: ParameterDirection::Out,
            default_value: None,
        });

        let initial = sub.initial_node_id.clone();
        let final_id = sub.final_node_ids[0].clone();

        let assign_id = sub.add_node(ActionNodeIR::Assign {
            id: "add".into(),
            target: "result".into(),
            value: ExprIR::BinaryOp {
                op: crate::expressions::BinOp::Add,
                left: Box::new(ExprIR::FeatureRef("input".into())),
                right: Box::new(ExprIR::LiteralInt(10)),
            },
        });

        sub.add_edge(initial, &assign_id);
        sub.add_edge(&assign_id, final_id);
        sub
    }

    #[test]
    fn perform_executes_subaction() {
        // Parent action performs a sub-action that assigns x = 42
        let mut sub = ActionGraphIR::new("set_x", "SetX");
        let sub_initial = sub.initial_node_id.clone();
        let sub_final = sub.final_node_ids[0].clone();
        let sub_assign = sub.add_node(ActionNodeIR::Assign {
            id: "sub_assign".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(42),
        });
        sub.add_edge(sub_initial, &sub_assign);
        sub.add_edge(&sub_assign, sub_final);

        let mut lib = HashMap::new();
        lib.insert("SetX".to_string(), sub);

        // Parent graph: initial -> perform SetX -> final
        let mut graph = ActionGraphIR::new("parent", "Parent");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        let perform_id = graph.add_node(ActionNodeIR::Perform {
            id: "perf1".into(),
            action_ref: "SetX".into(),
            inputs: Vec::new(),
            output_binding: None,
            sub_action: None,
        });
        graph.add_edge(initial, &perform_id);
        graph.add_edge(&perform_id, final_id);

        let mut runner = ActionRunner::with_library(graph, lib);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // Should see the sub-action's assignment output and completion
        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        assert!(all_outputs
            .iter()
            .any(|o| o.contains("x") && o.contains("42")));
        assert!(all_outputs
            .iter()
            .any(|o| o.contains("perform SetX completed")));
    }

    #[test]
    fn perform_passes_inputs() {
        // Sub-action: result = input + 10
        let sub = build_add_ten_action();
        let mut lib = HashMap::new();
        lib.insert("AddTen".to_string(), sub);

        // Parent performs AddTen with input=5
        let mut graph = ActionGraphIR::new("parent", "Parent");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        let perform_id = graph.add_node(ActionNodeIR::Perform {
            id: "perf1".into(),
            action_ref: "AddTen".into(),
            inputs: vec![("input".to_string(), ExprIR::LiteralInt(5))],
            output_binding: None,
            sub_action: None,
        });
        graph.add_edge(initial, &perform_id);
        graph.add_edge(&perform_id, final_id);

        let mut runner = ActionRunner::with_library(graph, lib);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // Sub-action should compute result = 5 + 10 = 15
        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        assert!(all_outputs
            .iter()
            .any(|o| o.contains("result") && o.contains("15")));
    }

    #[test]
    fn perform_captures_output() {
        // Sub-action: result = input + 10 (with declared out parameter)
        let sub = build_add_ten_action();
        let mut lib = HashMap::new();
        lib.insert("AddTen".to_string(), sub);

        // Parent: perform AddTen(input=7) -> capture output as "answer" -> assign check = answer
        let mut graph = ActionGraphIR::new("parent", "Parent");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        let perform_id = graph.add_node(ActionNodeIR::Perform {
            id: "perf1".into(),
            action_ref: "AddTen".into(),
            inputs: vec![("input".to_string(), ExprIR::LiteralInt(7))],
            output_binding: Some("answer".to_string()),
            sub_action: None,
        });
        // Use the output: check = answer
        let check_id = graph.add_node(ActionNodeIR::Assign {
            id: "check".into(),
            target: "check".into(),
            value: ExprIR::FeatureRef("answer".into()),
        });
        graph.add_edge(initial, &perform_id);
        graph.add_edge(&perform_id, &check_id);
        graph.add_edge(&check_id, final_id);

        let mut runner = ActionRunner::with_library(graph, lib);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        // check should be 17 (7 + 10)
        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        assert!(
            all_outputs
                .iter()
                .any(|o| o.contains("check") && o.contains("17")),
            "Expected check=17 in outputs, got: {:?}",
            all_outputs
        );
    }

    #[test]
    fn perform_nested_two_levels() {
        // Inner: assigns x = 100
        let mut inner = ActionGraphIR::new("inner", "Inner");
        let inner_initial = inner.initial_node_id.clone();
        let inner_final = inner.final_node_ids[0].clone();
        let inner_assign = inner.add_node(ActionNodeIR::Assign {
            id: "inner_set".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(100),
        });
        inner.add_edge(inner_initial, &inner_assign);
        inner.add_edge(&inner_assign, inner_final);

        // Middle: performs Inner, then assigns y = 200
        let mut middle = ActionGraphIR::new("middle", "Middle");
        let mid_initial = middle.initial_node_id.clone();
        let mid_final = middle.final_node_ids[0].clone();
        let mid_perf = middle.add_node(ActionNodeIR::Perform {
            id: "mid_perf".into(),
            action_ref: "Inner".into(),
            inputs: Vec::new(),
            output_binding: None,
            sub_action: None,
        });
        let mid_assign = middle.add_node(ActionNodeIR::Assign {
            id: "mid_set".into(),
            target: "y".into(),
            value: ExprIR::LiteralInt(200),
        });
        middle.add_edge(mid_initial, &mid_perf);
        middle.add_edge(&mid_perf, &mid_assign);
        middle.add_edge(&mid_assign, mid_final);

        let mut lib = HashMap::new();
        lib.insert("Inner".to_string(), inner);
        lib.insert("Middle".to_string(), middle);

        // Outer: performs Middle
        let mut graph = ActionGraphIR::new("outer", "Outer");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        let perf = graph.add_node(ActionNodeIR::Perform {
            id: "outer_perf".into(),
            action_ref: "Middle".into(),
            inputs: Vec::new(),
            output_binding: None,
            sub_action: None,
        });
        graph.add_edge(initial, &perf);
        graph.add_edge(&perf, final_id);

        let mut runner = ActionRunner::with_library(graph, lib);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        // Should see inner's x=100, middle's y=200, and both perform completions
        assert!(
            all_outputs
                .iter()
                .any(|o| o.contains("x") && o.contains("100")),
            "Expected inner x=100, got: {:?}",
            all_outputs
        );
        assert!(
            all_outputs
                .iter()
                .any(|o| o.contains("y") && o.contains("200")),
            "Expected middle y=200, got: {:?}",
            all_outputs
        );
        assert!(all_outputs
            .iter()
            .any(|o| o.contains("perform Inner completed")));
        assert!(all_outputs
            .iter()
            .any(|o| o.contains("perform Middle completed")));
    }

    // =====================================================================
    // S2d: Compiler from ModelGraph tests
    // =====================================================================

    /// Helper: create a simple ModelGraph with an action definition and child steps.
    fn build_test_model_graph() -> ModelGraph {
        let mut graph = ModelGraph::new();

        // Action definition: "MyAction"
        let action_def = Element::new(ElementId::new_v4(), ElementKind::ActionDefinition);
        let action_id = {
            let mut e = action_def;
            e.name = Some("MyAction".to_string());
            let id = e.id.clone();
            graph.add_element(e);
            id
        };

        // Step 1: assign step
        let mut step1 = Element::new(ElementId::new_v4(), ElementKind::AssignmentActionUsage);
        step1.name = Some("step1".to_string());
        step1.owner = Some(action_id.clone());
        step1.props.insert(
            Cow::Borrowed("targetFeature"),
            Value::String("x".to_string()),
        );
        let step1_id = step1.id.clone();
        graph.add_element(step1);

        // Step 2: assign step
        let mut step2 = Element::new(ElementId::new_v4(), ElementKind::AssignmentActionUsage);
        step2.name = Some("step2".to_string());
        step2.owner = Some(action_id.clone());
        step2.props.insert(
            Cow::Borrowed("targetFeature"),
            Value::String("y".to_string()),
        );
        let step2_id = step2.id.clone();
        graph.add_element(step2);

        // Succession: step1 -> step2
        let succession = Relationship::new(RelationshipKind::Transition, step1_id, step2_id);
        graph.add_relationship(succession);

        graph
    }

    #[test]
    fn compile_simple_sequence() {
        let model = build_test_model_graph();
        let ir = compile_action("MyAction", &model).expect("compilation should succeed");

        assert_eq!(ir.name, "MyAction");
        // Should have initial + final + 2 action steps = 4 nodes
        assert!(ir.nodes.len() >= 4);
        // Should have edges: initial->step1, step1->step2, step2->final
        assert!(ir.edges.len() >= 3);
    }

    #[test]
    fn compile_succession_to_edges() {
        let model = build_test_model_graph();
        let ir = compile_action("MyAction", &model).expect("compilation should succeed");

        // Find the two assign nodes
        let assign_nodes: Vec<&ActionNodeIR> = ir
            .nodes
            .iter()
            .filter(|n| matches!(n, ActionNodeIR::Assign { .. }))
            .collect();
        assert_eq!(assign_nodes.len(), 2);

        // Verify there's an edge between the two assign nodes
        let id_0 = assign_nodes[0].id().to_string();
        let id_1 = assign_nodes[1].id().to_string();
        let has_succession_edge = ir
            .edges
            .iter()
            .any(|e| (e.from == id_0 && e.to == id_1) || (e.from == id_1 && e.to == id_0));
        assert!(
            has_succession_edge,
            "Expected succession edge between assign nodes"
        );
    }

    #[test]
    fn compile_decision_node() {
        let mut graph = ModelGraph::new();

        let mut action = Element::new(ElementId::new_v4(), ElementKind::ActionDefinition);
        action.name = Some("DecisionAction".to_string());
        let action_id = action.id.clone();
        graph.add_element(action);

        let mut decision = Element::new(ElementId::new_v4(), ElementKind::DecisionNode);
        decision.name = Some("decide".to_string());
        decision.owner = Some(action_id.clone());
        let decision_id = decision.id.clone();
        graph.add_element(decision);

        let ir = compile_action("DecisionAction", &graph).expect("should compile");

        // Find the decision node in IR
        let decision_nodes: Vec<&ActionNodeIR> = ir
            .nodes
            .iter()
            .filter(|n| matches!(n, ActionNodeIR::Decision { .. }))
            .collect();
        assert_eq!(decision_nodes.len(), 1);
        assert_eq!(decision_nodes[0].id(), decision_id.to_string());
    }

    #[test]
    fn compile_fork_join() {
        let mut graph = ModelGraph::new();

        let mut action = Element::new(ElementId::new_v4(), ElementKind::ActionDefinition);
        action.name = Some("ForkJoinAction".to_string());
        let action_id = action.id.clone();
        graph.add_element(action);

        let mut fork = Element::new(ElementId::new_v4(), ElementKind::ForkNode);
        fork.name = Some("fork1".to_string());
        fork.owner = Some(action_id.clone());
        graph.add_element(fork);

        let mut join = Element::new(ElementId::new_v4(), ElementKind::JoinNode);
        join.name = Some("join1".to_string());
        join.owner = Some(action_id.clone());
        graph.add_element(join);

        let ir = compile_action("ForkJoinAction", &graph).expect("should compile");

        let fork_nodes: Vec<&ActionNodeIR> = ir
            .nodes
            .iter()
            .filter(|n| matches!(n, ActionNodeIR::Fork { .. }))
            .collect();
        let join_nodes: Vec<&ActionNodeIR> = ir
            .nodes
            .iter()
            .filter(|n| matches!(n, ActionNodeIR::Join { .. }))
            .collect();
        assert_eq!(fork_nodes.len(), 1);
        assert_eq!(join_nodes.len(), 1);
    }

    #[test]
    fn compile_action_parameters() {
        let mut graph = ModelGraph::new();

        let mut action = Element::new(ElementId::new_v4(), ElementKind::ActionDefinition);
        action.name = Some("ParamAction".to_string());
        let action_id = action.id.clone();
        graph.add_element(action);

        // Input parameter
        let mut param_in = Element::new(ElementId::new_v4(), ElementKind::ActionUsage);
        param_in.name = Some("speed".to_string());
        param_in.owner = Some(action_id.clone());
        param_in
            .props
            .insert(Cow::Borrowed("direction"), Value::String("in".to_string()));
        graph.add_element(param_in);

        // Output parameter
        let mut param_out = Element::new(ElementId::new_v4(), ElementKind::ActionUsage);
        param_out.name = Some("result".to_string());
        param_out.owner = Some(action_id.clone());
        param_out
            .props
            .insert(Cow::Borrowed("direction"), Value::String("out".to_string()));
        graph.add_element(param_out);

        let ir = compile_action("ParamAction", &graph).expect("should compile");

        assert_eq!(ir.parameters.len(), 2);

        let in_param = ir.parameters.iter().find(|p| p.name == "speed").unwrap();
        assert_eq!(in_param.direction, ParameterDirection::In);

        let out_param = ir.parameters.iter().find(|p| p.name == "result").unwrap();
        assert_eq!(out_param.direction, ParameterDirection::Out);
    }

    // =====================================================================
    // Accept node blocking tests
    // =====================================================================

    #[test]
    fn accept_blocks_when_inbox_empty() {
        // Build a simple action: Initial -> Accept("msg") -> Final
        let mut graph = ActionGraphIR::new("test", "AcceptBlock");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let accept_id = graph.add_node(ActionNodeIR::Accept {
            id: "accept1".into(),
            source: None,
            payload_binding: "msg".into(),
            port_source: None,
        });

        graph.add_edge(&initial, &accept_id);
        graph.add_edge(&accept_id, &final_id);

        let ctx = EvalContext::new();
        let mut runner = ActionRunner::new(graph);

        // Step 1: token moves from Initial to Accept node
        let r1 = runner.step(&ctx);
        assert!(!r1.completed, "Should not be completed — token at Accept");
        assert!(!runner.is_completed(), "Runner should not be completed");

        // Step 2: still blocked (no message in inbox)
        let r2 = runner.step(&ctx);
        assert!(!r2.completed, "Still blocked — no message");
        assert!(
            runner.is_blocked(),
            "All tokens should be blocked at Accept"
        );

        // Deliver a message
        runner.deliver_message(ActionMessage {
            source_action: "test".into(),
            target: "accept1".into(),
            payload: Value::String("hello".into()),
        });

        // Step 3: message consumed, token advances to Final
        let r3 = runner.step(&ctx);
        assert!(!runner.is_blocked(), "Should no longer be blocked");

        // Step 4: process Final node -> completed
        if !r3.completed {
            let r4 = runner.step(&ctx);
            assert!(r4.completed, "Should complete after accepting message");
        }
    }

    #[test]
    fn accept_advances_when_message_available() {
        // Same graph but deliver message BEFORE stepping to Accept
        let mut graph = ActionGraphIR::new("test", "AcceptImmediate");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let accept_id = graph.add_node(ActionNodeIR::Accept {
            id: "accept1".into(),
            source: None,
            payload_binding: "msg".into(),
            port_source: None,
        });

        graph.add_edge(&initial, &accept_id);
        graph.add_edge(&accept_id, &final_id);

        let ctx = EvalContext::new();
        let mut runner = ActionRunner::new(graph);

        // Deliver message before stepping
        runner.deliver_message(ActionMessage {
            source_action: "test".into(),
            target: "accept1".into(),
            payload: Value::Int(42),
        });

        // Run to completion — should not block since message is already available
        let results = run_to_completion(&mut runner, &ctx);
        assert!(
            results.last().unwrap().completed,
            "Should complete when message is pre-delivered"
        );
        // Verify the message was actually accepted
        let accepted = results
            .iter()
            .flat_map(|r| r.outputs.iter())
            .any(|o| o.contains("accepted message into msg"));
        assert!(accepted, "Should have accepted the message");
    }

    #[test]
    fn is_blocked_false_when_no_tokens() {
        let mut graph = ActionGraphIR::new("test", "EmptyBlock");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        graph.add_edge(initial, final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        // Run to completion
        run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());
        assert!(
            !runner.is_blocked(),
            "Completed runner should not be blocked"
        );
    }

    // =====================================================================
    // Feature 2.5: Inline sub-action invocation via sub_action field
    // =====================================================================

    #[test]
    fn perform_inline_subaction() {
        // Main action: grind -> perform(clean) -> pour
        // Sub-action (clean): rinse -> dry (inline, no library needed)
        let mut sub_graph = ActionGraphIR::new("clean", "Clean");
        let sub_initial = sub_graph.initial_node_id.clone();
        let sub_final = sub_graph.final_node_ids[0].clone();
        let rinse_id = sub_graph.add_node(ActionNodeIR::Assign {
            id: "rinse".into(),
            target: "step".into(),
            value: ExprIR::LiteralString("rinsing".into()),
        });
        let dry_id = sub_graph.add_node(ActionNodeIR::Assign {
            id: "dry".into(),
            target: "step".into(),
            value: ExprIR::LiteralString("drying".into()),
        });
        sub_graph.add_edge(&sub_initial, &rinse_id);
        sub_graph.add_edge(&rinse_id, &dry_id);
        sub_graph.add_edge(&dry_id, &sub_final);

        let mut main_graph = ActionGraphIR::new("main", "Main");
        let initial = main_graph.initial_node_id.clone();
        let final_id = main_graph.final_node_ids[0].clone();

        let grind_id = main_graph.add_node(ActionNodeIR::Assign {
            id: "grind".into(),
            target: "stage".into(),
            value: ExprIR::LiteralString("grinding".into()),
        });
        let clean_id = main_graph.add_node(ActionNodeIR::Perform {
            id: "clean".into(),
            action_ref: "Clean".into(),
            inputs: Vec::new(),
            output_binding: None,
            sub_action: Some(Box::new(sub_graph)),
        });
        let pour_id = main_graph.add_node(ActionNodeIR::Assign {
            id: "pour".into(),
            target: "stage".into(),
            value: ExprIR::LiteralString("pouring".into()),
        });

        main_graph.add_edge(&initial, &grind_id);
        main_graph.add_edge(&grind_id, &clean_id);
        main_graph.add_edge(&clean_id, &pour_id);
        main_graph.add_edge(&pour_id, &final_id);

        let mut runner = ActionRunner::new(main_graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        // Should see grind, sub-action steps (rinse, dry), perform completion, then pour
        assert!(
            all_outputs.iter().any(|o| o.contains("grinding")),
            "Expected grind output, got: {:?}",
            all_outputs
        );
        assert!(
            all_outputs.iter().any(|o| o.contains("rinsing")),
            "Expected rinse output from sub-action, got: {:?}",
            all_outputs
        );
        assert!(
            all_outputs.iter().any(|o| o.contains("drying")),
            "Expected dry output from sub-action, got: {:?}",
            all_outputs
        );
        assert!(
            all_outputs
                .iter()
                .any(|o| o.contains("perform Clean completed")),
            "Expected perform completion, got: {:?}",
            all_outputs
        );
        assert!(
            all_outputs.iter().any(|o| o.contains("pouring")),
            "Expected pour output, got: {:?}",
            all_outputs
        );
    }

    #[test]
    fn inline_subaction_merges_bindings_to_parent() {
        // Sub-action sets result = 99; parent should see it after perform
        let mut sub_graph = ActionGraphIR::new("compute", "Compute");
        let sub_initial = sub_graph.initial_node_id.clone();
        let sub_final = sub_graph.final_node_ids[0].clone();
        let sub_assign = sub_graph.add_node(ActionNodeIR::Assign {
            id: "sub_set".into(),
            target: "result".into(),
            value: ExprIR::LiteralInt(99),
        });
        sub_graph.add_edge(sub_initial, &sub_assign);
        sub_graph.add_edge(&sub_assign, sub_final);

        let mut graph = ActionGraphIR::new("parent", "Parent");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();

        let perform_id = graph.add_node(ActionNodeIR::Perform {
            id: "perf".into(),
            action_ref: "Compute".into(),
            inputs: Vec::new(),
            output_binding: None,
            sub_action: Some(Box::new(sub_graph)),
        });
        // After perform, use the merged binding: check = result
        let check_id = graph.add_node(ActionNodeIR::Assign {
            id: "check".into(),
            target: "check".into(),
            value: ExprIR::FeatureRef("result".into()),
        });
        graph.add_edge(&initial, &perform_id);
        graph.add_edge(&perform_id, &check_id);
        graph.add_edge(&check_id, &final_id);

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        assert!(
            all_outputs
                .iter()
                .any(|o| o.contains("check") && o.contains("99")),
            "Expected check=99 from merged sub-action bindings, got: {:?}",
            all_outputs
        );
    }

    #[test]
    fn with_sub_action_builder() {
        // Test the with_sub_action builder method on ActionGraphIR
        let mut sub_graph = ActionGraphIR::new("sub", "Sub");
        let sub_initial = sub_graph.initial_node_id.clone();
        let sub_final = sub_graph.final_node_ids[0].clone();
        let sub_assign = sub_graph.add_node(ActionNodeIR::Assign {
            id: "sub_assign".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(55),
        });
        sub_graph.add_edge(sub_initial, &sub_assign);
        sub_graph.add_edge(&sub_assign, sub_final);

        let mut graph = ActionGraphIR::new("parent", "Parent");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        let perform_id = graph.add_node(ActionNodeIR::Perform {
            id: "perf".into(),
            action_ref: "Sub".into(),
            inputs: Vec::new(),
            output_binding: None,
            sub_action: None, // Will be set by with_sub_action
        });
        graph.add_edge(&initial, &perform_id);
        graph.add_edge(&perform_id, &final_id);

        // Use the builder to attach the sub-action
        let graph = graph.with_sub_action("perf", sub_graph);

        // Verify the sub-action was attached
        let perf_node = graph.find_node("perf").unwrap();
        assert!(
            matches!(
                perf_node,
                ActionNodeIR::Perform {
                    sub_action: Some(_),
                    ..
                }
            ),
            "with_sub_action should have attached the sub-action graph"
        );

        // Run and verify it executes correctly
        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();
        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        assert!(
            all_outputs
                .iter()
                .any(|o| o.contains("x") && o.contains("55")),
            "Expected sub-action to execute via with_sub_action builder, got: {:?}",
            all_outputs
        );
    }

    #[test]
    fn inline_subaction_preferred_over_library() {
        // Both inline sub_action and library entry exist -- inline should win
        let mut inline_sub = ActionGraphIR::new("sub_inline", "Sub");
        let si = inline_sub.initial_node_id.clone();
        let sf = inline_sub.final_node_ids[0].clone();
        let sia = inline_sub.add_node(ActionNodeIR::Assign {
            id: "inline_set".into(),
            target: "source".into(),
            value: ExprIR::LiteralString("inline".into()),
        });
        inline_sub.add_edge(si, &sia);
        inline_sub.add_edge(&sia, sf);

        let mut lib_sub = ActionGraphIR::new("sub_lib", "Sub");
        let li = lib_sub.initial_node_id.clone();
        let lf = lib_sub.final_node_ids[0].clone();
        let la = lib_sub.add_node(ActionNodeIR::Assign {
            id: "lib_set".into(),
            target: "source".into(),
            value: ExprIR::LiteralString("library".into()),
        });
        lib_sub.add_edge(li, &la);
        lib_sub.add_edge(&la, lf);

        let mut lib = HashMap::new();
        lib.insert("Sub".to_string(), lib_sub);

        let mut graph = ActionGraphIR::new("parent", "Parent");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        let perform_id = graph.add_node(ActionNodeIR::Perform {
            id: "perf".into(),
            action_ref: "Sub".into(),
            inputs: Vec::new(),
            output_binding: None,
            sub_action: Some(Box::new(inline_sub)),
        });
        graph.add_edge(&initial, &perform_id);
        graph.add_edge(&perform_id, &final_id);

        let mut runner = ActionRunner::with_library(graph, lib);
        let ctx = EvalContext::new();
        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());

        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        // Should see "inline", not "library"
        assert!(
            all_outputs
                .iter()
                .any(|o| o.contains("source") && o.contains("inline")),
            "Inline sub-action should take priority over library, got: {:?}",
            all_outputs
        );
        assert!(
            !all_outputs
                .iter()
                .any(|o| o.contains("source") && o.contains("library")),
            "Library sub-action should NOT have been used, got: {:?}",
            all_outputs
        );
    }

    // =====================================================================
    // Feature 2.4: Streaming / continuous actions (StreamSource)
    // =====================================================================

    #[test]
    fn stream_source_emits_every_step() {
        // Create a graph with a StreamSource that emits every step
        let mut graph = ActionGraphIR::new("stream_test", "StreamTest");
        graph.add_node(ActionNodeIR::StreamSource {
            id: "sensor".into(),
            value_expr: ExprIR::LiteralReal(42.0),
            target: "controller".into(),
            port_target: None,
            emit_interval: 1,
        });

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        // Step 3 times -- should get 3 emissions
        let r1 = runner.step(&ctx);
        assert_eq!(r1.messages.len(), 1);
        let r2 = runner.step(&ctx);
        assert_eq!(r2.messages.len(), 1);
        let r3 = runner.step(&ctx);
        assert_eq!(r3.messages.len(), 1);

        assert_eq!(runner.stream_emissions("sensor"), 3);
        assert_eq!(
            runner.stream_last_value("sensor"),
            Some(&Value::Float(42.0))
        );
    }

    #[test]
    fn stream_source_with_interval() {
        let mut graph = ActionGraphIR::new("interval_test", "IntervalTest");
        graph.add_node(ActionNodeIR::StreamSource {
            id: "sensor".into(),
            value_expr: ExprIR::LiteralReal(100.0),
            target: "out".into(),
            port_target: None,
            emit_interval: 3, // emit every 3rd step
        });

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        runner.step(&ctx); // tick 1 -- no emit
        assert_eq!(runner.stream_emissions("sensor"), 0);
        runner.step(&ctx); // tick 2 -- no emit
        assert_eq!(runner.stream_emissions("sensor"), 0);
        runner.step(&ctx); // tick 3 -- EMIT
        assert_eq!(runner.stream_emissions("sensor"), 1);
        runner.step(&ctx); // tick 4 -- no emit
        assert_eq!(runner.stream_emissions("sensor"), 1);
        runner.step(&ctx); // tick 5 -- no emit
        assert_eq!(runner.stream_emissions("sensor"), 1);
        runner.step(&ctx); // tick 6 -- EMIT
        assert_eq!(runner.stream_emissions("sensor"), 2);
    }

    #[test]
    fn stream_source_context_dependent() {
        // Stream emits context-dependent value: temperature * 2
        let mut graph = ActionGraphIR::new("ctx_test", "CtxTest");
        graph.add_node(ActionNodeIR::StreamSource {
            id: "calc".into(),
            value_expr: ExprIR::BinaryOp {
                op: crate::expressions::BinOp::Multiply,
                left: Box::new(ExprIR::FeatureRef("temperature".into())),
                right: Box::new(ExprIR::LiteralReal(2.0)),
            },
            target: "out".into(),
            port_target: None,
            emit_interval: 1,
        });

        let mut runner = ActionRunner::new(graph);
        let mut ctx = EvalContext::new();
        ctx.set("temperature", Value::Float(50.0));

        runner.step(&ctx);
        assert_eq!(runner.stream_last_value("calc"), Some(&Value::Float(100.0)));

        ctx.set("temperature", Value::Float(75.0));
        runner.step(&ctx);
        assert_eq!(runner.stream_last_value("calc"), Some(&Value::Float(150.0)));
    }

    #[test]
    fn stream_source_messages() {
        let mut graph = ActionGraphIR::new("msg_test", "MsgTest");
        graph.add_node(ActionNodeIR::StreamSource {
            id: "s".into(),
            value_expr: ExprIR::LiteralReal(1.0),
            target: "recv".into(),
            port_target: Some("dataPort".into()),
            emit_interval: 1,
        });

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();
        let result = runner.step(&ctx);

        assert_eq!(result.messages.len(), 1);
        // port_target is used as the effective target
        assert_eq!(result.messages[0].target, "dataPort");
        assert_eq!(result.messages[0].source_action, "s");
    }

    #[test]
    fn stream_source_without_port_target_uses_target() {
        let mut graph = ActionGraphIR::new("tgt_test", "TgtTest");
        graph.add_node(ActionNodeIR::StreamSource {
            id: "s".into(),
            value_expr: ExprIR::LiteralReal(1.0),
            target: "recv".into(),
            port_target: None,
            emit_interval: 1,
        });

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();
        let result = runner.step(&ctx);

        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.messages[0].target, "recv");
    }

    #[test]
    fn stream_reset() {
        let mut graph = ActionGraphIR::new("reset_test", "ResetTest");
        graph.add_node(ActionNodeIR::StreamSource {
            id: "s".into(),
            value_expr: ExprIR::LiteralReal(1.0),
            target: "out".into(),
            port_target: None,
            emit_interval: 1,
        });

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();
        runner.step(&ctx);
        runner.step(&ctx);
        assert_eq!(runner.stream_emissions("s"), 2);

        runner.reset();
        assert_eq!(runner.stream_emissions("s"), 0);
        assert_eq!(runner.stream_last_value("s"), None);
    }

    #[test]
    fn stream_source_only_action_does_not_complete() {
        // An action with ONLY a StreamSource (no other flow) should stay active
        let mut graph = ActionGraphIR::new("stream_only", "StreamOnly");
        graph.add_node(ActionNodeIR::StreamSource {
            id: "sensor".into(),
            value_expr: ExprIR::LiteralReal(5.0),
            target: "out".into(),
            port_target: None,
            emit_interval: 1,
        });

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        // Step several times -- should never auto-complete
        for _ in 0..10 {
            let r = runner.step(&ctx);
            assert!(!r.completed, "Stream-only action should not complete");
        }
        assert!(!runner.is_completed());
        assert_eq!(runner.stream_emissions("sensor"), 10);
    }

    #[test]
    fn stream_source_eval_failure_skips_emission() {
        // Stream with expression that references undefined variable
        let mut graph = ActionGraphIR::new("fail_test", "FailTest");
        graph.add_node(ActionNodeIR::StreamSource {
            id: "bad".into(),
            value_expr: ExprIR::FeatureRef("undefined_var".into()),
            target: "out".into(),
            port_target: None,
            emit_interval: 1,
        });

        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();

        let r = runner.step(&ctx);
        // Should not emit any messages when eval fails
        assert_eq!(r.messages.len(), 0);
        assert_eq!(runner.stream_emissions("bad"), 0);
        assert_eq!(runner.stream_last_value("bad"), None);
    }

    #[test]
    fn stream_query_nonexistent_node() {
        let graph = ActionGraphIR::new("test", "Test");
        let runner = ActionRunner::new(graph);

        assert_eq!(runner.stream_emissions("nonexistent"), 0);
        assert_eq!(runner.stream_last_value("nonexistent"), None);
    }

    #[test]
    fn with_sub_action_nonexistent_node_is_noop() {
        // with_sub_action on a non-existent node ID should not panic
        let sub = ActionGraphIR::new("sub", "Sub");
        let mut graph = ActionGraphIR::new("parent", "Parent");
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        graph.add_edge(&initial, &final_id);

        let graph = graph.with_sub_action("nonexistent", sub);

        // Should complete fine without any sub-action execution
        let mut runner = ActionRunner::new(graph);
        let ctx = EvalContext::new();
        let results = run_to_completion(&mut runner, &ctx);
        assert!(runner.is_completed());
        // No perform outputs expected
        let all_outputs: Vec<String> = results.iter().flat_map(|r| r.outputs.clone()).collect();
        assert!(
            !all_outputs.iter().any(|o| o.contains("perform")),
            "No perform should execute, got: {:?}",
            all_outputs
        );
    }

    // -----------------------------------------------------------------------
    // RSC-2.4c: Action executor slot cutover
    // -----------------------------------------------------------------------

    use crate::expressions::BinOp;
    use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};

    /// One slot per entry, named `name` (canonical == runtime), owned by
    /// `writer`.
    fn slot_store_with(entries: &[(&str, Variability, WriterId, Value)]) -> SlotStore {
        let mut store = SlotStore::new();
        for (name, variability, writer, init) in entries {
            store.intern(
                SlotMeta::new(
                    RuntimeId::top_level(ElementId::from_string(format!("decl:{name}"))),
                    *variability,
                    *writer,
                    *name,
                    *name,
                ),
                init.clone(),
            );
        }
        store
    }

    /// The three write-set classes are collected (recursively through
    /// inline sub-actions, order-preserving dedupe); loop variables,
    /// perform inputs and pure control/bookkeeping nodes are excluded.
    #[test]
    fn rsc24c_collect_write_targets_classes_and_exclusions() {
        let mut sub = ActionGraphIR::new("sub", "Sub");
        sub.add_node(ActionNodeIR::Assign {
            id: "subAssign".into(),
            target: "subX".into(),
            value: ExprIR::LiteralInt(1),
        });

        let mut graph = ActionGraphIR::new("g", "G");
        graph.add_node(ActionNodeIR::Assign {
            id: "a1".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(1),
        });
        // Duplicate target — deduped.
        graph.add_node(ActionNodeIR::Assign {
            id: "a2".into(),
            target: "x".into(),
            value: ExprIR::LiteralInt(2),
        });
        graph.add_node(ActionNodeIR::Accept {
            id: "acc".into(),
            source: None,
            payload_binding: "msg".into(),
            port_source: None,
        });
        // Empty payload binding — excluded.
        graph.add_node(ActionNodeIR::Accept {
            id: "acc2".into(),
            source: None,
            payload_binding: String::new(),
            port_source: None,
        });
        graph.add_node(ActionNodeIR::Perform {
            id: "p1".into(),
            action_ref: "Sub".into(),
            inputs: vec![("inParam".into(), ExprIR::LiteralInt(3))],
            output_binding: Some("out1".into()),
            sub_action: Some(Box::new(sub)),
        });
        // Loop bookkeeping: the loop variable is token-local iteration
        // state, not a write target.
        graph.add_node(ActionNodeIR::ForLoop {
            id: "f1".into(),
            variable: "i".into(),
            sequence: ExprIR::LiteralInt(0),
            body_entry: "a1".into(),
            exit_node: "a2".into(),
        });
        graph.add_node(ActionNodeIR::Send {
            id: "s1".into(),
            payload: ExprIR::LiteralInt(9),
            target: "elsewhere".into(),
            port_target: None,
        });

        assert_eq!(
            collect_write_targets(&graph),
            vec![
                "x".to_owned(),
                "msg".to_owned(),
                "out1".to_owned(),
                "subX".to_owned()
            ],
            "Assign + Accept-payload + Perform-output, recursive, deduped"
        );

        // Token-local names additionally carry loop vars, perform inputs
        // and declared parameters (binder locals).
        let mut with_param = graph.clone();
        with_param.parameters.push(ActionParameter {
            name: "declared".into(),
            direction: ParameterDirection::In,
            default_value: None,
        });
        let locals = collect_token_local_names(&with_param);
        for expected in ["x", "msg", "out1", "subX", "declared", "i", "inParam"] {
            assert!(
                locals.iter().any(|n| n == expected),
                "'{expected}' must be a token-local name, got {locals:?}"
            );
        }
    }

    /// The prepared write-set resolves routes with the shared
    /// single-writer mechanics, the slot seam reports handled — and
    /// publishes NOTHING (the legacy action writeback published nothing:
    /// both sync hooks are no-ops, writes are token-local by design).
    #[test]
    fn rsc24c_prepared_writeback_publishes_nothing_and_reports_coverage() {
        use crate::orchestrator::Executor;

        let mut graph = ActionGraphIR::new("g", "G");
        graph.add_node(ActionNodeIR::Assign {
            id: "a1".into(),
            target: "claimed".into(),
            value: ExprIR::LiteralInt(1),
        });
        graph.add_node(ActionNodeIR::Assign {
            id: "a2".into(),
            target: "unminted".into(),
            value: ExprIR::LiteralInt(2),
        });
        let mut runner = ActionRunner::new(graph);

        // No write-set prepared → not handled, legacy (no-op) seam runs.
        let mut shared = EvalContext::new();
        assert!(!Executor::sync_context_out_slots(
            &runner,
            &mut shared,
            crate::ode::SignalEvalMode::FreshState
        ));
        assert!(Executor::slot_write_fallbacks(&runner).is_empty());

        // `claimed` is minted for this executor; `unminted` is not.
        let store = slot_store_with(&[(
            "claimed",
            Variability::Discrete,
            WriterId::Executor(0),
            Value::Null,
        )]);
        Executor::prepare_slot_writeback(&mut runner, &store, None, None, WriterId::Executor(0));

        assert!(
            Executor::sync_context_out_slots(
                &runner,
                &mut shared,
                crate::ode::SignalEvalMode::FreshState
            ),
            "prepared write-set takes the slot seam"
        );
        assert!(
            shared.variables.is_empty(),
            "action writeback must publish nothing — token-local discipline \
             preserved byte-for-byte, got {:?}",
            shared.variables
        );
        assert_eq!(
            Executor::slot_write_fallbacks(&runner),
            vec!["unminted".to_owned()],
            "mint-coverage report lists targets without a claimed route"
        );
    }

    /// Accept-payload dynamics under the prepared write-set: the payload
    /// still lands in token bindings (message-dependent VALUE under a
    /// compile-static KEY — actions need no dynamic-key class, unlike SM
    /// port payload keys), and still never reaches the shared context.
    #[test]
    fn rsc24c_accept_payload_stays_token_local_with_write_set() {
        use crate::orchestrator::Executor;

        let mut graph = ActionGraphIR::new("g", "G");
        graph.add_node(ActionNodeIR::Accept {
            id: "acc".into(),
            source: None,
            payload_binding: "msg".into(),
            port_source: None,
        });
        let initial = graph.initial_node_id.clone();
        let final_id = graph.final_node_ids[0].clone();
        graph.add_edge(&initial, "acc");
        graph.add_edge("acc", &final_id);

        let mut runner = ActionRunner::new(graph);
        let store = slot_store_with(&[]);
        Executor::prepare_slot_writeback(&mut runner, &store, None, None, WriterId::Executor(0));

        let ctx = EvalContext::new();
        // Initial → Accept (blocks: no message yet).
        runner.step(&ctx);
        runner.step(&ctx);
        assert!(runner.is_blocked(), "accept must block without a message");

        runner.deliver_message(ActionMessage {
            target: "g".into(),
            payload: Value::Float(42.5),
            source_action: "sender".into(),
        });
        let result = runner.step(&ctx);
        assert!(
            result.outputs.iter().any(|o| o.contains("msg")),
            "payload binding must be traced, got {:?}",
            result.outputs
        );
        assert_eq!(
            runner.current_bindings().get("msg"),
            Some(&Value::Float(42.5)),
            "payload lands in token bindings"
        );

        let mut shared = EvalContext::new();
        assert!(Executor::sync_context_out_slots(
            &runner,
            &mut shared,
            crate::ode::SignalEvalMode::FreshState
        ));
        assert!(
            shared.variables.is_empty(),
            "payload never reaches the shared context"
        );
        assert_eq!(
            Executor::slot_write_fallbacks(&runner),
            vec!["msg".to_owned()],
            "the static payload key reports as unminted coverage"
        );
    }

    /// RSC-2.4c expression binding: model-attribute reads in retained node
    /// expressions bind to slots; token-local names are declared binder
    /// locals (not rewritten, not RS003 candidates); genuinely unresolved
    /// names stay in the full report but are cleared from the public one
    /// (action expressions surface their own eval diagnostics). Verdicts
    /// are unchanged — evaluation is context-name-first.
    #[test]
    fn rsc24c_bind_expression_slots_binds_reads_and_skips_token_locals() {
        use crate::orchestrator::Executor;

        let build = || {
            let mut graph = ActionGraphIR::new("g", "G");
            // Reads: `level` (slot-minted), `x` (token-local Assign
            // target), `ghost` (resolves nowhere).
            graph.add_node(ActionNodeIR::If {
                id: "if1".into(),
                condition: ExprIR::BinaryOp {
                    op: BinOp::GreaterThan,
                    left: Box::new(ExprIR::FeatureRef("level".into())),
                    right: Box::new(ExprIR::LiteralReal(0.5)),
                },
                then_branch: "a1".into(),
                else_branch: None,
            });
            graph.add_node(ActionNodeIR::Assign {
                id: "a1".into(),
                target: "x".into(),
                value: ExprIR::BinaryOp {
                    op: BinOp::Add,
                    left: Box::new(ExprIR::FeatureRef("level".into())),
                    right: Box::new(ExprIR::FeatureRef("ghost".into())),
                },
            });
            let initial = graph.initial_node_id.clone();
            let final_id = graph.final_node_ids[0].clone();
            graph.add_edge(&initial, "if1");
            graph.add_edge("if1", "a1");
            graph.add_edge("a1", &final_id);
            graph
        };

        let store = slot_store_with(&[(
            "level",
            Variability::Continuous,
            WriterId::Orchestrator,
            Value::Float(2.0),
        )]);

        let mut bound = ActionRunner::new(build());
        let public = Executor::bind_expression_slots(&mut bound, &store, None);
        assert_eq!(public.bound_refs, 2, "both `level` reads bind to the slot");
        assert!(
            public.unresolved.is_empty(),
            "public report clears unresolved, got {:?}",
            public.unresolved
        );
        assert_eq!(
            bound.bind_report().unresolved,
            vec!["ghost".to_owned()],
            "full report keeps the genuinely unresolved name"
        );

        // Verdict equivalence: bound and unbound runners produce identical
        // traces and bindings when the names resolve from the context.
        let mut unbound = ActionRunner::new(build());
        let mut ctx = EvalContext::new();
        ctx.set("level".to_owned(), Value::Float(2.0));
        ctx.set("ghost".to_owned(), Value::Float(1.0));
        for _ in 0..4 {
            let rb = bound.step(&ctx);
            let ru = unbound.step(&ctx);
            assert_eq!(rb.outputs, ru.outputs, "trace parity");
            assert_eq!(rb.completed, ru.completed);
        }
        assert!(bound.is_completed());
        assert_eq!(
            bound.final_bindings().get("x"),
            Some(&Value::Float(3.0)),
            "assign evaluated through the bound expressions"
        );
        assert_eq!(bound.final_bindings(), unbound.final_bindings());
    }
}
