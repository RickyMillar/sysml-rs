//! # Orchestrator
//!
//! Multi-subsystem execution engine for SysML v2. Coordinates state machines,
//! actions, flows, and constraints on a unified timeline with a shared context.
//!
//! ## Architecture
//!
//! The orchestrator is a **layer on top** of the existing single-subsystem runners.
//! It does not replace them — it holds multiple runners and coordinates their execution
//! through a tick loop:
//!
//! ```text
//! tick → advance clock
//!      → drain scheduled events
//!      → sync shared context → step each subsystem → sync context back
//!      → route messages through FlowRouter
//!      → evaluate constraints
//!      → record snapshot
//! ```
//!
//! ## Spec Foundation
//!
//! SysML v2 execution is constraint-based (KerML Performances/Occurrences):
//! - `HappensBefore` / `succession` enforce sequencing
//! - `MessageTransfer` + `SendPerformance` / `AcceptPerformance` for async messaging
//! - `StatePerformance` has entry→do→exit ordering with run-to-completion
//! - Time is discrete event ordering, not continuous

#![allow(clippy::indexing_slicing)]
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;

use sysml_core::{ElementId, Value};

/// Max number of located-crossing tick indices retained per ODE subsystem for
/// the step-size under-resolution advisory (P1 dt-under-resolution arc). A
/// bounded ring so a long run doesn't grow the bookkeeping unboundedly; the
/// advisory only ever needs a recent window to estimate the current cycle
/// length. Large enough to span several full oscillation cycles.
const STEP_SIZE_CROSSING_HISTORY: usize = 32;

// ── PROFILING SCAFFOLDING (env-gated, remove before commit) ──────────────
// Set SYSML_PHASE_TIMING=1 to accumulate per-phase wall time in `step()`.
// Read back with `take_phase_timings()`. Near-zero cost when off.
thread_local! {
    static PHASE_ON: std::cell::Cell<i8> = const { std::cell::Cell::new(-1) };
    static PHASE_ACC: std::cell::RefCell<Vec<(&'static str, u128)>> =
        const { std::cell::RefCell::new(Vec::new()) };
}
#[inline]
fn phase_on() -> bool {
    PHASE_ON.with(|c| {
        let v = c.get();
        if v < 0 {
            let on = std::env::var_os("SYSML_PHASE_TIMING").is_some();
            c.set(on as i8);
            on
        } else {
            v == 1
        }
    })
}
#[inline]
fn phase_mark(name: &'static str, last: &mut std::time::Instant) {
    if phase_on() {
        let now = std::time::Instant::now();
        let d = now.duration_since(*last).as_nanos();
        PHASE_ACC.with(|m| {
            let mut m = m.borrow_mut();
            if let Some(e) = m.iter_mut().find(|(n, _)| *n == name) {
                e.1 += d;
            } else {
                m.push((name, d));
            }
        });
        *last = now;
    }
}
/// PROFILING (remove): per-phase accumulated nanoseconds since process start.
pub fn take_phase_timings() -> Vec<(&'static str, u128)> {
    PHASE_ACC.with(|m| m.borrow().clone())
}

use crate::actions::ActionRunner;
use crate::clock::ClockRegistry;
use crate::cases::VerdictKind;
use crate::constraints::PrecompiledConstraintSet;
use crate::exchange::ExchangePlane;
use crate::expressions::{EvalContext, ExprIR};
use crate::flows::FlowMessage;
use crate::occurrence::{OccurrenceKind, OccurrenceTracker};
use crate::ode::Rk4Solver;
use crate::sequence::{SuccessionConstraint, SuccessionQueue};
use crate::statemachine::StateMachineRunner;

// ---------------------------------------------------------------------------
// Gated computed expressions
// ---------------------------------------------------------------------------

/// A computed expression with an optional gate variable.
///
/// When the gate is `None`, the expression is always evaluated.
/// When the gate is `Some(var_name)`, the gate variable is checked first:
/// - If the gate variable is truthy (true, non-zero float), evaluate normally
/// - If the gate variable is falsy (false, 0.0, or missing), set target to 0.0
///
/// This enables state-dependent flow gating: when a state machine transitions
/// to a "tripped" or "disconnected" state, it sets a gate variable to false,
/// which causes all gated flow expressions through that part to output zero.
/// Generic — works for breakers, valves, relays, clutches.
#[derive(Debug, Clone)]
pub struct GatedExpression {
    /// Target variable name in the shared context.
    pub target: String,
    /// Expression to evaluate.
    pub expr: ExprIR,
    /// Optional gate variable. When present and falsy, target is set to 0.0.
    pub gate: Option<String>,
    /// RSC-4.2 (C.4): instance scope prefix for an instance-multiplied
    /// computed expression (e.g. `"circuit1"`). `Some(prefix)` marks an
    /// expression whose RHS is authored in the instance's **local**
    /// namespace (bare `bimetalTemp`, `config.tripTemperature`, …): the
    /// compile-time binder resolves those names through
    /// [`SlotBinder::for_subsystem`](crate::expressions::SlotBinder::for_subsystem)
    /// so each `FeatureRef` binds to *this instance's* slot, and
    /// [`evaluate_computed_expressions`](Orchestrator::evaluate_computed_expressions)
    /// evaluates it against a scoped read view (empty `variables`, slot
    /// reader only) so the name-first evaluator MISSES the bare name and
    /// the `SlotRef` wins — collision-safe across instances WITHOUT
    /// text-prefixing the expression string (the deleted
    /// `prefix_expression_identifiers`). `None` = orchestrator-scope
    /// expression (aggregates, top-level `= expr` bindings) — global bind,
    /// evaluated against the master context.
    pub scope_prefix: Option<String>,
}

impl GatedExpression {
    /// Create an ungated, orchestrator-scope expression (always evaluates).
    pub fn new(target: impl Into<String>, expr: ExprIR) -> Self {
        Self {
            target: target.into(),
            expr,
            gate: None,
            scope_prefix: None,
        }
    }

    /// Create a gated, orchestrator-scope expression.
    pub fn gated(target: impl Into<String>, expr: ExprIR, gate: impl Into<String>) -> Self {
        Self {
            target: target.into(),
            expr,
            gate: Some(gate.into()),
            scope_prefix: None,
        }
    }

    /// RSC-4.2 (C.4): create an ungated, **instance-scoped** computed
    /// expression bound to `scope_prefix`'s local namespace. See
    /// [`scope_prefix`](Self::scope_prefix).
    pub fn instance_scoped(
        target: impl Into<String>,
        expr: ExprIR,
        scope_prefix: impl Into<String>,
    ) -> Self {
        Self {
            target: target.into(),
            expr,
            gate: None,
            scope_prefix: Some(scope_prefix.into()),
        }
    }

    /// Check whether the gate allows evaluation.
    fn is_gate_open(&self, ctx: &EvalContext) -> bool {
        match &self.gate {
            None => true,
            Some(gate_var) => match ctx.get(gate_var) {
                Some(Value::Bool(b)) => *b,
                Some(Value::Float(f)) => *f != 0.0,
                Some(Value::Int(i)) => *i != 0,
                None => true, // Missing gate var = open (fail-open)
                _ => true,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// Snapshot observer
// ---------------------------------------------------------------------------

/// Push-notify hook invoked by [`Orchestrator::step`] after each snapshot is
/// produced. The service layer installs one to forward snapshots into a
/// broadcast channel, replacing the 33 ms WebSocket poll loop with lock-step
/// observation.
///
/// Kept as a plain `Fn` trait object so the runtime stays tokio-free — the
/// async machinery lives in `sysml-service`.
///
/// ## Clone semantics
///
/// Observers are **not** propagated across `Orchestrator::fork`. Cloning a
/// `SnapshotObserver` yields an empty slot; the service layer reinstalls a
/// fresh observer on the forked child so the child's ticks flow into its
/// own broadcast channel, not the parent's.
#[derive(Default)]
pub struct SnapshotObserver(Option<Arc<dyn Fn(&ExecutionSnapshot) + Send + Sync>>);

impl SnapshotObserver {
    /// Install an observer. Replaces any previous one.
    pub fn set(&mut self, observer: Arc<dyn Fn(&ExecutionSnapshot) + Send + Sync>) {
        self.0 = Some(observer);
    }

    /// Remove the currently installed observer.
    pub fn clear(&mut self) {
        self.0 = None;
    }

    /// Whether an observer is installed. OPT #3: `run_to_completion` keys its
    /// full-vs-light tick cadence on this — an observed run must build a full
    /// snapshot every tick (the streaming layer is owed every frame).
    pub fn is_set(&self) -> bool {
        self.0.is_some()
    }

    /// Invoke the observer if one is installed.
    fn notify(&self, snapshot: &ExecutionSnapshot) {
        if let Some(observer) = &self.0 {
            observer(snapshot);
        }
    }
}

impl fmt::Debug for SnapshotObserver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SnapshotObserver")
            .field("installed", &self.0.is_some())
            .finish()
    }
}

impl Clone for SnapshotObserver {
    fn clone(&self) -> Self {
        // Observers are never copied across forks — see type docs.
        SnapshotObserver(None)
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Controls the ordering of message routing vs subsystem stepping in the tick loop.
///
/// - `StepFirst` (default): Step subsystems, then route messages. This is the
///   original behavior — messages sent this tick are delivered next tick.
/// - `RouteFirst`: Route pending messages first, then step subsystems. This
///   allows port message triggers to fire in the same tick as delivery.
///   Required for Phase 12 reactive port simulation (ADR-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TickStrategy {
    /// Step subsystems first, then route messages (original behavior).
    #[default]
    StepFirst,
    /// Route messages first, then step subsystems (for port triggers).
    RouteFirst,
}

/// Configuration for the orchestrator.
#[derive(Debug, Clone)]
pub struct OrchestratorConfig {
    /// Time step per tick in milliseconds.
    pub dt_ms: f64,
    /// Maximum number of ticks (safety limit).
    pub max_ticks: u64,
    /// Maximum simulation time in milliseconds (safety limit).
    pub max_time_ms: f64,
    /// Tick loop ordering strategy.
    pub tick_strategy: TickStrategy,
    /// Snapshot recording interval: 1 = every tick (default), N = every Nth tick.
    /// Reduces memory usage for long simulations with many subsystems.
    pub snapshot_interval: u64,
    /// Maximum trace length (ring buffer). `None` = unlimited (default).
    /// When set, oldest snapshots are evicted when the trace exceeds this length.
    pub max_trace_length: Option<usize>,
    /// Maximum number of completed occurrences retained by the live
    /// [`OccurrenceTracker`] (ring buffer). Oldest entries are evicted
    /// when `completed` exceeds this length.
    ///
    /// `OccurrenceTracker::completed` used to grow unbounded (~180 KB/tick
    /// on large multi-subsystem workloads) and was deep-cloned into
    /// every rewind-archive slot, which OOM-crashed the host. This caps
    /// the parent session's live tracker the same way `max_trace_length`
    /// caps the execution trace; archive slots additionally strip their
    /// occurrence history entirely (see
    /// [`Orchestrator::fork_for_archive`]).
    pub max_occurrence_history: usize,
    /// Convergence iteration limit per tick. 0 = disabled (default, single-pass).
    /// When > 0, after all subsystems step, the orchestrator checks if any context
    /// variable changed and re-steps up to this many iterations until convergence.
    pub convergence_max_iterations: u32,
    /// Relative change threshold for convergence. Iteration stops when the maximum
    /// relative change across all float variables is below this threshold.
    pub convergence_epsilon: f64,
    /// Whether FlowRouter capacity loss is tolerated (RSC-1.5 / G14).
    ///
    /// `false` (default, fail-hard): the first tick on which the router
    /// evicts pending messages at capacity records a runtime error
    /// ([`Orchestrator::flow_error`]) and `step_until` /
    /// `run_to_completion` stop stepping. `true` (explicit opt-in lossy
    /// mode): the run continues; loss stays visible through
    /// [`ExecutionSnapshot::flow_drop_warnings`] and the router's drop
    /// counters. Loss is never silent in either mode.
    pub allow_lossy_flows: bool,
}

impl Default for OrchestratorConfig {
    fn default() -> Self {
        OrchestratorConfig {
            dt_ms: 1.0,
            max_ticks: 10_000,
            max_time_ms: 60_000.0,
            tick_strategy: TickStrategy::StepFirst,
            snapshot_interval: 1,
            // Ring-buffer cap: on large workspaces each ExecutionSnapshot
            // carries a full variable HashMap (~1.4 MB for a 14k-var
            // elaborated context). Unbounded growth OOM-killed the
            // backend during interactive Run sessions — 10 Hz × 60 s
            // × 1.4 MB ≈ 840 MB/minute. 512 ticks caps the trace at
            // ~720 MB worst case (seconds-scale history window), which
            // is enough for `sessions.diff_timeline` over typical
            // interactive ranges and for archive-on-stop. Callers that
            // want the full trace (batch runs, long analyses) override
            // explicitly via OrchestratorConfig.
            max_trace_length: Some(512),
            // Mirrors `max_trace_length`'s memory-budget rationale above,
            // applied to `OccurrenceTracker::completed` (see doc comment
            // on the field for the measured leak).
            max_occurrence_history: 512,
            convergence_max_iterations: 0,
            convergence_epsilon: 1e-6,
            // Fail-hard default: silent message loss is forbidden
            // (RSC-1.5). Lossy flows are an explicit opt-in.
            allow_lossy_flows: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Subsystem types
// ---------------------------------------------------------------------------

/// Unique identity for a subsystem, captured at registration (RSC-4.2 L40).
///
/// A typed view over the same index space as [`WriterId::Executor`](crate::slots::WriterId::Executor)
/// — not a new identity plane (rsc-4.0-scheduler.md ruling #2 stands). Every
/// `Orchestrator::add_*` registration API returns the `SubsystemIndex` the new
/// subsystem was given in [`Orchestrator::subsystems`]; callers carry it
/// forward and never re-derive it by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct SubsystemIndex(pub(crate) u16);

impl SubsystemIndex {
    /// The raw index into `Orchestrator::subsystems`.
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// A named subsystem instance within the orchestrator.
pub struct Subsystem {
    /// Display name (e.g., "circuit1.ThermalProtectionModel")
    pub name: String,
    /// The actual executor (trait object).
    pub executor: Box<dyn Executor>,
    /// Optional variable prefix for instance scoping.
    ///
    /// When set (e.g., `"circuit1"`), the orchestrator creates a scoped view
    /// of the shared context for this subsystem:
    /// - `sync_context_in`: `circuit1.bimetalTemp` is aliased as `bimetalTemp`
    /// - `sync_context_out`: `bimetalTemp` is written back as `circuit1.bimetalTemp`
    ///
    /// This enables instance multiplication: the same ODE/SM definition runs
    /// independently for each instance with isolated variable namespaces.
    pub var_prefix: Option<String>,
    /// The `ElementId` of the source model element that this subsystem was
    /// compiled from (e.g., the `StateDefinition` or ODE owner). Used by the
    /// topology response so the frontend can map overlay data to diagram nodes
    /// without a separate lookup (ADR-006).
    pub source_element_id: Option<ElementId>,
    /// Canonical full-tree-path prefix for this subsystem's state
    /// variables (e.g., `"Panel.circuit5.thermalModel"`). When
    /// set, writeback in `sync_context_out` ALSO emits every state var
    /// under `{canonical_prefix}.{var}` — aliasing alongside the
    /// regular `{var_prefix}.{var}` key so frontend consumers keying
    /// off the tree `ownerPath` (which reflects the full containment
    /// chain: container → instance → sub-part → var) find their live
    /// value instead of falling back to the compiler's static initial
    /// binding under the bare name.
    ///
    /// `None` → no canonical aliasing (back-compat for subsystems
    /// added via the pre-canonical `add_*_prefixed` APIs).
    pub canonical_prefix: Option<String>,
}

impl Clone for Subsystem {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            executor: self.executor.clone_boxed(),
            var_prefix: self.var_prefix.clone(),
            canonical_prefix: self.canonical_prefix.clone(),
            source_element_id: self.source_element_id.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Executor trait (Phase 3 — unified subsystem dispatch)
// ---------------------------------------------------------------------------

/// Execution phase — determines ordering within the orchestrator tick loop.
///
/// `Physics` executors step **first** (conservation constraints, effort equalities),
/// then `ContinuousDynamics` (ODE solvers, so `when()` guards on state
/// machines see updated continuous state). All others step in declaration order after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutionPhase {
    /// Physics domain solvers — step before ODE and state machines.
    /// Applies conservation constraints (KCL, mass balance, etc.) and
    /// effort equalities from the connection graph topology.
    Physics,
    /// Continuous-time ODE solvers — step before state machines.
    ContinuousDynamics,
    /// Discrete-time state-space solvers.
    DiscreteDynamics,
    /// Event-driven state machines.
    StateMachine,
    /// Token-flow action executors.
    Action,
}

/// Read-only input bag for a single executor tick.
pub struct TickContext<'a> {
    /// Current simulation time in **seconds**.
    pub t: f64,
    /// Time step in **seconds**.
    pub dt: f64,
    /// Current tick number.
    pub tick: u64,
    /// Shared evaluation context (read-only during step).
    pub context: &'a EvalContext,
    /// Effective event for this subsystem (if any).
    pub event: Option<&'a str>,
    /// Port event payloads for this subsystem: `(port_name, payload)`.
    pub port_payloads: &'a [(String, Value)],
    /// Local clock time override (if a local clock is registered for this subsystem).
    pub local_clock_time: Option<f64>,
}

/// Output from a single executor tick.
pub struct TickOutput {
    /// Current state label (state name, node ID, or formatted primary value).
    pub current_state: String,
    /// Whether this executor has completed.
    pub completed: bool,
    /// Available transitions (only meaningful for state machines).
    pub available_transitions: Vec<(String, String)>,
    /// Trace outputs for logging/debugging.
    pub outputs: Vec<String>,
    /// Events to send via FlowRouter (state machine sends).
    pub sends: Vec<String>,
    /// Port-addressed payload sends: `(port_name, payload)`. The orchestrator
    /// resolves the sender's owner key and routes each as an addressed
    /// MessageTransfer (`{owner}.{port}`) carrying the real evaluated payload —
    /// the Value-carrying replacement for the `send via <port>` trace strings.
    pub port_sends: Vec<(String, Value)>,
    /// Messages to route: `(source_key, payload)` for action messages.
    pub messages: Vec<(String, Value)>,
    /// RSC-3.3c D4 — occurrence-addressed messages:
    /// `(source_key, named_target, payload)`. Produced by action send nodes
    /// (the Send node's `port_target`/`target` names the receiver) and
    /// routed via [`ExchangePlane::send_message`](crate::exchange::ExchangePlane::send_message): a declared flow on the
    /// source still wins; otherwise the named target resolves against the
    /// registered accepting surfaces (fail-loud on ambiguity, strict-loss
    /// on zero candidates).
    pub addressed_messages: Vec<(String, String, Value)>,
    /// The triggering event that drove entry into `current_state` this tick,
    /// when a state machine fired a *triggered* (message) transition; `None`
    /// otherwise. Forwarded to `SubsystemState.incoming_transition_trigger`
    /// (mirrors `StatePerformances.kerml:48` `incomingTransitionTrigger`).
    pub incoming_trigger: Option<String>,
}

impl TickOutput {
    /// Create a minimal output for solvers that just report state.
    pub fn solver(state_label: String, outputs: Vec<String>) -> Self {
        Self {
            current_state: state_label,
            completed: false,
            available_transitions: Vec::new(),
            outputs,
            sends: Vec::new(),
            port_sends: Vec::new(),
            messages: Vec::new(),
            addressed_messages: Vec::new(),
            incoming_trigger: None,
        }
    }
}

/// Unified subsystem executor trait for the orchestrator.
///
/// Implementations wrap domain-specific runners (state machines, ODE solvers,
/// action runners, etc.) and present them through a common interface. The
/// orchestrator dispatches via `Box<dyn Executor>` instead of enum matching.
///
/// # Object Safety
///
/// This trait is object-safe. All methods take `&self` or `&mut self`, no
/// associated types, no generics, no `Self` in return position.
pub trait Executor: Send + Sync {
    /// Which execution phase this executor belongs to.
    fn phase(&self) -> ExecutionPhase;

    /// Human-readable kind label (e.g. `"stateMachine"`, `"ode"`, `"action"`).
    fn kind_label(&self) -> &'static str;

    /// Execute one tick step.
    fn tick(&mut self, ctx: &TickContext<'_>) -> TickOutput;

    /// Reset to initial state.
    fn reset_executor(&mut self);

    /// Whether this executor has completed its work.
    fn is_completed(&self) -> bool;

    /// Deep-clone this executor into a new `Box<dyn Executor>`.
    ///
    /// Required by [`Orchestrator::fork`] so every subsystem can be duplicated
    /// as part of an independent session. The clone must share no mutable
    /// state with `self` — stepping one must not affect the other.
    ///
    /// # Implementation notes
    ///
    /// - **Stateless shared resources** (e.g. the RHS closure in `Rk4Solver`)
    ///   may be wrapped in `Arc<dyn Fn + Send + Sync>` and shared across
    ///   clones — there is nothing to deep-copy because the closure holds
    ///   no mutable state.
    /// - **Mutable shared state** (e.g. `Mutex<HashMap>` caches) must be
    ///   locked, the contents cloned, and re-wrapped in a new mutex so
    ///   the two executors cannot observe each other's writes. Handle
    ///   poisoned locks with `PoisonError::into_inner()` rather than
    ///   silently returning a default — losing state on poisoning can
    ///   produce spurious behaviour on the first post-fork tick.
    /// - **Recursive executors** (e.g. `StateMachineRunner` with nested
    ///   `sub_runners`) must deep-clone their children, not share them.
    /// - **Infallible**: this method cannot fail. If a clone cannot be
    ///   produced meaningfully, the executor should panic with a clear
    ///   message rather than return a degenerate placeholder.
    /// - **Not concurrent with `tick`**: the orchestrator only calls
    ///   `clone_boxed` from `Orchestrator::fork`, which takes `&self`
    ///   and does not interleave with `tick` (`&mut self`). Impls may
    ///   assume exclusive access for the duration of the clone.
    fn clone_boxed(&self) -> Box<dyn Executor>;

    // -- Context synchronization (defaults to no-op) --

    /// WS-A2 (hybrid event location): rewrite the transition at `index` from a
    /// tick-sampled `when` trigger to a located zero-crossing trigger that fires
    /// on the named injected crossing event. Returns `true` when a matching
    /// `when`-triggered transition was found and rewritten. Default `false`
    /// (non-state-machine executors have no transitions); overridden by the
    /// state-machine runner.
    fn rewrite_when_to_located(&mut self, _index: usize, _event_name: &str) -> bool {
        false
    }

    /// Merge shared context INTO this executor's internal context (pre-step).
    fn sync_context_in(&mut self, _shared: &EvalContext) {}

    /// RSC-2.4a/b/c/d: write this executor's state changes back through its
    /// precomputed slot write-set (`EvalContext::set_slot` mirrors both name
    /// spellings into the legacy map). Returns `false` when no write-set is
    /// installed — the caller then runs the legacy
    /// [`sync_context_out`](Self::sync_context_out) path. All four compiled
    /// kinds are migrated: ODE (2.4a), SM (2.4b), Action (2.4c — a
    /// publish-nothing seam by construction, see `ActionRunner`), Physics
    /// (2.4d — write-set restriction + short-alias plane; slot routing
    /// itself deferred to Phase 3, see `PhysicsExecutor`).
    ///
    /// RSC-4.2: `mode` selects the context an ODE executor evaluates its
    /// signal expressions against — [`FreshState`](crate::ode::SignalEvalMode::FreshState)
    /// for the main per-phase loop (pre-coupling), [`FullAccumulated`]
    /// (crate::ode::SignalEvalMode::FullAccumulated) for the convergence loop
    /// (post-coupling fixpoint). Executors that publish stored values rather
    /// than re-evaluating signals (SM, physics, action) ignore it.
    fn sync_context_out_slots(
        &self,
        _shared: &mut EvalContext,
        _mode: crate::ode::SignalEvalMode,
    ) -> bool {
        false
    }

    /// RSC-2.4b/c/d: writeback keys this executor still publishes through
    /// the name-keyed fallback instead of a claimed slot route (unminted or
    /// spelling-mismatched targets, runtime-dynamic keys). For action
    /// executors the list is a mint-coverage report (nothing is actually
    /// published — see `ActionRunner`); for physics it is the full write
    /// universe plus minted short aliases (no physics slots exist until
    /// Phase 3). Default empty (unmigrated kinds / fully-routed
    /// write-sets). Surfaced per subsystem via
    /// `Orchestrator::sm_slot_fallbacks` / `action_slot_fallbacks` /
    /// `physics_slot_fallbacks`.
    fn slot_write_fallbacks(&self) -> Vec<String> {
        Vec::new()
    }

    /// A2 / RS005: the write-set entries this executor publishes through the
    /// **strict** [`WriteRoute::apply`](crate::slots::WriteRoute::apply) path
    /// (state vectors, ODE signal outputs, SM assignment targets, hybrid
    /// continuous state) whose route failed to mint a slot. These are the exact
    /// keys `apply` would silently drop in a release build (the unrouted branch
    /// is a `debug_assert` + tracing-gated warn, so in production it is neither a
    /// panic nor a log) — a principle-1 mint gap. The compiler's **RS005** gate
    /// (`ModelCompiler::rs005_diagnostics`) turns any non-empty result into a
    /// hard build error, so the gap is caught once at compile rather than
    /// dropped on every tick forever.
    ///
    /// This is deliberately NARROWER than [`slot_write_fallbacks`]
    /// (Self::slot_write_fallbacks): it EXCLUDES the name-keyed
    /// [`apply_name_keyed`](crate::slots::WriteRoute::apply_name_keyed) path
    /// (physics port/flow writes, SM port payloads — both L34-gated, ledger L31)
    /// and runtime-dynamic keys (`__clock_time`, port-payload bindings), which
    /// are not mint gaps. Default empty (kinds with no strict-apply write-set).
    fn unrouted_slot_writes(&self) -> Vec<String> {
        Vec::new()
    }

    /// RSC-2.4a: precompute this executor's slot write-set against the
    /// compile-minted slot table. Called by the orchestrator right after
    /// [`bind_expression_slots`](Self::bind_expression_slots) (the write-set
    /// depends on the bind outcome). `writer` is this executor's minted
    /// `WriterId` — the single-writer guard validates every write-set slot
    /// against it. Default no-op (unmigrated kinds).
    fn prepare_slot_writeback(
        &mut self,
        _store: &crate::slots::SlotStore,
        _var_prefix: Option<&str>,
        _canonical_prefix: Option<&str>,
        _writer: crate::slots::WriterId,
    ) {
    }

    /// RSC-2.4a: whether every read this executor performs at tick time is
    /// servable without the orchestrator's per-prefix scoped-context clone
    /// (slot-bound expressions + read-only slot handle). When `true`, the
    /// ODE pre-pass hands the executor a thin slot-read view instead of
    /// building/maintaining the scoped clone for its prefix.
    fn scoped_view_bypass(&self) -> bool {
        false
    }

    // -- Compile-time slot binding (RSC-2.3, defaults to no-op) --

    /// Rebind this executor's retained compiled expressions against the
    /// compile-minted slot table, in the executor's **subsystem-local**
    /// scope (`var_prefix` names the instance whose local names resolve to
    /// its own slots). Implementations that hold `ExprIR` captured by
    /// closures (ODE solvers) rebind their retained spec and rebuild the
    /// closures; state machines (RSC-2.4b) bind their compiled-once guard
    /// cache and persistent trigger expressions; executors that hold no
    /// expressions keep the default no-op.
    fn bind_expression_slots(
        &mut self,
        _store: &crate::slots::SlotStore,
        _var_prefix: Option<&str>,
    ) -> crate::expressions::BindReport {
        crate::expressions::BindReport::default()
    }

    // -- Read-set accessor (RSC-4.1, defaults to empty) --

    /// RSC-4.1: the set of slots this executor **reads** at tick time,
    /// resolved to [`SlotId`](crate::slots::SlotId)s from its already-bound IR
    /// — the `SlotRef` / `SlotChainHead` nodes
    /// [`bind_expression_slots`](Self::bind_expression_slots) produced (§9 Q2
    /// of `rsc-4.0-scheduler.md`: read-set = compiler-resolved slot-ids; we
    /// harvest the resolved slots rather than re-deriving from expression
    /// text).
    ///
    /// This is the read-side dual of the authoritative write-sets
    /// (`SlotMeta.writer = WriterId::Executor(i)`). The per-phase scheduler
    /// (RSC-4.1 DAG, Wave 2b) orders executor B after A within a phase when
    /// `A.write_set ∩ B.read_slots() ≠ ∅` — the intra-phase write→read
    /// (Gap-4) edges.
    ///
    /// Purely **additive introspection**: it changes NO execution behaviour
    /// and is never consulted on the tick hot path. The default is empty —
    /// correct for kinds whose tick-time reads are not slot-bound (token-local
    /// actions, the bond-graph physics exchange plane, closure-built discrete
    /// solvers). The returned vector is sorted ascending by slot index and
    /// de-duplicated for determinism.
    fn read_slots(&self) -> Vec<crate::slots::SlotId> {
        Vec::new()
    }

    // -- ODE-specific (for zero-crossing detection) --

    /// Get the current ODE state vector for zero-crossing detection.
    fn get_state_snapshot(&self) -> Option<Vec<f64>> {
        None
    }

    /// RSC-4.3 (time-accurate re-step): restore the continuous state vector to a
    /// previously captured snapshot (`y_start`), so the solver can re-integrate a
    /// sub-interval of the current tick from the pre-step state. Returns `true`
    /// if the executor holds re-settable continuous state matching `y`'s length
    /// (ODE solvers); `false` otherwise. NOT [`reset_executor`](Self::reset_executor)
    /// — that returns to *initial* conditions; this returns to an arbitrary
    /// mid-run point captured this tick (Q9: save-`y_start`, not true solver rollback).
    fn restore_state(&mut self, _y: &[f64]) -> bool {
        false
    }

    /// RSC-4.3: invalidate any cached first-stage derivative (FSAL `k1`) so the
    /// next step recomputes `f(t, y)`. Must be called after [`restore_state`](Self::restore_state)
    /// (the cached `k1` belongs to the abandoned trajectory) and after a mid-tick
    /// drive change (the RHS at the current point changed when the SM flipped
    /// `V_applied`). No-op for solvers without an FSAL cache.
    fn invalidate_step_cache(&mut self) {}

    /// RSC-4.3: integrate the continuous state from `t_start` to `t_target`
    /// within the current tick (a sub-interval re-step), reading the drive from
    /// `ctx`. Returns `true` if the executor performed the integration (ODE
    /// solvers); `false` for executors that do not integrate continuous state —
    /// a `false` on a re-step-eligible detector is a hard error (RS014), never a
    /// silent drop. Distinct from [`tick`](Self::tick), which integrates a full `dt`.
    fn integrate_interval(&mut self, _t_start: f64, _t_target: f64, _ctx: &EvalContext) -> bool {
        false
    }

    /// Return `(state_variable_name, dy/dt)` pairs for the most recent
    /// tick. ODE solvers return the `k1` they computed at the tick
    /// boundary; non-ODE executors leave this as the default empty
    /// vec. Used by the snapshot projection to populate
    /// `ExecutionSnapshot.derivatives` without a second RHS eval
    /// (closes GAP-ODE-002).
    fn current_derivatives(&self) -> Vec<(String, f64)> {
        Vec::new()
    }

    // -- SM-specific introspection (for causation analysis, guard diagnosis) --

    /// Current state name (for occurrence tracking).
    fn current_state_name(&self) -> &str {
        ""
    }

    /// Diagnose guard conditions at this tick.
    fn diagnose_guards(&self, _event: Option<&str>) -> Vec<crate::statemachine::GuardDiagnosis> {
        Vec::new()
    }

    /// Access the internal eval context (for causation analysis).
    fn eval_context(&self) -> Option<&EvalContext> {
        None
    }

    /// Access transition IR (for causation analysis).
    fn transitions(&self) -> Option<&[crate::TransitionIR]> {
        None
    }

    /// Number of deferred events currently queued (SM-specific, 0 for others).
    fn deferred_event_count(&self) -> usize {
        0
    }

    // -- Accept-surface introspection (RSC-3.3c, defaults to none) --

    /// All accept ports this executor can ever receive on (SM `PortMessage`
    /// trigger ports, RSC-3.3c D4). The orchestrator registers each as an
    /// accepting surface `"{subsystem}.{port}"` on the
    /// [`ExchangePlane`](crate::exchange::ExchangePlane) so occurrence-addressed
    /// MessageTransfers can resolve their named receiver. Default: none.
    fn accept_ports(&self) -> Vec<String> {
        Vec::new()
    }

    /// The accept ports armed in the executor's CURRENT state (RSC-3.3c
    /// U1). The orchestrator polls these each tick and pull-initiates
    /// parked pull-mode transfers for `"{subsystem}.{port}"` — the accept
    /// path of the labeled pull extension. Default: none.
    fn armed_accept_ports(&self) -> Vec<String> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Scheduled events
// ---------------------------------------------------------------------------

/// A timed event to be injected into a subsystem.
#[derive(Debug, Clone)]
pub struct ScheduledEvent {
    /// Target time in milliseconds.
    pub time_ms: f64,
    /// Name of the target subsystem.
    pub target_subsystem: String,
    /// Event name to inject.
    pub event: String,
}

// ---------------------------------------------------------------------------
// Execution snapshots
// ---------------------------------------------------------------------------

/// RSC-5.4 (D-5.0.7): measurement metadata for a snapshot variable, sourced
/// from its slot's [`MeasurementRef`](crate::slots::MeasurementRef) at snapshot
/// time. A slim, serializable view of the m_ref carrying only what the snapshot
/// sinks surface — `dimension` (present for every ISQ-typed slot) and `unit`
/// (`Some` only when an explicit `[unit]` was minted; `None` for type-only ISQ
/// slots). Scale/offset stay internal to the eval engine and are deliberately
/// not on the wire. `MeasurementRef` itself is not `Serialize` (scale/offset are
/// `f64` engine internals); this is the boundary projection.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ValueMeasurement {
    /// ISQ 7-exponent dimension vector of the slot.
    pub dimension: sysml_core::physics::DimensionVector,
    /// Canonical unit name (`"K"`, `"mA"`) when the slot's m_ref carries one;
    /// `None` for type-only ISQ slots (SI-base magnitude, no explicit unit).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub unit: Option<String>,
}

/// `skip_serializing_if` helper for the `Arc`-wrapped [`ExecutionSnapshot::value_units`]
/// map — `HashMap::is_empty` can't be named directly through the `Arc`.
#[cfg(feature = "serde")]
fn arc_value_units_is_empty(m: &Arc<HashMap<String, ValueMeasurement>>) -> bool {
    m.is_empty()
}

/// Lightweight outcome of an [`Orchestrator::advance`] tick (OPT #3). Carries
/// only the control-flow signals a driver needs to decide whether to keep
/// stepping — no snapshot allocation, no graph-walk. The full state for the
/// tick is still recorded (as a light snapshot) in the trace.
#[derive(Debug, Clone)]
pub struct TickResult {
    /// The tick number just completed.
    pub tick: u64,
    /// Simulation time after this tick, in milliseconds.
    pub time_ms: f64,
    /// Whether every subsystem reports completion.
    pub completed: bool,
    /// Fail-hard flow error that halts stepping, if one occurred.
    pub flow_error: Option<String>,
}

/// Snapshot of the entire system state at one tick.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ExecutionSnapshot {
    /// Tick number.
    pub tick: u64,
    /// Simulation time in milliseconds.
    pub time_ms: f64,
    /// State of each subsystem.
    pub subsystem_states: HashMap<String, SubsystemState>,
    /// Current shared variable bindings.
    ///
    /// Wrapped in an `Arc` so the orchestrator's per-tick snapshot — which
    /// is held by the trace (`max_trace_length` deep), the broadcast
    /// channel (`SNAPSHOT_BROADCAST_CAPACITY` deep), and the Snapshot
    /// observer clone — does not deep-copy the full variable map every
    /// tick. The underlying `EvalContext.variables` is already
    /// `Arc<HashMap>` (Phase D CoW); this field preserves the sharing all
    /// the way through the snapshot so 3× full-clones per tick collapse
    /// into 3× refcount bumps. Consumers (indexing, `.get(&key)`, `.iter()`,
    /// `.len()`, `.keys()`) continue to work unchanged via `Deref`.
    pub variables: Arc<HashMap<String, Value>>,
    /// Messages routed during this tick.
    pub messages: Vec<FlowMessage>,
    /// Constraint evaluation results.
    pub constraint_results: Vec<ConstraintEvalResult>,
    /// Assertion checkpoints evaluated at this tick (populated during scenario playback).
    pub assertion_checkpoints: Vec<AssertionCheckpoint>,
    /// Guard diagnoses for all state machine subsystems at this tick.
    pub guard_diagnoses: Vec<crate::statemachine::GuardDiagnosis>,
    /// Causation links: variable changes that affected guards in other subsystems.
    pub causation_links: Vec<CausationLink>,
    /// Whether all subsystems have completed.
    pub completed: bool,
    /// Port feature values at this tick (port_key → feature_name → value).
    /// Only populated when a `PortRegistry` is configured on the orchestrator.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub port_values: HashMap<String, HashMap<String, Value>>,
    /// Instantaneous `dy/dt` values for every ODE state variable in
    /// every ODE subsystem, keyed by state variable name. Populated
    /// from [`Executor::current_derivatives`] after each tick. Empty
    /// when no ODE subsystems are present. Closes GAP-ODE-002.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub derivatives: HashMap<String, f64>,
    /// Concrete values for variables whose raw `variables` binding is
    /// a `Value::Ref` sentinel that the orchestrator was able to
    /// resolve through the graph at capture time.
    ///
    /// The runtime stashes `Value::Ref(element_id)` into
    /// [`EvalContext`] for every feature that has no direct literal
    /// binding (see [`compiler::context_from_graph_with_options`]);
    /// the expression evaluator resolves those lazily on read. Snapshot
    /// consumers (UI projections, time-series bridges) don't invoke the
    /// evaluator, so before this map they saw only the raw Ref and had
    /// no way to surface the attribute's live value. Populated via
    /// [`expressions::resolve_ref_value`] during [`Orchestrator::capture_snapshot`].
    ///
    /// Keyed by the same name used in `variables` so
    /// [`snapshot_view::normalize_with`] can overlay the resolved value
    /// when projecting into `scalar_vars` / `string_vars`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub resolved_refs: HashMap<String, Value>,
    /// Human-readable warnings for messages the FlowRouter dropped at
    /// pending-queue capacity during this tick (RSC-1.5 / G14: message
    /// loss must be session-visible, never event-log-only). One entry per
    /// source endpoint, e.g.
    /// `"flow 'sensor.out': dropped 3 message(s) (pending queue at capacity 1000)"`.
    /// Empty on ticks without loss. In strict mode
    /// (`OrchestratorConfig::allow_lossy_flows == false`, the default) the
    /// same loss also sets [`Orchestrator::flow_error`] and halts
    /// `step_until` / `run_to_completion`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub flow_drop_warnings: Vec<String>,
    /// RSC-5.4 (D-5.0.7): per-variable measurement metadata (dimension +
    /// optional unit) for slot-backed variables, sourced from the slot store's
    /// immutable `m_ref`. Keyed by the **same name spellings as `variables`** —
    /// both the canonical and runtime forms are mirrored (matching the slot
    /// writeback), so snapshot sinks ([`snapshot_view::normalize_with`], the
    /// service JSON sink) join directly by variable key. Built once and
    /// `Arc`-shared into every snapshot (m_ref is immutable post-mint — no
    /// per-tick rebuild, mirroring the `variables` Arc-sharing). Empty when no
    /// slot carries an m_ref (today's default; byte-identical wire).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "arc_value_units_is_empty")
    )]
    pub value_units: Arc<HashMap<String, ValueMeasurement>>,
    /// Per-ODE-subsystem step-size (under-resolution) advisory, computed while
    /// stepping (P1 dt-under-resolution arc). Sibling to `guard_diagnoses`:
    /// purely observational, derived from the located-crossing tick
    /// bookkeeping, and NEVER coupled into `dt` selection — it tells the user
    /// when an observed oscillation is step-bound rather than physics-bound.
    /// One entry per ODE subsystem with a registered crossing detector; the
    /// advisory is [`StepSizeAdvisory::NotApplicable`] until that subsystem has
    /// produced enough located crossings to measure a cycle. Empty on light
    /// (`advance()`) ticks, exactly like `guard_diagnoses`.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Vec::is_empty")
    )]
    pub step_size_health: Vec<crate::step_size_advisory::SubsystemStepSizeHealth>,
}

pub use crate::SubsystemState;

/// Result of evaluating a single constraint.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct ConstraintEvalResult {
    /// Constraint name or ID.
    pub name: String,
    /// The verdict for this constraint on this tick.
    ///
    /// Four-valued per the standard library's `VerdictKind`
    /// (`VerificationCases.sysml`), **not** a boolean: a constraint whose
    /// parameters are unbound cannot be decided, and reporting it as
    /// `Fail` would claim the run checked the model and found it violated.
    /// Such a row is [`Inconclusive`](VerdictKind::Inconclusive). Derived
    /// in one place, [`EvaluationResult::verdict`].
    pub verdict: VerdictKind,
    /// Human-readable expression text.
    pub expression: Option<String>,
    /// Live scalar values for every identifier the constraint expression
    /// references, captured at eval time. Closes GAP-CONSTR-002. Empty
    /// when the constraint has no free variables or when no referenced
    /// value was f64-coercible.
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "HashMap::is_empty")
    )]
    pub operands: HashMap<String, f64>,
    /// `ElementId` of the constraint usage in the model graph.
    /// Forwarded straight from `ConstraintIR.owner_id` so projections
    /// (and ultimately the frontend) can build an id-keyed lookup —
    /// matching by short name silently collides across nested scopes.
    /// `None` when the constraint IR has no owner id (legacy path).
    #[cfg_attr(
        feature = "serde",
        serde(default, skip_serializing_if = "Option::is_none")
    )]
    pub element_id: Option<sysml_core::ElementId>,
}

/// A requirement assertion evaluated at a specific tick during scenario execution.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct AssertionCheckpoint {
    /// Tick at which this assertion was evaluated.
    pub tick: u64,
    /// Simulation time when evaluated.
    pub time_ms: f64,
    /// The requirement identifier.
    pub requirement_id: String,
    /// Human-readable requirement text, if available.
    pub requirement_text: Option<String>,
    /// Verdict for this evaluation.
    pub verdict: crate::cases::VerdictKind,
    /// Detailed message explaining the result.
    pub message: String,
    /// Names of variables this requirement references.
    pub referenced_variables: Vec<String>,
}

/// A causal link: a variable change in one subsystem affected guards in another.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
pub struct CausationLink {
    /// Tick at which this causation occurred.
    pub tick: u64,
    /// The variable that changed.
    pub variable: String,
    /// Value before this tick.
    pub old_value: Value,
    /// Value after this tick.
    pub new_value: Value,
    /// The subsystem that wrote the new value.
    pub writer_subsystem: String,
    /// Guards in other subsystems affected by this change.
    /// (subsystem_name, transition_description, newly_satisfied)
    pub affected_guards: Vec<(String, String, bool)>,
}

// ---------------------------------------------------------------------------
// Orchestrator
// ---------------------------------------------------------------------------

/// Multi-subsystem execution orchestrator.
///
/// Coordinates state machines, actions, flows, and constraints on a unified
/// timeline with a shared EvalContext.
///
/// `Clone` is implemented manually (below): the RSC-2.2 slot store must be
/// deep-copied and re-linked into the cloned context, which a derive cannot
/// express.
pub struct Orchestrator {
    /// Named subsystems.
    subsystems: Vec<Subsystem>,

    /// Shared variable context across all subsystems.
    pub context: EvalContext,

    /// Message routing between subsystems, backed by the interned-`LinkId`
    /// [`ExchangePlane`](crate::exchange::ExchangePlane) (the sole routing
    /// backend since RSC-3.5e.2 retired the legacy string-keyed `FlowRouter`).
    router: ExchangePlane,

    /// Pre-compiled constraints for live monitoring (optional).
    ///
    /// Held as `Arc` so that orchestrator clones (fork-at-tick,
    /// per-revision rebuilds) are cheap pointer bumps rather than
    /// deep clones of the full `PrecompiledConstraintSet`. Per
    /// ADR-011 §3 the set itself is salsa-cached upstream; the runtime
    /// borrows the same allocation across all orchestrators built
    /// against a given graph revision.
    constraints: Option<std::sync::Arc<PrecompiledConstraintSet>>,

    /// Discrete event schedule (timed event injection).
    event_schedule: Vec<ScheduledEvent>,

    /// Global tick counter.
    tick: u64,

    /// Simulation time in milliseconds.
    time_ms: f64,

    /// Execution trace for timeline/replay.
    trace: Vec<ExecutionSnapshot>,

    /// Configuration.
    config: OrchestratorConfig,

    /// Port events pending delivery to subsystems (for RouteFirst strategy).
    /// Maps subsystem name → list of (port_name, payload) tuples from FlowRouter delivery.
    port_events: HashMap<String, Vec<(String, Value)>>,

    /// GAP 2 (L23 sub-gap B): maps an owner-instance key — the part-usage name
    /// an `accept … via <port>` flow targets (`breaker` in the routed target
    /// key `breaker.tripIn`) — to the subsystem name(s) of the state machine(s)
    /// realising that owner's behaviour. Built at SM registration:
    /// instance-multiplied SMs key on the instance prefix (`breaker1` →
    /// `breaker1.Logic`); non-multiplied SMs key on the owning part-usage name
    /// (`breaker` → `Logic`), or on the SM name itself when the `state def` is
    /// package-level with no owning part usage (documented spec-silent
    /// identity). The `Vec` is a fan-out for a part owning ≥2 state defs.
    ///
    /// The receiver of a transfer is the part-usage Occurrence
    /// (`TransitionPerformances.kerml:43-46`, `Transfers.kerml:254-265`), NOT
    /// the `state def`; this map reconciles the two identities WITHOUT renaming
    /// subsystems — `subsystem.name` (and thus `subsystem_states` snapshots)
    /// stays byte-identical. Consumed by
    /// [`convert_deliveries_to_port_events`](Self::convert_deliveries_to_port_events).
    owner_to_subsystems: HashMap<String, Vec<String>>,

    /// First flow-capacity loss recorded in strict mode
    /// (`config.allow_lossy_flows == false`). Once set, `step_until` and
    /// `run_to_completion` stop stepping (RSC-1.5 fail-hard). Queryable
    /// via [`flow_error`](Self::flow_error); cleared by
    /// [`reset`](Self::reset).
    flow_error: Option<String>,

    /// Universal clock (per Clocks.kerml). Advances each tick and writes
    /// `__clock_time` into the shared `EvalContext`.
    universal_clock: crate::clock::Clock,

    /// Per-subsystem local clocks with independent rate multipliers (Feature 10.1).
    /// When the global clock advances by dt, each local clock advances by dt * rate.
    /// Subsystems without a registered local clock use the universal clock time.
    clock_registry: ClockRegistry,

    /// Zero-crossing detectors keyed by the ODE's [`SubsystemIndex`] (RSC-4.2
    /// L39 — previously keyed by non-unique display name, `orchestrator.rs`
    /// history). Each detector monitors event functions on the ODE state and
    /// injects discrete events into the paired `SubsystemIndex` (the target
    /// state machine captured at wire time, `.1`) when crossings are
    /// detected — never re-derived by "first StateMachine subsystem" or by
    /// name at tick time.
    crossing_detectors:
        HashMap<SubsystemIndex, (SubsystemIndex, crate::ode_events::ZeroCrossingDetector)>,

    /// RSC-4.3: ODE `SubsystemIndex`es whose located crossings are eligible for
    /// the time-accurate mid-tick re-step (roll back → integrate to `t_cross` →
    /// fire the target SM mid-tick → re-sync drive → continue to `t_end`). ONLY
    /// the `wire_when_crossings_for_pair` path marks eligibility: it carries a
    /// real per-transition target-SM binding (the `set_sm_located_trigger`
    /// pairing). The `wire_ssr_zero_crossing_events` path deliberately does NOT
    /// — a `ZeroCrossingEventDef` occurrence expresses no effect binding, so its
    /// target SM is only a positional guess; re-stepping through it would hand a
    /// synchronous mid-tick knife to a name-derived target (ledger L46). An ODE
    /// absent from this set takes the exact pre-RSC-4.3 post-step
    /// [`inject_event`](Self::inject_event) path, so the byte-identical gate for
    /// non-re-stepping models is STRUCTURAL, not numeric-degeneracy hope.
    restep_eligible_odes: std::collections::HashSet<SubsystemIndex>,

    /// Per-cycle duty-cycle asymmetry trackers keyed by the ODE's
    /// [`SubsystemIndex`] (RSC-4.2 L39; WS-D Stage 2, SPEC-SILENT). Each
    /// tracker consumes the comparator crossing events its paired detector
    /// produces and write-throughs to a compile-minted slot — the
    /// fault-detection duty observable. See [`crate::ode_events::DutyCycleTracker`],
    /// [`ModelCompiler::mint_slot_store`](crate::compiler::ModelCompiler::mint_slot_store)
    /// step 2c.
    duty_trackers: HashMap<SubsystemIndex, crate::ode_events::DutyCycleTracker>,

    /// Most recently located crossing events per ODE `SubsystemIndex`
    /// (RSC-4.2 S2 oracle support). Test/introspection sugar only — mirrors
    /// the `duty_trackers` pattern of surfacing detector-internal state that
    /// external observers (snapshots) don't otherwise carry, so a test can
    /// assert sub-tick location precision instead of only which tick a
    /// transition fired in (which is insensitive to the y_end resolution bug
    /// this arc fixes — see [`Self::last_crossing_time`]).
    last_crossing_events: HashMap<SubsystemIndex, Vec<crate::ode_events::CrossingEvent>>,

    /// Tick numbers at which a located zero-crossing fired, per ODE
    /// `SubsystemIndex` (P1 dt-under-resolution arc). Feeds the step-size
    /// under-resolution advisory: consecutive gaps between crossing ticks are
    /// half-cycles of an oscillation, so twice the mean gap is the observed
    /// cycle length in ticks. Purely observational bookkeeping — nothing on the
    /// stepping path reads it back; only the reporting-time
    /// `compute_step_size_health` helper consumes it. Bounded to the most
    /// recent [`STEP_SIZE_CROSSING_HISTORY`] ticks per subsystem so a long run
    /// doesn't grow it unbounded.
    step_size_crossing_ticks: HashMap<SubsystemIndex, std::collections::VecDeque<u64>>,

    /// Succession constraint queue for HappensBefore temporal ordering (Feature 10.2).
    /// Tracks pending successors and fires them when their delay has elapsed
    /// and guard (if any) is satisfied.
    succession_queue: SuccessionQueue,

    /// Occurrence lifecycle tracker (Feature 10.3).
    /// Records start/end of state executions with timing and captured context.
    occurrence_tracker: OccurrenceTracker,

    /// Computed expressions evaluated each tick after all subsystems step.
    /// Each entry is a `GatedExpression` with an optional gate variable.
    /// When gated and the gate is false/0.0, the target is set to 0.0
    /// instead of evaluating the expression (flow gating for trip/disconnect).
    computed_expressions: Vec<GatedExpression>,

    /// Port registry for runtime port value propagation (Phase 6).
    /// When present, port values are propagated through flow connections each tick.
    port_registry: Option<crate::flows::PortRegistry>,

    /// Flow gate configuration: maps SM subsystem name → list of gating state names.
    /// When an SM enters a gating state, the gate variable `{sm_name}.__flow_gate`
    /// is set to false, causing gated computed expressions to output 0.0.
    flow_gates: HashMap<String, Vec<String>>,

    /// Spec occurrence lifecycle registry (Phase 3).
    /// Shared across all evaluation contexts for `create`/`destroy`/`isDuring`/`addNew`/`addNewAt`.
    occurrence_registry:
        std::sync::Arc<std::sync::Mutex<sysml_core::occurrence::OccurrenceRegistry>>,

    /// Backward-causation recorder (R7.1). Append-only ring buffer of
    /// `CausationEvent`s that the UI can walk backwards from a failure
    /// event. Actual event recording is wired in follow-up work — today
    /// the recorder is created empty and `sysml.causation.trace` returns
    /// an empty chain for any session until the recording hooks land.
    causation: crate::causation::CausationRecorder,

    /// Optional push-notify hook fired after every snapshot is produced
    /// by [`step`](Orchestrator::step). The service layer installs one
    /// to forward snapshots into a broadcast channel; cleared on fork so
    /// the child doesn't inherit the parent's subscribers (see
    /// [`SnapshotObserver`] clone semantics).
    snapshot_observer: SnapshotObserver,

    /// Snapshot-scoped lazy cache of compiled expression IRs used by
    /// per-tick `Value::Ref` resolution in [`capture_snapshot`] via
    /// [`crate::expressions::resolve_ref_value_cached`]. Without this,
    /// every lazily-resolvable attribute re-walks its AST and re-compiles
    /// an [`ExprIR`] each tick, which dominates per-tick cost on
    /// workspaces with many expression-resolved attributes.
    ///
    /// Per ADR-011 §6 (Q-RT-7, S3.T14): the cache lives behind an
    /// `Arc<Mutex<...>>` so its lifetime can be tied to the elaborated
    /// graph revision rather than the orchestrator session. Production
    /// callers go through `Snapshot::build_workspace_orchestrator` +
    /// `SysmlService::workspace_ref_resolve_cache`, which install a
    /// salsa-cached Arc that every orchestrator built against the same
    /// revision shares (one populated cache, no per-session re-pay).
    /// `Orchestrator::new` and the test/synthetic-graph paths default to
    /// a fresh empty `Arc<Mutex<HashMap>>` per orchestrator.
    ref_resolve_cache: Arc<std::sync::Mutex<crate::expressions::RefResolveCache>>,

    /// Compile-minted typed slot table (RSC-2.1/2.2, ADR-017 D1/D2).
    ///
    /// Populated by `ModelCompiler::build_orchestrator` /
    /// `build_workspace_orchestrator` with one slot per compile-known
    /// variable (RuntimeId + variability + writer + both name forms).
    /// As of RSC-2.2 this is **live storage behind the legacy map**: the
    /// same handle is attached to [`context`](Self::context)`.slots`, so
    /// every `EvalContext::set` on the master context write-throughs to the
    /// bound slot (design doc D-2.0.6). Routing is unconditional as of
    /// RSC-3.5f.3 — the RSC-2.2 `SlotStore::enabled` rollback gate was
    /// deleted, so a slot store being attached is the only condition.
    /// Empty for orchestrators assembled without the compiler (tests,
    /// manual builds) — no handle is attached and routing never engages.
    ///
    /// Clone semantics: the manual [`Clone`] impl below deep-copies the
    /// store into a fresh handle and re-links the cloned context, so forks
    /// share no mutable slot state with the parent (the
    /// [`fork`](Self::fork) contract).
    slot_store: crate::slots::SharedSlotStore,

    /// RSC-2.3: aggregated outcome of the compile-time `bind_slots` pass
    /// over this orchestrator's expressions (computed/gated, constraints,
    /// per-subsystem ODE specs). Populated by
    /// [`bind_expression_slots`](Self::bind_expression_slots); default
    /// (all-zero) for orchestrators assembled without the compiler.
    slot_bind_report: crate::expressions::BindReport,

    /// RSC-2.5: per-scope unresolved names recorded by
    /// [`bind_expression_slots`](Self::bind_expression_slots) —
    /// `(scope label, names)` where the scope is either the orchestrator's
    /// own expression scope or a subsystem name. Feeds the compiler's
    /// `RS003 unresolved runtime name` hard error (the per-name list also
    /// lives flattened in [`slot_bind_report`](Self::slot_bind_report)).
    bind_unresolved_scopes: Vec<(String, Vec<String>)>,
    /// Non-fatal diagnostics surfaced by the compiler during the build
    /// (RS003 warnings for constraint-scope-only unresolved names — see
    /// [`RS003_CONSTRAINT_SCOPE`]). Carried on the orchestrator so
    /// transports can surface them next to the session.
    compile_warnings: Vec<sysml_span::Diagnostic>,

    /// RSC-3.1: the classified link graph (design doc D-3.0.1). Built by the
    /// compiler between `build_port_flow_resources` and the physics executor;
    /// stored here for inspection (`flow_inspect`) and for the per-class
    /// execution passes that land in RSC-3.2/3.3/3.4. The tick loop does NOT
    /// consume it in 3.1 — this is an additive, inert graph until the
    /// exchange-plane cutover. Empty for orchestrators assembled without the
    /// compiler (tests, manual builds).
    link_graph: crate::links::LinkGraph,

    /// RSC-3.2: the compiled SignalLink directed-propagation plan (design doc
    /// D-3.0.3). Built by `build_workspace_orchestrator` from the
    /// [`link_graph`](Self::link_graph) + slot table. Per tick, after the
    /// producing phase, `propagate_port_values` routes signal-classified links
    /// through this plan via `set_slot` (skipping them in the legacy string
    /// copy). Empty for orchestrators with no signal links / assembled without
    /// the compiler.
    signal_propagation: crate::links::SignalPropagation,

    /// RSC-4.1: the per-phase topological execution order (design doc
    /// D-4.1.1/D-4.1.2). Built at the end of [`bind_expression_slots`] from the
    /// subsystem write-sets (`WriterId::Executor` plane) + read-sets
    /// (`Executor::read_slots()`) + signal-link `dependency_edges()`; consulted
    /// by index at tick via [`ExecutionSchedule::remap`] so same-phase producers
    /// step before consumers. The identity permutation when no phase has
    /// intra-phase coupling (corpus-wide today), so tick order is byte-identical
    /// to `Vec` insertion order there.
    execution_schedule: crate::scheduler::ExecutionSchedule,

    /// RSC-5.4 (D-5.0.7): memoized per-variable measurement metadata, built
    /// lazily on the first snapshot from the slot store's immutable `m_ref`s and
    /// `Arc`-shared into every [`ExecutionSnapshot::value_units`]. `None` until
    /// first computed. The fork-cloned value stays valid — `m_ref` is immutable
    /// post-mint and `Clone` deep-copies identical m_refs.
    value_units_cache: Option<Arc<HashMap<String, ValueMeasurement>>>,

    /// RSC-3.5d (Piece B) empirical-deletion probe. When `false`, the phase-2
    /// non-signal string copy in `propagate_port_values` is skipped — exactly
    /// the deletion the design doc proposes (§7, D-3.0.3). Defaults to `true`;
    /// flipped to `false` only by the 3.5d parity tests to measure whether the
    /// copy is load-bearing for `port_values`. It IS load-bearing today
    /// (PowerBond→physics and MessageChannel→ExchangePlane delivery are
    /// 3.5e/3.5f, still held), so the deletion stays NO-GO and this flag is the
    /// in-tree proof artifact rather than an actual code deletion.
    #[cfg(test)]
    nonsignal_phase2_enabled: bool,
}

/// Scope label for constraint-set names in
/// [`Orchestrator::bind_unresolved_scopes`]. Names unresolved ONLY in this
/// scope are RS003 *warnings*, not compile errors (RSC-2.5): constraints
/// have pinned tick-time skip semantics for missing operands, and the
/// service verification path injects extra variables at evaluation time.
pub(crate) const RS003_CONSTRAINT_SCOPE: &str = "constraints (deferred eval)";

/// Error applying session overrides (RSC-2.5, design doc D-2.0.7).
///
/// Returned by [`Orchestrator::apply_overrides_with_aliases`]; surfaced by
/// the service layer on `sysml.sessions.step` / `sessions.inject` /
/// overrides amendment 2026-06-11).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum OverrideError {
    /// `RS002` — the override names neither a runtime slot alias
    /// (canonical or runtime spelling) nor an existing context variable.
    /// Pre-2.5 behaviour silently created the variable; that path is
    /// deleted (user-approved contract amendment, design doc §8 Q3).
    #[error(
        "RS002 unknown override target '{name}': resolves to neither a runtime \
         slot alias nor an existing context variable"
    )]
    UnknownTarget {
        /// The unresolvable override name.
        name: String,
    },
    /// The override targets a slot classified
    /// [`Variability::Constant`](crate::slots::Variability::Constant) —
    /// constants may not be written, not even out-of-band.
    #[error("override target '{name}' is a Constant slot and cannot be written")]
    ConstantSlot {
        /// The rejected override name.
        name: String,
    },
}

impl Clone for Orchestrator {
    fn clone(&self) -> Self {
        // Deep-copy the slot store into a fresh handle: `fork()` promises
        // parent and child share no mutable state, and the master
        // context's `slots` handle must point at the clone's own store —
        // a derived clone would alias the parent's RwLock (RSC-2.2).
        let slot_store: crate::slots::SharedSlotStore = Arc::new(std::sync::RwLock::new(
            self.slot_store
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
        ));
        // Deep-copy the occurrence registry into a fresh handle for the same
        // reason as `slot_store` above: `reset()` mutates the registry
        // in-place through the shared `Arc<Mutex<..>>` (`*reg =
        // OccurrenceRegistry::new()`, ~orchestrator.rs:3479). An `Arc::clone`
        // here would let a forked child's `reset()` wipe the parent's (and
        // every other sibling's) occurrence history — violating
        // session-backend-contract.md:249 ("parent and child share no
        // mutable state"). `OccurrenceRegistry` derives `Clone`
        // (sysml-core/src/occurrence.rs:105).
        let occurrence_registry = std::sync::Arc::new(std::sync::Mutex::new(
            self.occurrence_registry
                .lock()
                .unwrap_or_else(|p| p.into_inner())
                .clone(),
        ));
        let mut context = self.context.alias_live();
        if context.slots.is_some() {
            context.slots = Some(Arc::clone(&slot_store));
        }
        if context.slot_reader.is_some() {
            context.slot_reader = Some(Arc::clone(&slot_store));
        }
        if context.occurrence_registry.is_some() {
            // `alias_live()` Arc-clones the parent's registry handle into
            // the new context (expressions/mod.rs `alias_live`) — repoint it
            // at the freshly deep-cloned registry so the clone's EvalContext
            // doesn't still alias the parent's mutable registry.
            context.occurrence_registry = Some(std::sync::Arc::clone(&occurrence_registry));
        }
        Orchestrator {
            subsystems: self.subsystems.clone(),
            context,
            router: self.router.clone(),
            constraints: self.constraints.clone(),
            event_schedule: self.event_schedule.clone(),
            tick: self.tick,
            time_ms: self.time_ms,
            trace: self.trace.clone(),
            config: self.config.clone(),
            port_events: self.port_events.clone(),
            owner_to_subsystems: self.owner_to_subsystems.clone(),
            flow_error: self.flow_error.clone(),
            universal_clock: self.universal_clock.clone(),
            clock_registry: self.clock_registry.clone(),
            crossing_detectors: self.crossing_detectors.clone(),
            restep_eligible_odes: self.restep_eligible_odes.clone(),
            duty_trackers: self.duty_trackers.clone(),
            last_crossing_events: self.last_crossing_events.clone(),
            step_size_crossing_ticks: self.step_size_crossing_ticks.clone(),
            succession_queue: self.succession_queue.clone(),
            occurrence_tracker: self.occurrence_tracker.clone(),
            computed_expressions: self.computed_expressions.clone(),
            port_registry: self.port_registry.clone(),
            flow_gates: self.flow_gates.clone(),
            occurrence_registry,
            causation: self.causation.clone(),
            snapshot_observer: self.snapshot_observer.clone(),
            ref_resolve_cache: self.ref_resolve_cache.clone(),
            slot_store,
            slot_bind_report: self.slot_bind_report.clone(),
            bind_unresolved_scopes: self.bind_unresolved_scopes.clone(),
            compile_warnings: self.compile_warnings.clone(),
            link_graph: self.link_graph.clone(),
            signal_propagation: self.signal_propagation.clone(),
            execution_schedule: self.execution_schedule.clone(),
            value_units_cache: self.value_units_cache.clone(),
            #[cfg(test)]
            nonsignal_phase2_enabled: self.nonsignal_phase2_enabled,
        }
    }
}

/// RSC-2.4a: thin read view for slot-bound ODE executors — Arc-cloned
/// registries (graph for chain tails / `Value::Ref` resolution, occurrence +
/// frame registries) plus the read-only slot handle, but NO variable-map
/// walk/clone. Executors whose
/// expressions are fully slot-bound resolve every non-local name through
/// the slot store; their RHS-local bindings (RK4 stage values, `t`, signal
/// targets, template params/config) are set by the closures themselves.
fn build_slot_read_context(base: &EvalContext) -> EvalContext {
    let mut sc = EvalContext::new();
    sc.graph = base.graph.clone();
    sc.occurrence_registry = base.occurrence_registry.clone();
    sc.frame_registry = base.frame_registry.clone();
    sc.slot_reader = base
        .slot_reader
        .as_ref()
        .or(base.slots.as_ref())
        .map(Arc::clone);
    sc
}

impl Orchestrator {
    /// Create a new orchestrator with the given configuration.
    pub fn new(config: OrchestratorConfig) -> Self {
        let occurrence_registry = std::sync::Arc::new(std::sync::Mutex::new(
            sysml_core::occurrence::OccurrenceRegistry::new(),
        ));
        let mut context = EvalContext::new();
        context.occurrence_registry = Some(occurrence_registry.clone());
        // Captured before `config` is moved into the struct literal below.
        let occurrence_history_cap = config.max_occurrence_history;
        Orchestrator {
            subsystems: Vec::new(),
            context,
            // RSC-3.5e.2: the ExchangePlane is the sole routing backend.
            router: ExchangePlane::new(),
            constraints: None,
            event_schedule: Vec::new(),
            tick: 0,
            time_ms: 0.0,
            trace: Vec::new(),
            config,
            port_events: HashMap::new(),
            owner_to_subsystems: HashMap::new(),
            flow_error: None,
            universal_clock: crate::clock::Clock::new("universalClock"),
            clock_registry: ClockRegistry::new(),
            crossing_detectors: HashMap::new(),
            restep_eligible_odes: std::collections::HashSet::new(),
            duty_trackers: HashMap::new(),
            last_crossing_events: HashMap::new(),
            step_size_crossing_ticks: HashMap::new(),
            succession_queue: SuccessionQueue::new(),
            occurrence_tracker: OccurrenceTracker::with_capacity(occurrence_history_cap),
            computed_expressions: Vec::new(),
            port_registry: None,
            flow_gates: HashMap::new(),
            occurrence_registry,
            causation: crate::causation::CausationRecorder::default(),
            snapshot_observer: SnapshotObserver::default(),
            ref_resolve_cache: Arc::new(std::sync::Mutex::new(
                crate::expressions::RefResolveCache::new(),
            )),
            slot_store: Arc::new(std::sync::RwLock::new(crate::slots::SlotStore::new())),
            slot_bind_report: crate::expressions::BindReport::default(),
            bind_unresolved_scopes: Vec::new(),
            compile_warnings: Vec::new(),
            link_graph: crate::links::LinkGraph::new(),
            signal_propagation: crate::links::SignalPropagation::default(),
            execution_schedule: crate::scheduler::ExecutionSchedule::default(),
            value_units_cache: None,
            #[cfg(test)]
            nonsignal_phase2_enabled: true,
        }
    }

    /// RSC-3.1: install the classified link graph (called by the compiler).
    pub fn set_link_graph(&mut self, link_graph: crate::links::LinkGraph) {
        self.link_graph = link_graph;
    }

    /// RSC-3.2: install the compiled SignalLink propagation plan (called by
    /// the compiler after the slot table is live, design doc D-3.0.3).
    pub fn set_signal_propagation(&mut self, plan: crate::links::SignalPropagation) {
        self.signal_propagation = plan;
    }

    /// RSC-3.2: read access to the compiled SignalLink propagation plan. The
    /// Phase-4 scheduler consumes
    /// [`dependency_edges`](crate::links::SignalPropagation::dependency_edges)
    /// from this. Empty when assembled without the compiler / no signal links.
    pub fn signal_propagation(&self) -> &crate::links::SignalPropagation {
        &self.signal_propagation
    }

    /// RSC-4.1: the compiled per-phase topological execution schedule (design
    /// doc D-4.1.1). Exposed for the RSC-4.B0 read-set inventory gate so it
    /// validates the production scheduler's own edges/order.
    pub fn execution_schedule(&self) -> &crate::scheduler::ExecutionSchedule {
        &self.execution_schedule
    }

    /// RSC-3.5d (Piece B) test probe: toggle the phase-2 non-signal string
    /// copy in `propagate_port_values`. Test-only — the production path always
    /// runs the copy. Used by the 3.5d parity tests to measure empirically
    /// whether deleting the non-signal copy diverges `port_values`.
    #[cfg(test)]
    fn set_nonsignal_phase2_enabled(&mut self, enabled: bool) {
        self.nonsignal_phase2_enabled = enabled;
    }

    /// RSC-3.1: read access to the classified link graph (D-3.0.1). Empty
    /// when this orchestrator was assembled without the `ModelCompiler`
    /// build path. The tick loop does not consume it in Phase 3.1.
    pub fn link_graph(&self) -> &crate::links::LinkGraph {
        &self.link_graph
    }

    /// Read access to the compile-minted typed slot table (RSC-2.1/2.2).
    /// Empty when this orchestrator was assembled without the
    /// `ModelCompiler` build path.
    pub fn slot_store(&self) -> std::sync::RwLockReadGuard<'_, crate::slots::SlotStore> {
        self.slot_store.read().unwrap_or_else(|p| p.into_inner())
    }

    /// The shared slot-store handle (RSC-2.2). Same allocation the master
    /// context routes writes into; used by tests and (later phases) by
    /// executors holding slot read/write sets.
    pub fn slot_store_handle(&self) -> crate::slots::SharedSlotStore {
        Arc::clone(&self.slot_store)
    }

    /// Install the compile-minted slot table and switch on write-through
    /// routing (RSC-2.2, design doc D-2.0.6): the store is seeded from the
    /// context's current legacy map (coherence at attach time), wrapped in
    /// the shared handle, and attached to the master context so every
    /// subsequent `EvalContext::set` on it routes to the bound slot.
    pub(crate) fn set_slot_store(&mut self, mut slot_store: crate::slots::SlotStore) {
        slot_store.seed_from_map(&self.context.variables);
        let handle: crate::slots::SharedSlotStore = Arc::new(std::sync::RwLock::new(slot_store));
        self.context.slots = Some(Arc::clone(&handle));
        // RSC-2.3: read handle alongside the write handle, so contexts
        // derived from the master via `merge_from` / `build_slot_read_context`
        // inherit by-`SlotId` READ access (never the name-write routing).
        self.context.slot_reader = Some(Arc::clone(&handle));
        self.slot_store = handle;
    }

    /// Test-support (`#[doc(hidden)]`): mint a bare-name Continuous/Discrete
    /// slot per `(subsystem_index, var, initial)` owned by that subsystem's
    /// `WriterId::Executor(index)`, attach the store, and run the bind pass.
    ///
    /// Primitive for **graph-less** hand-built-IR orchestrator tests — ones
    /// that construct a `StateMachineRunner` (or similar) directly from a
    /// hand-rolled IR with no backing `ModelGraph`, so there is nothing for
    /// `ModelCompiler` to compile (`orchestrator_integration.rs`'s
    /// `test_full_brew_scenario`/`test_overheated_recovery`). A test that
    /// *does* have a real graph (a compiled `StateMachineIR` from
    /// `execution_snapshot`/`compile_state_machine`) should call
    /// `ModelCompiler::build_sm_orchestrator` instead (ledger L44) — that is
    /// now the production single-SM-no-ODE mint+bind entry point
    /// (`sysml.simulate.start` calls it), and the former L43 test fixture
    /// that used to call this method collapsed onto it. Hand-built
    /// orchestrators (raw `add_*`, no compiler) publish state ONLY through
    /// their slot-routed writeback since the string-identity cull deleted
    /// the legacy `sync_context_out` — this method exists so a graph-less
    /// test can still exercise that path. NOT a production API.
    #[doc(hidden)]
    pub fn mint_state_slots_for_test(&mut self, slots: &[(usize, &str, f64)]) {
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};

        let mut store = SlotStore::new();
        for (idx, var, initial) in slots {
            store.intern(
                SlotMeta::new(
                    RuntimeId::top_level(sysml_core::ElementId::from_string(format!(
                        "decl:{var}"
                    ))),
                    Variability::Discrete,
                    WriterId::Executor(*idx as u16),
                    *var,
                    *var,
                ),
                Value::Float(*initial),
            );
        }
        self.set_slot_store(store);
        self.bind_expression_slots(None);
    }

    /// RSC-2.3 (design doc D-2.0.4): run the compile-time `bind_slots` pass
    /// over every expression this orchestrator evaluates at tick time —
    /// computed/gated expressions and constraints in the **orchestrator
    /// scope** (canonical + runtime spellings via the store's alias table),
    /// and each subsystem's retained expressions (ODE derivative/signal
    /// specs) in the **subsystem-local scope** (the instance's local names
    /// resolve to that instance's slots via its `var_prefix`).
    ///
    /// Called by `ModelCompiler` immediately after
    /// [`set_slot_store`](Self::set_slot_store) (i.e. before any tick). The
    /// aggregated [`BindReport`](crate::expressions::BindReport) is kept on
    /// the orchestrator ([`slot_bind_report`](Self::slot_bind_report)); the
    /// per-scope unresolved names ([`bind_unresolved_scopes`]
    /// (Self::bind_unresolved_scopes)) feed the compiler's RS003 hard
    /// error (RSC-2.5). No-op when
    /// no slot table was minted or routing is disabled (the RSC-2.2
    /// rollback gate doubles as the binder skip flag — the design doc's
    /// rollback clause for this pass).
    pub(crate) fn bind_expression_slots(
        &mut self,
        graph: Option<&sysml_core::ModelGraph>,
    ) -> crate::expressions::BindReport {
        use crate::expressions::{bind_slots, BindReport, SlotBinder};

        let handle = Arc::clone(&self.slot_store);
        let store = handle.read().unwrap_or_else(|p| p.into_inner());
        if store.is_empty() {
            return BindReport::default();
        }

        let mut total = BindReport::default();
        let mut unresolved_scopes: Vec<(String, Vec<String>)> = Vec::new();

        // Orchestrator scope: computed/gated expressions evaluate against
        // the master context. (Gate variables stay name-resolved strings.)
        // Names already bound in the master context without being slots
        // (orchestrator-injected `__sf_*` waveforms, port keys, canonical
        // aliases) ARE resolvable at eval time — declare them so they are
        // not reported as RS003 candidates.
        let mut computed_report = BindReport::default();
        let global_locals: Vec<String> = self.context.variables.keys().cloned().collect();
        for gated in &mut self.computed_expressions {
            match &gated.scope_prefix {
                // Orchestrator-scope: resolve through the store's alias table
                // (canonical + runtime spellings). Master-context names that
                // aren't slots (`__sf_*`, port keys, canonical aliases) are
                // declared as locals so they aren't RS003 candidates.
                None => {
                    let global =
                        SlotBinder::global(&store).with_locals(global_locals.iter().cloned());
                    bind_slots(&mut gated.expr, &global, &mut computed_report);
                }
                // RSC-4.2 (C.4): instance-scope. The RHS is authored in the
                // instance's LOCAL namespace (bare `bimetalTemp`,
                // `config.tripTemperature`). `for_subsystem` maps every store
                // name `{prefix}.{local}` to its slot under the bare `local`
                // spelling, so each reference binds to THIS instance's slot —
                // exactly the slot the deleted text-prefix path bound the
                // `{prefix}.{local}` spelling to via the global binder.
                // Unresolved names are re-qualified with the prefix so the
                // RS003 hard-error path (graph-feature / master-context
                // exemptions) behaves byte-identically to the pre-cull
                // text-prefixed spelling.
                Some(prefix) => {
                    let binder = SlotBinder::for_subsystem(&store, Some(prefix));
                    let mut sub_report = BindReport::default();
                    bind_slots(&mut gated.expr, &binder, &mut sub_report);
                    for name in &mut sub_report.unresolved {
                        *name = format!("{prefix}.{name}");
                    }
                    computed_report.merge(sub_report);
                }
            }
        }
        if !computed_report.unresolved.is_empty() {
            unresolved_scopes.push((
                "orchestrator computed expressions".to_owned(),
                computed_report.unresolved.clone(),
            ));
        }
        total.merge(computed_report);

        // Constraint scope — SEPARATE from the computed scope because its
        // unresolved names are NOT compile-stoppers (RSC-2.5): constraints
        // have pinned tick-time skip semantics for missing operands, and
        // the service verification path re-evaluates them with injected
        // variables (`sim_time_ms`, `sim_completed`, analysis parameters —
        // see sysml-service evaluation.rs) that no orchestrator slot can
        // know about. The compiler downgrades this scope to RS003 warnings.
        // The set arrives as a shared (salsa-cached) Arc, so binding
        // rewrites a private copy — the upstream cache keeps serving
        // unbound IR to other consumers.
        // RSC-3.1 (D-3.0.6-B): each constraint binds with an OWNER-SCOPED
        // binder — the constraint owner's feature-tree attributes first, then
        // its ancestors', then the global store. This mirrors the eval-time
        // overlay `check_constraints`/`evaluate_constraints` apply (owner
        // children take precedence over global names), moved to compile
        // binding. Names that resolve to an owner-local slot become real
        // `SlotRef`s instead of constraint-scope RS003 warnings.
        let local_names: Vec<String> = self.context.variables.keys().cloned().collect();
        let mut constraint_report = BindReport::default();
        if let Some(set) = &self.constraints {
            let mut bound = (**set).clone();
            for tc in &mut bound.compiled {
                let owner_aliases = graph
                    .and_then(|g| tc.constraint.owner_id.as_ref().map(|oid| (g, oid)))
                    .map(|(g, oid)| crate::compiler::owner_scoped_slot_aliases(g, &store, oid))
                    .unwrap_or_default();
                let binder = SlotBinder::global(&store)
                    .with_locals(local_names.iter().cloned())
                    .with_owner_aliases(owner_aliases);
                bind_slots(&mut tc.expr_ir, &binder, &mut constraint_report);
            }
            self.constraints = Some(std::sync::Arc::new(bound));
        }
        if !constraint_report.unresolved.is_empty() {
            unresolved_scopes.push((
                RS003_CONSTRAINT_SCOPE.to_owned(),
                constraint_report.unresolved.clone(),
            ));
        }
        total.merge(constraint_report);

        // Subsystem scope: each executor rebinds its own retained
        // expressions against its instance-local namespace, then (RSC-2.4a)
        // precomputes its slot write-set — prepare depends on the bind
        // outcome (template omissions / bypass eligibility), so the order
        // matters. The index is the executor's minted `WriterId` — the same
        // `SubsystemIndex` the compiler captured at registration (RSC-4.2
        // L40), not re-derived from this vector by name.
        for (idx, sub) in self.subsystems.iter_mut().enumerate() {
            let report = sub
                .executor
                .bind_expression_slots(&store, sub.var_prefix.as_deref());
            if !report.unresolved.is_empty() {
                unresolved_scopes.push((sub.name.clone(), report.unresolved.clone()));
            }
            total.merge(report);
            sub.executor.prepare_slot_writeback(
                &store,
                sub.var_prefix.as_deref(),
                sub.canonical_prefix.as_deref(),
                crate::slots::WriterId::Executor(idx as u16),
            );
        }

        // RSC-4.1: compile the per-phase topological execution order now that
        // every subsystem's write-set (`WriterId::Executor` slots) and read-set
        // (`Executor::read_slots()`, bound in the loop above) are live. Consumed
        // by index at tick via `execution_schedule.remap_order()`.
        self.execution_schedule =
            crate::scheduler::ExecutionSchedule::build(&self.subsystems, &store);
        // RS011 within-phase-cycle diagnostics join the compile-warning surface
        // (unified with RS010; empty corpus-wide today at intra_phase_edges=0).
        for d in self.execution_schedule.diagnostics() {
            self.compile_warnings.push(d.clone());
        }

        drop(store);
        self.slot_bind_report = total.clone();
        self.bind_unresolved_scopes = unresolved_scopes;
        total
    }

    /// RSC-2.3: aggregated outcome of the compile-time slot-binding pass.
    /// All-zero when this orchestrator was assembled without the compiler.
    pub fn slot_bind_report(&self) -> &crate::expressions::BindReport {
        &self.slot_bind_report
    }

    /// Every prefixed subsystem that is NOT scoped-view-bypass-eligible —
    /// i.e. `var_prefix.is_some() && !scoped_view_bypass()`, across the
    /// non-`Physics` phases (`Physics` is stepped separately and never reads a
    /// scoped view). Since RSC-3.5f.3 deleted `build_scoped_context`, such a
    /// subsystem can no longer run: the compiler's **RS004** gate
    /// ([`ModelCompiler::rs004_diagnostics`](crate::compiler::ModelCompiler))
    /// turns a non-empty result into a hard compile error, and the tick loop's
    /// `unreachable!` guards the same condition. This is the read-side
    /// generalization of [`ode_scoped_fallbacks`](Self::ode_scoped_fallbacks)
    /// (which only saw `ContinuousDynamics`). Returned as `(name, phase)` so a
    /// non-empty result names the blocking executor kind. The corpus-wide
    /// `rsc36_bypass_eligibility_census` pins this empty.
    pub fn scoped_view_fallbacks(&self) -> Vec<(String, ExecutionPhase)> {
        self.subsystems
            .iter()
            .filter(|s| {
                s.var_prefix.is_some()
                    && !s.executor.scoped_view_bypass()
                    && s.executor.phase() != ExecutionPhase::Physics
            })
            .map(|s| (s.name.clone(), s.executor.phase()))
            .collect()
    }

    /// RSC-2.4a: prefixed ODE executors that still need the legacy
    /// scoped-context clone (reads not provably servable from slots —
    /// unresolved dynamic names, chains through instance config maps, or
    /// hand-built solvers without a retained spec). Empty when every
    /// prefixed continuous-dynamics executor runs on the bypass path.
    /// Observability hook for the narrow-fallback rule (design doc
    /// D-2.0.5); the RSC-2.5 scoped-cache deletion is gated on this
    /// shrinking to zero across the corpus. The `ContinuousDynamics`-filtered
    /// view of [`scoped_view_fallbacks`](Self::scoped_view_fallbacks).
    pub fn ode_scoped_fallbacks(&self) -> Vec<String> {
        self.scoped_view_fallbacks()
            .into_iter()
            .filter(|(_, phase)| *phase == ExecutionPhase::ContinuousDynamics)
            .map(|(name, _)| name)
            .collect()
    }

    /// RSC-2.4b: per state-machine subsystem, the writeback keys still
    /// published through the name-keyed fallback instead of a claimed slot
    /// route — compiled targets whose route was refused (unminted target,
    /// instance-scoped canonical-spelling mismatch, placeholder writer)
    /// plus runtime-dynamic keys (port payload bindings, local
    /// `__clock_time`). Mirror of [`ode_scoped_fallbacks`]
    /// (Self::ode_scoped_fallbacks) for the SM cutover; the RSC-2.5
    /// rationalization is gated on understanding every entry here.
    /// Subsystems with an empty fallback list are omitted.
    pub fn sm_slot_fallbacks(&self) -> Vec<(String, Vec<String>)> {
        self.subsystems
            .iter()
            .filter(|s| s.executor.phase() == ExecutionPhase::StateMachine)
            .filter_map(|s| {
                let fallbacks = s.executor.slot_write_fallbacks();
                if fallbacks.is_empty() {
                    None
                } else {
                    Some((s.name.clone(), fallbacks))
                }
            })
            .collect()
    }

    /// A2 / RS005: per subsystem, the strict-`apply` write-set entries that
    /// failed to mint a slot (`Executor::unrouted_slot_writes`) — the mint gaps
    /// that would silently drop in a release build. The compiler's
    /// [`rs005_diagnostics`](crate::compiler::ModelCompiler) turns any non-empty
    /// result into a hard build error, so the strict `apply` never has to
    /// enforce the "100% routed" invariant at tick time. Empty corpus-wide (the
    /// invariant the RS005 gate now pins at compile instead of the old runtime
    /// `debug_assert` + release silent-skip). Subsystems with an empty list are
    /// omitted.
    pub fn unrouted_slot_writes(&self) -> Vec<(String, Vec<String>)> {
        self.subsystems
            .iter()
            .filter_map(|s| {
                let unrouted = s.executor.unrouted_slot_writes();
                if unrouted.is_empty() {
                    None
                } else {
                    Some((s.name.clone(), unrouted))
                }
            })
            .collect()
    }

    /// RSC-2.4c: per action subsystem, the compiled write targets without
    /// a claimed slot route. For actions this is a **mint-coverage
    /// report**, not a live fallback path: the legacy action writeback
    /// published nothing (token-local bindings by design — see
    /// `ActionRunner`'s `sync_context_out_slots`), so no name-keyed
    /// writes occur either way. Until the compiler registers action
    /// subsystems and mints their write-target claims, every target
    /// reports here. Mirror of [`sm_slot_fallbacks`]
    /// (Self::sm_slot_fallbacks); subsystems with an empty list are
    /// omitted.
    pub fn action_slot_fallbacks(&self) -> Vec<(String, Vec<String>)> {
        self.subsystems
            .iter()
            .filter(|s| s.executor.phase() == ExecutionPhase::Action)
            .filter_map(|s| {
                let fallbacks = s.executor.slot_write_fallbacks();
                if fallbacks.is_empty() {
                    None
                } else {
                    Some((s.name.clone(), fallbacks))
                }
            })
            .collect()
    }

    /// RSC-2.4d: per physics subsystem, the writeback keys published through
    /// the name-keyed path — every compiled write target (the slot table
    /// deliberately mints NO physics claims: port/flow identity is Phase 3
    /// scope, design doc §1, so coverage is 0 by construction until the
    /// exchange plane lands) plus the short-alias keys minted so far
    /// (`owner.port.feature` → `port.feature`, runtime-dynamic). Mirror of
    /// [`sm_slot_fallbacks`](Self::sm_slot_fallbacks) /
    /// [`action_slot_fallbacks`](Self::action_slot_fallbacks); subsystems
    /// with an empty list are omitted. This list is the physics share of
    /// the RSC-2.5 remains-name-keyed inventory.
    pub fn physics_slot_fallbacks(&self) -> Vec<(String, Vec<String>)> {
        self.subsystems
            .iter()
            .filter(|s| s.executor.phase() == ExecutionPhase::Physics)
            .filter_map(|s| {
                let fallbacks = s.executor.slot_write_fallbacks();
                if fallbacks.is_empty() {
                    None
                } else {
                    Some((s.name.clone(), fallbacks))
                }
            })
            .collect()
    }

    /// RSC-2.5: per-scope unresolved names from the slot-binding pass —
    /// `(scope label, names)`. Input to the compiler's `RS003 unresolved
    /// runtime name` hard error. Empty for orchestrators assembled without
    /// the compiler, and for every successfully compiled orchestrator
    /// (a non-empty list whose names are not graph features fails the
    /// build).
    /// Non-fatal compile diagnostics attached to this orchestrator
    /// (RS003 warnings for constraint-scope-only unresolved names).
    pub fn compile_warnings(&self) -> &[sysml_span::Diagnostic] {
        &self.compile_warnings
    }

    /// Attach non-fatal compile diagnostics (compiler-internal).
    pub(crate) fn push_compile_warnings(
        &mut self,
        warnings: impl IntoIterator<Item = sysml_span::Diagnostic>,
    ) {
        self.compile_warnings.extend(warnings);
    }

    pub fn bind_unresolved_scopes(&self) -> &[(String, Vec<String>)] {
        &self.bind_unresolved_scopes
    }

    /// Install a push-notify observer invoked after every snapshot. Used by
    /// the service layer to forward snapshots into a broadcast channel
    /// instead of polling. Replaces any previously installed observer.
    ///
    /// Observers are **not** propagated across [`fork`](Self::fork); the
    /// caller re-installs the observer on the forked child.
    pub fn set_snapshot_observer(
        &mut self,
        observer: Arc<dyn Fn(&ExecutionSnapshot) + Send + Sync>,
    ) {
        self.snapshot_observer.set(observer);
    }

    /// Remove any installed snapshot observer.
    pub fn clear_snapshot_observer(&mut self) {
        self.snapshot_observer.clear();
    }

    /// Apply key=value session overrides (RSC-2.5, design doc D-2.0.7).
    ///
    /// Each name resolves, in order:
    /// 1. **Slot alias table** ([`SlotStore::slot_by_name`] — the compiler
    ///    registers BOTH the canonical tree-path spelling and the runtime
    ///    instance-scoped spelling per slot since RSC-2.1, plus bare aliases
    ///    where unambiguous). A resolved override is a typed write through
    ///    [`EvalContext::set_slot`], whose dual-spelling mirror keeps the
    ///    legacy map readable under both spellings — this replaces the
    ///    pre-2.5 bidirectional string fan-out. Writing a
    ///    [`Variability::Constant`](crate::slots::Variability::Constant)
    ///    slot is an error.
    /// 2. **Existing context variable** — names that legitimately exist
    ///    only in the context map (physics short aliases, port payload
    ///    keys, `__sf_*` waveforms — runtime-dynamic Phase 3 identity the
    ///    slot table deliberately never mints) keep working as plain map
    ///    writes.
    /// 3. Neither → **`RS002 unknown override target`** — a hard error.
    ///    Silent creation of a typo'd variable died at RSC-2.5
    ///    (user-approved contract amendment, design doc §8 Q3; see
    ///
    /// All-or-nothing: every name is resolved before anything is applied,
    /// so a failing override batch leaves the context untouched.
    pub fn apply_overrides_with_aliases(
        &mut self,
        overrides: &[(String, String)],
    ) -> Result<(), OverrideError> {
        enum Target {
            /// Slot write + the slot's two mirrored spellings (captured so
            /// the apply phase can tell whether the caller's verbatim key
            /// is already covered by the mirror).
            Slot(crate::slots::SlotId, Arc<str>, Arc<str>),
            ContextKey,
        }

        // Phase 1 — resolve every name (no writes yet).
        let mut resolved: Vec<(Target, &String, Value)> = Vec::with_capacity(overrides.len());
        {
            let store = self.slot_store.read().unwrap_or_else(|p| p.into_inner());
            for (key, val_str) in overrides {
                let value = crate::compiler::parse_value_string(val_str);
                if let Some(id) = store.slot_by_name(key) {
                    let meta = store.meta(id).expect("slot_by_name returns live ids");
                    if meta.variability == crate::slots::Variability::Constant {
                        return Err(OverrideError::ConstantSlot { name: key.clone() });
                    }
                    resolved.push((
                        Target::Slot(
                            id,
                            Arc::clone(&meta.canonical_name),
                            Arc::clone(&meta.runtime_name),
                        ),
                        key,
                        value,
                    ));
                    continue;
                }
                if self.context.get(key).is_some() {
                    resolved.push((Target::ContextKey, key, value));
                    continue;
                }
                return Err(OverrideError::UnknownTarget { name: key.clone() });
            }
        }

        // Phase 2 — apply.
        for (target, key, value) in resolved {
            match target {
                Target::Slot(id, canonical, runtime) => {
                    if !self.context.set_slot(id, value.clone()) {
                        // Routing refused (store disabled between phases /
                        // detached context): plain map write of the
                        // caller's key keeps the override observable.
                        self.context.set(key.clone(), value);
                    } else if key.as_str() != canonical.as_ref() && key.as_str() != runtime.as_ref()
                    {
                        // The caller used a third registered alias (bare
                        // spelling): keep that spelling coherent too —
                        // pre-2.5 behaviour always wrote the verbatim key.
                        self.context.set(key.clone(), value);
                    }
                }
                Target::ContextKey => self.context.set(key.clone(), value),
            }
        }
        Ok(())
    }

    /// Access the backward-causation recorder (R7.1). Returns an empty
    /// recorder when no events have been captured yet — which is always
    /// the case until the recording hooks are wired in.
    pub fn causation(&self) -> &crate::causation::CausationRecorder {
        &self.causation
    }

    /// Add a state machine subsystem. Returns the [`SubsystemIndex`] this
    /// subsystem was registered under (RSC-4.2 L40).
    pub fn add_state_machine(
        &mut self,
        name: impl Into<String>,
        runner: StateMachineRunner,
    ) -> SubsystemIndex {
        let name = name.into();
        // RSC-3.3c D4: the SM's PortMessage accept ports are accepting
        // surfaces for occurrence-addressed MessageTransfers.
        for port in runner.accept_ports() {
            self.router.register_acceptor(format!("{name}.{port}"));
        }
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name,
            executor: Box::new(runner),
            var_prefix: None,
            source_element_id: None,
            canonical_prefix: None,
        });
        index
    }

    /// Add a state machine subsystem with a variable prefix for instance
    /// scoping. Returns the [`SubsystemIndex`] this subsystem was registered
    /// under (RSC-4.2 L40).
    pub fn add_state_machine_prefixed(
        &mut self,
        name: impl Into<String>,
        runner: StateMachineRunner,
        prefix: impl Into<String>,
    ) -> SubsystemIndex {
        let name = name.into();
        // RSC-3.3c D4: see add_state_machine.
        for port in runner.accept_ports() {
            self.router.register_acceptor(format!("{name}.{port}"));
        }
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name,
            executor: Box::new(runner),
            var_prefix: Some(prefix.into()),
            source_element_id: None,
            canonical_prefix: None,
        });
        index
    }

    /// RSC-3.5b (leftover-C): like [`add_state_machine_prefixed`] but also
    /// records the canonical tree-path prefix (`{container}.{instance}`).
    /// Instance-scoped SM assignment targets are minted with a canonical
    /// spelling that differs from their `{prefix}.{var}` runtime key; without
    /// the canonical prefix, [`WriteRoute::resolve`](crate::slots::WriteRoute)
    /// refuses the route on the canonical-spelling mismatch and the target
    /// falls back to the name-keyed path (reported in
    /// [`sm_slot_fallbacks`](Self::sm_slot_fallbacks)). Supplying it lets the
    /// route resolve against the slot's canonical name. Empty disables it
    /// (identical to [`add_state_machine_prefixed`]).
    pub fn add_state_machine_prefixed_with_canonical(
        &mut self,
        name: impl Into<String>,
        runner: StateMachineRunner,
        prefix: impl Into<String>,
        canonical_prefix: impl Into<String>,
    ) -> SubsystemIndex {
        let name = name.into();
        // RSC-3.3c D4: see add_state_machine.
        for port in runner.accept_ports() {
            self.router.register_acceptor(format!("{name}.{port}"));
        }
        let canonical = canonical_prefix.into();
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name,
            executor: Box::new(runner),
            var_prefix: Some(prefix.into()),
            source_element_id: None,
            canonical_prefix: if canonical.is_empty() {
                None
            } else {
                Some(canonical)
            },
        });
        index
    }

    /// GAP 2 (L23 sub-gap B): record that the SM subsystem `subsystem_name`
    /// realises the behaviour of the part-usage instance keyed `owner_key`.
    /// A routed message to `{owner_key}.{port}` is delivered to this subsystem's
    /// `port_events` even though the subsystem is named after its `state def`
    /// (non-multiplied) or `{prefix}.{state-def}` (multiplied), not after the
    /// owning part. Idempotent and append-only (a part owning ≥2 state defs
    /// fans out to several subsystems). See [`owner_to_subsystems`].
    pub fn register_owner_subsystem(
        &mut self,
        owner_key: impl Into<String>,
        subsystem_name: impl Into<String>,
    ) {
        let subsystem_name = subsystem_name.into();
        let entry = self
            .owner_to_subsystems
            .entry(owner_key.into())
            .or_default();
        if !entry.contains(&subsystem_name) {
            entry.push(subsystem_name);
        }
    }

    /// Inverse of [`owner_to_subsystems`]: given an SM subsystem name, find the
    /// owner-instance key whose realising set contains it. This is the SEND-side
    /// dual of [`convert_deliveries_to_port_events`]'s owner→subsystem fan-out
    /// (GAP 2): a sending SM knows only its own subsystem name + port, but the
    /// router's link-graph source index keys on the part-usage instance
    /// (`{owner}.{port}`, e.g. `relay.tripOut`). Subsystem names are unique per
    /// orchestrator (`subsystem_states` is name-keyed), so the inverse is 1:1.
    ///
    /// An associated fn (not a `&self` method) so it borrows only the map field,
    /// leaving `&mut self.subsystems` free for the step loop's caller.
    fn owner_key_for(map: &HashMap<String, Vec<String>>, subsystem_name: &str) -> Option<String> {
        map.iter().find_map(|(owner, subs)| {
            subs.iter()
                .any(|s| s == subsystem_name)
                .then(|| owner.clone())
        })
    }

    /// RSC-4.3 F1: apply the occurrence-lifecycle + sends/port_sends/messages/
    /// addressed_messages routing side effects for ONE mid-tick fire, in
    /// isolation from step 3's main tail. Used for every fire in a
    /// multi-crossing tick EXCEPT the last (which still falls through to the
    /// existing inline tail below — unchanged, so the single-fire/no-restep
    /// path stays byte-identical). Mirrors the tail's logic exactly; kept in
    /// one place so a future change to the routing/occurrence contract can't
    /// drift between the "earlier fires" and "last fire" code paths.
    ///
    /// An associated fn (not `&self`/`&mut self`) so it borrows only the
    /// fields it needs, leaving `&mut self.subsystems[si]` (held by the
    /// caller's `subsystem` binding across this call) untouched — mirrors
    /// [`owner_key_for`](Self::owner_key_for)'s borrow-splitting pattern.
    #[allow(clippy::too_many_arguments)]
    fn apply_fired_sm_tail(
        occurrence_tracker: &mut OccurrenceTracker,
        router: &mut ExchangePlane,
        owner_to_subsystems: &HashMap<String, Vec<String>>,
        context: &EvalContext,
        time_ms: f64,
        subsystem_name: &str,
        phase: ExecutionPhase,
        prev_state: &str,
        output: &TickOutput,
        all_sends: &mut Vec<(String, String)>,
    ) {
        // Occurrence lifecycle tracking (Feature 10.3) — identical to the
        // step-3 tail below.
        if output.current_state != prev_state && phase == ExecutionPhase::StateMachine {
            let time_s = time_ms / 1000.0;
            occurrence_tracker.end(subsystem_name, prev_state, time_s, HashMap::new());
            let features: HashMap<String, Value> = context
                .variables
                .iter()
                .filter(|(k, _)| !crate::expressions::is_internal_var(k))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect();
            occurrence_tracker.begin(
                OccurrenceKind::StateExecution,
                subsystem_name,
                &output.current_state,
                time_s,
                features,
            );
        }

        // Collect sends for routing — see the step-3 tail's comment for why
        // bare sends and `via`-port sends are handled differently.
        for send_event in &output.sends {
            if send_event.starts_with("send via ") {
                continue;
            }
            all_sends.push((subsystem_name.to_owned(), send_event.clone()));
        }
        for (port, payload) in &output.port_sends {
            let owner = Self::owner_key_for(owner_to_subsystems, subsystem_name)
                .unwrap_or_else(|| subsystem_name.to_owned());
            router.send(&format!("{}.{}", owner, port), payload.clone());
        }
        for (source_key, payload) in &output.messages {
            router.send(source_key, payload.clone());
        }
        for (source_key, target, payload) in &output.addressed_messages {
            router.send_message(source_key, target, payload.clone());
        }
    }

    /// Convert routed `FlowRouter`/`ExchangePlane` deliveries into `port_events`
    /// keyed by the subsystem that will read them. Each delivery's target key is
    /// `owner.port`; the owner (a part-usage instance) is reconciled to the SM
    /// subsystem(s) realising it via [`owner_to_subsystems`] (GAP 2). When no
    /// mapping exists — hand-built orchestrators that name the subsystem after
    /// the owner directly, or non-SM acceptors — the literal owner key is used,
    /// preserving the pre-GAP-2 RouteFirst behaviour byte-identically.
    ///
    /// GAP 3: this was previously inlined in the RouteFirst-only pre-step branch,
    /// so the production `StepFirst` path never populated `port_events` and
    /// delivered messages sat undrained in the router queue. It is now called
    /// post-route under BOTH strategies (one home, principle #5).
    fn convert_deliveries_to_port_events(&mut self, delivered: &[FlowMessage]) {
        for msg in delivered {
            // Parse target key "owner.port" → (owner, port).
            let mut split = msg.target.splitn(2, '.');
            let (Some(owner), Some(port)) = (split.next(), split.next()) else {
                continue;
            };
            match self.owner_to_subsystems.get(owner) {
                Some(subsystems) if !subsystems.is_empty() => {
                    // GAP 2: fan out to every SM subsystem realising this owner.
                    for sub in subsystems.clone() {
                        self.port_events
                            .entry(sub)
                            .or_default()
                            .push((port.to_owned(), msg.payload.clone()));
                    }
                }
                _ => {
                    self.port_events
                        .entry(owner.to_owned())
                        .or_default()
                        .push((port.to_owned(), msg.payload.clone()));
                }
            }
        }
    }

    /// Add an action subsystem. Returns the [`SubsystemIndex`] this
    /// subsystem was registered under (RSC-4.2 L40).
    pub fn add_action(&mut self, name: impl Into<String>, runner: ActionRunner) -> SubsystemIndex {
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name: name.into(),
            executor: Box::new(runner),
            var_prefix: None,
            source_element_id: None,
            canonical_prefix: None,
        });
        index
    }

    /// Add a continuous-time ODE subsystem (Phase 15).
    ///
    /// ODE subsystems are stepped each tick before state machines, writing their
    /// state variables into the shared `EvalContext`. This allows state machine
    /// `when()` triggers to react to continuous state changes.
    ///
    /// Returns the [`SubsystemIndex`] this subsystem was registered under
    /// (RSC-4.2 L40).
    pub fn add_ode(&mut self, name: impl Into<String>, solver: Rk4Solver) -> SubsystemIndex {
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name: name.into(),
            executor: Box::new(solver),
            var_prefix: None,
            source_element_id: None,
            canonical_prefix: None,
        });
        index
    }

    /// Add an ODE subsystem with a variable prefix for instance scoping.
    /// Returns the [`SubsystemIndex`] this subsystem was registered under
    /// (RSC-4.2 L40).
    pub fn add_ode_prefixed(
        &mut self,
        name: impl Into<String>,
        solver: Rk4Solver,
        prefix: impl Into<String>,
    ) -> SubsystemIndex {
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name: name.into(),
            executor: Box::new(solver),
            var_prefix: Some(prefix.into()),
            source_element_id: None,
            canonical_prefix: None,
        });
        index
    }

    /// Like [`add_ode_prefixed`] but also records a canonical tree-path
    /// prefix. State vars get written back under both `{var_prefix}.{var}`
    /// (in-subsystem scoping compat) and `{canonical_prefix}.{var}`
    /// (so frontend tree `ownerPath` lookups find the live value
    /// instead of the compiler's static initial literal). Empty
    /// `canonical_prefix` disables aliasing.
    /// Returns the [`SubsystemIndex`] this subsystem was registered under
    /// (RSC-4.2 L40).
    pub fn add_ode_prefixed_with_canonical(
        &mut self,
        name: impl Into<String>,
        solver: Rk4Solver,
        prefix: impl Into<String>,
        canonical_prefix: impl Into<String>,
    ) -> SubsystemIndex {
        let canonical = canonical_prefix.into();
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name: name.into(),
            executor: Box::new(solver),
            var_prefix: Some(prefix.into()),
            source_element_id: None,
            canonical_prefix: if canonical.is_empty() {
                None
            } else {
                Some(canonical)
            },
        });
        index
    }

    /// Add an adaptive-step RK45 ODE subsystem (Phase 15C). Returns the
    /// [`SubsystemIndex`] this subsystem was registered under (RSC-4.2 L40).
    pub fn add_ode45(
        &mut self,
        name: impl Into<String>,
        solver: crate::ode45::Rk45Solver,
    ) -> SubsystemIndex {
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name: name.into(),
            executor: Box::new(solver),
            var_prefix: None,
            source_element_id: None,
            canonical_prefix: None,
        });
        index
    }

    /// Instance-scoped RK45 subsystem — the adaptive-explicit analogue of
    /// [`Self::add_ode_prefixed_with_canonical`], for multiplied parts. Needed
    /// now that RK45 is the un-annotated default (WS-B2): a multiplied
    /// un-annotated ODE must carry the instance prefix + canonical tree-path
    /// like its RK4/BDF siblings, or its slot routing breaks.
    /// Instance-scoped RK45 subsystem — the adaptive-explicit analogue of
    /// [`Self::add_ode_prefixed_with_canonical`], for multiplied parts.
    /// Returns the [`SubsystemIndex`] this subsystem was registered under
    /// (RSC-4.2 L40).
    pub fn add_ode45_prefixed_with_canonical(
        &mut self,
        name: impl Into<String>,
        solver: crate::ode45::Rk45Solver,
        prefix: impl Into<String>,
        canonical_prefix: impl Into<String>,
    ) -> SubsystemIndex {
        let canonical = canonical_prefix.into();
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name: name.into(),
            executor: Box::new(solver),
            var_prefix: Some(prefix.into()),
            source_element_id: None,
            canonical_prefix: if canonical.is_empty() {
                None
            } else {
                Some(canonical)
            },
        });
        index
    }

    /// Add a BDF (implicit, variable-order) ODE subsystem (R7.3).
    ///
    /// BDF is the integrator of choice for **stiff** systems where explicit
    /// methods (RK4/RK45) are either unstable or forced onto a prohibitively
    /// small step. See [`crate::solvers::bdf::BdfSolver`].
    ///
    /// Returns the [`SubsystemIndex`] this subsystem was registered under
    /// (RSC-4.2 L40).
    pub fn add_bdf(
        &mut self,
        name: impl Into<String>,
        solver: crate::solvers::bdf::BdfSolver,
    ) -> SubsystemIndex {
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name: name.into(),
            executor: Box::new(solver),
            var_prefix: None,
            source_element_id: None,
            canonical_prefix: None,
        });
        index
    }

    /// Instance-scoped BDF subsystem — the stiff-solver analogue of
    /// [`Self::add_ode_prefixed_with_canonical`], for multiplied parts.
    /// Returns the [`SubsystemIndex`] this subsystem was registered under
    /// (RSC-4.2 L40).
    pub fn add_bdf_prefixed_with_canonical(
        &mut self,
        name: impl Into<String>,
        solver: crate::solvers::bdf::BdfSolver,
        prefix: impl Into<String>,
        canonical_prefix: impl Into<String>,
    ) -> SubsystemIndex {
        let canonical = canonical_prefix.into();
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name: name.into(),
            executor: Box::new(solver),
            var_prefix: Some(prefix.into()),
            source_element_id: None,
            canonical_prefix: if canonical.is_empty() {
                None
            } else {
                Some(canonical)
            },
        });
        index
    }

    /// Add a discrete-time state-space subsystem (Feature 6.6). Returns the
    /// [`SubsystemIndex`] this subsystem was registered under (RSC-4.2 L40).
    pub fn add_discrete(
        &mut self,
        name: impl Into<String>,
        solver: crate::ode::DiscreteStateSolver,
    ) -> SubsystemIndex {
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name: name.into(),
            executor: Box::new(solver),
            var_prefix: None,
            source_element_id: None,
            canonical_prefix: None,
        });
        index
    }

    /// Add any executor type (generic method). Returns the [`SubsystemIndex`]
    /// this subsystem was registered under (RSC-4.2 L40).
    pub fn add_executor(
        &mut self,
        name: impl Into<String>,
        executor: Box<dyn Executor>,
    ) -> SubsystemIndex {
        let index = SubsystemIndex(self.subsystems.len() as u16);
        self.subsystems.push(Subsystem {
            name: name.into(),
            executor,
            var_prefix: None,
            source_element_id: None,
            canonical_prefix: None,
        });
        index
    }

    /// Add a physics domain executor (Phase 6).
    ///
    /// Physics executors run before ODE solvers each tick, applying conservation
    /// constraints and effort equalities from the connection graph topology.
    ///
    /// Returns the [`SubsystemIndex`] this subsystem was registered under
    /// (RSC-4.2 L40).
    pub fn add_physics(
        &mut self,
        name: impl Into<String>,
        executor: crate::physics::executor::PhysicsExecutor,
    ) -> SubsystemIndex {
        self.add_executor(name, Box::new(executor))
    }

    /// Set the `source_element_id` on the most-recently-added subsystem.
    ///
    /// Called by `ModelCompiler` after each `add_*` call to attach the
    /// originating `ElementId` for topology overlay mapping (ADR-006).
    pub fn set_last_source_element_id(&mut self, element_id: ElementId) {
        if let Some(last) = self.subsystems.last_mut() {
            last.source_element_id = Some(element_id);
        }
    }

    /// Return the number of subsystems in the orchestrator.
    pub fn subsystem_count(&self) -> usize {
        self.subsystems.len()
    }

    /// Set the port registry for runtime port value propagation.
    ///
    /// When set, the orchestrator propagates port values through the classified
    /// link graph each tick: context variables → output ports → flow resolution
    /// → input ports → context. The non-signal flow mirror reads the FlowUsage
    /// subset of the link graph installed via [`set_link_graph`](Self::set_link_graph)
    /// (RSC-3.5e.5 — the legacy `flow_connections` field is retired).
    pub fn set_port_registry(&mut self, registry: crate::flows::PortRegistry) {
        self.port_registry = Some(registry);
    }

    /// Install a snapshot-scoped `RefResolveCache` shared across
    /// orchestrators built against the same elaborated-graph revision.
    ///
    /// Production callers pass the salsa-cached
    /// `Arc<Mutex<RefResolveCache>>` returned by
    /// `SysmlService::workspace_ref_resolve_cache` (via
    /// `Snapshot::build_workspace_orchestrator`) so the per-tick
    /// `resolve_ref_value_cached` lookups in [`capture_snapshot`]
    /// hit a cache populated by prior sessions on the same revision.
    /// Test / synthetic-graph paths can skip this and rely on the
    /// default fresh empty cache that `Orchestrator::new` installs.
    pub fn set_ref_resolve_cache(
        &mut self,
        cache: Arc<std::sync::Mutex<crate::expressions::RefResolveCache>>,
    ) {
        self.ref_resolve_cache = cache;
    }

    /// Enable convergence iteration with the given maximum iteration count.
    ///
    /// When convergence is enabled, the orchestrator re-steps all subsystems and
    /// re-evaluates computed expressions after each tick until the relative change
    /// in all float variables drops below `convergence_epsilon`, or the iteration
    /// limit is reached. Useful for multi-domain coupling (e.g., electrical → thermal).
    pub fn set_convergence_iterations(&mut self, max_iterations: u32) {
        self.config.convergence_max_iterations = max_iterations;
    }

    /// Returns the current convergence iteration limit.
    pub fn convergence_iterations(&self) -> u32 {
        self.config.convergence_max_iterations
    }

    /// Add a computed expression that is evaluated each tick after all subsystems step.
    ///
    /// The expression is evaluated against the shared context and the result is
    /// written to `target_variable`. Use this for derived/aggregate values like
    /// `totalCurrent = circuit1.loadCurrent + circuit2.loadCurrent + ...`.
    pub fn add_computed_expression(&mut self, target_variable: impl Into<String>, expr: ExprIR) {
        self.computed_expressions
            .push(GatedExpression::new(target_variable, expr));
    }

    /// RSC-4.2 (C.4): add an **instance-scoped** computed expression whose
    /// RHS is authored in the instance's local namespace (bare
    /// `bimetalTemp`, `config.tripTemperature`, …). The expression is bound
    /// at compile time via
    /// [`SlotBinder::for_subsystem`](crate::expressions::SlotBinder::for_subsystem)
    /// (`bind_expression_slots`) so each local name resolves to *this
    /// instance's* slot, and evaluated against a scoped read view so the
    /// name-first evaluator misses the bare name and the `SlotRef` wins.
    /// This replaces the deleted text-prefixing path
    /// (`prefix_expression_identifiers`) with collision-safe slot binding.
    pub fn add_instance_computed_expression(
        &mut self,
        target_variable: impl Into<String>,
        expr: ExprIR,
        scope_prefix: impl Into<String>,
    ) {
        self.computed_expressions
            .push(GatedExpression::instance_scoped(
                target_variable,
                expr,
                scope_prefix,
            ));
    }

    /// Add a gated computed expression.
    ///
    /// When the `gate_variable` is truthy (true, non-zero), the expression
    /// evaluates normally. When falsy (false, 0.0), the target is set to 0.0.
    ///
    /// Use this for flow values through parts that can be disconnected:
    /// a state machine enters a "tripped" state → sets gate to false →
    /// all gated flows through that part output zero.
    pub fn add_gated_computed_expression(
        &mut self,
        target_variable: impl Into<String>,
        expr: ExprIR,
        gate_variable: impl Into<String>,
    ) {
        self.computed_expressions.push(GatedExpression::gated(
            target_variable,
            expr,
            gate_variable,
        ));
    }

    /// Register flow gate states for a state machine subsystem.
    ///
    /// When the SM named `sm_name` enters any of the `gating_states`, the
    /// gate variable `{sm_name}.__flow_gate` is set to `false`, causing
    /// gated computed expressions to output 0.0. When the SM is in any
    /// other state, the gate is `true`.
    ///
    /// The gate variable is initialized to `true` on registration.
    pub fn add_flow_gate(&mut self, sm_name: impl Into<String>, gating_states: Vec<String>) {
        let name = sm_name.into();
        let gate_var = format!("{}.__flow_gate", name);
        self.context.set(gate_var, Value::Bool(true));
        self.flow_gates.insert(name, gating_states);
    }

    /// Register a zero-crossing detector for an ODE subsystem (Phase 15D;
    /// re-keyed by [`SubsystemIndex`] under RSC-4.2 L39).
    ///
    /// When the ODE at `ode_index` is stepped, the detector checks its event
    /// functions for sign changes. Detected crossings are injected as
    /// discrete events into `target_sm_index` — the state machine captured
    /// at wire time by the caller (e.g.
    /// `ModelCompiler::wire_when_crossings_for_pair`), never re-derived by
    /// name or by "first StateMachine subsystem" at tick time.
    pub fn add_crossing_detector(
        &mut self,
        ode_index: SubsystemIndex,
        target_sm_index: SubsystemIndex,
        detector: crate::ode_events::ZeroCrossingDetector,
    ) {
        self.crossing_detectors
            .insert(ode_index, (target_sm_index, detector));
    }

    /// RSC-4.3: mark an ODE `SubsystemIndex`'s located crossings as eligible for
    /// the time-accurate mid-tick re-step. Called ONLY from
    /// `wire_when_crossings_for_pair` (the `accept when` comparator path, which
    /// carries a real per-transition target-SM binding). The
    /// `wire_ssr_zero_crossing_events` path intentionally never calls this — see
    /// [`restep_eligible_odes`](Self::restep_eligible_odes) and ledger L46.
    pub fn mark_restep_eligible(&mut self, ode_index: SubsystemIndex) {
        self.restep_eligible_odes.insert(ode_index);
    }

    /// RSC-4.3: process one ODE's zero-crossing detector for the current tick.
    ///
    /// Runs the stateful [`check`](crate::ode_events::ZeroCrossingDetector::check)
    /// over `[t_start, t_end]` — updating the cross-tick sign bookkeeping and the
    /// `last_crossing_events` / duty / step-size-history observables EXACTLY as
    /// before RSC-4.3 — then dispatches the located crossings:
    ///   * re-step-ELIGIBLE ODE → time-accurate mid-tick re-step + fire
    ///     ([`restep_and_fire`](Self::restep_and_fire)), stashing every fired
    ///     SM's output in `fired_sm` (a `Vec`, fire-time order) so step 3
    ///     records ALL of them without re-ticking (RSC-4.3 F1: a tick can
    ///     locate MULTIPLE crossings targeting the SAME SM — e.g. two
    ///     opposing-threshold `accept when` comparators both sign-changing in
    ///     the window — and every fire genuinely happened at its instant
    ///     under Option A, so none may be silently dropped).
    ///   * otherwise (SSR path) → the pre-RSC-4.3 post-step `inject_event`.
    ///
    /// A detector that finds no crossing this tick touches none of the re-step
    /// machinery, so a no-crossing model is byte-identical (structural gate). The
    /// detector is removed from the map for the duration so the re-step body can
    /// borrow `&mut self` freely, then re-inserted.
    #[allow(clippy::too_many_arguments)]
    fn handle_crossing_detector(
        &mut self,
        ode_index: SubsystemIndex,
        t_start: f64,
        dt_seconds: f64,
        t_seconds: f64,
        y_start: &[f64],
        slots_enabled: bool,
        slot_read_ctx: &mut Option<EvalContext>,
        fired_sm: &mut HashMap<SubsystemIndex, Vec<(String, TickOutput)>>,
    ) {
        let Some((sm_index, mut detector)) = self.crossing_detectors.remove(&ode_index) else {
            return;
        };
        let t_end = t_start + dt_seconds;
        let y_end: Vec<f64> = self
            .subsystems
            .get(ode_index.index())
            .and_then(|s| s.executor.get_state_snapshot())
            .unwrap_or_default();

        let crossings = detector.check(t_start, t_end, y_start, &y_end, &self.context);
        if !crossings.is_empty() {
            // RSC-4.2 S2 oracle support: retain the located crossings for
            // introspection (see `last_crossing_time`). Unchanged by RSC-4.3.
            self.last_crossing_events
                .insert(ode_index, crossings.clone());

            // P1 dt-under-resolution advisory: record the tick of each located
            // crossing (consecutive crossings are half-cycles). Unchanged.
            let hist = self.step_size_crossing_ticks.entry(ode_index).or_default();
            for _ in &crossings {
                hist.push_back(self.tick);
                if hist.len() > STEP_SIZE_CROSSING_HISTORY {
                    hist.pop_front();
                }
            }

            // WS-D Stage 2 (SPEC-SILENT): feed the comparator crossings to the
            // duty tracker and publish `<ode>.duty`. Unchanged by RSC-4.3 — the
            // duty metric keys on the LOCATED crossing times, identical whether
            // or not the SM effect is applied mid-tick.
            //
            // Zero-order hold (spec-silent, hybrid-sim tick-boundary semantics):
            // the published `duty` is frozen at this crossing update and read by
            // the ODE RHS / signal expressions unchanged across every RK stage
            // of the following ticks until the next crossing updates it — a ZOH
            // on a tooling-measured observable, not a modeled continuous state
            // (StateSpaceRepresentation treats the RHS as f(u, x)).
            let duty_now = if let Some(tracker) = self.duty_trackers.get_mut(&ode_index) {
                for c in &crossings {
                    tracker.observe(&c.name, c.time);
                }
                tracker.current_duty()
            } else {
                None
            };
            if let Some(d) = duty_now {
                if let Some(ode_name) = self.subsystems.get(ode_index.index()).map(|s| s.name.clone())
                {
                    // Task #8 (steward-ruled): publish by SlotId via `set_slot`,
                    // NOT `set("{ode}.duty")`. The duty slot is minted with both
                    // name forms = bare `duty` plus a `{ode}.duty` alias (compiler
                    // step 2c). `set_slot` mirrors the value into the legacy
                    // variables map under the slot's runtime+canonical names
                    // (both `duty`), so the RHS's name-first `SlotRef` read of the
                    // bare `duty` sees the live value. `set("{ode}.duty")` wrote
                    // the slot + the *qualified* variable only, leaving the bare
                    // `duty` map entry frozen at its t=0 default — which read
                    // stale and starved the faultIntegral chain (name-first
                    // SlotRef eval). The qualified alias resolves this ODE's own
                    // duty slot; the slot always exists for an ODE that registered
                    // a duty tracker.
                    let duty_key = format!("{ode_name}.duty");
                    match self.context.slot_id(&duty_key) {
                        Some(duty_slot) => {
                            // Must-succeed: the slot exists (checked above) and is
                            // a writable duty tracker; a dropped write starves the
                            // faultIntegral chain (the bug this comment describes).
                            let wrote = self.context.set_slot(duty_slot, Value::Float(d));
                            debug_assert!(
                                wrote,
                                "duty slot '{duty_key}' write must succeed \
                                 (resolved slot, writable duty tracker)"
                            );
                        }
                        None => debug_assert!(
                            false,
                            "duty slot '{duty_key}' must exist for an ODE with a \
                             registered duty tracker (compiler step 2c mints it)"
                        ),
                    }
                }
            }

            if self.restep_eligible_odes.contains(&ode_index) {
                // Option A (addendum): mid-tick re-step + fire.
                self.restep_and_fire(
                    ode_index,
                    sm_index,
                    &mut detector,
                    t_start,
                    t_end,
                    t_seconds,
                    dt_seconds,
                    y_start,
                    &crossings,
                    slots_enabled,
                    slot_read_ctx,
                    fired_sm,
                );
            } else {
                // SSR / non-eligible path: pre-RSC-4.3 post-step injection. The
                // event is delivered to the target SM in step 3 (drained via
                // `drain_due_events`), one tick's ODE integration late — the
                // documented oscillator-vs-SSR asymmetry (ledger L46).
                if let Some(sm_name) =
                    self.subsystems.get(sm_index.index()).map(|s| s.name.clone())
                {
                    for c in &crossings {
                        self.inject_event(&sm_name, &c.name);
                    }
                }
            }
        }

        self.crossing_detectors
            .insert(ode_index, (sm_index, detector));
    }

    /// RSC-4.3 Option A: time-accurate mid-tick re-step for one ODE's located
    /// crossings (addendum contract). Iterates the crossings the initial
    /// [`check`](crate::ode_events::ZeroCrossingDetector::check) located this
    /// tick, EARLIEST FIRST (Q10). For each: roll the ODE back to the previous
    /// sub-interval boundary, integrate to `t_cross` under the CURRENT drive,
    /// commit that state, FIRE the target SM mid-tick (re-syncing its `V_applied`
    /// flip through the slot store) — the next crossing then integrates under the
    /// reversed drive. After the last crossing, continue to `t_end` under the
    /// final drive, so overshoot is bounded by the bisection residual, not `∝ dt`.
    /// Finally re-prime the detector to the committed end-of-tick state so the
    /// next tick's `check` starts from the true `g`.
    ///
    /// The located list is the authoritative crossing set — the loop does NOT
    /// re-detect under the reversed drive (a stateless re-detect spuriously
    /// re-fires the just-crossed event when the signal stays past the threshold,
    /// e.g. a one-shot ramp). A tick with more located crossings than the bound
    /// is catastrophically under-resolved (dt ≫ half-period) → fail-hard RS014,
    /// never silent drop (FORBIDDEN §: the bound is a crossing-count bound,
    /// distinct from the detector's internal bisection max-iterations).
    #[allow(clippy::too_many_arguments)]
    fn restep_and_fire(
        &mut self,
        ode_index: SubsystemIndex,
        sm_index: SubsystemIndex,
        detector: &mut crate::ode_events::ZeroCrossingDetector,
        t_start: f64,
        t_end: f64,
        t_seconds: f64,
        dt_seconds: f64,
        y_start: &[f64],
        located_crossings: &[crate::ode_events::CrossingEvent],
        slots_enabled: bool,
        slot_read_ctx: &mut Option<EvalContext>,
        fired_sm: &mut HashMap<SubsystemIndex, Vec<(String, TickOutput)>>,
    ) {
        const MAX_RESTEP_CROSSINGS_PER_TICK: usize = 64;
        if located_crossings.len() > MAX_RESTEP_CROSSINGS_PER_TICK {
            let ode_name = self.subsystems[ode_index.index()].name.clone();
            panic!(
                "RS014: ODE subsystem '{ode_name}' located {} zero-crossings in a single \
                 tick (dt={dt_seconds}s), exceeding the per-tick bound of \
                 {MAX_RESTEP_CROSSINGS_PER_TICK} — the step is catastrophically \
                 under-resolved. Refusing to drop crossings; reduce dt.",
                located_crossings.len()
            );
        }

        // Does this ODE read its drive through the thin slot view (bypass) or the
        // master context directly? Mirrors the ODE pre-pass gate so the re-step
        // reads exactly what a normal tick would.
        let ode_bypass = slots_enabled
            && self.subsystems[ode_index.index()].var_prefix.is_some()
            && self.subsystems[ode_index.index()]
                .executor
                .scoped_view_bypass();
        let ode_name = self.subsystems[ode_index.index()].name.clone();

        let mut cursor_t = t_start;
        let mut cursor_y = y_start.to_vec();

        // `located_crossings` is already sorted earliest-first by `check`.
        for cr in located_crossings {
            let t_cross = cr.time.clamp(cursor_t, t_end);

            // (1) Roll the ODE back to the current sub-interval boundary, drop the
            // FSAL cache, and integrate to the crossing under the CURRENT drive
            // (self.context still holds the drive as of the previous fire — the
            // OLD drive for the first crossing).
            {
                let exec = &mut self.subsystems[ode_index.index()].executor;
                if !exec.restore_state(&cursor_y) {
                    // `Executor::restore_state`'s trait default returns `false`
                    // unconditionally (no re-settable continuous state at all) —
                    // NOT necessarily a length mismatch; `Rk45Solver`/`Rk4Solver`
                    // only return `false` on a genuine length mismatch. This
                    // branch should be structurally unreachable post-L47 (the
                    // compiler pins RK45 for every restep-eligible ODE), so
                    // reaching it means either a non-RK45/RK4 executor slipped
                    // past that gate or the captured `y_start` truly doesn't
                    // match — either way, fail hard rather than guess.
                    panic!(
                        "RS014: ODE subsystem '{ode_name}' rejected a state rollback \
                         during the time-accurate re-step — the solver either does not \
                         implement the restep protocol at all, or the captured y_start \
                         does not match its state vector length."
                    );
                }
                exec.invalidate_step_cache();
            }
            {
                let drive_owned;
                let drive: &EvalContext = if ode_bypass {
                    drive_owned = build_slot_read_context(&self.context);
                    &drive_owned
                } else {
                    &self.context
                };
                if !self.subsystems[ode_index.index()]
                    .executor
                    .integrate_interval(cursor_t, t_cross, drive)
                {
                    panic!(
                        "RS014: ODE subsystem '{ode_name}' does not support sub-interval \
                         re-integration (non-RK45 solver on the re-step path). Fixed-step \
                         re-step is a Wave-2b deferral — a located crossing must not be \
                         silently dropped."
                    );
                }
            }

            // State at the located crossing (start point for the NEXT crossing).
            let y_cross = self.subsystems[ode_index.index()]
                .executor
                .get_state_snapshot()
                .unwrap_or_default();

            // (2) Commit the crossing-time ODE state so the SM reads crossing-time
            // values, then FIRE the target SM mid-tick (re-syncs the V_applied flip).
            self.subsystems[ode_index.index()]
                .executor
                .sync_context_out_slots(&mut self.context, crate::ode::SignalEvalMode::FreshState);

            let fired = self.fire_sm_mid_tick(
                sm_index,
                &cr.name,
                t_seconds,
                dt_seconds,
                slots_enabled,
                slot_read_ctx,
            );
            // F1 fix: ACCUMULATE — a tick can locate multiple crossings
            // targeting the SAME SM (e.g. two opposing-threshold comparators
            // both sign-changing in the window); every fire genuinely
            // happened at its instant under Option A, so an `insert` here
            // would silently overwrite (and drop) an earlier fire's
            // occurrence record and routed sends. `Vec::push` preserves every
            // fire in the earliest-first order `located_crossings` already
            // establishes.
            fired_sm.entry(sm_index).or_default().push(fired);

            // (3) The SM writeback changed the drive; invalidate the thin slot
            // view so the continuation + step 3 rebuild from the updated context.
            *slot_read_ctx = None;

            cursor_t = t_cross;
            cursor_y = y_cross;
        }

        // (4) Continue from the last crossing to tick-end under the (now reversed)
        // drive. Drop the FSAL cache (the RHS at cursor_t changed with the drive).
        self.subsystems[ode_index.index()]
            .executor
            .invalidate_step_cache();
        {
            let drive_owned;
            let drive: &EvalContext = if ode_bypass {
                drive_owned = build_slot_read_context(&self.context);
                &drive_owned
            } else {
                &self.context
            };
            if !self.subsystems[ode_index.index()]
                .executor
                .integrate_interval(cursor_t, t_end, drive)
            {
                panic!(
                    "RS014: ODE subsystem '{ode_name}' does not support sub-interval \
                     re-integration on the crossing continuation."
                );
            }
        }

        // Commit the continued state at tick-end.
        self.subsystems[ode_index.index()]
            .executor
            .sync_context_out_slots(&mut self.context, crate::ode::SignalEvalMode::FreshState);

        // (5) Re-prime the detector's prev_values to g at the COMMITTED
        // end-of-tick state — the re-step changed the trajectory, so the stateful
        // `check`'s earlier g_end is stale. Ensures the NEXT tick computes
        // g_start from the true committed state (else it could miss or fabricate
        // a crossing at the tick boundary), and correctly re-arms level events.
        let final_y = self.subsystems[ode_index.index()]
            .executor
            .get_state_snapshot()
            .unwrap_or_default();
        if ode_bypass {
            let drive = build_slot_read_context(&self.context);
            detector.initialize(t_end, &final_y, &drive);
        } else {
            detector.initialize(t_end, &final_y, &self.context);
        }
    }

    /// RSC-4.3: fire ONE state machine mid-tick on a located crossing event —
    /// the SAME dispatch step 3 uses (sync context in → `tick()` → slot
    /// writeback), invoked early and targeted by `SubsystemIndex` (obligation i).
    /// Returns `(prev_state, output)` for step 3 to record without re-ticking
    /// (obligation ii — fire exactly once). The writeback re-syncs the SM's
    /// `V_applied` flip through the slot store (obligation iii). Fails hard
    /// (RS014) if the target SM is not slot-attached — its writeback would be
    /// silently dropped and the ODE continuation would never see the reversed
    /// drive (obligation iv — the L44 raw-`add_state_machine` shape).
    fn fire_sm_mid_tick(
        &mut self,
        sm_index: SubsystemIndex,
        event: &str,
        t_seconds: f64,
        dt_seconds: f64,
        slots_enabled: bool,
        slot_read_ctx: &mut Option<EvalContext>,
    ) -> (String, TickOutput) {
        let si = sm_index.index();
        let prefix_opt = self.subsystems[si].var_prefix.clone();
        let bypass =
            slots_enabled && prefix_opt.is_some() && self.subsystems[si].executor.scoped_view_bypass();
        let prev_state = self.subsystems[si].executor.current_state_name().to_owned();
        let local_clock = self.clock_registry.local_time(&self.subsystems[si].name);

        let output = if bypass {
            let thin =
                slot_read_ctx.get_or_insert_with(|| build_slot_read_context(&self.context));
            self.subsystems[si].executor.sync_context_in(thin);
            let tick_ctx = TickContext {
                t: t_seconds,
                dt: dt_seconds,
                tick: self.tick,
                context: thin,
                event: Some(event),
                port_payloads: &[],
                local_clock_time: local_clock,
            };
            self.subsystems[si].executor.tick(&tick_ctx)
        } else {
            self.subsystems[si].executor.sync_context_in(&self.context);
            let tick_ctx = TickContext {
                t: t_seconds,
                dt: dt_seconds,
                tick: self.tick,
                context: &self.context,
                event: Some(event),
                port_payloads: &[],
                local_clock_time: local_clock,
            };
            self.subsystems[si].executor.tick(&tick_ctx)
        };

        let wrote = self.subsystems[si]
            .executor
            .sync_context_out_slots(&mut self.context, crate::ode::SignalEvalMode::FreshState);
        if !wrote {
            panic!(
                "RS014: mid-tick zero-crossing re-step targeted state machine '{}', which is \
                 NOT slot-attached (no write-set). Its mode/drive writeback would be silently \
                 dropped and the ODE continuation would never see the reversed drive. Re-step \
                 requires a compiler-built, slot-attached SM (the raw add_state_machine L44 \
                 shape is unsupported here).",
                self.subsystems[si].name
            );
        }

        (prev_state, output)
    }

    /// Number of zero-crossing event functions registered on the named ODE's
    /// detector (0 if none). Lets tests assert that `accept when` crossing
    /// wiring (WS-A2) engaged without depending on trace output.
    ///
    /// **Ambiguous for duplicate-named subsystems** (RSC-4.2 ruling 5): this
    /// resolves `ode_name` to the first subsystem carrying that display name,
    /// which may not be the intended one if an SM and an ODE share a name
    /// (the exact scenario this arc fixes on the tick path). A test that must
    /// disambiguate should capture the `SubsystemIndex` at registration and
    /// call [`crossing_detector_event_count_by_index`](Self::crossing_detector_event_count_by_index).
    pub fn crossing_detector_event_count(&self, ode_name: &str) -> usize {
        match self.subsystems.iter().position(|s| s.name == ode_name) {
            Some(idx) => self.crossing_detector_event_count_by_index(SubsystemIndex(idx as u16)),
            None => 0,
        }
    }

    /// Index-keyed counterpart of
    /// [`crossing_detector_event_count`](Self::crossing_detector_event_count)
    /// that disambiguates a duplicate-named ODE/SM pair (RSC-4.2 ruling 5).
    pub fn crossing_detector_event_count_by_index(&self, ode_index: SubsystemIndex) -> usize {
        self.crossing_detectors
            .get(&ode_index)
            .map(|(_, d)| d.event_count())
            .unwrap_or(0)
    }

    /// Register a per-cycle duty-cycle tracker for an ODE subsystem (WS-D
    /// Stage 2, SPEC-SILENT; re-keyed by [`SubsystemIndex`] under RSC-4.2
    /// L39). It is fed the same comparator crossing events the paired
    /// detector produces and write-throughs to a compile-minted slot (see
    /// [`ModelCompiler::mint_slot_store`](crate::compiler::ModelCompiler::mint_slot_store)
    /// step 2c) — never a raw `context.set` at this call site.
    pub fn add_duty_tracker(
        &mut self,
        ode_index: SubsystemIndex,
        tracker: crate::ode_events::DutyCycleTracker,
    ) {
        self.duty_trackers.insert(ode_index, tracker);
    }

    /// The [`SubsystemIndex`]es of every ODE with a registered duty-cycle
    /// tracker. Lets [`ModelCompiler::mint_slot_store`](crate::compiler::ModelCompiler::mint_slot_store)
    /// (step 2c) know which ODEs need a `duty` slot minted, without exposing
    /// `duty_trackers` itself or requiring the compiler to re-derive the set
    /// from `cmp_meta` (a second, driftable path to the same fact).
    pub(crate) fn duty_tracker_indices(&self) -> impl Iterator<Item = SubsystemIndex> + '_ {
        self.duty_trackers.keys().copied()
    }

    /// Most recently computed duty-cycle asymmetry for the named ODE, or `None`
    /// before its first post-transient oscillation cycle completes. Lets tests
    /// read the detection metric without parsing snapshots.
    ///
    /// **Ambiguous for duplicate-named subsystems** (RSC-4.2 ruling 5) — see
    /// [`crossing_detector_event_count`](Self::crossing_detector_event_count).
    /// Use [`duty_cycle_by_index`](Self::duty_cycle_by_index) to disambiguate.
    pub fn duty_cycle(&self, ode_name: &str) -> Option<f64> {
        let idx = self.subsystems.iter().position(|s| s.name == ode_name)?;
        self.duty_cycle_by_index(SubsystemIndex(idx as u16))
    }

    /// Index-keyed counterpart of [`duty_cycle`](Self::duty_cycle) that
    /// disambiguates a duplicate-named ODE/SM pair (RSC-4.2 ruling 5).
    pub fn duty_cycle_by_index(&self, ode_index: SubsystemIndex) -> Option<f64> {
        self.duty_trackers
            .get(&ode_index)
            .and_then(|t| t.current_duty())
    }

    /// Names of ODE subsystems with a registered duty-cycle tracker. Lets tests
    /// find the duty observable key without hardcoding the ODE label.
    pub fn duty_tracker_names(&self) -> Vec<String> {
        self.duty_trackers
            .keys()
            .filter_map(|idx| self.subsystems.get(idx.index()).map(|s| s.name.clone()))
            .collect()
    }

    /// The most recently located crossing time for `event_name` on the ODE at
    /// `ode_index`, or `None` if that event hasn't crossed yet this run.
    /// RSC-4.2 S2 test/introspection sugar — see the doc comment on
    /// [`last_crossing_events`](Self) for why this exists: the discrete tick
    /// a transition fires on is insensitive to the y_end name-resolution bug
    /// this arc fixes (the sign change itself is computed correctly either
    /// way), so a test proving sub-tick location needs the detector's
    /// internal, sub-tick-precision crossing time, not just the tick index.
    pub fn last_crossing_time(&self, ode_index: SubsystemIndex, event_name: &str) -> Option<f64> {
        self.last_crossing_events
            .get(&ode_index)?
            .iter()
            .find(|c| c.name == event_name)
            .map(|c| c.time)
    }

    /// WS-A2 (hybrid event location): rewrite the `when`-triggered transition at
    /// `transition_index` of the state-machine subsystem at `sm_index` to a
    /// located zero-crossing trigger that fires on `event_name`. The compiler
    /// calls this after registering a matching crossing detector, so the
    /// located crossing drives the transition instead of the missed
    /// tick-boundary rising edge.
    ///
    /// RSC-4.2 L39: takes the exact `SubsystemIndex` captured at the SM's
    /// registration call site — never a name to search for — so a
    /// duplicate-named SM/ODE pair can no longer route to the wrong
    /// subsystem.
    ///
    /// Returns `true` when the subsystem exists and the transition carried a
    /// `when` trigger that was rewritten. `false` signals an index/transition
    /// mismatch (the detector would then fire an event no transition
    /// matches) — callers should treat that as a compile-time wiring bug.
    pub fn set_sm_located_trigger(
        &mut self,
        sm_index: SubsystemIndex,
        transition_index: usize,
        event_name: &str,
    ) -> bool {
        self.subsystems.get_mut(sm_index.index()).is_some_and(|s| {
            s.executor
                .rewrite_when_to_located(transition_index, event_name)
        })
    }

    /// Register a succession (HappensBefore) constraint (Feature 10.2).
    ///
    /// The constraint specifies that occurrence `before` must complete before
    /// occurrence `after` can fire, with optional minimum delay, maximum delay
    /// (deadline), and guard condition.
    pub fn add_succession(&mut self, constraint: SuccessionConstraint) {
        self.succession_queue.add_constraint(constraint);
    }

    /// Register a local clock for a subsystem with the given rate multiplier.
    ///
    /// Rate 1.0 = real-time, 2.0 = double speed, 0.5 = half speed.
    /// When the universal clock advances by dt, the subsystem's local clock
    /// advances by dt * rate. The local time is written to `__clock_time`
    /// in that subsystem's eval context each tick.
    pub fn set_clock(&mut self, subsystem: &str, rate: f64) {
        self.clock_registry
            .register(crate::clock::LocalClock::new(subsystem, rate));
    }

    /// Register a local clock with a phase offset for a subsystem.
    ///
    /// The phase offset shifts the initial local time (e.g., offset=5.0 means
    /// the subsystem starts at local time 5.0 seconds).
    pub fn set_clock_with_offset(&mut self, subsystem: &str, rate: f64, phase_offset: f64) {
        self.clock_registry.register(
            crate::clock::LocalClock::new(subsystem, rate).with_phase_offset(phase_offset),
        );
    }

    /// Get a reference to the clock registry.
    pub fn clock_registry(&self) -> &ClockRegistry {
        &self.clock_registry
    }

    /// Get a reference to the occurrence tracker (Feature 10.3).
    pub fn occurrences(&self) -> &OccurrenceTracker {
        &self.occurrence_tracker
    }

    /// Install a populated [`ExchangePlane`] as the routing backend
    /// (RSC-3.5a.2-ii; the sole installer since RSC-3.5e.2 retired
    /// `set_router`/`FlowRouter`). The accepting surfaces of every
    /// already-registered subsystem are re-registered on the new plane
    /// (RSC-3.3c D4 — acceptor topology follows the subsystems, not the
    /// routing-backend instance). Used to feed the plane compiled flows.
    pub fn set_exchange_plane(&mut self, plane: ExchangePlane) {
        self.router = plane;
        self.reregister_acceptors();
    }

    /// Re-register every subsystem's accepting surfaces on the current routing
    /// backend (used by [`set_exchange_plane`](Self::set_exchange_plane)).
    fn reregister_acceptors(&mut self) {
        // Collect first so the immutable subsystem borrow does not overlap the
        // mutable `self.router` borrow.
        let surfaces: Vec<String> = self
            .subsystems
            .iter()
            .flat_map(|subsystem| {
                subsystem
                    .executor
                    .accept_ports()
                    .into_iter()
                    .map(move |port| format!("{}.{}", subsystem.name, port))
            })
            .collect();
        for surface in surfaces {
            self.router.register_acceptor(surface);
        }
    }

    // RSC-3.5e.1: `router_ref()` (FlowRouter-only test introspection) removed —
    // counters are queried directly on the ExchangePlane routing backend
    // (e.g. `self.router.unrouted_message_total()`).

    /// Send a message directly into the flow router (for testing/external injection).
    pub fn send_to_router(&mut self, source_key: &str, payload: Value) {
        self.router.send(source_key, payload);
    }

    /// Send an occurrence-addressed MessageTransfer directly into the flow
    /// router (RSC-3.3c D4; for testing/external injection). The named
    /// `target` resolves against the registered accepting surfaces when no
    /// declared flow matches `source_key` — see
    /// [`ExchangePlane::send_message`](crate::exchange::ExchangePlane::send_message).
    pub fn send_message_to_router(&mut self, source_key: &str, target: &str, payload: Value) {
        self.router.send_message(source_key, target, payload);
    }

    /// The runtime error recorded when the FlowRouter dropped messages at
    /// capacity in strict mode (`config.allow_lossy_flows == false`, the
    /// default). `None` while no loss has occurred (or in lossy mode).
    ///
    /// `step()` itself keeps returning snapshots (session contract — no
    /// error channel), with the loss also visible per tick in
    /// [`ExecutionSnapshot::flow_drop_warnings`]; once this is set,
    /// [`step_until`](Self::step_until) and
    /// [`run_to_completion`](Self::run_to_completion) stop stepping.
    pub fn flow_error(&self) -> Option<&str> {
        self.flow_error.as_deref()
    }

    /// Set pre-compiled constraints for live monitoring.
    ///
    /// Takes `Arc` so callers can pass the salsa-cached set directly
    /// (see `sysml_ide_db::precompiled_constraints`). Orchestrator clones
    /// (fork-at-tick) share the same underlying allocation.
    pub fn set_constraints(&mut self, constraints: std::sync::Arc<PrecompiledConstraintSet>) {
        self.constraints = Some(constraints);
    }

    /// Schedule a timed event.
    pub fn schedule_event(
        &mut self,
        time_ms: f64,
        subsystem: impl Into<String>,
        event: impl Into<String>,
    ) {
        let ev = ScheduledEvent {
            time_ms,
            target_subsystem: subsystem.into(),
            event: event.into(),
        };
        // The queue is maintained sorted ascending by time (drain_due_events
        // relies on this via partition_point). Insert at the upper bound of the
        // equal-time run to preserve insertion order among same-instant events
        // (matching the old stable sort), in O(log n + shift) instead of
        // re-sorting the whole vec O(n log n) on every insertion.
        let idx = self
            .event_schedule
            .partition_point(|e| e.time_ms <= time_ms);
        self.event_schedule.insert(idx, ev);
    }

    /// Inject an event into a named subsystem immediately (on next step).
    pub fn inject_event(&mut self, subsystem: &str, event: &str) {
        // Schedule at the current time so it fires on the next step
        self.schedule_event(self.time_ms, subsystem, event);
    }

    /// Get the names of all subsystems.
    pub fn subsystem_names(&self) -> Vec<String> {
        self.subsystems.iter().map(|s| s.name.clone()).collect()
    }

    /// Get a read-only slice of all subsystems.
    pub fn subsystems(&self) -> &[Subsystem] {
        &self.subsystems
    }

    /// Get a mutable slice of all subsystems.
    pub fn subsystems_mut(&mut self) -> &mut [Subsystem] {
        &mut self.subsystems
    }

    /// Get the current tick.
    pub fn tick(&self) -> u64 {
        self.tick
    }

    /// Get the current simulation time.
    pub fn time_ms(&self) -> f64 {
        self.time_ms
    }

    /// Get the execution trace.
    pub fn trace(&self) -> &[ExecutionSnapshot] {
        &self.trace
    }

    /// Check if all subsystems have completed.
    pub fn is_completed(&self) -> bool {
        self.subsystems.iter().all(|s| s.executor.is_completed())
    }

    /// Produce a deep-copy of this orchestrator at its current tick.
    ///
    /// Parent and child share no mutable state — stepping one does not affect
    /// the other. All subsystems are cloned via `Executor::clone_boxed`; flow
    /// router, event schedule, trace, clocks, and constraint state are all
    /// cloned as well.
    ///
    /// Used by `sysml.sessions.fork` to branch a running simulation.
    pub fn fork(&self) -> Self {
        self.clone()
    }

    /// Strip all "record-of-past-ticks" observational state: the
    /// execution trace, the occurrence tracker's history, and the
    /// causation ring buffer.
    ///
    /// These are all logs of what already happened, not execution
    /// *state* — a rewind archive slot needs the latter (to fork from
    /// tick N) but not the former (unbounded per-tick bookkeeping
    /// deep-cloned into every archive slot is what OOM-crashed the
    /// host: ~180 KB/tick of occurrences × 256 slots, on top of the
    /// trace). Does NOT touch `occurrence_registry` — that's the
    /// spec-normative, Arc-shared occurrence registry backing
    /// `create`/`destroy`/`isDuring`/`addNew`/`addNewAt` semantics, not
    /// observational bookkeeping.
    fn strip_observational_history(&mut self) {
        self.trace.clear();
        self.trace.shrink_to_fit();
        self.occurrence_tracker.reset();
        self.causation.clear();
    }

    /// Clone for archival storage — skips the trace/occurrence/causation
    /// history.
    ///
    /// Intended for per-tick archives (e.g.
    /// `RuntimeSession.orchestrator_archive`), whose job is only to
    /// support "rewind and fork from tick N". That history in each
    /// archived orchestrator would then accumulate quadratically
    /// (archive_cap × per-tick-history × snapshot_size), which on
    /// large multi-subsystem workloads reached multi-GB before the
    /// session was even evicted. Archive consumers that need the
    /// trace/occurrences/causation can subscribe to live snapshots
    /// instead — the forked session will build its own history as it
    /// runs.
    pub fn fork_for_archive(&self) -> Self {
        let mut clone = self.clone();
        clone.strip_observational_history();
        clone
    }

    /// Reset the orchestrator to initial state.
    pub fn reset(&mut self) {
        for s in &mut self.subsystems {
            s.executor.reset_executor();
        }
        // Reset the occurrence registry
        {
            let mut reg = self
                .occurrence_registry
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            *reg = sysml_core::occurrence::OccurrenceRegistry::new();
        }
        let mut new_ctx = EvalContext::new();
        new_ctx.occurrence_registry = Some(self.occurrence_registry.clone());
        // Keep slot routing alive across reset (RSC-2.2): the slot table is
        // compile-minted state, not run state — the fresh context must keep
        // writing through to it.
        if self.context.slots.is_some() {
            new_ctx.slots = Some(Arc::clone(&self.slot_store));
        }
        if self.context.slot_reader.is_some() {
            new_ctx.slot_reader = Some(Arc::clone(&self.slot_store));
        }
        self.context = new_ctx;
        self.tick = 0;
        self.time_ms = 0.0;
        self.trace.clear();
        self.event_schedule.clear();
        self.universal_clock.reset();
        self.clock_registry.reset_all();
        for (_, detector) in self.crossing_detectors.values_mut() {
            detector.reset();
        }
        self.last_crossing_events.clear();
        self.step_size_crossing_ticks.clear();
        self.succession_queue.reset();
        self.occurrence_tracker.reset();
        self.flow_error = None;
        // NOTE: routing plane intentionally not reset (pre-3.5e behaviour;
        // revisit separately). RSC-3.5e.2 preserved this — the legacy
        // RoutingPlane seam also skipped routing-plane reset; adding one here
        // would be a behaviour change, out of scope for the deletion.
        // `ref_resolve_cache` is intentionally NOT cleared here: per
        // ADR-011 §6 (S3.T14) its lifetime is tied to the elaborated
        // graph revision via the salsa-cached Arc the orchestrator
        // was built with — clearing on session reset would wipe the
        // populated cache that sibling orchestrators on the same
        // revision share. When the graph revision actually changes
        // (file edit), salsa hands out a new empty Arc on the next
        // `build_workspace_orchestrator`; the old Arc drops along
        // with its compile results.
    }

    /// Step the entire system forward by one tick.
    ///
    /// The tick loop:
    /// 1. Advance tick + time
    /// 2. Drain scheduled events due at this time
    /// 3. For each subsystem: sync context → step → sync back → collect sends
    /// 4. Route messages through FlowRouter
    /// 5. Evaluate constraints
    /// 6. Record snapshot
    /// RSC-5.4 (D-5.0.7): build the per-variable measurement map from the slot
    /// store's `m_ref`s. One entry per m_ref-bearing slot under BOTH name
    /// spellings (canonical + runtime, matching the slot writeback) so snapshot
    /// consumers join by whatever variable key they hold. Dimension is always
    /// present; `unit` is `Some` only for explicit-`[unit]` slots.
    fn build_value_units(&self) -> Arc<HashMap<String, ValueMeasurement>> {
        let store = self.slot_store.read().unwrap_or_else(|p| p.into_inner());
        let mut map: HashMap<String, ValueMeasurement> = HashMap::new();
        for (_, meta, _) in store.iter() {
            let Some(m) = &meta.m_ref else { continue };
            let vm = ValueMeasurement {
                dimension: m.dimension,
                unit: m.unit.as_ref().map(|u| u.to_string()),
            };
            map.insert(meta.canonical_name.to_string(), vm.clone());
            map.insert(meta.runtime_name.to_string(), vm);
        }
        Arc::new(map)
    }

    /// One simulation tick. `build_full` selects whether the genuinely
    /// expensive, purely-cosmetic snapshot fields (`resolved_refs`,
    /// `causation_links`, `guard_diagnoses`, `port_values`, `derivatives`)
    /// are computed and whether a registered observer is notified. The
    /// behaviour-bearing work (physics/ODE/SM stepping, routing, bookkeeping,
    /// constraint evaluation, trace thinning) runs identically either way, so
    /// a `build_full = false` ("light") tick advances state byte-identically
    /// but produces a snapshot with those five fields defaulted — see OPT #3
    fn step_inner(&mut self, build_full: bool) -> ExecutionSnapshot {
        // RSC-5.4: build the per-variable measurement map once (m_ref is
        // immutable post-mint) and reuse the Arc on every snapshot.
        if self.value_units_cache.is_none() {
            self.value_units_cache = Some(self.build_value_units());
        }
        let mut _pt = std::time::Instant::now();
        self.tick += 1;
        self.time_ms += self.config.dt_ms;

        // Update time in shared context
        self.context.set("t_ms", Value::Float(self.time_ms));
        self.context.set("tick", Value::Int(self.tick as i64));

        // Advance universal clock (Clocks.kerml) and expose in context
        let dt_seconds = self.config.dt_ms / 1000.0;
        self.universal_clock.advance(dt_seconds);
        self.context.set(
            "__clock_time".to_owned(),
            Value::Float(self.universal_clock.current_time),
        );

        // Advance all local clocks (Feature 10.1: multi-rate clocks)
        self.clock_registry.advance_all(dt_seconds);

        // RouteFirst: route pending messages from previous tick BEFORE stepping
        // This converts flow deliveries into port events that state machines can trigger on
        let early_delivered = if self.config.tick_strategy == TickStrategy::RouteFirst {
            // RSC-3.3c U1 (labeled extension): pull-initiate parked
            // pull-mode transfers for every accept port armed in its SM's
            // current state — "delivery happens when the target's accept
            // becomes enabled". Pulled transfers join this tick's port
            // events exactly like pushed deliveries.
            let pull_targets: Vec<String> = self
                .subsystems
                .iter()
                .flat_map(|s| {
                    s.executor
                        .armed_accept_ports()
                        .into_iter()
                        .map(move |port| format!("{}.{}", s.name, port))
                })
                .collect();
            let mut delivered: Vec<FlowMessage> = Vec::new();
            for target in pull_targets {
                delivered.extend(self.router.pull(&target));
            }
            delivered.extend(self.router.route_pending());
            // Convert delivered messages to port events for subsystems so the
            // SM port triggers fire THIS tick (the RouteFirst contract). GAP 2:
            // the convert now reconciles the owner key to the SM subsystem name.
            self.convert_deliveries_to_port_events(&delivered);
            delivered
        } else {
            Vec::new()
        };

        // 0. Step Physics executors first — conservation constraints, effort equalities.
        //    These write computed port values (currents, voltages, etc.) into the context
        //    so that ODE solvers and state machines see physics-consistent values.
        let t_seconds = self.time_ms / 1000.0;
        let dt_seconds = self.config.dt_ms / 1000.0;

        // Whether a slot store is attached to the master context. Compiler-built
        // orchestrators always attach one (so slot reads/writebacks are live);
        // hand-built NON-prefixed orchestrators may run without one and stay on
        // the legacy writeback path below. (The RSC-2.2 `enabled` rollback flag
        // was removed in RSC-3.5f.3 — routing is unconditional when attached.)
        let slots_enabled = self.context.slots.is_some();

        // RSC-4.1: positional within-phase permutation of the subsystem indices
        // (topological order per phase). The identity at intra_phase_edges=0, so
        // the per-phase passes below stay byte-identical; where a phase has a
        // same-phase write→read edge, the producer is visited before its consumer.
        // The permutation is precomputed once at build (B4); copy the flat slice
        // to own it across the `&mut self.subsystems` loops below (the private
        // `remap` field can't be disjoint-field-borrowed across the module seam).
        // A hand-built orchestrator that never ran `bind_expression_slots` has an
        // unbuilt (empty) schedule — it carries no topological constraints, so the
        // identity `Vec` order is its correct order. The `== len` guard is exact:
        // a built schedule always has `remap.len() == subsystems.len()`.
        let n = self.subsystems.len();
        let sched = self.execution_schedule.remap_order();
        let exec_order: Vec<usize> = if sched.len() == n {
            sched.to_vec()
        } else {
            (0..n).collect()
        };

        for &si in &exec_order {
            let subsystem = &mut self.subsystems[si];
            if subsystem.executor.phase() != ExecutionPhase::Physics {
                continue;
            }
            subsystem.executor.sync_context_in(&self.context);
            let tick_ctx = TickContext {
                t: t_seconds,
                dt: dt_seconds,
                tick: self.tick,
                context: &self.context,
                event: None,
                port_payloads: &[],
                local_clock_time: None,
            };
            let _output = subsystem.executor.tick(&tick_ctx);
            // Writeback: restricted to the physics write-set plus the
            // short-alias exchange plane. The legacy whole-local-context dump
            // (`sync_context_out`) was deleted with the string-identity cull —
            // slot-routed writeback is now unconditional for every executor.
            subsystem
                .executor
                .sync_context_out_slots(&mut self.context, crate::ode::SignalEvalMode::FreshState);
        }

        // 1. Step ContinuousDynamics executors first (ODE solvers) — writes continuous
        //    state to context so that state machine `when()` triggers see updated values.
        //    Capture pre-step snapshots for zero-crossing detection (Phase 15D).
        //    Keyed by SubsystemIndex (RSC-4.2 L39) — the exact registered
        //    subsystem, not a name that a co-located SM wrapper could share.
        let mut ode_snapshots: Vec<(SubsystemIndex, f64, Vec<f64>)> = Vec::new();
        // Capture ODE tick outputs during pre-pass to avoid double-stepping
        let mut ode_tick_outputs: HashMap<String, TickOutput> = HashMap::new();

        // RSC-2.4a: one thin slot-read view shared by every bypassing ODE
        // executor — registries + read-only slot handle, NO variable-map
        // clone. It carries no per-tick state, so it never goes stale and
        // is built lazily at most once per step. (The legacy per-prefix
        // scoped-context cache `ode_ctx_cache` was deleted with
        // `build_scoped_context` in RSC-3.5f.3.)
        let mut slot_read_ctx: Option<EvalContext> = None;

        for &si in &exec_order {
            let subsystem = &mut self.subsystems[si];
            if subsystem.executor.phase() != ExecutionPhase::ContinuousDynamics {
                continue;
            }
            // Capture pre-step state for crossing detection. `si` IS this
            // subsystem's SubsystemIndex (the raw index into `self.subsystems`).
            let ode_index = SubsystemIndex(si as u16);
            if self.crossing_detectors.contains_key(&ode_index) {
                if let Some(snap) = subsystem.executor.get_state_snapshot() {
                    ode_snapshots.push((ode_index, t_seconds, snap));
                }
            }
            let prefix_opt = subsystem.var_prefix.clone();
            // RSC-2.4a scoped-clone bypass: a fully slot-bound prefixed
            // executor reads through the thin slot view (RK4 stage values
            // still bind by name inside the RHS local context and keep
            // winning — slot reads only serve names absent from it).
            // Unprefixed executors keep reading the master context directly
            // (no clone exists to bypass).
            let bypass_scope =
                slots_enabled && prefix_opt.is_some() && subsystem.executor.scoped_view_bypass();
            let ctx_ref: &EvalContext = if bypass_scope {
                slot_read_ctx.get_or_insert_with(|| build_slot_read_context(&self.context))
            } else {
                match &prefix_opt {
                    // RSC-3.5f.3: `build_scoped_context` is deleted, so a
                    // prefixed ContinuousDynamics subsystem that is NOT
                    // bypass-eligible can no longer be scoped. Every prefixed
                    // executor the compiler mints is slot-attached + bypass-
                    // eligible (proven corpus-wide by the bypass census; the
                    // RS004 build-time gate rejects any ineligible prefixed
                    // subsystem). The only way to reach here is the retired
                    // no-slots footgun: a prefixed executor built via the raw
                    // `add_*_prefixed` API without `set_slot_store`.
                    Some(prefix) => unreachable!(
                        "RSC-3.5f.3: prefixed ContinuousDynamics subsystem \
                         '{}' (prefix '{prefix}') reached the deleted \
                         build_scoped_context branch — it is neither slot-\
                         attached nor bypass-eligible. Prefixed executors must \
                         be compiler-built (slot-backed); raw add_*_prefixed \
                         without set_slot_store is unsupported.",
                        subsystem.name
                    ),
                    None => &self.context,
                }
            };
            let tick_ctx = TickContext {
                t: t_seconds,
                dt: dt_seconds,
                tick: self.tick,
                context: ctx_ref,
                event: None,
                port_payloads: &[],
                local_clock_time: None,
            };
            let output = subsystem.executor.tick(&tick_ctx);
            // Writeback: slot-routed unconditionally. The executor's
            // compile-built write-set routes each state var by `SlotId`
            // (`set_slot` mirrors both name spellings into the legacy map).
            // The legacy `sync_context_out` + `temp`-context prefix-copy mirror
            // was deleted with the string-identity cull.
            subsystem
                .executor
                .sync_context_out_slots(&mut self.context, crate::ode::SignalEvalMode::FreshState);
            ode_tick_outputs.insert(subsystem.name.clone(), output);
        }

        // 1b. Zero-crossing detection (Phase 15D) + RSC-4.3 time-accurate re-step.
        //
        // RSC-4.2 L39: every lookup is a direct index into `self.subsystems` off
        // the `SubsystemIndex` captured at registration/wire time — no name-based
        // `find`, so a co-located SM wrapper and ODE sharing a display name (the
        // oscillator-fixture shape) can no longer make `y_end` resolve to the wrong
        // (empty-snapshot) subsystem, nor inject a crossing into "the first
        // StateMachine subsystem" when it belongs to a different instance's SM.
        //
        // RSC-4.3 (Option A, addendum): when a located crossing fires on a
        // re-step-ELIGIBLE ODE (`wire_when_crossings_for_pair` path),
        // `handle_crossing_detector` rolls the ODE back to `y_start`, integrates
        // to the crossing, fires the target SM MID-TICK (stashed in `fired_sm` so
        // step 3 records it without re-ticking), re-syncs the drive through the
        // slot store, and continues to tick-end under the reversed drive — so the
        // SM's `V_applied` flip is seen by the ODE continuation THIS tick. A
        // non-eligible detector (SSR path) keeps the pre-RSC-4.3 post-step
        // injection. An ODE with no firing detector never enters the re-step
        // branch, so the byte-identical gate is STRUCTURAL.
        let mut fired_sm: HashMap<SubsystemIndex, Vec<(String, TickOutput)>> = HashMap::new();
        for (ode_index, t_start, y_start) in &ode_snapshots {
            self.handle_crossing_detector(
                *ode_index,
                *t_start,
                dt_seconds,
                t_seconds,
                y_start,
                slots_enabled,
                &mut slot_read_ctx,
                &mut fired_sm,
            );
        }

        phase_mark("ode+crossing", &mut _pt);

        // 2. Drain scheduled events due at or before this time
        let due_events = self.drain_due_events();

        // 3. Step all non-Physics/non-ContinuousDynamics subsystems via trait dispatch
        let mut subsystem_states = HashMap::new();
        let mut all_sends: Vec<(String, String)> = Vec::new();

        for &si in &exec_order {
            let subsystem = &mut self.subsystems[si];
            let phase = subsystem.executor.phase();

            // Physics already stepped in step 0 — skip
            if phase == ExecutionPhase::Physics {
                continue;
            }

            // ContinuousDynamics already stepped in pre-pass — use cached output
            if phase == ExecutionPhase::ContinuousDynamics {
                if let Some(output) = ode_tick_outputs.remove(&subsystem.name) {
                    subsystem_states.insert(
                        subsystem.name.clone(),
                        SubsystemState {
                            name: subsystem.name.clone(),
                            kind: subsystem.executor.kind_label(),
                            current_state: output.current_state,
                            completed: output.completed,
                            available_transitions: output.available_transitions,
                            outputs: output.outputs,
                            sends: output.sends,
                            incoming_transition_trigger: output.incoming_trigger,
                            deferred_event_count: subsystem.executor.deferred_event_count(),
                            source_element_id: subsystem.source_element_id.clone(),
                        },
                    );
                }
                continue;
            }

            // RSC-4.3: an SM fired MID-TICK during the time-accurate re-step
            // already had its context synced in, `tick()` run, and writeback
            // done inside `handle_crossing_detector` (fire-exactly-once). Reuse
            // the stashed fire(s) instead of re-ticking; the shared tail below
            // (occurrence, sends, subsystem_state) then runs identically for
            // both paths, preserving trace/routing behaviour.
            //
            // F1: `fired_sm` stores a `Vec` — a tick can locate MULTIPLE
            // crossings targeting the SAME SM. Every fire genuinely happened
            // at its instant under Option A, so every fire's occurrence
            // records and every fire's sends/port_sends route: apply
            // `apply_fired_sm_tail` to every fire but the last (in fire-time
            // order — `located_crossings`/`restep_and_fire` already push
            // earliest-first), then let the LAST fire fall through to the
            // existing tail below unchanged (so the single-fire/no-restep case
            // is byte-identical to pre-F1-fix).
            let mut extra_outputs: Vec<String> = Vec::new();
            let mut extra_sends: Vec<String> = Vec::new();
            let (prev_state, mut output) = if let Some(mut fires) =
                fired_sm.remove(&SubsystemIndex(si as u16))
            {
                // FIFO watch (addendum; StatePerformances.kerml:97-113): a
                // non-crossing due event racing the crossing(s) for the SAME
                // SM in the SAME tick is left undrained by the mid-tick
                // fire(s). RSC-4.3 Wave 1 does not resolve
                // `earlierFirstIncomingTransferSort` for this case — fail hard
                // (RS014) rather than silently drop the due event. Corpus SMs
                // on the re-step path are crossing-only, so this never
                // triggers; a model that hits it must be escalated.
                if due_events
                    .iter()
                    .any(|e| e.target_subsystem == subsystem.name)
                {
                    panic!(
                        "RS014: state machine '{}' had BOTH a located zero-crossing (fired \
                         mid-tick) and a scheduled due event in the same tick. The FIFO \
                         ordering of a crossing vs a competing due event \
                         (earlierFirstIncomingTransferSort) is unresolved in RSC-4.3 Wave 1 \
                         — escalate rather than silently drop the due event.",
                        subsystem.name
                    );
                }
                let last = fires.pop().unwrap_or_else(|| {
                    unreachable!("F1: fired_sm never stores an empty Vec — pushed once per fire")
                });
                for (fire_prev, fire_out) in &fires {
                    Self::apply_fired_sm_tail(
                        &mut self.occurrence_tracker,
                        &mut self.router,
                        &self.owner_to_subsystems,
                        &self.context,
                        self.time_ms,
                        &subsystem.name,
                        phase,
                        fire_prev,
                        fire_out,
                        &mut all_sends,
                    );
                    extra_outputs.extend(fire_out.outputs.iter().cloned());
                    extra_sends.extend(fire_out.sends.iter().cloned());
                }
                last
            } else {
                // Find events targeted at this subsystem
                let targeted_event = due_events
                    .iter()
                    .find(|e| e.target_subsystem == subsystem.name)
                    .map(|e| e.event.as_str());

                // Check router for incoming messages (the source routing key is
                // the only field consumed below).
                let incoming_event: Option<String> =
                    self.router.peek(&subsystem.name).map(|m| m.source);

                // Collect port events
                let port_events_for_sub: Vec<String> = self
                    .port_events
                    .get(&subsystem.name)
                    .map(|events| {
                        events
                            .iter()
                            .map(|(port_name, _)| port_name.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                let port_event = port_events_for_sub.first().cloned();

                // Priority: targeted scheduled event > port event > incoming message
                let effective_event = targeted_event
                    .map(|s| s.to_owned())
                    .or(port_event)
                    .or(incoming_event);

                // Build port payloads slice for TickContext
                let empty_payloads: Vec<(String, Value)> = Vec::new();
                let port_payloads = self
                    .port_events
                    .get(&subsystem.name)
                    .unwrap_or(&empty_payloads);

                // Pre-step: sync shared context in (with prefix scoping if applicable).
                // RSC-2.4b scoped-clone bypass: a prefixed SM whose every tick-time
                // read is provably slot-servable (fully-bound guards/triggers, no
                // structured actions — see `StateMachineRunner::scoped_bypass_eligible`)
                // reads through the same thin slot view the ODE pre-pass uses
                // instead of the per-prefix scoped clone.
                let prefix_opt = subsystem.var_prefix.clone();
                let bypass_scope = slots_enabled
                    && prefix_opt.is_some()
                    && subsystem.executor.scoped_view_bypass();
                let ctx_ref: &EvalContext = if bypass_scope {
                    let thin = slot_read_ctx
                        .get_or_insert_with(|| build_slot_read_context(&self.context));
                    subsystem.executor.sync_context_in(thin);
                    thin
                } else {
                    match &prefix_opt {
                        // RSC-3.5f.3: `build_scoped_context` is deleted. Every
                        // prefixed StateMachine the compiler mints is slot-attached
                        // + bypass-eligible (proven corpus-wide by the bypass
                        // census; the RS004 build-time gate rejects any ineligible
                        // prefixed subsystem before a session can run). The only
                        // way to reach here is the retired no-slots footgun: a
                        // prefixed executor built via the raw `add_*_prefixed` API
                        // without `set_slot_store`.
                        Some(prefix) => unreachable!(
                            "RSC-3.5f.3: prefixed {:?} subsystem '{}' (prefix \
                             '{prefix}') reached the deleted build_scoped_context \
                             branch — it is neither slot-attached nor bypass-\
                             eligible. Prefixed executors must be compiler-built \
                             (slot-backed); raw add_*_prefixed without \
                             set_slot_store is unsupported.",
                            subsystem.executor.phase(),
                            subsystem.name
                        ),
                        None => {
                            subsystem.executor.sync_context_in(&self.context);
                            &self.context
                        }
                    }
                };

                // Capture previous state for occurrence tracking
                let prev_state = subsystem.executor.current_state_name().to_owned();

                // Build TickContext and step
                let tick_ctx = TickContext {
                    t: t_seconds,
                    dt: dt_seconds,
                    tick: self.tick,
                    context: ctx_ref,
                    event: effective_event.as_deref(),
                    port_payloads,
                    local_clock_time: self.clock_registry.local_time(&subsystem.name),
                };
                let output = subsystem.executor.tick(&tick_ctx);

                // Post-step: sync context back. Writeback is slot-routed
                // unconditionally — restricted to the executor's compiled
                // write-set + its runtime-dynamic keys, routed by `SlotId`
                // (`set_slot` mirrors both name spellings into the legacy map).
                // The legacy `sync_context_out` + `temp`-context prefix-copy mirror
                // was deleted with the string-identity cull. (Action executors are
                // a publish-nothing seam by construction — their
                // `sync_context_out_slots` returns true and writes nothing.)
                subsystem.executor.sync_context_out_slots(
                    &mut self.context,
                    crate::ode::SignalEvalMode::FreshState,
                );

                (prev_state, output)
            };

            // Occurrence lifecycle tracking (Feature 10.3)
            if output.current_state != prev_state && phase == ExecutionPhase::StateMachine {
                let time_s = self.time_ms / 1000.0;
                self.occurrence_tracker
                    .end(&subsystem.name, &prev_state, time_s, HashMap::new());
                let features: HashMap<String, Value> = self
                    .context
                    .variables
                    .iter()
                    .filter(|(k, _)| !crate::expressions::is_internal_var(k))
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                self.occurrence_tracker.begin(
                    OccurrenceKind::StateExecution,
                    &subsystem.name,
                    &output.current_state,
                    time_s,
                    features,
                );
            }

            // Per-step context dump — debug scrap, off by default because
            // it writes every variable in the EvalContext on every tick
            // (14k vars × N subsystems × 10 Hz on a large multi-subsystem model ⇒
            // ~60 MB of stderr per second, which has already OOM-killed
            // the service at least once). Opt in with SYSML_TRACE_CONTEXT=1.
            #[cfg(debug_assertions)]
            #[allow(clippy::print_stderr)]
            if std::env::var_os("SYSML_TRACE_CONTEXT").is_some() {
                let entries: Vec<_> = self
                    .context
                    .variables
                    .iter()
                    .filter(|(k, _)| k.as_str() != "t_ms" && k.as_str() != "tick")
                    .map(|(k, v)| format!("{}={:?}", k, v))
                    .collect();
                eprintln!(
                    "[ORCH] subsystem '{}' step synced context: [{}]",
                    subsystem.name,
                    entries.join(", ")
                );
            }

            // Collect sends for routing.
            //
            // The `send via <port>` strings in `output.sends` are now TRACE
            // ONLY — they remain on `SubsystemState.sends` (the snapshot
            // surface) but no longer drive routing. The payload-carrying
            // `output.port_sends` channel routes instead (below): a string
            // cannot carry a `Value`, so the Value channel is the one home for
            // the addressed-MessageTransfer payload. Bare event sends (no
            // `via`) still flow to `all_sends`.
            for send_event in &output.sends {
                if send_event.starts_with("send via ") {
                    continue;
                }
                all_sends.push((subsystem.name.clone(), send_event.clone()));
            }
            // SM-send (A-scalar): route each `send <payload> via <port>` as an
            // addressed MessageTransfer carrying the real evaluated payload.
            // Resolve the sender's owner-instance key (inverse of the GAP 2
            // `owner_to_subsystems` map) so the source key matches the
            // link-graph source the router routes on. The post-loop
            // `route_pending()` + `convert_deliveries_to_port_events` (GAP 2+3)
            // delivers it to the receiver's port_events — the path proven from
            // `send_to_router`, now driven by a compiled SM with a real Value.
            for (port, payload) in &output.port_sends {
                let owner = Self::owner_key_for(&self.owner_to_subsystems, &subsystem.name)
                    .unwrap_or_else(|| subsystem.name.clone());
                self.router
                    .send(&format!("{}.{}", owner, port), payload.clone());
            }
            // Route action messages to FlowRouter
            for (source_key, payload) in &output.messages {
                self.router.send(source_key, payload.clone());
            }
            // RSC-3.3c D4: occurrence-addressed messages (action send nodes
            // name their receiver). A declared flow on the source still
            // wins; otherwise the named target resolves against the
            // registered accepting surfaces.
            for (source_key, target, payload) in &output.addressed_messages {
                self.router
                    .send_message(source_key, target, payload.clone());
            }

            // F1: fold in the trace-surface contributions (`outputs`/`sends`)
            // from any EARLIER fire(s) this tick (empty for the single-fire/
            // no-restep case, so this is a no-op there). `apply_fired_sm_tail`
            // already routed their `port_sends`/`messages`/`addressed_messages`
            // above via the loop that populated `extra_outputs`/`extra_sends`;
            // this only concerns the `SubsystemState` trace fields, so the
            // snapshot doesn't silently drop an earlier fire's trace entries
            // either.
            extra_outputs.extend(std::mem::take(&mut output.outputs));
            extra_sends.extend(std::mem::take(&mut output.sends));

            subsystem_states.insert(
                subsystem.name.clone(),
                SubsystemState {
                    name: subsystem.name.clone(),
                    kind: subsystem.executor.kind_label(),
                    current_state: output.current_state,
                    completed: output.completed,
                    available_transitions: output.available_transitions,
                    outputs: extra_outputs,
                    sends: extra_sends,
                    incoming_transition_trigger: output.incoming_trigger,
                    deferred_event_count: subsystem.executor.deferred_event_count(),
                    source_element_id: subsystem.source_element_id.clone(),
                },
            );
        }

        phase_mark("sm+subsystems", &mut _pt);

        // 2a. Populate state introspection variables for spec functions
        // allSubstatePerformances() and allSubtransitionPerformances().
        {
            // Iterate `subsystem_states` (a HashMap) in sorted name order so the
            // bookkeeping lists are byte-identical across builds — HashMap
            // iteration order is per-process-random and would otherwise make the
            // list ORDER nondeterministic (WS-C build determinism). The list is a
            // set of active substates / recent transitions, so name order is a
            // fine canonical ordering.
            let mut sorted_states: Vec<&SubsystemState> = subsystem_states.values().collect();
            sorted_states.sort_by(|a, b| a.name.cmp(&b.name));

            let active_substates: Vec<Value> = sorted_states
                .iter()
                .filter(|s| s.kind == "stateMachine" && !s.completed)
                .map(|s| Value::String(s.current_state.clone()))
                .collect();
            self.context
                .set("__active_substates", Value::List(active_substates));

            let mut recent_transitions: Vec<Value> = Vec::new();
            if let Some(prev_snap) = self.trace.last() {
                for state in &sorted_states {
                    if let Some(prev_state) = prev_snap.subsystem_states.get(&state.name) {
                        if prev_state.current_state != state.current_state {
                            recent_transitions.push(Value::String(format!(
                                "{}: {} -> {}",
                                state.name, prev_state.current_state, state.current_state
                            )));
                        }
                    }
                }
            }
            self.context
                .set("__recent_transitions", Value::List(recent_transitions));
        }

        // 2b. Update flow gate variables based on SM current states.
        // When an SM enters a gating state, its flow gate is set to false.
        if !self.flow_gates.is_empty() {
            for (sm_name, gating_states) in &self.flow_gates {
                if let Some(ss) = subsystem_states.get(sm_name) {
                    let in_gating_state = gating_states.iter().any(|gs| gs == &ss.current_state);
                    let gate_var = format!("{}.__flow_gate", sm_name);
                    self.context.set(gate_var, Value::Bool(!in_gating_state));
                }
            }
        }

        // 2c. Succession queue: notify completed transitions, drain ready successors (Feature 10.2)
        {
            let current_time_seconds = self.time_ms / 1000.0;

            if let Some(prev) = self.trace.last() {
                for (name, state) in &subsystem_states {
                    if let Some(prev_state) = prev.subsystem_states.get(name) {
                        if prev_state.current_state != state.current_state {
                            self.succession_queue
                                .notify_completed(&prev_state.current_state, current_time_seconds);
                            let qualified = format!("{}.{}", name, prev_state.current_state);
                            self.succession_queue
                                .notify_completed(&qualified, current_time_seconds);
                        }
                    }
                    if state.completed {
                        self.succession_queue
                            .notify_completed(name, current_time_seconds);
                    }
                }
            }

            let ready = self
                .succession_queue
                .drain_ready(current_time_seconds, &self.context);
            let mut succession_events: Vec<(String, String)> = Vec::new();
            let sm_names: Vec<String> = self
                .subsystems
                .iter()
                .filter(|s| s.executor.phase() == ExecutionPhase::StateMachine)
                .map(|s| s.name.clone())
                .collect();
            for successor in ready {
                if let Some((target, event)) = successor.split_once('.') {
                    succession_events.push((target.to_owned(), event.to_owned()));
                } else {
                    for sm_name in &sm_names {
                        succession_events.push((sm_name.clone(), successor.clone()));
                    }
                }
            }
            for (target, event) in succession_events {
                self.schedule_event(self.time_ms, &target, &event);
            }
        }

        // GAP 3 (clear reposition — the lifecycle trap): evict the port events
        // the state machines just consumed (their read happened in the subsystem
        // loop above) BEFORE re-populating from this tick's deliveries. Was at
        // after-route, which silently dropped every StepFirst delivery (cleared
        // before any convert could populate it). Byte-identical for no-message
        // models — `port_events` stays empty the whole tick.
        self.port_events.clear();

        phase_mark("bookkeeping(2a-2c)", &mut _pt);

        // 3. Route all pending messages, then convert deliveries into
        //    `port_events` for the NEXT tick's state-machine read (GAP 3 —
        //    StepFirst now feeds port_events, giving port-triggered SMs a
        //    1-tick delivery latency; the spec is silent on tick granularity).
        //    RouteFirst already converted its pre-step (early) deliveries above,
        //    so only the mid-tick deliveries are converted here — converting all
        //    of `delivered` would re-populate the already-consumed early ones.
        let delivered = if self.config.tick_strategy == TickStrategy::StepFirst {
            let new_delivered = self.router.route_pending();
            self.convert_deliveries_to_port_events(&new_delivered);
            new_delivered
        } else {
            let new_delivered = self.router.route_pending();
            self.convert_deliveries_to_port_events(&new_delivered);
            let mut all = early_delivered;
            all.extend(new_delivered);
            all
        };

        // 3x. Surface FlowRouter capacity loss (RSC-1.5 / G14): message
        // drops must never be silent. Evictions recorded since the last
        // tick become human-readable snapshot warnings; in strict mode
        // (default) the first loss also records `flow_error`, halting
        // `step_until` / `run_to_completion`.
        let mut flow_drop_warnings: Vec<String> = {
            let capacity = self.router.max_queue_size();
            self.router
                .take_recent_drops()
                .into_iter()
                .map(|(source, count)| {
                    format!(
                        "flow '{source}': dropped {count} message(s) (pending queue at capacity {capacity})"
                    )
                })
                .collect()
        };
        flow_drop_warnings.sort();
        if !self.config.allow_lossy_flows
            && !flow_drop_warnings.is_empty()
            && self.flow_error.is_none()
        {
            self.flow_error = Some(format!(
                "tick {}: {}",
                self.tick,
                flow_drop_warnings.join("; ")
            ));
            #[cfg(feature = "tracing")]
            tracing::error!(
                tick = self.tick,
                error = self.flow_error.as_deref().unwrap_or_default(),
                "flow capacity loss in strict mode — run will halt"
            );
        }

        // 3y. RSC-3.3b D3: payload-conformance rejections (derived endpoint
        // typing, Transfers.kerml:100-118) surface like capacity loss —
        // snapshot warnings, and in strict mode (default) a fail-hard
        // flow_error that halts step_until / run_to_completion.
        {
            let rejections = self.router.take_recent_conformance_rejections();
            if !rejections.is_empty() {
                for r in &rejections {
                    flow_drop_warnings.push(format!("flow payload conformance: {r}"));
                }
                flow_drop_warnings.sort();
                if !self.config.allow_lossy_flows && self.flow_error.is_none() {
                    self.flow_error = Some(format!(
                        "tick {}: flow payload conformance violation: {}",
                        self.tick,
                        rejections.join("; ")
                    ));
                    #[cfg(feature = "tracing")]
                    tracing::error!(
                        tick = self.tick,
                        error = self.flow_error.as_deref().unwrap_or_default(),
                        "flow payload conformance violation in strict mode — run will halt"
                    );
                }
            }
        }

        // 3z. RSC-3.3b D2: per-TRANSFER move semantics — for every delivered
        // message on an `is_move` link, the payload leaves the source: the
        // source port's payload feature value(s) are cleared in the registry
        // and mirrored onto the matching `{owner}.{port}.{feature}` slots
        // (the RSC-3.2 signal-feature slot plane). Where the source port has
        // no features — the whole exercised corpus today (L23/L25) — there
        // is nothing to clear and delivery proceeds unchanged. Note the
        // context→out-port mirror in `propagate_port_values` (3a) may
        // republish a value the context still holds; the spec obligation is
        // that the TRANSFERRED payload leaves the source port, which the
        // clear models.
        self.apply_move_semantics(&delivered);

        // 3a. Propagate port values through flow connections
        self.propagate_port_values();

        phase_mark("routing(3-3a)", &mut _pt);

        // 3b. Evaluate computed expressions (derived/aggregate values)
        //     Gated expressions check their gate variable first — if falsy,
        //     target is set to 0.0 (flow gating for trip/disconnect).
        self.evaluate_computed_expressions();

        // 3c. Convergence iteration (optional — re-step if context changed)
        //
        // RSC-4.2 (D-4.2.1 item 2): the re-tick writeback routes through the
        // unified slot path (`sync_context_out_slots` +
        // `SignalEvalMode::FullAccumulated`), mirroring the main per-phase
        // loop. This FIXED a confirmed string-collision bug (steward-approved
        // 2026-07-01):
        //
        // The former legacy `sync_context_out` on a prefixed ODE wrote only the
        // BARE state name (`bimetalTemp`) via `EvalContext::set`. That bare
        // name is NOT slot-bound (the slot is `circuit1.bimetalTemp`), so the
        // write landed in the map only and collided across all ten circuits
        // (last writer = `circuit10`). The prefixed slot `circuit1.bimetalTemp`
        // was therefore NEVER updated by the convergence loop — it kept the
        // value the FreshState main loop routed pre-convergence, so convergence
        // SILENTLY NO-OP'd on every instance-multiplied ODE state. Routing by
        // `SlotId` (`write_states`) updates each per-instance state slot with
        // the re-integrated converged value — the first working per-instance
        // convergence. `FullAccumulated` evaluates any signal expressions
        // against the full master context (this iteration's computed-expr +
        // peer writes — the coupling that IS the fixpoint).
        //
        // The string-identity cull deleted the legacy `sync_context_out`
        // fallback here too: `sync_context_out_slots` is now called
        // unconditionally, exactly as in the main per-phase loop. Hybrid routes
        // its continuous state + SM half by `SlotId` (RSC-4.3); physics routes
        // through its own explicit name-keyed path (ledger L31).
        if self.config.convergence_max_iterations > 0 {
            let t_seconds = self.time_ms / 1000.0;
            let dt_seconds = self.config.dt_ms / 1000.0;

            for _iter in 0..self.config.convergence_max_iterations {
                let prev_vars = self.context.variables.clone();

                // Re-step all subsystems with updated context. RSC-4.1: same
                // per-phase topological order as the main passes (via the shared
                // `exec_order` remap) — finishes D-4.2.1's "iterate the
                // phase-ordered schedule" dependency. Byte-identical at
                // intra_phase_edges=0 (remap is the identity there).
                for &si in &exec_order {
                    let subsystem = &mut self.subsystems[si];
                    // GAP 3: do NOT re-tick a continuous-dynamics executor in
                    // the convergence loop. It is a stateful time integrator —
                    // its `tick` advances `self.state` by one `dt` on every
                    // call from the CURRENT state (it never re-reads state from
                    // the shared context). Re-ticking it here therefore advanced
                    // the ODE an extra `dt` per iteration, so merely adding a
                    // physics topology (which auto-enables convergence) silently
                    // over-advanced the ODE by `convergence_max_iterations`
                    // steps — a disconnected bond-graph perturbed the trajectory
                    // (relief event 1593 → 399 ticks in the repro; 1593/399 ≈
                    // 4 = 1 main pass + 3 convergence iterations). Each ODE is
                    // integrated exactly once per tick, in the main
                    // ContinuousDynamics pass with the FreshState drive; the
                    // convergence loop resolves only the ALGEBRAIC coupling
                    // (SM outputs, physics, computed expressions) to a fixpoint,
                    // reading each ODE's already-published state held constant.
                    if subsystem.executor.phase() == ExecutionPhase::ContinuousDynamics {
                        continue;
                    }
                    subsystem.executor.sync_context_in(&self.context);
                    let tick_ctx = TickContext {
                        t: t_seconds,
                        dt: dt_seconds,
                        tick: self.tick,
                        context: &self.context,
                        event: None,
                        port_payloads: &[],
                        local_clock_time: None,
                    };
                    let _output = subsystem.executor.tick(&tick_ctx);
                    subsystem.executor.sync_context_out_slots(
                        &mut self.context,
                        crate::ode::SignalEvalMode::FullAccumulated,
                    );
                }

                // Re-evaluate computed expressions (with gating)
                self.evaluate_computed_expressions();

                // Check convergence: max relative change across float variables
                let mut max_change: f64 = 0.0;
                for (key, new_val) in self.context.variables.iter() {
                    if let (Some(Value::Float(old)), Value::Float(new)) =
                        (prev_vars.get(key), new_val)
                    {
                        let denom = old.abs().max(1e-15);
                        let rel = ((new - old) / denom).abs();
                        if rel > max_change {
                            max_change = rel;
                        }
                    }
                }

                if max_change < self.config.convergence_epsilon {
                    break;
                }
            }
        }

        phase_mark("computed_exprs(3b-3c)", &mut _pt);

        // 4. Evaluate constraints
        let constraint_results = self.evaluate_constraints();
        phase_mark("constraints", &mut _pt);

        // 5. Guard diagnoses (OPT #3: pure reporting — skipped in a light
        //    `advance()` tick; `run_to_completion` recomputes it once on the
        //    final snapshot).
        let guard_diagnoses = if build_full {
            self.collect_guard_diagnoses()
        } else {
            Vec::new()
        };

        // 6. Causation links (OPT #3: pure reporting — skipped in light ticks).
        let causation_links = if build_full {
            self.compute_causation()
        } else {
            Vec::new()
        };

        // 7. Check completion
        let completed = self.subsystems.iter().all(|s| s.executor.is_completed());

        // Collect port values / derivatives / resolved refs for the snapshot.
        // OPT #3: these three (with guard_diagnoses + causation_links above)
        // are pure REPORTING — skipped on a light `advance()` tick and
        // recomputed once by `run_to_completion` on the final snapshot. The
        // bodies live in `&self` helpers so both paths share one implementation.
        let port_values = if build_full {
            self.collect_port_values()
        } else {
            HashMap::new()
        };
        let derivatives = if build_full {
            self.collect_derivatives()
        } else {
            HashMap::new()
        };
        let resolved_refs = if build_full {
            self.compute_resolved_refs()
        } else {
            HashMap::new()
        };
        let step_size_health = if build_full {
            self.compute_step_size_health()
        } else {
            Vec::new()
        };

        let snapshot = ExecutionSnapshot {
            tick: self.tick,
            time_ms: self.time_ms,
            subsystem_states,
            variables: Arc::clone(&self.context.variables),
            messages: delivered,
            constraint_results,
            assertion_checkpoints: Vec::new(),
            guard_diagnoses,
            causation_links,
            completed,
            port_values,
            derivatives,
            resolved_refs,
            flow_drop_warnings,
            value_units: Arc::clone(
                self.value_units_cache
                    .as_ref()
                    .expect("value_units_cache populated at step() entry"),
            ),
            step_size_health,
        };

        // Snapshot thinning: only record every Nth tick.
        if self.config.snapshot_interval <= 1 || self.tick % self.config.snapshot_interval == 0 {
            self.trace.push(snapshot.clone());
            // Ring-buffer eviction. `Vec::remove(0)` shifts every
            // remaining element, so an in-loop `while .. remove(0)`
            // pattern is O(n²) when the cap is continuously exceeded;
            // a single `drain` call is O(n) and Rust compiles it to a
            // memmove. Overflow is almost always 1 (one push past the
            // cap), but this keeps the math honest if the cap ever
            // shrinks mid-run.
            if let Some(max_len) = self.config.max_trace_length {
                if self.trace.len() > max_len {
                    let overflow = self.trace.len() - max_len;
                    self.trace.drain(0..overflow);
                }
            }
        }
        // Push-notify any registered observer. Fires on every tick, not
        // just retained-trace ticks, so the streaming layer never drops a
        // frame to match snapshot-thinning. OPT #3: only a FULL build
        // notifies — a light `advance()` snapshot must never reach a
        // subscriber (that would be a silent frame degradation), and
        // `advance()` is only ever used when no observer is present.
        if build_full {
            self.snapshot_observer.notify(&snapshot);
        }
        phase_mark("finalize+snapshot", &mut _pt);
        snapshot
    }

    /// Advance one tick, building and returning the FULL `ExecutionSnapshot`.
    /// This is the unchanged, contract-bearing drive method: it notifies
    /// observers and records the full snapshot per the thinning interval.
    pub fn step(&mut self) -> ExecutionSnapshot {
        self.step_inner(true)
    }

    /// OPT #3 (runtime-hotpath-perf-plan): advance one tick WITHOUT computing
    /// the purely-cosmetic snapshot fields (`resolved_refs`, `causation_links`,
    /// `guard_diagnoses`, `port_values`, `derivatives`) and without notifying
    /// observers. State advances byte-identically to [`step`](Self::step);
    /// only the reporting is deferred. A light snapshot is still recorded to
    /// the trace per the thinning interval so the *next* tick's
    /// `trace.last()`-dependent bookkeeping (recent-transition detection, the
    /// succession queue) and `compute_causation` see the correct previous tick.
    ///
    /// MUST NOT be used while a `snapshot_observer` is registered (the
    /// streaming layer is owed a full frame every tick); callers gate on
    /// observer presence. Returns a lightweight [`TickResult`].
    pub fn advance(&mut self) -> TickResult {
        let snap = self.step_inner(false);
        TickResult {
            tick: snap.tick,
            time_ms: snap.time_ms,
            completed: snap.completed,
            flow_error: self.flow_error.clone(),
        }
    }

    /// Collect the per-tick port-feature value map for a snapshot. Pure
    /// reporting (OPT #3): shared by [`step`](Self::step) and the final-snapshot
    /// upgrade in [`run_to_completion`](Self::run_to_completion).
    fn collect_port_values(&self) -> HashMap<String, HashMap<String, Value>> {
        self.port_registry
            .as_ref()
            .map(|reg| {
                reg.iter()
                    .map(|(key, port)| {
                        let feats: HashMap<String, Value> = port
                            .features
                            .iter()
                            .map(|(name, feat)| (name.clone(), feat.value.clone()))
                            .collect();
                        (key.to_owned(), feats)
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Collect ODE derivatives from every subsystem for a snapshot. Non-ODE
    /// executors return an empty vec by default. Prefix-scoped instances write
    /// their derivatives under the prefixed state name so the keys line up with
    /// the corresponding scalar_vars entries the frontend already uses for
    /// state values. Pure reporting (OPT #3).
    fn collect_derivatives(&self) -> HashMap<String, f64> {
        let mut derivatives: HashMap<String, f64> = HashMap::new();
        for sub in &self.subsystems {
            let prefix = sub.var_prefix.as_deref();
            for (name, value) in sub.executor.current_derivatives() {
                let key = match prefix {
                    Some(p) if !p.is_empty() => format!("{p}.{name}"),
                    _ => name,
                };
                derivatives.insert(key, value);
            }
        }
        derivatives
    }

    /// Resolve every lazy `Value::Ref(id)` binding to its concrete live value
    /// using the same graph-walk the evaluator does at read time. Produces the
    /// side-map snapshot consumers overlay onto the raw variable bindings so UI
    /// projections can surface expression-resolved attributes (otherwise
    /// invisible — the raw Ref is not user-facing data and projects to nothing).
    ///
    /// Uses a snapshot-scoped compile cache (`ref_resolve_cache`, ADR-011 §6 /
    /// S3.T14) so expression ASTs are compiled once per elaborated-graph
    /// revision and re-evaluated each tick instead of re-walked every time. The
    /// context (variable bindings the expressions read) still changes every
    /// tick, so evaluation is live — only the IR is cached. Pure reporting
    /// (OPT #3): this is the priciest snapshot field and the main thing a light
    /// `advance()` tick skips.
    fn compute_resolved_refs(&self) -> HashMap<String, Value> {
        let ref_bindings: Vec<(String, sysml_core::ElementId)> = self
            .context
            .variables
            .iter()
            .filter_map(|(name, v)| match v {
                Value::Ref(id) => Some((name.clone(), id.clone())),
                _ => None,
            })
            .collect();
        ref_bindings
            .into_iter()
            .filter_map(|(name, id)| {
                crate::expressions::resolve_ref_value_cached(
                    &id,
                    &self.context,
                    &self.ref_resolve_cache,
                )
                .map(|resolved| (name, resolved))
            })
            .collect()
    }

    /// Run until a target time, returning all snapshots.
    pub fn step_until(&mut self, target_time_ms: f64) -> Vec<ExecutionSnapshot> {
        let mut snapshots = Vec::new();
        while self.time_ms < target_time_ms
            && self.tick < self.config.max_ticks
            && !self.is_completed()
            && self.flow_error.is_none()
        {
            snapshots.push(self.step());
        }
        snapshots
    }

    /// Run until completion or safety limit, returning the final snapshot.
    ///
    /// OPT #3 (runtime-hotpath-perf-plan): when no observer is registered, the
    /// intermediate ticks run as light [`advance`](Self::advance) ticks — the
    /// five expensive cosmetic snapshot fields are computed only ONCE, on the
    /// final snapshot, instead of on every discarded intermediate tick. State
    /// evolution is byte-identical to a `step()` loop (the only callers consume
    /// just this returned final snapshot — verified by the
    /// `run_to_completion_matches_step_loop` gate). If an observer IS present
    /// the streaming layer is owed a full frame per tick, so we stay on
    /// `step()` (the cadence keys on observer presence, never silently drops).
    /// Run-continuation predicate: keep advancing only while under the
    /// configured tick/time ceilings, not yet completed, and no flow error has
    /// halted the run. This is the ONE home for that condition — shared by
    /// [`run_to_completion`](Self::run_to_completion) and any external
    /// bulk-step loop over [`step`](Self::step) (e.g. the service's
    /// `sessions.step { ticks }`). Raw `step()` has no internal guard, so a
    /// loop over it MUST gate on this to honor the configured safety limits
    /// (fail-hard boundary — never blow past `max_ticks`/`max_time_ms`).
    pub fn can_run_more(&self) -> bool {
        self.within_run_bounds() && !self.is_completed()
    }

    /// Safety-bounds check WITHOUT the completion predicate: within the
    /// configured `max_ticks` / `max_time_ms` limits and not flow-errored.
    ///
    /// Exists for callers honouring an EXPLICIT tick budget (the session
    /// contract's bulk `sessions.step {ticks: N}`): plain ODE executors
    /// return `is_completed() = true` as an ABSTAIN under the `all()`
    /// rollup — completion is decided by whichever subsystem has a real
    /// notion of "done" (SM final state, discrete completion). A workspace
    /// with ZERO such subsystems therefore reports vacuous completion
    /// (steward-ruled framing; ledger L57), so gating an explicit budget
    /// on completion would stop such a run after one tick.
    /// `run_to_completion` keeps using [`can_run_more`].
    pub fn within_run_bounds(&self) -> bool {
        self.tick < self.config.max_ticks
            && self.time_ms < self.config.max_time_ms
            && self.flow_error.is_none()
    }

    pub fn run_to_completion(&mut self) -> Option<ExecutionSnapshot> {
        let has_observer = self.snapshot_observer.is_set();
        while self.can_run_more() {
            if has_observer {
                self.step();
            } else {
                self.advance();
            }
        }
        if has_observer {
            return self.trace.last().cloned();
        }
        // Upgrade the final light snapshot to full: recompute the five deferred
        // fields once from current state. `compute_causation` reads
        // `trace.last()` for the PREVIOUS tick, so pop the final entry first
        // (making the prior tick `last()`), fill it, then push it back.
        let mut last = self.trace.pop()?;
        last.guard_diagnoses = self.collect_guard_diagnoses();
        last.causation_links = self.compute_causation();
        last.port_values = self.collect_port_values();
        last.derivatives = self.collect_derivatives();
        last.resolved_refs = self.compute_resolved_refs();
        last.step_size_health = self.compute_step_size_health();
        self.trace.push(last.clone());
        Some(last)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Drain scheduled events that are due at or before the current time.
    fn drain_due_events(&mut self) -> Vec<ScheduledEvent> {
        let cutoff = self.time_ms;
        let split_idx = self.event_schedule.partition_point(|e| e.time_ms <= cutoff);
        self.event_schedule.drain(..split_idx).collect()
    }

    /// Collect guard diagnoses from all state machine subsystems.
    fn collect_guard_diagnoses(&self) -> Vec<crate::statemachine::GuardDiagnosis> {
        let mut all = Vec::new();
        for subsystem in &self.subsystems {
            all.extend(subsystem.executor.diagnose_guards(None));
        }
        all
    }

    /// Compute the per-ODE-subsystem step-size under-resolution advisory from
    /// the located-crossing tick history (P1 dt-under-resolution arc).
    ///
    /// Purely observational reporting — mirrors `collect_guard_diagnoses` and,
    /// like it, is only invoked on a full snapshot build. One entry per ODE
    /// subsystem that has a registered crossing detector (i.e. can oscillate);
    /// its advisory is `NotApplicable` until at least two located crossings
    /// have accumulated, at which point ticks/cycle is derived as
    /// `2 * mean(gap between consecutive crossing ticks)`.
    fn compute_step_size_health(
        &self,
    ) -> Vec<crate::step_size_advisory::SubsystemStepSizeHealth> {
        use crate::step_size_advisory::{StepSizeAdvisory, SubsystemStepSizeHealth};
        // Iterate the detectors (the subsystems that CAN oscillate) in
        // subsystem order for deterministic output.
        let mut out = Vec::new();
        for (idx, subsystem) in self.subsystems.iter().enumerate() {
            let ode_index = SubsystemIndex(idx as u16);
            if !self.crossing_detectors.contains_key(&ode_index) {
                continue;
            }
            let advisory = match self.step_size_crossing_ticks.get(&ode_index) {
                Some(ticks) if ticks.len() >= 2 => {
                    // Half-cycle = mean gap between consecutive crossing ticks;
                    // a full oscillation cycle spans two crossings (rising +
                    // falling), so ticks/cycle = 2 * mean(gap).
                    let first = *ticks.front().unwrap();
                    let last = *ticks.back().unwrap();
                    let n_gaps = (ticks.len() - 1) as f64;
                    let mean_gap = (last.saturating_sub(first)) as f64 / n_gaps;
                    let ticks_per_cycle = (2.0 * mean_gap).round() as u32;
                    StepSizeAdvisory::classify(ticks_per_cycle, self.config.dt_ms)
                }
                _ => StepSizeAdvisory::NotApplicable,
            };
            out.push(SubsystemStepSizeHealth {
                subsystem: subsystem.name.clone(),
                advisory,
            });
        }
        out
    }

    /// Compare previous context with current to find causation links.
    fn compute_causation(&self) -> Vec<CausationLink> {
        let prev_snapshot = self.trace.last();
        let Some(prev) = prev_snapshot else {
            return Vec::new();
        };

        // Diff the current context against the previous snapshot's raw variable
        // map directly. The common no-change-this-tick case now allocates
        // nothing and we avoid building a full EvalContext before knowing there
        // is a change. Semantically identical to the old
        // `prev_ctx.diff(&self.context)` (keys present in the current context
        // that are new or differ vs prev).
        let changes: std::collections::HashMap<String, Value> = self
            .context
            .variables
            .iter()
            .filter(|(k, v)| prev.variables.get(*k).map(|old| old != *v).unwrap_or(true))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if changes.is_empty() {
            return Vec::new();
        }

        // Only now (we know at least one variable changed) materialise the
        // previous context for guard re-evaluation.
        let prev_ctx = {
            let mut c = EvalContext::new();
            for (k, v) in prev.variables.iter() {
                c.set(k.clone(), v.clone());
            }
            c
        };

        // Memoise guard dependency analysis within this call: without this the
        // same guard string is re-analysed for every changed-var × subsystem ×
        // transition combination in the nested loops below.
        let mut guard_dep_cache: std::collections::HashMap<String, _> =
            std::collections::HashMap::new();

        let mut links = Vec::new();

        for (var_name, new_value) in &changes {
            if var_name == "t_ms" || var_name == "tick" {
                continue;
            }

            let old_value = prev.variables.get(var_name).cloned().unwrap_or(Value::Null);

            // Determine which subsystem wrote this variable via trait introspection
            let writer = self
                .subsystems
                .iter()
                .find(|s| {
                    s.executor
                        .eval_context()
                        .and_then(|ctx| ctx.get(var_name))
                        .map(|v| v != &old_value)
                        .unwrap_or(false)
                })
                .map(|s| s.name.clone())
                .unwrap_or_else(|| "unknown".to_owned());

            // Check which guards in OTHER subsystems reference this variable
            let mut affected = Vec::new();
            for subsystem in &self.subsystems {
                if subsystem.name == writer {
                    continue;
                }
                if let Some(transitions) = subsystem.executor.transitions() {
                    let current = subsystem.executor.current_state_name().to_owned();
                    for transition in transitions {
                        if transition.from != current {
                            continue;
                        }
                        if let Some(guard) = &transition.guard {
                            let deps = guard_dep_cache.entry(guard.clone()).or_insert_with(|| {
                                crate::expressions::analyze_guard_dependencies(guard)
                            });
                            if deps.contains(var_name) {
                                let was_satisfied =
                                    crate::statemachine::evaluate_guard(guard, &prev_ctx, None);
                                let now_satisfied =
                                    crate::statemachine::evaluate_guard(guard, &self.context, None);
                                let newly_satisfied = !was_satisfied && now_satisfied;

                                affected.push((
                                    subsystem.name.clone(),
                                    format!("{} -> {} [{}]", transition.from, transition.to, guard),
                                    newly_satisfied,
                                ));
                            }
                        }
                    }
                }
            }

            if !affected.is_empty() {
                links.push(CausationLink {
                    tick: self.tick,
                    variable: var_name.clone(),
                    old_value,
                    new_value: new_value.clone(),
                    writer_subsystem: writer,
                    affected_guards: affected,
                });
            }
        }
        links
    }

    /// RSC-3.3b D2 — apply per-transfer move semantics to this tick's
    /// delivered messages (Transfers.kerml:84-90: "the entire payload leaves
    /// the source"). For every delivered message whose flow has `is_move`,
    /// the source port's payload feature value(s) are cleared (set `Null`)
    /// in the [`PortRegistry`], and each clear is mirrored onto the
    /// `{owner}.{port}.{feature}` slot when one was minted (the RSC-3.2
    /// signal-feature slot plane), keeping registry and slots in lockstep.
    ///
    /// Each send is its own transfer instance — clearing never suppresses a
    /// later delivery (the dead `move_delivered` once-per-flow gate this
    /// replaces is deleted). Ports without features (the whole exercised
    /// corpus today, ledger L23/L25) have nothing to clear.
    fn apply_move_semantics(&mut self, delivered: &[crate::flows::FlowMessage]) {
        if delivered.is_empty() {
            return;
        }
        // The production registry is DEFINITION-keyed (ports are definition-
        // owned; ledger L36) and shared across sibling instances — it must stay
        // immutable, so the value clear lands on the per-instance SlotStore, the
        // one value-clear path here. The moved message carries only its instance
        // source key; we recover the classified link's source `LinkEndpoint`
        // (which carries the resolved registry key) via the flow id, discover
        // the moved features through the shared def-keyed bridge, and clear the
        // instance slots `{endpoint.key()}.{feature}`.
        let mut cleared_slot_names: Vec<String> = Vec::new();
        if let Some(registry) = &self.port_registry {
            for m in delivered {
                let Some(link) = self.router.flow_by_id(&m.flow_id) else {
                    continue;
                };
                if !link.is_move {
                    continue;
                }
                let source = &link.source;
                let available = crate::links::endpoint_feature_names(registry, source);
                let prefix = source.key();
                for feature in crate::flows::moved_feature_names(&available, &m.payload) {
                    cleared_slot_names.push(format!("{prefix}.{feature}"));
                }
            }
        }
        if !cleared_slot_names.is_empty() {
            let handle = self.slot_store_handle();
            let mut store = handle.write().unwrap_or_else(|p| p.into_inner());
            for name in &cleared_slot_names {
                // Inert when the slot was never minted or routing is off.
                let _ = store.set_by_name(name, &Value::Null);
            }
        }
    }

    /// Propagate port values through flow connections.
    ///
    /// Three phases each tick:
    /// 1. Context → output ports: copy matching context variables to port features
    /// 2. Flow resolution: copy output port features to connected input port features
    /// 3. Input ports → context: copy port feature values back to context variables
    ///
    /// Naming convention: context variable `{owner}.{port}.{feature}` maps to
    /// port `{owner}.{port}` feature `{feature}`.
    fn propagate_port_values(&mut self) {
        use crate::flows::PortDirection;

        let Some(registry) = &mut self.port_registry else {
            return;
        };

        // Phase 1: Context → output ports
        // Collect port keys and feature names for output ports
        let output_mappings: Vec<(String, String, String)> = registry
            .iter()
            .filter(|(_, port)| matches!(port.direction, PortDirection::Out | PortDirection::InOut))
            .flat_map(|(key, port)| {
                port.features.keys().map(move |feat_name| {
                    let ctx_key = format!("{}.{}", key, feat_name);
                    (key.to_owned(), feat_name.clone(), ctx_key)
                })
            })
            .collect();

        for (port_key, feat_name, ctx_key) in &output_mappings {
            // RSC-3.5d (Piece A): read the port-feature value from its minted
            // slot when one exists (signal-link endpoints are minted by
            // `mint_signal_feature_slots`, spelled exactly `ctx_key`), else
            // fall back to the legacy string key. Byte-identical: `set_slot`
            // mirrors writes into the variables map under the same spelling and
            // the slot is seeded from that map at attach time, so the slot and
            // the string key always carry the same value. Where no slot was
            // minted (non-signal ports), the string path is the only source —
            // we never invent slots.
            let val = match self.context.slot_value_if_enabled(ctx_key) {
                Some(v) => Some(v),
                None => self.context.get(ctx_key).cloned(),
            };
            if let Some(val) = val {
                if let Some(port) = registry.get_mut(port_key) {
                    if let Some(feat) = port.features.get_mut(feat_name) {
                        feat.value = val;
                    }
                }
            }
        }

        // Phase 2: Flow resolution — copy source port features to target port
        // features. RSC-3.2: links classified as SignalLink are routed by the
        // directed slot-propagation pass below (`propagate_signal_links`)
        // instead of this string copy; skip them here so the value is not
        // copied twice. Non-signal links (PowerBond / MessageChannel) keep the
        // string copy: RSC-3.5d (Piece B) measured that deleting it diverges
        // `port_values` because their class planes (PowerBond→physics,
        // MessageChannel→ExchangePlane delivery) are 3.5e/3.5f, still held.
        // The skip set keys on `(src_key, tgt_key)` — the same endpoint
        // spellings the signal pass routes.
        //
        // `nonsignal_phase2_enabled` is `true` in production (the cfg-gated
        // probe field only exists under `#[cfg(test)]`); the 3.5d parity tests
        // flip it to `false` to prove the copy is load-bearing.
        #[cfg(test)]
        let nonsignal_phase2_enabled = self.nonsignal_phase2_enabled;
        #[cfg(not(test))]
        let nonsignal_phase2_enabled = true;

        let signal_skip: std::collections::HashSet<(String, String)> = self
            .signal_propagation
            .pairs()
            .iter()
            .map(|p| (p.source_port_key.clone(), p.target_port_key.clone()))
            .collect();

        // RSC-3.5e.5 (W1): the FlowUsage subset of the classified link graph is
        // the sole source for the non-signal mirror (the `flow_connections` field
        // is retired). Flow-derived links carry `LinkSourceKind::FlowUsage` and
        // are interned first, in `compile_flows` order, so this reproduces the
        // former `for flow in &self.flow_connections` walk byte-identically:
        // `LinkEndpoint::key()` == `FlowEndpoint::key()`, and the `signal_skip`
        // pair check (NOT a class filter) is preserved verbatim.
        for link in self
            .link_graph
            .iter()
            .filter(|l| l.kind == crate::links::LinkSourceKind::FlowUsage)
        {
            if !nonsignal_phase2_enabled {
                break;
            }
            let src_key = link.source.key();
            let tgt_key = link.target.key();

            if signal_skip.contains(&(src_key.clone(), tgt_key.clone())) {
                continue;
            }

            // Collect source feature values
            let src_values: Vec<(String, Value)> = registry
                .get(&src_key)
                .map(|port| {
                    port.features
                        .iter()
                        .map(|(name, feat)| (name.clone(), feat.value.clone()))
                        .collect()
                })
                .unwrap_or_default();

            // Write to target port features
            if let Some(tgt_port) = registry.get_mut(&tgt_key) {
                for (feat_name, val) in &src_values {
                    if let Some(feat) = tgt_port.features.get_mut(feat_name) {
                        feat.value = val.clone();
                    }
                }
            }
        }

        // Phase 2 (signal links): directed slot-routed propagation (RSC-3.2,
        // design doc D-3.0.3). Replaces the string copy above for classified
        // SignalLinks. Uses the live `registry` borrow for the registry
        // read/write and disjoint-field access to `self.signal_propagation`
        // (read) and `self.context` (the slot write-through, dual-spelling
        // mirror). Pairs are pre-ordered in link-dependency order so source
        // slots are settled before they're consumed (cycle → interning order,
        // RS010 at compile).
        for pair in self.signal_propagation.pairs() {
            // Read the source value from the source port feature (the same
            // value the string copy read post-phase-1).
            let Some(value) = registry
                .get(&pair.source_port_key)
                .and_then(|p| p.features.get(&pair.feature))
                .map(|f| f.value.clone())
            else {
                continue;
            };
            // RSC-5.3 (D-5.0.4): convert the magnitude at the boundary when the
            // source and target slots use different units of the same dimension
            // (e.g. an `[A]` source feeding a `[mA]` target). The conversion is
            // precomputed on the pair from the slot mRefs; `None` (the corpus
            // default — identical/untyped units) is a byte-identical pass-through.
            let value = match &pair.convert {
                Some(c) => c.apply(value),
                None => value,
            };
            // Keep the target registry feature updated so the `port_values`
            // snapshot stays byte-identical with the string path it replaced.
            if let Some(tgt) = registry.get_mut(&pair.target_port_key) {
                if let Some(feat) = tgt.features.get_mut(&pair.feature) {
                    feat.value = value.clone();
                }
            }
            // Route the value through the target slot (set_slot mirrors both
            // name spellings into the legacy variables map). Must-succeed: the
            // target slot was resolved when the pair was built; a dropped write
            // would desync port_values from the string path it replaced.
            let wrote = self.context.set_slot(pair.target_slot, value);
            debug_assert!(
                wrote,
                "port-flow target slot write must succeed (target_slot resolved at pair build)"
            );
        }

        // Phase 3: Input ports → context
        let input_mappings: Vec<(String, Value)> = registry
            .iter()
            .filter(|(_, port)| matches!(port.direction, PortDirection::In | PortDirection::InOut))
            .flat_map(|(key, port)| {
                port.features.iter().map(move |(feat_name, feat)| {
                    let ctx_key = format!("{}.{}", key, feat_name);
                    (ctx_key, feat.value.clone())
                })
            })
            .collect();

        for (ctx_key, val) in input_mappings {
            // RSC-3.5d (Piece A): write the in-port feature value through its
            // minted slot when one exists (`set_slot` mirrors the value into
            // the variables map under both name spellings, so the legacy string
            // key stays coherent — byte-identical with the previous
            // `context.set`). Non-signal in-ports have no slot; `set_slot`
            // returns `false` and we fall back to the string write. We never
            // mint slots here.
            let routed = self
                .context
                .slot_id(&ctx_key)
                .is_some_and(|id| self.context.set_slot(id, val.clone()));
            if !routed {
                self.context.set(ctx_key, val);
            }
        }
    }

    /// Evaluate all computed expressions, respecting gate variables.
    ///
    /// For each expression:
    /// - If ungated, evaluate and write to target
    /// - If gated and gate is truthy, evaluate and write to target
    /// - If gated and gate is falsy, write 0.0 to target (flow blocked)
    fn evaluate_computed_expressions(&mut self) {
        if self.computed_expressions.is_empty() {
            return;
        }
        let evaluator = crate::expressions::ExpressionEvaluator::new();
        // RSC-4.2 (C.4): instance-scoped expressions are bound to their
        // instance's slots (`SlotRef`) and MUST be evaluated against a scoped
        // read view — empty `variables`, slot reader only — so the name-first
        // evaluator misses the bare local name and resolves the `SlotRef` to
        // the correct per-instance slot (collision-safe, replacing text
        // prefixing). The view shares the master's slot-store handle, so it
        // reads every write made this pass (each `context.set` write-throughs
        // to the target's slot). Orchestrator-scope expressions keep
        // evaluating against the master context, which carries non-slot names
        // (`__sf_*` waveforms, port keys) the scoped view intentionally drops.
        let scoped = build_slot_read_context(&self.context);
        for gated in &self.computed_expressions {
            let result = if !gated.is_gate_open(&self.context) {
                // Gate closed — zero the flow.
                Some(Value::Float(0.0))
            } else if gated.scope_prefix.is_some() {
                evaluator.eval(&gated.expr, &scoped).ok()
            } else {
                evaluator.eval(&gated.expr, &self.context).ok()
            };
            if let Some(val) = result {
                self.context.set(gated.target.clone(), val);
            }
        }
    }

    /// Evaluate all constraints against the current shared context.
    fn evaluate_constraints(&self) -> Vec<ConstraintEvalResult> {
        let Some(constraints) = &self.constraints else {
            return Vec::new();
        };

        constraints
            .evaluate_all(&self.context)
            .into_iter()
            .map(|r| ConstraintEvalResult {
                name: r
                    .constraint
                    .description
                    .clone()
                    .unwrap_or_else(|| r.constraint.expr.clone()),
                // Carry the verdict, not a collapsed bool: an inconclusive
                // evaluation used to arrive here as `satisfied: false`,
                // indistinguishable from a real violation.
                verdict: r.verdict(),
                expression: Some(r.constraint.expr),
                operands: r.operands,
                element_id: r.constraint.owner_id,
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use crate::statemachine::TriggerKind;
    use crate::{AssignmentIR, StateIR, StateMachineIR, TransitionActionIR, TransitionIR};

    /// Mint a bare-name Continuous slot per `(subsystem_index, var, initial)`
    /// owned by that subsystem's `WriterId::Executor(index)`, attach the store,
    /// and run the compile-time bind pass — the minimal stand-in for the
    /// `ModelCompiler` mint that production always performs.
    ///
    /// Hand-built no-slot orchestrators are unsupported since the string-identity
    /// cull deleted the legacy `sync_context_out` writeback: an executor now
    /// publishes state ONLY through its slot write-set (`sync_context_out_slots`),
    /// which needs a minted, executor-claimed slot per write target. These unit
    /// tests exercise real tick-loop behaviour, so they mint through this helper
    /// (the routed production path) rather than the deleted fallback. `var` is
    /// both the runtime and canonical spelling (unprefixed), matching what a
    /// bare `add_ode`/`add_sm`/`add_discrete` executor writes.
    fn mint_and_bind_state_slots(orch: &mut Orchestrator, slots: &[(usize, &str, f64)]) {
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};

        let mut store = SlotStore::new();
        for (idx, var, initial) in slots {
            store.intern(
                SlotMeta::new(
                    RuntimeId::top_level(sysml_core::ElementId::from_string(format!(
                        "decl:{var}"
                    ))),
                    Variability::Continuous,
                    WriterId::Executor(*idx as u16),
                    *var,
                    *var,
                ),
                Value::Float(*initial),
            );
        }
        orch.set_slot_store(store);
        orch.bind_expression_slots(None);
    }

    /// Helper: build a simple 2-state machine (A → B) with a structured action
    /// that sets a variable on transition.
    fn make_sm_with_assignment(name: &str, var_name: &str, value: f64) -> StateMachineRunner {
        let ir = StateMachineIR {
            name: name.to_string(),
            states: vec![StateIR::new("A"), StateIR::new("B").final_state()],
            transitions: vec![TransitionIR::new("A", "B").with_event("go").with_action(
                TransitionActionIR::Structured {
                    assignments: vec![AssignmentIR::set(var_name, value)],
                    sends: vec![],
                    port_send_ops: Vec::new(),
                },
            )],
            initial: "A".to_string(),
            regions: vec![],
        };
        StateMachineRunner::new(ir)
    }

    /// Helper: build a simple 2-state machine (X → Y) where the transition guard
    /// checks a variable value.
    fn make_sm_with_guard(name: &str, var_name: &str, threshold: f64) -> StateMachineRunner {
        let ir = StateMachineIR {
            name: name.to_string(),
            states: vec![StateIR::new("X"), StateIR::new("Y").final_state()],
            transitions: vec![TransitionIR::new("X", "Y")
                .with_event("check")
                .with_guard(format!("{} > {}", var_name, threshold))],
            initial: "X".to_string(),
            regions: vec![],
        };
        StateMachineRunner::new(ir)
    }

    #[test]
    fn test_single_sm_step() {
        let sm = make_sm_with_assignment("sm1", "speed", 100.0);
        let mut orch = Orchestrator::new(OrchestratorConfig::default());
        orch.add_state_machine("sm1", sm);
        mint_and_bind_state_slots(&mut orch, &[(0, "speed", 0.0)]);

        // Initial step — no event, SM stays in A
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["sm1"].current_state, "A");
        assert!(!snap.completed);

        // Inject event and step
        orch.inject_event("sm1", "go");
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["sm1"].current_state, "B");
        assert!(snap.completed);

        // Verify the assignment executed — "speed" should be in the shared context
        assert_eq!(orch.context.get("speed"), Some(&Value::Float(100.0)),);
    }

    #[test]
    fn flow_capacity_drops_surface_in_snapshot_and_halt_strict_run() {
        // Fail-hard is the default (RSC-1.5): lossy flows are opt-in.
        let config = OrchestratorConfig::default();
        assert!(!config.allow_lossy_flows);

        let sm = make_sm_with_assignment("sm1", "speed", 100.0);
        let mut orch = Orchestrator::new(config);
        orch.add_state_machine("sm1", sm);
        // 1-slot pending queue: the second send evicts the first.
        orch.set_exchange_plane(ExchangePlane::with_max_queue_size(1));
        orch.send_to_router("sensor.out", Value::Int(1));
        orch.send_to_router("sensor.out", Value::Int(2)); // evicts message 1

        let snap = orch.step();
        // The loss is session-visible in the tick snapshot.
        assert_eq!(snap.flow_drop_warnings.len(), 1);
        let warning = &snap.flow_drop_warnings[0];
        assert!(warning.contains("flow 'sensor.out'"), "got: {warning}");
        assert!(warning.contains("dropped 1 message"), "got: {warning}");
        assert!(warning.contains("capacity 1"), "got: {warning}");

        // Strict mode records a runtime error and halts batch stepping.
        let err = orch
            .flow_error()
            .expect("strict mode must record a flow error on capacity loss");
        assert!(err.contains("sensor.out"), "got: {err}");
        assert!(
            orch.step_until(1_000.0).is_empty(),
            "step_until must not step past a strict-mode flow error"
        );
        assert!(
            orch.run_to_completion().is_some(),
            "run_to_completion returns the last snapshot without stepping"
        );
        assert_eq!(orch.tick(), 1);
    }

    #[test]
    fn flow_capacity_drops_warn_only_in_lossy_mode() {
        let config = OrchestratorConfig {
            allow_lossy_flows: true,
            ..Default::default()
        };
        let sm = make_sm_with_assignment("sm1", "speed", 100.0);
        let mut orch = Orchestrator::new(config);
        orch.add_state_machine("sm1", sm);
        orch.set_exchange_plane(ExchangePlane::with_max_queue_size(1));
        orch.send_to_router("sensor.out", Value::Int(1));
        orch.send_to_router("sensor.out", Value::Int(2)); // evicts message 1

        let snap = orch.step();
        // Visible warning, but no runtime error — the run continues.
        assert_eq!(snap.flow_drop_warnings.len(), 1);
        assert!(orch.flow_error().is_none());

        // Ticks without loss carry no warnings.
        let snap = orch.step();
        assert!(snap.flow_drop_warnings.is_empty());

        // Batch stepping keeps going in lossy mode.
        assert!(!orch.step_until(20.0).is_empty());
    }

    #[test]
    fn test_two_sms_shared_context() {
        // SM1: A→B on "go", sets speed=100
        // SM2: X→Y on "check", guard: speed > 50
        let sm1 = make_sm_with_assignment("sm1", "speed", 100.0);
        let sm2 = make_sm_with_guard("sm2", "speed", 50.0);

        let mut orch = Orchestrator::new(OrchestratorConfig::default());
        orch.add_state_machine("sm1", sm1);
        orch.add_state_machine("sm2", sm2);
        // sm1 (idx 0) writes `speed`; sm2 (idx 1) reads it through the slot.
        mint_and_bind_state_slots(&mut orch, &[(0, "speed", 0.0)]);

        // Step 1: fire "go" on sm1 — sets speed=100
        orch.inject_event("sm1", "go");
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["sm1"].current_state, "B");
        assert_eq!(snap.subsystem_states["sm2"].current_state, "X"); // guard not checked yet

        // Step 2: fire "check" on sm2 — guard reads speed=100 > 50, passes
        orch.inject_event("sm2", "check");
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["sm2"].current_state, "Y");
        assert!(snap.completed); // both done
    }

    #[test]
    fn test_scheduled_events() {
        let sm = make_sm_with_assignment("sm1", "x", 42.0);
        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 10.0,
            ..Default::default()
        });
        orch.add_state_machine("sm1", sm);

        // Schedule "go" at t=50ms
        orch.schedule_event(50.0, "sm1", "go");

        // Step through — event should fire at t=50
        for _ in 0..4 {
            let snap = orch.step();
            assert_eq!(snap.subsystem_states["sm1"].current_state, "A");
        }
        // t=50ms — event fires
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["sm1"].current_state, "B");
        assert_eq!(snap.time_ms, 50.0);
    }

    #[test]
    fn test_step_until() {
        let sm = make_sm_with_assignment("sm1", "x", 1.0);
        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 10.0,
            ..Default::default()
        });
        orch.add_state_machine("sm1", sm);
        orch.schedule_event(30.0, "sm1", "go");

        let snapshots = orch.step_until(50.0);
        assert!(snapshots.len() >= 3); // at least 3 ticks (10, 20, 30)
        let last = snapshots.last().unwrap();
        assert_eq!(last.subsystem_states["sm1"].current_state, "B");
    }

    #[test]
    fn test_sends_from_sm() {
        // SM that sends "alert" on transition
        let ir = StateMachineIR {
            name: "sender".to_string(),
            states: vec![StateIR::new("idle"), StateIR::new("done").final_state()],
            transitions: vec![TransitionIR::new("idle", "done")
                .with_event("go")
                .with_action(TransitionActionIR::Structured {
                    assignments: vec![],
                    sends: vec!["alert".to_string()],
                    port_send_ops: Vec::new(),
                })],
            initial: "idle".to_string(),
            regions: vec![],
        };
        let sm = StateMachineRunner::new(ir);
        let mut orch = Orchestrator::new(OrchestratorConfig::default());
        orch.add_state_machine("sender", sm);

        orch.inject_event("sender", "go");
        let snap = orch.step();

        // The send should appear in the subsystem state
        assert_eq!(snap.subsystem_states["sender"].sends, vec!["alert"]);
    }

    #[test]
    fn test_assertion_checkpoint_construction() {
        let cp = AssertionCheckpoint {
            tick: 5,
            time_ms: 5.0,
            requirement_id: "req1".into(),
            requirement_text: Some("speed < 100".into()),
            verdict: crate::cases::VerdictKind::Pass,
            message: "all constraints satisfied".into(),
            referenced_variables: vec!["speed".into()],
        };
        assert_eq!(cp.tick, 5);
        assert!(cp.verdict.is_pass());
    }

    #[test]
    fn test_guard_diagnosis() {
        // Build a SM with a guarded transition
        let ir = StateMachineIR {
            name: "test".to_string(),
            states: vec![StateIR::new("idle"), StateIR::new("active")],
            transitions: vec![TransitionIR::new("idle", "active")
                .with_event("go")
                .with_guard("ready > 0")],
            initial: "idle".to_string(),
            regions: vec![],
        };

        let runner = crate::statemachine::StateMachineRunner::new(ir);
        // No 'ready' variable — guard should be unsatisfied
        let diagnoses = runner.diagnose_guards(Some("go"));
        assert_eq!(diagnoses.len(), 1);
        assert!(!diagnoses[0].satisfied);
        assert!(diagnoses[0].explanation.contains("blocked"));
        assert!(diagnoses[0].dependencies.contains("ready"));
    }

    #[test]
    fn test_analyze_guard_dependencies() {
        let deps = crate::expressions::analyze_guard_dependencies("pressure > 8 and temp < 100");
        assert!(deps.contains("pressure"));
        assert!(deps.contains("temp"));
        assert_eq!(deps.len(), 2);
    }

    #[test]
    fn test_causation_links_cross_subsystem() {
        // SM1: A→B on "go", sets speed=100
        // SM2: X→Y on "check", guard: speed > 50
        let sm1 = make_sm_with_assignment("sm1", "speed", 100.0);
        let sm2 = make_sm_with_guard("sm2", "speed", 50.0);

        let mut orch = Orchestrator::new(OrchestratorConfig::default());
        orch.add_state_machine("sm1", sm1);
        orch.add_state_machine("sm2", sm2);
        // sm1 (idx 0) writes `speed`; sm2 (idx 1) reads it through the slot.
        mint_and_bind_state_slots(&mut orch, &[(0, "speed", 0.0)]);

        // Step 1: no events — baseline
        let snap1 = orch.step();
        assert!(snap1.causation_links.is_empty()); // no changes yet (first tick has no prev)

        // Step 2: fire "go" on sm1 — sets speed=100, should create causation link
        orch.inject_event("sm1", "go");
        let snap2 = orch.step();

        // SM1 should have transitioned, and speed=100 should affect SM2's guard
        assert_eq!(snap2.subsystem_states["sm1"].current_state, "B");

        // Should have at least one causation link for 'speed'
        let speed_links: Vec<_> = snap2
            .causation_links
            .iter()
            .filter(|cl| cl.variable == "speed")
            .collect();
        assert!(
            !speed_links.is_empty(),
            "expected causation link for 'speed' variable"
        );
        let link = &speed_links[0];
        assert_eq!(link.writer_subsystem, "sm1");
        assert!(!link.affected_guards.is_empty());
        assert_eq!(link.affected_guards[0].0, "sm2"); // affected subsystem
        assert!(link.affected_guards[0].2); // newly satisfied
    }

    #[test]
    fn test_guard_diagnoses_in_snapshot() {
        // SM with a guarded transition
        let sm = make_sm_with_guard("sm1", "temp", 50.0);
        let mut orch = Orchestrator::new(OrchestratorConfig::default());
        orch.add_state_machine("sm1", sm);

        let snap = orch.step();
        // Guard should be diagnosed as unsatisfied since temp is not set
        assert!(!snap.guard_diagnoses.is_empty());
        assert!(!snap.guard_diagnoses[0].satisfied);
        assert!(snap.guard_diagnoses[0].dependencies.contains("temp"));
    }

    #[test]
    fn test_after_trigger_fires_in_orchestrator() {
        // State machine: A --(after 50ms)--> B
        let ir = StateMachineIR {
            name: "test".to_string(),
            states: vec![StateIR::new("A"), StateIR::new("B")],
            transitions: vec![TransitionIR {
                from: "A".into(),
                to: "B".into(),
                event: Some("after(50ms)".into()),
                guard: None,
                action: None,
                is_completion: false,
                is_guard_only: false,
                accept_param: None,
            }],
            initial: "A".to_string(),
            regions: vec![],
        };

        let config = OrchestratorConfig {
            dt_ms: 10.0,
            max_ticks: 100,
            max_time_ms: 10000.0,
            ..Default::default()
        };
        let mut orch = Orchestrator::new(config);
        let runner = StateMachineRunner::new(ir);
        orch.add_state_machine("sm", runner);

        // Initial step
        let snap = orch.step();
        assert_eq!(snap.subsystem_states.get("sm").unwrap().current_state, "A");

        // Step 4 more times (total 50ms = 5 * 10ms)
        for _ in 0..4 {
            orch.step();
        }

        // At 50ms+, the after trigger should have fired
        let snap = orch.step();
        assert_eq!(
            snap.subsystem_states.get("sm").unwrap().current_state,
            "B",
            "After trigger should have fired after 50ms"
        );
    }

    // -------------------------------------------------------------------
    // Phase 12: Port-triggered reactive simulation tests
    // -------------------------------------------------------------------

    #[test]
    fn port_trigger_fires_state_transition() {
        // Setup: State machine "breaker" with transition triggered by port "overcurrent"
        // Flow: "sensor.currentOut" → "breaker.overcurrent"
        // When flow delivers to breaker.overcurrent, the state machine should transition

        let ir = StateMachineIR {
            name: "BreakerController".into(),
            states: vec![
                StateIR::new("closed"),
                StateIR::new("tripped").final_state(),
            ],
            transitions: vec![TransitionIR {
                from: "closed".into(),
                to: "tripped".into(),
                event: Some("overcurrent".into()), // matches port name
                guard: None,
                action: None,
                is_completion: false,
                is_guard_only: false,
                accept_param: None,
            }],
            initial: "closed".into(),
            regions: vec![],
        };

        let mut runner = StateMachineRunner::new(ir);

        // Set up port message trigger on the transition
        runner.set_transition_trigger(
            0,
            TriggerKind::PortMessage {
                port_name: "overcurrent".into(),
                payload_type: None,
                param_name: None,
            },
        );

        let mut config = OrchestratorConfig::default();
        config.dt_ms = 10.0;
        config.tick_strategy = TickStrategy::RouteFirst;

        let mut orch = Orchestrator::new(config);
        orch.add_state_machine("breaker", runner);

        // Add a flow from sensor to breaker port
        let mut flow =
            crate::links::LinkIR::message_channel("sensor", "currentOut", "breaker", "overcurrent");
        flow.is_move = false;
        orch.set_exchange_plane({
            let mut plane = ExchangePlane::new();
            plane.add_flow(flow, "currentFlow");
            plane
        });

        // Initial state should be "closed"
        let snap = orch.step();
        assert_eq!(
            snap.subsystem_states.get("breaker").unwrap().current_state,
            "closed"
        );

        // Inject a message into the flow source (simulating sensor overcurrent)
        orch.send_to_router("sensor.currentOut", Value::Float(150.0));

        // Next tick: RouteFirst delivers the message, port event triggers transition
        let snap = orch.step();
        assert_eq!(
            snap.subsystem_states.get("breaker").unwrap().current_state,
            "tripped",
            "Port message should trigger transition from closed to tripped"
        );
    }

    #[test]
    fn port_trigger_payload_captured_in_context() {
        // Verify that the payload from a port delivery is accessible in EvalContext
        let ir = StateMachineIR {
            name: "Monitor".into(),
            states: vec![StateIR::new("idle"), StateIR::new("alert")],
            transitions: vec![TransitionIR {
                from: "idle".into(),
                to: "alert".into(),
                event: Some("tempPort".into()),
                guard: None,
                action: None,
                is_completion: false,
                is_guard_only: false,
                accept_param: None,
            }],
            initial: "idle".into(),
            regions: vec![],
        };

        let mut runner = StateMachineRunner::new(ir);
        runner.set_transition_trigger(
            0,
            TriggerKind::PortMessage {
                port_name: "tempPort".into(),
                payload_type: None,
                param_name: None,
            },
        );

        let mut config = OrchestratorConfig::default();
        config.dt_ms = 10.0;
        config.tick_strategy = TickStrategy::RouteFirst;

        let mut orch = Orchestrator::new(config);
        orch.add_state_machine("monitor", runner);
        // Mint the SM's (idx 0) port-payload slots so its payload_routes route
        // by SlotId — the compile-static receiving-port payload keys.
        mint_and_bind_state_slots(
            &mut orch,
            &[(0, "tempPort.payload", 0.0), (0, "tempPort_payload", 0.0)],
        );

        let mut flow =
            crate::links::LinkIR::message_channel("sensor", "tempOut", "monitor", "tempPort");
        flow.is_move = false;
        orch.set_exchange_plane({
            let mut plane = ExchangePlane::new();
            plane.add_flow(flow, "tempFlow");
            plane
        });

        // First tick: idle
        orch.step();

        // Send temperature payload
        orch.send_to_router("sensor.tempOut", Value::Float(105.5));

        // Second tick: should transition and capture payload
        let snap = orch.step();
        assert_eq!(
            snap.subsystem_states.get("monitor").unwrap().current_state,
            "alert"
        );

        // Check that payload was captured in context
        let temp_payload = snap.variables.get("tempPort_payload");
        assert_eq!(
            temp_payload,
            Some(&Value::Float(105.5)),
            "Port payload should be captured in context as tempPort_payload"
        );
    }

    /// Build the breaker-shaped SM used by the RSC-3.3c orchestrator tests:
    /// `closed → tripped` on a `PortMessage("tripIn")` trigger.
    fn trip_on_port_sm() -> StateMachineRunner {
        let ir = StateMachineIR {
            name: "BreakerSM".into(),
            states: vec![
                StateIR::new("closed"),
                StateIR::new("tripped").final_state(),
            ],
            transitions: vec![TransitionIR {
                from: "closed".into(),
                to: "tripped".into(),
                event: Some("tripIn".into()),
                guard: None,
                action: None,
                is_completion: false,
                is_guard_only: false,
                accept_param: None,
            }],
            initial: "closed".into(),
            regions: vec![],
        };
        let mut runner = StateMachineRunner::new(ir);
        runner.set_transition_trigger(
            0,
            TriggerKind::PortMessage {
                port_name: "tripIn".into(),
                payload_type: None,
                param_name: None,
            },
        );
        runner
    }

    #[test]
    fn occurrence_addressed_send_reaches_sm_accept_surface() {
        // RSC-3.3c D4 end-to-end: NO declared flow exists. An
        // occurrence-addressed send naming the receiver resolves against
        // the SM's PortMessage accept surface (auto-registered by
        // add_state_machine) and triggers the transition.
        let mut config = OrchestratorConfig::default();
        config.dt_ms = 10.0;
        config.tick_strategy = TickStrategy::RouteFirst;
        let mut orch = Orchestrator::new(config);
        orch.add_state_machine("breaker", trip_on_port_sm());

        let snap = orch.step();
        assert_eq!(
            snap.subsystem_states.get("breaker").unwrap().current_state,
            "closed"
        );

        // MessageTransfer: sender names the receiving participant — the
        // sole accepting surface "breaker.tripIn" resolves it.
        orch.send_message_to_router("firmware.tripOut", "breaker", Value::Bool(true));

        let snap = orch.step();
        assert_eq!(
            snap.subsystem_states.get("breaker").unwrap().current_state,
            "tripped",
            "occurrence-addressed MessageTransfer must reach the SM accept surface"
        );
        assert_eq!(orch.router.unrouted_message_total(), 0);
    }

    #[test]
    fn pull_mode_link_delivers_on_armed_accept() {
        // RSC-3.3c U1 end-to-end (labeled extension): a pull link parks the
        // transfer; the orchestrator pull-polls the SM's ARMED accept port
        // and delivery lands as a port event on the next tick.
        let mut config = OrchestratorConfig::default();
        config.dt_ms = 10.0;
        config.tick_strategy = TickStrategy::RouteFirst;
        let mut orch = Orchestrator::new(config);
        orch.add_state_machine("breaker", trip_on_port_sm());

        let mut flow =
            crate::links::LinkIR::message_channel("firmware", "tripOut", "breaker", "tripIn");
        flow.is_move = false;
        flow.is_push = false; // pull transfer
        orch.set_exchange_plane({
            let mut plane = ExchangePlane::new();
            plane.add_flow(flow, "pullTrip");
            plane
        });

        orch.step(); // tick 1: idle
        orch.send_to_router("firmware.tripOut", Value::Bool(true));

        // Tick 2: the routing pass PARKS the transfer (no eager delivery —
        // the pre-pass pull ran before routing, store was still empty).
        let snap = orch.step();
        assert_eq!(
            snap.subsystem_states.get("breaker").unwrap().current_state,
            "closed",
            "pull link must not deliver eagerly"
        );

        // Tick 3: the armed accept (closed state, PortMessage tripIn) pulls
        // the parked transfer; the port event fires the transition.
        let snap = orch.step();
        assert_eq!(
            snap.subsystem_states.get("breaker").unwrap().current_state,
            "tripped",
            "armed accept must pull-initiate the parked transfer"
        );
    }

    #[test]
    fn step_first_strategy_preserves_existing_behavior() {
        // Verify that StepFirst (default) doesn't break when port triggers exist
        let ir = StateMachineIR {
            name: "SM".into(),
            states: vec![StateIR::new("A"), StateIR::new("B").final_state()],
            transitions: vec![TransitionIR {
                from: "A".into(),
                to: "B".into(),
                event: Some("go".into()),
                guard: None,
                action: None,
                is_completion: false,
                is_guard_only: false,
                accept_param: None,
            }],
            initial: "A".into(),
            regions: vec![],
        };

        let runner = StateMachineRunner::new(ir);

        let mut config = OrchestratorConfig::default();
        config.dt_ms = 10.0;
        // Explicitly StepFirst (default)
        config.tick_strategy = TickStrategy::StepFirst;

        let mut orch = Orchestrator::new(config);
        orch.add_state_machine("sm", runner);

        let snap = orch.step();
        assert_eq!(snap.subsystem_states["sm"].current_state, "A");

        orch.inject_event("sm", "go");
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["sm"].current_state, "B");
    }

    // ── Phase 15: ODE integration tests ──────────────────────────────

    #[test]
    fn test_ode_subsystem_basic() {
        // ODE alone: dT/dt = 1.0 (constant heating, 1 degree per second)
        let solver = crate::ode::Rk4Solver::new(
            "heater",
            vec!["temperature".to_string()],
            vec![20.0],
            std::sync::Arc::new(|_t, _y, _ctx| vec![1.0]),
        );

        let config = OrchestratorConfig {
            dt_ms: 1000.0, // 1 second per tick
            max_ticks: 100,
            max_time_ms: 100_000.0,
            ..Default::default()
        };
        let mut orch = Orchestrator::new(config);
        orch.add_ode("heater", solver);
        mint_and_bind_state_slots(&mut orch, &[(0, "temperature", 20.0)]);

        // Step 5 times = 5 seconds → temperature should be ~25.0
        for _ in 0..5 {
            orch.step();
        }
        let temp = orch.context.get("temperature").unwrap();
        match temp {
            Value::Float(f) => assert!((*f - 25.0).abs() < 0.01, "expected ~25.0, got {f}"),
            _ => panic!("expected Float, got {:?}", temp),
        }
    }

    #[test]
    fn test_ode_context_sync() {
        // Verify ODE writes state to context each tick
        let solver = crate::ode::Rk4Solver::new(
            "thermal",
            vec!["temperature".to_string()],
            vec![50.0],
            std::sync::Arc::new(|_t, _y, _ctx| vec![0.0]), // constant, no change
        );

        let mut orch = Orchestrator::new(Default::default());
        orch.add_ode("thermal", solver);
        mint_and_bind_state_slots(&mut orch, &[(0, "temperature", 50.0)]);
        orch.step();

        let temp = orch.context.get("temperature").unwrap();
        match temp {
            Value::Float(f) => assert!((*f - 50.0).abs() < 0.001),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_hybrid_ode_with_state_machine() {
        // ODE heats from 20°C at 10°C/s. State machine transitions when temp >= 50.
        // At dt=1s, should take ~3 ticks to reach 50°C.
        let solver = crate::ode::Rk4Solver::new(
            "thermal",
            vec!["temperature".to_string()],
            vec![20.0],
            std::sync::Arc::new(|_t, _y, _ctx| vec![10.0]), // 10°C per second
        );

        // Build a state machine: heating -> ready (when temperature >= 50)
        let ir = StateMachineIR {
            name: "boiler".to_string(),
            states: vec![StateIR::new("heating"), StateIR::new("ready")],
            transitions: vec![TransitionIR::new("heating", "ready").with_guard("temperature >= 50")],
            initial: "heating".to_string(),
            regions: vec![],
        };
        let runner = crate::statemachine::StateMachineRunner::new(ir);

        let config = OrchestratorConfig {
            dt_ms: 1000.0, // 1 second per tick
            max_ticks: 100,
            max_time_ms: 100_000.0,
            ..Default::default()
        };
        let mut orch = Orchestrator::new(config);
        orch.add_ode("thermal", solver);
        orch.add_state_machine("boiler", runner);
        // ODE "thermal" (idx 0) owns the `temperature` write; the SM reads it.
        mint_and_bind_state_slots(&mut orch, &[(0, "temperature", 20.0)]);

        // Tick 1: temp=30, still heating
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["boiler"].current_state, "heating");

        // Tick 2: temp=40, still heating
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["boiler"].current_state, "heating");

        // Tick 3: temp=50, transition fires!
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["boiler"].current_state, "ready");

        // Verify temperature in context
        let temp = snap.variables.get("temperature").unwrap();
        match temp {
            Value::Float(f) => assert!(*f >= 50.0, "expected >= 50, got {f}"),
            _ => panic!("expected Float"),
        }
    }

    #[test]
    fn test_ode_reset() {
        let solver = crate::ode::Rk4Solver::new(
            "thermal",
            vec!["temperature".to_string()],
            vec![20.0],
            std::sync::Arc::new(|_t, _y, _ctx| vec![5.0]),
        );

        let config = OrchestratorConfig {
            dt_ms: 1000.0,
            ..Default::default()
        };
        let mut orch = Orchestrator::new(config);
        orch.add_ode("thermal", solver);
        mint_and_bind_state_slots(&mut orch, &[(0, "temperature", 20.0)]);

        // Step a few times
        for _ in 0..3 {
            orch.step();
        }

        // Reset
        orch.reset();

        // After reset, ODE state should be back at initial
        orch.step();
        let temp = orch.context.get("temperature").unwrap();
        match temp {
            Value::Float(f) => assert!(
                (*f - 25.0).abs() < 0.01,
                "expected ~25.0 after reset+step, got {f}"
            ),
            _ => panic!("expected Float"),
        }
    }

    // ── Phase 15D: Zero-crossing detector integration ────────────────

    #[test]
    fn test_crossing_fires_sm_transition() {
        use crate::ode_events::{CrossingDirection, ZeroCrossingDetector};

        // SM: idle -> triggered on "temp_crosses_50" event
        let ir = StateMachineIR::new("sm", "idle")
            .with_state(StateIR::new("idle"))
            .with_state(StateIR::new("triggered").final_state())
            .with_transition(TransitionIR::new("idle", "triggered").with_event("temp_crosses_50"));

        let runner = StateMachineRunner::new(ir);

        // ODE: temperature rises linearly at 10/s from 0
        let solver = crate::ode::Rk4Solver::new(
            "thermal",
            vec!["temperature".to_string()],
            vec![0.0],
            std::sync::Arc::new(|_t, _y, _ctx| vec![10.0]), // dT/dt = 10
        );

        // Crossing detector: fires when temperature crosses 50
        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "temp_crosses_50",
            CrossingDirection::Rising,
            std::sync::Arc::new(|_t, y, _ctx| y[0] - 50.0),
        );

        let config = OrchestratorConfig {
            dt_ms: 1000.0, // 1 second per tick
            max_ticks: 100,
            max_time_ms: 100_000.0,
            ..Default::default()
        };

        let mut orch = Orchestrator::new(config);
        let sm_index = orch.add_state_machine("sm", runner);
        let ode_index = orch.add_ode("thermal", solver);
        orch.add_crossing_detector(ode_index, sm_index, detector);

        // Step 4 times: t=1s(10C), t=2s(20C), t=3s(30C), t=4s(40C) — no crossing yet
        for _ in 0..4 {
            orch.step();
        }
        assert_eq!(
            orch.subsystems
                .iter()
                .find(|s| s.name == "sm")
                .map(|s| s.executor.current_state_name())
                .unwrap_or("?"),
            "idle"
        );

        // Step to t=5s (50C) — crossing should fire, event injected
        orch.step();
        // Process the injected event (it was scheduled at current time, fires next step)
        orch.step();

        let sm_state = orch
            .subsystems
            .iter()
            .find(|s| s.name == "sm")
            .map(|s| s.executor.current_state_name().to_string())
            .unwrap_or_default();
        assert_eq!(
            sm_state, "triggered",
            "SM should have transitioned after crossing event"
        );
    }

    /// RSC-4.2 S2 (L39) — oscillator-fixture-shape name-collision oracle.
    ///
    /// Reproduces the exact adversarial shape ledgered as the pre-arc A1
    /// smell: an SM subsystem and a ContinuousDynamics ODE subsystem share
    /// one display name ("Widget"), and — as in the legacy oscillator fixture, where a
    /// `part def` with an inline `exhibit state` compiles as BOTH a
    /// StateMachine wrapper AND (via its own `calc def :> GetDerivative`) a
    /// ContinuousDynamics subsystem under the part def's name, with the SM
    /// wrapper registering first — the SM wins first-match by registration
    /// order.
    ///
    /// The discrete tick a `when`-located transition fires on is INSENSITIVE
    /// to this bug (the Rising/Falling sign change is computed from the
    /// scalar `g_start`/`g_end`, both of which happen to read correctly
    /// regardless of `y_end`'s identity — see the design doc). What the bug
    /// actually degrades is the BISECTED crossing time itself: with an empty
    /// `y_end` (the SM's `get_state_snapshot()` default), `ZeroCrossingDetector`'s
    /// `y_start.iter().zip(y_end.iter())` collapses interpolation to nothing,
    /// and the returned `crossing_time` collapses toward the tick's `t_start`
    /// instead of the true sub-tick fraction. This test asserts on that
    /// internal crossing time (via `last_crossing_time`, RSC-4.2 test sugar)
    /// — the existing tolerant ≤2-tick gates (`when_crossing_location.rs`,
    /// `protection_core_physics.rs`) only ever check which TICK a transition fires
    /// in, which is why they never caught this.
    #[test]
    fn test_crossing_y_end_resolves_by_index_not_colliding_name() {
        use crate::ode_events::{CrossingDirection, ZeroCrossingDetector};

        // The colliding-name wrapper SM: no transitions, so it never
        // interferes with stepping — its only job is to occupy the FIRST
        // subsystem slot under the name "Widget" and return `None` from
        // `get_state_snapshot()` (the `Executor` trait default, un-overridden
        // by `StateMachineRunner`), exactly like the oscillator fixture's
        // part-def-with-inline-state wrapper.
        let wrapper_ir = StateMachineIR::new("Widget", "idle").with_state(StateIR::new("idle"));
        let wrapper = StateMachineRunner::new(wrapper_ir);

        // The real ODE, ALSO named "Widget": dx/dt = 1.0 (units/s), x(0) = 0.
        let solver = crate::ode::Rk4Solver::new(
            "Widget",
            vec!["x".to_string()],
            vec![0.0],
            std::sync::Arc::new(|_t, _y, _ctx| vec![1.0]),
        );

        // Rising crossing at x = 0.55. With dt = 0.1s, x(t) = t, so the true
        // crossing is at t = 0.55s — the midpoint of the tick window
        // straddling it (whichever tick that resolves to once the
        // orchestrator's internal time bookkeeping is applied; the point of
        // this test is not that absolute value but the ~dt/2 gap between the
        // degraded and correctly-located answer).
        // Guarded exactly like the compiler's real generated event_fn
        // (`ModelCompiler::wire_when_crossings_for_pair`): when `y` doesn't
        // carry the state var (empty on the buggy path), fall back to
        // reading it from `ctx` — which the ODE has already written the
        // CORRECT post-step value into via `sync_context_out_slots` before
        // crossing detection runs. This is what makes the bug degrade
        // bisection precision instead of index-panicking: production never
        // panics here, it silently mislocates.
        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "x_crosses_055",
            CrossingDirection::Rising,
            std::sync::Arc::new(|_t, y, ctx| {
                let x = y
                    .first()
                    .copied()
                    .unwrap_or_else(|| ctx.get("x").and_then(|v| v.as_float()).unwrap_or(0.0));
                x - 0.55
            }),
        );

        let config = OrchestratorConfig {
            dt_ms: 100.0, // coarse 100ms ticks so sub-tick location is observable
            max_ticks: 20,
            max_time_ms: 2000.0,
            ..Default::default()
        };
        let mut orch = Orchestrator::new(config);
        // Adversarial registration order: the SM wrapper FIRST (index 0),
        // matching the oscillator fixture's actual compile order (primary SMs register in step
        // 1, before ODE registration in step 3) — `find(|s| s.name ==
        // "Widget")` would hit this wrapper before ever reaching the ODE.
        let sm_index = orch.add_state_machine("Widget", wrapper);
        let ode_index = orch.add_ode("Widget", solver);
        assert!(
            sm_index.index() < ode_index.index(),
            "the adversarial shape requires the SM to register before the ODE"
        );
        // Bind a minimal slot store so the ODE's post-step state actually
        // writes through to `self.context["x"]` (RSC-2.2: without a slot
        // store, `sync_context_out_slots` is a no-op — the legacy unconditional
        // writeback was deleted with the string-identity cull). Production
        // orchestrators are always slot-backed; this makes the hand-built
        // test match that shape instead of accidentally masking the bug
        // behind an always-stale (never-written) `ctx.get("x")`.
        mint_and_bind_state_slots(&mut orch, &[(ode_index.index(), "x", 0.0)]);
        // The injection target is irrelevant to this oracle (no transition
        // consumes it); reuse the wrapper's index.
        orch.add_crossing_detector(ode_index, sm_index, detector);

        for _ in 0..10 {
            orch.step();
        }

        let crossing_time = orch
            .last_crossing_time(ode_index, "x_crosses_055")
            .expect("the Rising crossing at x=0.55 must have been detected within 1s");

        // The true crossing is at t=0.55s. RSC-4.2 L39 fixes `y_end`
        // resolution to index directly into `self.subsystems[ode_index]`
        // instead of `find(|s| s.name == *ode_name)`, so the detector's
        // bisection sees the ODE's REAL interpolated state and locates the
        // crossing to its exact sub-tick fraction (bisection tolerance
        // 1e-6s) — a fixed, small, constant offset from 0.55s regardless of
        // which tick it falls in (the orchestrator labels the crossing
        // window one full tick ahead of the physical time the (y_start,
        // y_end) pair represents; bisection is alpha-relative to whatever
        // window it's given, so this labeling offset is exact and constant).
        // Before the fix, `y_end` resolves to the wrapper SM (empty
        // snapshot), collapsing `y_start.zip(y_end)` to nothing on every
        // bisection probe — `crossing_time` then collapses toward the
        // window's `t_start` label, roughly `dt/2 = 0.05s` away from the
        // correctly-located value. Assert tightly enough (5ms, 1/20th of a
        // tick) that only genuine sub-tick interpolation — not the collapsed
        // fallback — can pass.
        let expected = 0.65; // 0.55s true crossing + the one-tick window-label offset
        assert!(
            (crossing_time - expected).abs() < 0.005,
            "expected the located crossing near t={expected}s (interpolated from the ODE's \
             real state), got t={crossing_time}s — y_end likely resolved to the wrong \
             (name-colliding) subsystem"
        );
    }

    // ── P1 dt-under-resolution advisory (step_size_health) ──────────────
    //
    // These build a sine oscillator ODE (`dx/dt = cos(t)`, `x(0)=0` ⇒
    // `x(t)=sin(t)`) with an `Either`-direction zero-crossing detector, so two
    // located crossings fire per full oscillation cycle (one falling, one
    // rising) exactly π seconds apart. The observed cycle length in ticks is a
    // pure function of `dt_ms`, which is what the advisory measures.

    /// Build an orchestrator with a single `sin(t)` oscillator ODE plus an
    /// `Either` zero-crossing detector at `x = 0`, run it for `ticks` ticks,
    /// and return the advisory for that ODE from the final full snapshot.
    fn run_sine_oscillator_advisory(
        dt_ms: f64,
        ticks: u64,
    ) -> crate::step_size_advisory::StepSizeAdvisory {
        use crate::ode_events::{CrossingDirection, ZeroCrossingDetector};

        let solver = crate::ode::Rk4Solver::new(
            "Osc",
            vec!["x".to_string()],
            vec![0.0],
            std::sync::Arc::new(|t, _y, _ctx| vec![t.cos()]),
        );

        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "x_crosses_zero",
            CrossingDirection::Either,
            std::sync::Arc::new(|_t, y, ctx| {
                y.first()
                    .copied()
                    .unwrap_or_else(|| ctx.get("x").and_then(|v| v.as_float()).unwrap_or(0.0))
            }),
        );

        // A no-transition wrapper SM to receive injected crossing events (the
        // detector needs an injection target; nothing consumes them here).
        let wrapper = StateMachineRunner::new(
            StateMachineIR::new("SinkSM", "idle").with_state(StateIR::new("idle")),
        );

        let config = OrchestratorConfig {
            dt_ms,
            max_ticks: ticks + 10,
            max_time_ms: dt_ms * (ticks as f64 + 10.0),
            ..Default::default()
        };
        let mut orch = Orchestrator::new(config);
        let ode_index = orch.add_ode("Osc", solver);
        let sm_index = orch.add_state_machine("SinkSM", wrapper);
        mint_and_bind_state_slots(&mut orch, &[(ode_index.index(), "x", 0.0)]);
        orch.add_crossing_detector(ode_index, sm_index, detector);

        let mut last = None;
        for _ in 0..ticks {
            last = Some(orch.step());
        }
        let snap = last.expect("at least one tick");
        snap.step_size_health
            .iter()
            .find(|h| h.subsystem == "Osc")
            .map(|h| h.advisory.clone())
            .expect("the Osc ODE must carry a step-size advisory (it has a detector)")
    }

    #[test]
    fn step_size_advisory_trips_under_resolved_at_coarse_dt() {
        use crate::step_size_advisory::{StepSizeAdvisory, TARGET_TICKS_PER_CYCLE};
        // dt = 1s/tick ⇒ full cycle 2π ≈ 6.28 ticks — well under the target of
        // 20 ticks/cycle. Run long enough for several cycles' worth of
        // crossings (π ≈ 3.14 ticks apart).
        let advisory = run_sine_oscillator_advisory(1000.0, 40);
        match advisory {
            StepSizeAdvisory::UnderResolved {
                ticks_per_cycle,
                suggested_dt_ms,
            } => {
                assert!(
                    ticks_per_cycle < TARGET_TICKS_PER_CYCLE,
                    "observed cycle {ticks_per_cycle} ticks should be below target \
                     {TARGET_TICKS_PER_CYCLE}"
                );
                // Suggested dt must be smaller than the observed dt and would
                // resolve the observed period to ≈ the target.
                assert!(
                    suggested_dt_ms < 1000.0 && suggested_dt_ms > 0.0,
                    "suggested dt {suggested_dt_ms}ms must be a finer positive step"
                );
            }
            other => panic!("expected UnderResolved at dt=1000ms, got {other:?}"),
        }
    }

    #[test]
    fn step_size_advisory_ok_when_well_resolved() {
        use crate::step_size_advisory::{StepSizeAdvisory, TARGET_TICKS_PER_CYCLE};
        // dt = 100ms/tick ⇒ full cycle 2π/0.1 ≈ 63 ticks — comfortably above
        // the 20 tick/cycle target. First crossing ≈ π/0.1 ≈ 31 ticks, second
        // ≈ 63 ticks, so run ~140 ticks to accumulate several crossings.
        let advisory = run_sine_oscillator_advisory(100.0, 140);
        match advisory {
            StepSizeAdvisory::Ok { ticks_per_cycle } => {
                assert!(
                    ticks_per_cycle >= TARGET_TICKS_PER_CYCLE,
                    "observed cycle {ticks_per_cycle} ticks should meet target \
                     {TARGET_TICKS_PER_CYCLE}"
                );
            }
            other => panic!("expected Ok at dt=100ms, got {other:?}"),
        }
    }

    #[test]
    fn step_size_advisory_not_applicable_without_crossings() {
        use crate::ode_events::{CrossingDirection, ZeroCrossingDetector};
        use crate::step_size_advisory::StepSizeAdvisory;

        // Monotonic ODE dx/dt = 1 with a crossing event at x = 100 that the run
        // window never reaches: the subsystem HAS a detector (so it appears in
        // step_size_health) but produces zero located crossings — the explicit
        // NotApplicable state, NOT a silent "Ok".
        let solver = crate::ode::Rk4Solver::new(
            "Ramp",
            vec!["x".to_string()],
            vec![0.0],
            std::sync::Arc::new(|_t, _y, _ctx| vec![1.0]),
        );
        let mut detector = ZeroCrossingDetector::new();
        detector.add_event(
            "x_crosses_100",
            CrossingDirection::Rising,
            std::sync::Arc::new(|_t, y, ctx| {
                let x = y
                    .first()
                    .copied()
                    .unwrap_or_else(|| ctx.get("x").and_then(|v| v.as_float()).unwrap_or(0.0));
                x - 100.0
            }),
        );
        let wrapper = StateMachineRunner::new(
            StateMachineIR::new("SinkSM", "idle").with_state(StateIR::new("idle")),
        );
        let config = OrchestratorConfig {
            dt_ms: 100.0,
            max_ticks: 50,
            max_time_ms: 6000.0,
            ..Default::default()
        };
        let mut orch = Orchestrator::new(config);
        let ode_index = orch.add_ode("Ramp", solver);
        let sm_index = orch.add_state_machine("SinkSM", wrapper);
        mint_and_bind_state_slots(&mut orch, &[(ode_index.index(), "x", 0.0)]);
        orch.add_crossing_detector(ode_index, sm_index, detector);

        let mut last = None;
        for _ in 0..20 {
            last = Some(orch.step());
        }
        let snap = last.expect("at least one tick");
        let advisory = snap
            .step_size_health
            .iter()
            .find(|h| h.subsystem == "Ramp")
            .map(|h| h.advisory.clone())
            .expect("the Ramp ODE must carry a step-size advisory (it has a detector)");
        assert_eq!(
            advisory,
            StepSizeAdvisory::NotApplicable,
            "no located crossing ⇒ NotApplicable, never a silent Ok"
        );
    }

    #[test]
    fn test_trigger_at_via_orchestrator() {
        use crate::statemachine::TriggerKind;

        // SM with At(0.5) trigger: idle -> timed_out after 0.5 seconds
        let ir = StateMachineIR::new("timer", "idle")
            .with_state(StateIR::new("idle"))
            .with_state(StateIR::new("timed_out").final_state())
            .with_transition(TransitionIR::new("idle", "timed_out"));

        let mut runner = StateMachineRunner::new(ir);
        runner.set_transition_trigger(0, TriggerKind::At(0.5));

        let config = OrchestratorConfig {
            dt_ms: 100.0, // 100ms per tick
            max_ticks: 100,
            max_time_ms: 10_000.0,
            ..Default::default()
        };

        let mut orch = Orchestrator::new(config);
        orch.add_state_machine("timer", runner);

        // Step 4 times: 0.1s, 0.2s, 0.3s, 0.4s — should still be idle
        for _ in 0..4 {
            orch.step();
        }
        let state_at_4 = orch
            .subsystems
            .iter()
            .find(|s| s.name == "timer")
            .map(|s| s.executor.current_state_name().to_string())
            .unwrap_or_default();
        assert_eq!(state_at_4, "idle", "should still be idle at 0.4s");

        // Step to 0.5s — trigger should fire
        orch.step();
        let state_at_5 = orch
            .subsystems
            .iter()
            .find(|s| s.name == "timer")
            .map(|s| s.executor.current_state_name().to_string())
            .unwrap_or_default();
        assert_eq!(
            state_at_5, "timed_out",
            "At(0.5) trigger should fire at 0.5s"
        );
    }

    #[test]
    fn test_orchestrator_multi_rate_clocks() {
        // Two state machines: "fast" at 2x speed and "slow" at 0.5x speed.
        // Both have a when-guard on __clock_time to verify they see different times.

        // SM1 "fast": A -> B on "go", sets fast_time = __clock_time
        let ir_fast = StateMachineIR {
            name: "fast".to_string(),
            states: vec![StateIR::new("A"), StateIR::new("B").final_state()],
            transitions: vec![TransitionIR::new("A", "B").with_event("go").with_action(
                TransitionActionIR::Structured {
                    assignments: vec![],
                    sends: vec![],
                    port_send_ops: Vec::new(),
                },
            )],
            initial: "A".to_string(),
            regions: vec![],
        };
        let sm_fast = StateMachineRunner::new(ir_fast);

        // SM2 "slow": X -> Y on "go", sets slow_time = __clock_time
        let ir_slow = StateMachineIR {
            name: "slow".to_string(),
            states: vec![StateIR::new("X"), StateIR::new("Y").final_state()],
            transitions: vec![TransitionIR::new("X", "Y").with_event("go").with_action(
                TransitionActionIR::Structured {
                    assignments: vec![],
                    sends: vec![],
                    port_send_ops: Vec::new(),
                },
            )],
            initial: "X".to_string(),
            regions: vec![],
        };
        let sm_slow = StateMachineRunner::new(ir_slow);

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 1000.0, // 1 second per tick
            ..Default::default()
        });
        orch.add_state_machine("fast", sm_fast);
        orch.add_state_machine("slow", sm_slow);

        // Register local clocks: "fast" at 2x, "slow" at 0.5x
        orch.set_clock("fast", 2.0);
        orch.set_clock("slow", 0.5);

        // Step 1: advance by 1 second global
        orch.step();

        // After 1 tick (1s global):
        // - universal clock = 1.0s
        // - fast local clock = 2.0s
        // - slow local clock = 0.5s
        assert!((orch.clock_registry().local_time("fast").unwrap() - 2.0).abs() < 1e-10);
        assert!((orch.clock_registry().local_time("slow").unwrap() - 0.5).abs() < 1e-10);

        // Step 2: advance another second
        orch.step();
        assert!((orch.clock_registry().local_time("fast").unwrap() - 4.0).abs() < 1e-10);
        assert!((orch.clock_registry().local_time("slow").unwrap() - 1.0).abs() < 1e-10);

        // Verify the registry state after reset
        orch.reset();
        assert!((orch.clock_registry().local_time("fast").unwrap() - 0.0).abs() < 1e-10);
        assert!((orch.clock_registry().local_time("slow").unwrap() - 0.0).abs() < 1e-10);
    }

    #[test]
    fn test_orchestrator_clock_time_in_context() {
        // Verify that __clock_time is set per-subsystem from local clocks.
        // SM that reads __clock_time via a guard: transition when __clock_time > 3.0
        let ir = StateMachineIR {
            name: "ctrl".to_string(),
            states: vec![StateIR::new("waiting"), StateIR::new("done").final_state()],
            transitions: vec![TransitionIR::new("waiting", "done")
                .with_event("check")
                .with_guard("__clock_time > 3.0".to_string())],
            initial: "waiting".to_string(),
            regions: vec![],
        };
        let sm = StateMachineRunner::new(ir);

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 1000.0, // 1 second per tick
            ..Default::default()
        });
        orch.add_state_machine("ctrl", sm);

        // Register local clock at 2x speed: after 2 ticks (2s global), local time = 4s > 3.0
        orch.set_clock("ctrl", 2.0);

        // After 1 tick: local = 2.0s, guard "__clock_time > 3.0" fails
        orch.inject_event("ctrl", "check");
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["ctrl"].current_state, "waiting");

        // After 2 ticks: local = 4.0s, guard passes
        orch.inject_event("ctrl", "check");
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["ctrl"].current_state, "done");
    }

    #[test]
    fn test_orchestrator_no_local_clock_uses_global() {
        // Without a registered local clock, __clock_time should use universal clock
        let ir = StateMachineIR {
            name: "default".to_string(),
            states: vec![StateIR::new("waiting"), StateIR::new("done").final_state()],
            transitions: vec![TransitionIR::new("waiting", "done")
                .with_event("check")
                .with_guard("__clock_time > 1.5".to_string())],
            initial: "waiting".to_string(),
            regions: vec![],
        };
        let sm = StateMachineRunner::new(ir);

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 1000.0,
            ..Default::default()
        });
        orch.add_state_machine("default", sm);
        // No set_clock call — uses global time

        // After 1 tick: global = 1.0s, guard fails
        orch.inject_event("default", "check");
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["default"].current_state, "waiting");

        // After 2 ticks: global = 2.0s > 1.5, guard passes
        orch.inject_event("default", "check");
        let snap = orch.step();
        assert_eq!(snap.subsystem_states["default"].current_state, "done");
    }

    // ── Phase B3: Orchestrator::fork ──────────────────────────────────

    #[test]
    fn test_fork_state_machine_independence() {
        // Run a SM a few ticks, fork, drive each copy divergently, verify independence.
        let sm = make_sm_with_assignment("sm1", "speed", 100.0);
        let mut parent = Orchestrator::new(OrchestratorConfig::default());
        parent.add_state_machine("sm1", sm);
        mint_and_bind_state_slots(&mut parent, &[(0, "speed", 0.0)]);

        // Step 5 times without firing the transition — both should be in A.
        for _ in 0..5 {
            parent.step();
        }
        assert_eq!(parent.tick(), 5);
        assert_eq!(parent.subsystems[0].executor.current_state_name(), "A");

        // Fork. Child picks up at the same tick/state.
        let mut child = parent.fork();
        assert_eq!(child.tick(), 5);
        assert_eq!(child.subsystems[0].executor.current_state_name(), "A");

        // Drive child through the transition; parent untouched.
        child.inject_event("sm1", "go");
        let child_snap = child.step();
        assert_eq!(child_snap.subsystem_states["sm1"].current_state, "B");
        assert_eq!(
            child.context.get("speed"),
            Some(&Value::Float(100.0)),
            "child context should reflect the transition"
        );

        // Parent continues in A; its context still lacks "speed".
        let parent_snap = parent.step();
        assert_eq!(parent_snap.subsystem_states["sm1"].current_state, "A");
        assert_eq!(
            parent.context.get("speed"),
            None,
            "parent context must NOT see the child's assignment"
        );
    }

    #[test]
    fn test_fork_preserves_tick_and_time() {
        let sm = make_sm_with_assignment("sm1", "x", 1.0);
        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 10.0,
            ..Default::default()
        });
        orch.add_state_machine("sm1", sm);

        for _ in 0..7 {
            orch.step();
        }
        let parent_tick = orch.tick();
        let parent_time = orch.time_ms();

        let child = orch.fork();
        assert_eq!(child.tick(), parent_tick);
        assert_eq!(child.time_ms(), parent_time);
        assert_eq!(child.trace().len(), orch.trace().len());
    }

    #[test]
    fn test_fork_ode_solver_carries_state() {
        // Heat from 20°C at 10°C/s. After 3 ticks (dt=1s) temp ~= 50°C.
        // Fork, then verify the child's ODE solver sees the same temperature.
        let solver = crate::ode::Rk4Solver::new(
            "thermal",
            vec!["temperature".to_string()],
            vec![20.0],
            std::sync::Arc::new(|_t, _y, _ctx| vec![10.0]),
        );
        let mut parent = Orchestrator::new(OrchestratorConfig {
            dt_ms: 1000.0,
            max_ticks: 100,
            max_time_ms: 100_000.0,
            ..Default::default()
        });
        parent.add_ode("thermal", solver);
        mint_and_bind_state_slots(&mut parent, &[(0, "temperature", 20.0)]);

        for _ in 0..3 {
            parent.step();
        }
        let parent_temp = match parent.context.get("temperature").unwrap() {
            Value::Float(f) => *f,
            _ => panic!("expected Float"),
        };

        let mut child = parent.fork();
        // Child's context should match at fork time.
        let child_temp = match child.context.get("temperature").unwrap() {
            Value::Float(f) => *f,
            _ => panic!("expected Float"),
        };
        assert!((parent_temp - child_temp).abs() < 1e-10);

        // Step child independently — parent's ODE state must not drift.
        for _ in 0..3 {
            child.step();
        }
        let parent_temp_after = match parent.context.get("temperature").unwrap() {
            Value::Float(f) => *f,
            _ => panic!("expected Float"),
        };
        assert!(
            (parent_temp - parent_temp_after).abs() < 1e-10,
            "parent temperature should not have drifted: {parent_temp} vs {parent_temp_after}",
        );

        let child_temp_after = match child.context.get("temperature").unwrap() {
            Value::Float(f) => *f,
            _ => panic!("expected Float"),
        };
        assert!(
            child_temp_after > parent_temp_after + 10.0,
            "child should have advanced past parent: {child_temp_after} vs {parent_temp_after}",
        );
    }

    #[test]
    fn test_fork_occurrence_registry_independence() {
        // Regression guard for the session-backend-contract.md:249 promise
        // ("parent and child share no mutable state"): the parent's
        // occurrence_registry must NOT be affected by mutating (or
        // resetting) a forked child's registry.
        let parent = Orchestrator::new(OrchestratorConfig::default());
        {
            let mut reg = parent.occurrence_registry.lock().unwrap();
            reg.create(
                sysml_core::ElementId::from_string("life1"),
                0.0,
                std::collections::HashMap::new(),
                "default".into(),
            );
        }
        assert_eq!(
            parent.occurrence_registry.lock().unwrap().instance_count(),
            1
        );

        let child = parent.fork();
        assert_eq!(
            child.occurrence_registry.lock().unwrap().instance_count(),
            1,
            "child should inherit the parent's occurrence at fork time"
        );

        // Mutate the child directly: add another occurrence.
        {
            let mut reg = child.occurrence_registry.lock().unwrap();
            reg.create(
                sysml_core::ElementId::from_string("life2"),
                1.0,
                std::collections::HashMap::new(),
                "default".into(),
            );
        }
        assert_eq!(
            child.occurrence_registry.lock().unwrap().instance_count(),
            2,
            "child registry should reflect its own new occurrence"
        );
        assert_eq!(
            parent.occurrence_registry.lock().unwrap().instance_count(),
            1,
            "parent registry must be unaffected by the child's mutation"
        );

        // Also verify reset() on the child doesn't wipe the parent's
        // registry — this was the exact bug: reset() replaces the
        // registry *in place* through the shared Arc<Mutex<..>>, so an
        // Arc::clone in Clone would let this reset wipe the parent too.
        let mut child2 = parent.fork();
        child2.reset();
        assert_eq!(
            child2.occurrence_registry.lock().unwrap().instance_count(),
            0,
            "reset() should clear the child's own registry"
        );
        assert_eq!(
            parent.occurrence_registry.lock().unwrap().instance_count(),
            1,
            "parent registry must survive the child's reset()"
        );
    }

    // ═══════════════════════════════════════════════════════════════════
    //  Multi-Circuit Fixture Engine Gap Tests
    // ═══════════════════════════════════════════════════════════════════

    #[test]
    fn test_computed_expressions_aggregate() {
        // Phase 3: Computed expression sums 3 ODE outputs
        use crate::ode::Rk4Solver;
        use crate::ode_builder;
        use std::sync::Arc;

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 10.0,
            ..Default::default()
        });

        // 3 simple ODEs: dx/dt = 1 (each grows linearly at 1/s)
        for i in 1..=3 {
            let name = format!("ode{i}");
            let var = format!("x{i}");
            let rhs: crate::ode::OdeRhs = Arc::new(move |_t, _y, _ctx| vec![1.0]);
            let solver = Rk4Solver::new(&name, vec![var.clone()], vec![0.0], rhs);
            orch.add_ode(&name, solver);
        }

        // Computed expression: total = x1 + x2 + x3
        let expr = ode_builder::parse_derivative("x1 + x2 + x3").unwrap();
        orch.add_computed_expression("total", expr);
        // ode1/ode2/ode3 (idx 0/1/2) each write their own state var x1/x2/x3;
        // mint after the computed expr so the bind pass sees it too.
        mint_and_bind_state_slots(&mut orch, &[(0, "x1", 0.0), (1, "x2", 0.0), (2, "x3", 0.0)]);

        // Step 10 times (10 ticks × 10ms = 100ms = 0.1s)
        for _ in 0..10 {
            orch.step();
        }

        let total = match orch.context.get("total").unwrap() {
            Value::Float(f) => *f,
            _ => panic!("expected Float"),
        };

        // Each ODE at t=0.1s with dx/dt=1 → x ≈ 0.1
        // total ≈ 0.3 (3 × 0.1)
        assert!(
            (total - 0.3).abs() < 0.01,
            "total should be ~0.3, got {total}"
        );
    }

    #[test]
    fn test_gated_expression_blocks_when_gate_false() {
        use crate::ode_builder;

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 1.0,
            ..Default::default()
        });

        // Add a trivial SM so step() doesn't fail
        let sm = make_sm_with_assignment("sm1", "dummy", 0.0);
        orch.add_state_machine("sm1", sm);

        // Add a gated expression: flow = source_current, gate = breaker_closed
        let expr = ode_builder::parse_derivative("source_current").unwrap();
        orch.add_gated_computed_expression("target_flow", expr, "breaker_closed");

        // Set source value and gate = true (open)
        orch.context
            .set("source_current".to_string(), Value::Float(10.0));
        orch.context
            .set("breaker_closed".to_string(), Value::Bool(true));

        orch.step();

        // Flow should propagate
        let flow = orch.context.get("target_flow").and_then(|v| v.as_float());
        assert_eq!(flow, Some(10.0), "gate open → flow should be 10.0");

        // Close the gate (trip)
        orch.context
            .set("breaker_closed".to_string(), Value::Bool(false));

        orch.step();

        // Flow should be zero
        let flow = orch.context.get("target_flow").and_then(|v| v.as_float());
        assert_eq!(flow, Some(0.0), "gate closed → flow should be 0.0");

        // Re-open the gate (reset)
        orch.context
            .set("breaker_closed".to_string(), Value::Bool(true));

        orch.step();

        // Flow should be back to 10.0
        let flow = orch.context.get("target_flow").and_then(|v| v.as_float());
        assert_eq!(
            flow,
            Some(10.0),
            "gate reopened → flow should be 10.0 again"
        );
    }

    #[test]
    fn test_gated_expression_with_float_gate() {
        use crate::ode_builder;

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 1.0,
            ..Default::default()
        });

        let sm = make_sm_with_assignment("sm1", "dummy", 0.0);
        orch.add_state_machine("sm1", sm);

        // Gate on a float: 0.0 = closed, 1.0 = open
        let expr = ode_builder::parse_derivative("pressure").unwrap();
        orch.add_gated_computed_expression("downstream_pressure", expr, "valve_position");

        orch.context
            .set("pressure".to_string(), Value::Float(100.0));
        orch.context
            .set("valve_position".to_string(), Value::Float(1.0));

        orch.step();
        assert_eq!(
            orch.context
                .get("downstream_pressure")
                .and_then(|v| v.as_float()),
            Some(100.0)
        );

        // Close valve (position = 0)
        orch.context
            .set("valve_position".to_string(), Value::Float(0.0));
        orch.step();
        assert_eq!(
            orch.context
                .get("downstream_pressure")
                .and_then(|v| v.as_float()),
            Some(0.0)
        );
    }

    #[test]
    fn test_flow_gate_with_sm_transition() {
        // Phase 2 acceptance: SM enters terminal state → gated flow goes to zero
        use crate::ode_builder;

        // Build a 2-state SM: "open" → "closed" on event "close_valve"
        let ir = crate::StateMachineIR::new("ValveSM", "open")
            .with_state(crate::StateIR::new("open"))
            .with_state(crate::StateIR::new("closed")) // terminal state
            .with_transition(crate::TransitionIR::new("open", "closed").with_event("close_valve"));

        // Verify terminal state detection
        assert_eq!(ir.terminal_states(), vec!["closed"]);

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 1.0,
            ..Default::default()
        });

        orch.add_state_machine("ValveSM", StateMachineRunner::new(ir));

        // Register flow gate: "closed" is a gating state
        orch.add_flow_gate("ValveSM", vec!["closed".to_string()]);

        // Add a gated flow expression
        let expr = ode_builder::parse_derivative("upstream_pressure").unwrap();
        orch.add_gated_computed_expression("downstream_pressure", expr, "ValveSM.__flow_gate");

        // Set upstream pressure
        orch.context
            .set("upstream_pressure".to_string(), Value::Float(100.0));

        // Step — valve is open, flow should propagate
        orch.step();
        let flow = orch
            .context
            .get("downstream_pressure")
            .and_then(|v| v.as_float());
        assert_eq!(flow, Some(100.0), "valve open → flow should be 100.0");

        // Close the valve via SM event
        orch.inject_event("ValveSM", "close_valve");
        orch.step();

        // SM should be in "closed" state → gate closed → flow zero
        let flow = orch
            .context
            .get("downstream_pressure")
            .and_then(|v| v.as_float());
        assert_eq!(flow, Some(0.0), "valve closed → flow should be 0.0");

        // Verify gate variable
        let gate = orch
            .context
            .get("ValveSM.__flow_gate")
            .and_then(|v| match v {
                Value::Bool(b) => Some(*b),
                _ => None,
            });
        assert_eq!(
            gate,
            Some(false),
            "gate should be false when SM is in terminal state"
        );
    }

    #[test]
    fn test_snapshot_thinning() {
        // Phase 4: Only record every 5th tick
        let sm = make_sm_with_assignment("sm1", "speed", 100.0);
        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 1.0,
            snapshot_interval: 5,
            max_trace_length: Some(10),
            ..Default::default()
        });
        orch.add_state_machine("sm1", sm);

        // Step 25 times
        for _ in 0..25 {
            orch.step();
        }

        // With interval=5 over 25 ticks: ticks 5,10,15,20,25 → 5 snapshots
        let trace_len = orch.trace.len();
        assert_eq!(
            trace_len, 5,
            "expected 5 snapshots with interval=5 over 25 ticks, got {trace_len}"
        );
    }

    #[test]
    fn test_max_trace_ring_buffer() {
        // Phase 4: Ring buffer eviction
        let sm = make_sm_with_assignment("sm1", "speed", 100.0);
        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 1.0,
            snapshot_interval: 1,
            max_trace_length: Some(3),
            ..Default::default()
        });
        orch.add_state_machine("sm1", sm);

        for _ in 0..10 {
            orch.step();
        }

        // Max 3 snapshots retained
        assert!(
            orch.trace.len() <= 3,
            "trace should be capped at 3, got {}",
            orch.trace.len()
        );
        // Last snapshot should be tick 10
        assert_eq!(orch.trace.last().unwrap().tick, 10);
    }

    #[test]
    fn test_convergence_iteration_breaker_trip() {
        // Phase 5: Convergence iteration fixes one-tick-delay feedback
        use crate::ode::Rk4Solver;
        use std::sync::Arc;

        // SM: transitions from "armed" to "tripped" when temperature > 50
        // On trip, sets current = 0 (feedback to ODE)
        let mut transition = TransitionIR::new("armed", "tripped")
            .with_guard("temperature > 50")
            .with_action(TransitionActionIR::Structured {
                assignments: vec![AssignmentIR::set("current", 0.0)],
                sends: vec![],
                port_send_ops: Vec::new(),
            });
        transition.is_guard_only = true;

        let sm_ir = StateMachineIR {
            name: "breaker".to_string(),
            states: vec![StateIR::new("armed"), StateIR::new("tripped").final_state()],
            transitions: vec![transition],
            initial: "armed".to_string(),
            regions: vec![],
        };
        let runner = StateMachineRunner::new(sm_ir);

        // ODE: dT/dt = current^2 (I²R heating)
        let rhs: crate::ode::OdeRhs = Arc::new(|_t, _y, ctx| {
            let i = ctx
                .variables
                .get("current")
                .and_then(|v| match v {
                    Value::Float(f) => Some(*f),
                    _ => None,
                })
                .unwrap_or(0.0);
            vec![i * i]
        });
        let solver = Rk4Solver::new("thermal", vec!["temperature".to_string()], vec![20.0], rhs);

        // With convergence enabled
        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 100.0, // large dt to make the delay visible
            convergence_max_iterations: 3,
            convergence_epsilon: 1e-6,
            ..Default::default()
        });
        orch.add_state_machine("breaker", runner);
        orch.add_ode("thermal", solver);
        orch.context.set("current", Value::Float(10.0));
        // SM "breaker" (idx 0) writes `current` on trip; ODE "thermal" (idx 1)
        // writes `temperature` and reads `current`.
        mint_and_bind_state_slots(&mut orch, &[(0, "current", 10.0), (1, "temperature", 20.0)]);

        // Step until breaker trips
        let mut tripped_tick = 0u64;
        for _ in 0..100 {
            let snap = orch.step();
            if snap
                .subsystem_states
                .get("breaker")
                .map_or(false, |s| s.current_state == "tripped")
            {
                tripped_tick = snap.tick;
                break;
            }
        }

        assert!(tripped_tick > 0, "breaker should have tripped");

        // After trip, current should be 0 in context
        let current = match orch.context.get("current").unwrap() {
            Value::Float(f) => *f,
            _ => panic!("expected Float"),
        };
        assert_eq!(current, 0.0, "current should be 0 after trip");
    }

    #[test]
    fn test_port_value_propagation() {
        // Phase 6: Port values propagate through flow connections
        use crate::flows::{PortDirection, PortFeature, PortInstanceIR, PortRegistry};
        use crate::links::{LinkClass, LinkEndpoint, LinkGraph, LinkIR, LinkSourceKind};
        use std::sync::Arc;
        use sysml_core::physics::classify::ClassificationConfidence;

        let mut orch = Orchestrator::new(OrchestratorConfig::default());

        // Create a sensor subsystem (ODE that writes a value)
        let rhs: crate::ode::OdeRhs = Arc::new(|_t, _y, _ctx| vec![5.0]); // 5 units/s
        let solver = Rk4Solver::new(
            "sensor",
            vec!["sensor.currentOut.rms_current".to_string()],
            vec![0.0],
            rhs,
        );
        orch.add_ode("sensor", solver);
        // ODE "sensor" (idx 0) writes the dotted state var it declares.
        mint_and_bind_state_slots(&mut orch, &[(0, "sensor.currentOut.rms_current", 0.0)]);

        // Create port registry with sensor output and firmware input
        let mut registry = PortRegistry::new();

        let mut sensor_port = PortInstanceIR {
            owner: "sensor".to_string(),
            name: "currentOut".to_string(),
            definition: Some("CurrentSensePort".to_string()),
            features: std::collections::HashMap::new(),
            direction: PortDirection::Out,
            is_conjugated: false,
            multiplicity: None,
        };
        sensor_port.features.insert(
            "rms_current".to_string(),
            PortFeature {
                name: "rms_current".to_string(),
                direction: PortDirection::Out,
                type_name: Some("Real".to_string()),
                value: Value::Float(0.0),
            },
        );
        registry.register(sensor_port);

        let mut firmware_port = PortInstanceIR {
            owner: "firmware".to_string(),
            name: "currentIn".to_string(),
            definition: Some("CurrentSenseInPort".to_string()),
            features: std::collections::HashMap::new(),
            direction: PortDirection::In,
            is_conjugated: false,
            multiplicity: None,
        };
        firmware_port.features.insert(
            "rms_current".to_string(),
            PortFeature {
                name: "rms_current".to_string(),
                direction: PortDirection::In,
                type_name: Some("Real".to_string()),
                value: Value::Float(0.0),
            },
        );
        registry.register(firmware_port);

        // Flow connection: sensor.currentOut → firmware.currentIn. RSC-3.5e.5:
        // the non-signal mirror reads the FlowUsage subset of the link graph, so
        // install a single non-signal (MessageChannel) FlowUsage link. No signal
        // propagation plan is installed, so the pair is not in `signal_skip` and
        // the phase-2 mirror copies its features.
        let mut lg = LinkGraph::new();
        lg.intern(LinkIR {
            element_id: sysml_core::ElementId::from_string("link:flow1"),
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: "sensor".into(),
                port: "currentOut".into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: "firmware".into(),
                port: "currentIn".into(),
                resolved_registry_key: None,
            },
            class: LinkClass::MessageChannel,
            class_confidence: ClassificationConfidence::Declared,
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        });

        orch.set_port_registry(registry);
        orch.set_link_graph(lg);

        // Step 10 times (10ms at dt=1ms)
        for _ in 0..10 {
            orch.step();
        }

        // The ODE writes to "sensor.currentOut.rms_current" in context
        // Port propagation should copy this to the sensor output port,
        // then through the flow to the firmware input port,
        // then to "firmware.currentIn.rms_current" in context
        let firmware_val =
            orch.context
                .get("firmware.currentIn.rms_current")
                .and_then(|v| match v {
                    Value::Float(f) => Some(*f),
                    _ => None,
                });

        assert!(
            firmware_val.is_some(),
            "firmware.currentIn.rms_current should be set in context"
        );

        let val = firmware_val.unwrap();
        // At t=10ms=0.01s, sensor value = 5.0 * 0.01 = 0.05
        assert!(
            val > 0.0,
            "firmware should have received a non-zero current value, got {val}"
        );

        // Also check snapshot port_values
        let last_snap = orch.trace.last().unwrap();
        assert!(
            !last_snap.port_values.is_empty(),
            "snapshot should contain port values"
        );
    }

    #[test]
    fn test_50_subsystems_performance() {
        // Phase 4: Stress test with 50+ subsystems
        use crate::ode::Rk4Solver;
        use std::sync::Arc;

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 1.0,
            snapshot_interval: 10, // thin snapshots for perf
            max_trace_length: Some(100),
            ..Default::default()
        });

        // 30 ODEs
        for i in 0..30 {
            let var = format!("temp_{i}");
            let rhs: crate::ode::OdeRhs = Arc::new(move |_t, _y, _ctx| vec![1.0]);
            let solver = Rk4Solver::new(&format!("ode_{i}"), vec![var], vec![20.0], rhs);
            orch.add_ode(&format!("ode_{i}"), solver);
        }

        // 30 SMs (simple 2-state, stays in initial)
        for i in 0..30 {
            let ir = StateMachineIR {
                name: format!("sm_{i}"),
                states: vec![StateIR::new("idle"), StateIR::new("done").final_state()],
                transitions: vec![TransitionIR::new("idle", "done").with_event("trigger")],
                initial: "idle".to_string(),
                regions: vec![],
            };
            orch.add_state_machine(&format!("sm_{i}"), StateMachineRunner::new(ir));
        }

        assert_eq!(orch.subsystems.len(), 60);
        // The 30 ODEs (idx 0..29) each write their own `temp_i`. The 30 SMs
        // never fire (no assignment) so need no slot.
        let temp_names: Vec<String> = (0..30).map(|i| format!("temp_{i}")).collect();
        let slots: Vec<(usize, &str, f64)> = temp_names
            .iter()
            .enumerate()
            .map(|(i, n)| (i, n.as_str(), 20.0))
            .collect();
        mint_and_bind_state_slots(&mut orch, &slots);

        // Step 1000 ticks
        let start = std::time::Instant::now();
        for _ in 0..1000 {
            orch.step();
        }
        let elapsed = start.elapsed();

        // Verify correctness
        let temp_0 = match orch.context.get("temp_0").unwrap() {
            Value::Float(f) => *f,
            _ => panic!("expected Float"),
        };
        // 1000 ticks × 1ms = 1s, dT/dt = 1 → T = 20 + 1 = 21
        assert!(
            (temp_0 - 21.0).abs() < 0.1,
            "temp_0 should be ~21.0, got {temp_0}"
        );

        // Performance: should complete in well under 5 seconds
        assert!(
            elapsed.as_secs() < 5,
            "1000 ticks with 60 subsystems took {:?}, expected < 5s",
            elapsed
        );

        // Memory: trace should be capped
        assert!(
            orch.trace.len() <= 100,
            "trace should be <= 100 (ring buffer), got {}",
            orch.trace.len()
        );
    }

    // RSC-3.6: `test_instance_multiplication_prefixed_odes` was retired with
    // no-slots mode (RSC-3.5f.3). It built two prefixed ODEs via the raw
    // `add_ode_prefixed` no-slots API and relied on `build_scoped_context`
    // (now deleted) for per-instance variable scoping. Slot-backed instance
    // isolation for prefixed ODEs is covered by
    // `compiler::tests::rsc24a_per_instance_isolation_with_slot_writeback`
    // (pins `ode_scoped_fallbacks()` empty + divergent per-instance trajectories).

    /// Build an orchestrator carrying one slot-stored variable
    /// (`circuit5.loadCurrent` ↔ `Panel.circuit5.thermalModel.loadCurrent`)
    /// the way the compiler does: mint the slot table, attach it via
    /// `set_slot_store`. Used by the RSC-2.5 override tests below.
    fn orchestrator_with_load_current_slot() -> Orchestrator {
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};

        let mut orch = Orchestrator::new(OrchestratorConfig::default());
        let mut store = SlotStore::new();
        store.intern(
            SlotMeta::new(
                RuntimeId::scoped(
                    sysml_core::ElementId::from_string("decl:loadCurrent"),
                    [sysml_core::ElementId::from_string("usage:circuit5")],
                ),
                Variability::Parameter,
                WriterId::External,
                "Panel.circuit5.thermalModel.loadCurrent",
                "circuit5.loadCurrent",
            ),
            Value::Float(10.0),
        );
        store.intern(
            SlotMeta::new(
                RuntimeId::top_level(sysml_core::ElementId::from_string("decl:pi")),
                Variability::Constant,
                WriterId::External,
                "pi",
                "pi",
            ),
            Value::Float(std::f64::consts::PI),
        );
        orch.set_slot_store(store);
        orch
    }

    #[test]
    fn apply_overrides_with_aliases_resolves_canonical_spelling_via_slot_table() {
        // RSC-2.5: the UI builds override keys from the tree's `ownerPath`
        // (the canonical path). The slot alias table registers both
        // spellings per slot, and the set_slot dual-spelling mirror keeps
        // the legacy map readable under BOTH — replacing the deleted
        // bidirectional string fan-out.
        let mut orch = orchestrator_with_load_current_slot();

        orch.apply_overrides_with_aliases(&[(
            "Panel.circuit5.thermalModel.loadCurrent".to_string(),
            "60.0".to_string(),
        )])
        .expect("slot-aliased override must resolve");

        let canon = orch
            .context
            .get("Panel.circuit5.thermalModel.loadCurrent")
            .and_then(|v| match v {
                Value::Float(f) => Some(*f),
                _ => None,
            });
        assert_eq!(canon, Some(60.0), "canonical spelling readable");
        let runtime = orch
            .context
            .get("circuit5.loadCurrent")
            .and_then(|v| match v {
                Value::Float(f) => Some(*f),
                _ => None,
            });
        assert_eq!(
            runtime,
            Some(60.0),
            "runtime spelling readable via the set_slot dual-spelling mirror"
        );
        // The typed slot value itself was written.
        let id = orch
            .slot_store()
            .slot_by_name("circuit5.loadCurrent")
            .expect("slot bound");
        assert_eq!(orch.slot_store().get(id), Some(&Value::Float(60.0)));
    }

    #[test]
    fn apply_overrides_with_aliases_resolves_runtime_spelling_via_slot_table() {
        // Reverse direction: runtime spelling in → canonical spelling
        // readable too, through the same single alias-table lookup.
        let mut orch = orchestrator_with_load_current_slot();

        orch.apply_overrides_with_aliases(&[(
            "circuit5.loadCurrent".to_string(),
            "42.0".to_string(),
        )])
        .expect("slot-aliased override must resolve");

        let canon = orch
            .context
            .get("Panel.circuit5.thermalModel.loadCurrent")
            .and_then(|v| match v {
                Value::Float(f) => Some(*f),
                _ => None,
            });
        assert_eq!(
            canon,
            Some(42.0),
            "runtime spelling routes to the slot; canonical mirror readable"
        );
    }

    #[test]
    fn apply_overrides_with_aliases_unknown_name_is_rs002_error() {
        // RSC-2.5 / RS002 (design doc D-2.0.7, §8 Q3, user-approved):
        // a name resolving to neither a slot alias nor an existing context
        // variable fails hard — silent creation died at 2.5. The batch is
        // all-or-nothing: the valid first entry must NOT be applied.
        let mut orch = orchestrator_with_load_current_slot();

        let err = orch
            .apply_overrides_with_aliases(&[
                ("circuit5.loadCurrent".to_string(), "99.0".to_string()),
                ("noSuchVariableXyz".to_string(), "1.0".to_string()),
            ])
            .expect_err("unknown override target must fail hard");
        assert_eq!(
            err,
            OverrideError::UnknownTarget {
                name: "noSuchVariableXyz".to_string()
            }
        );
        assert!(
            err.to_string().contains("RS002"),
            "error names its code: {err}"
        );
        assert!(
            orch.context.get("noSuchVariableXyz").is_none(),
            "the unknown name must not materialize"
        );
        let id = orch
            .slot_store()
            .slot_by_name("circuit5.loadCurrent")
            .expect("slot bound");
        assert_eq!(
            orch.slot_store().get(id),
            Some(&Value::Float(10.0)),
            "all-or-nothing: the valid entry of a failing batch is not applied"
        );
    }

    #[test]
    fn apply_overrides_with_aliases_constant_slot_is_error() {
        // Writing a Variability::Constant slot is an error (D-2.0.3 /
        // D-2.0.7) — even though the name resolves in the alias table.
        let mut orch = orchestrator_with_load_current_slot();

        let err = orch
            .apply_overrides_with_aliases(&[("pi".to_string(), "3.0".to_string())])
            .expect_err("Constant slot override must fail");
        assert_eq!(
            err,
            OverrideError::ConstantSlot {
                name: "pi".to_string()
            }
        );
        assert_eq!(
            orch.context.get("pi"),
            None,
            "the rejected write must not leak into the legacy map"
        );
        let id = orch.slot_store().slot_by_name("pi").expect("slot bound");
        assert_eq!(
            orch.slot_store().get(id),
            Some(&Value::Float(std::f64::consts::PI)),
            "the Constant slot value is untouched"
        );
    }

    #[test]
    fn apply_overrides_with_aliases_dynamic_context_name_still_works() {
        // Names that exist only in the context map (physics short aliases,
        // port payload keys, `__sf_*` waveforms — Phase 3 identity the slot
        // table never mints) must keep accepting overrides: slot aliases
        // resolve first, then existing context keys; only names in NEITHER
        // are RS002.
        let mut orch = orchestrator_with_load_current_slot();
        orch.context
            .set("phaseIn.current".to_string(), Value::Float(0.0));
        orch.context
            .set("__sf_waveform".to_string(), Value::Float(0.0));

        orch.apply_overrides_with_aliases(&[
            ("phaseIn.current".to_string(), "7.5".to_string()),
            ("__sf_waveform".to_string(), "1.0".to_string()),
        ])
        .expect("existing context-map names must keep working");

        assert_eq!(
            orch.context.get("phaseIn.current"),
            Some(&Value::Float(7.5))
        );
        assert_eq!(orch.context.get("__sf_waveform"), Some(&Value::Float(1.0)));
    }

    // RSC-3.6: `canonical_prefix_aliases_state_var_for_tree_lookup` and
    // `test_instance_multiplication_prefixed_sm_with_guard` were retired with
    // no-slots mode (RSC-3.5f.3). Both built prefixed executors via the raw
    // no-slots `add_*_prefixed` API and relied on `build_scoped_context` (now
    // deleted) for per-instance variable scoping. Slot-backed coverage:
    //   - canonical tree-path writeback aliasing — a canonical-spelling
    //     100-tick smoke test (asserts the `Panel.circuitN.bimetalTemp`
    //     canonical spelling mirrors the live integrated value) +
    //     `rsc2_override_roundtrip_canonical_and_runtime` (override round-trip).
    //   - prefixed-SM-reads-prefixed-ODE-state — the instance-multiplied SM
    //     fixtures (`rsc24b_*` two_unit_sm) + the exchange-plane TripLogic
    //     conformance fixture, all compiler-built with slots.

    #[test]
    fn test_state_introspection_vars_populated() {
        let ir = StateMachineIR {
            name: "sm".to_string(),
            states: vec![StateIR::new("A"), StateIR::new("B")],
            transitions: vec![TransitionIR::new("A", "B").with_event("go")],
            initial: "A".to_string(),
            regions: vec![],
        };
        let mut orch = Orchestrator::new(OrchestratorConfig::default());
        orch.add_state_machine("sm1", StateMachineRunner::new(ir));

        // Tick 1: should be in A; recent_transitions empty (no prior snapshot).
        orch.step();
        let substates = orch.context.get("__active_substates").cloned();
        assert!(
            matches!(&substates, Some(Value::List(v)) if v.contains(&Value::String("A".into()))),
            "expected __active_substates to contain \"A\", got {:?}",
            substates
        );

        // Tick 2: inject "go" event, transition to B.
        orch.inject_event("sm1", "go");
        orch.step();
        let substates = orch.context.get("__active_substates").cloned();
        assert!(
            matches!(&substates, Some(Value::List(v)) if v.contains(&Value::String("B".into()))),
            "expected __active_substates to contain \"B\", got {:?}",
            substates
        );

        let transitions = orch.context.get("__recent_transitions").cloned();
        assert!(
            matches!(&transitions, Some(Value::List(v)) if !v.is_empty()),
            "expected __recent_transitions to be non-empty after A→B, got {:?}",
            transitions
        );
    }

    // -----------------------------------------------------------------------
    // RSC-3.2 — SignalLink directed-propagation cutover (behavioural)
    // -----------------------------------------------------------------------

    /// Build an orchestrator that routes one signal feature `v` from
    /// `sensor.out` (Out) to `monitor.in` (In) over a classified SignalLink:
    /// a feature-bearing port registry, a matching flow connection, a slot
    /// table with `sensor.out.v` / `monitor.in.v` slots, and the compiled
    /// [`SignalPropagation`] plan. No subsystems — `step` still runs
    /// `propagate_port_values`, which is the unit under test.
    fn signal_cutover_orchestrator() -> Orchestrator {
        use crate::flows::port::{PortFeature, PortInstanceIR};
        use crate::flows::PortDirection;
        use crate::links::{LinkClass, LinkEndpoint, LinkGraph, LinkIR, LinkSourceKind};
        use crate::slots::{RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
        use sysml_core::physics::classify::ClassificationConfidence;

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 10.0,
            max_ticks: 1000,
            max_time_ms: 10_000.0,
            ..Default::default()
        });

        // Registry: sensor.out (Out, feat v) -> monitor.in (In, feat v).
        let mut reg = crate::flows::PortRegistry::new();
        let mut src = PortInstanceIR::new("sensor", "out").with_direction(PortDirection::Out);
        src.add_feature(PortFeature {
            name: "v".into(),
            direction: PortDirection::Out,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        reg.register(src);
        let mut tgt = PortInstanceIR::new("monitor", "in").with_direction(PortDirection::In);
        tgt.add_feature(PortFeature {
            name: "v".into(),
            direction: PortDirection::In,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        reg.register(tgt);

        // Slot table: the two feature slots (Continuous, Orchestrator), spelled
        // as the port-value keys.
        let mut store = SlotStore::new();
        for name in ["sensor.out.v", "monitor.in.v"] {
            store.intern(
                SlotMeta::new(
                    RuntimeId::top_level(sysml_core::ElementId::from_string(format!(
                        "decl:{name}"
                    ))),
                    Variability::Continuous,
                    WriterId::Orchestrator,
                    name,
                    name,
                ),
                Value::Float(0.0),
            );
        }

        // SignalLink graph + compiled propagation plan.
        let mut lg = LinkGraph::new();
        lg.intern(LinkIR {
            element_id: sysml_core::ElementId::from_string("link:sig"),
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: "sensor".into(),
                port: "out".into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: "monitor".into(),
                port: "in".into(),
                resolved_registry_key: None,
            },
            class: LinkClass::SignalLink,
            class_confidence: ClassificationConfidence::Declared,
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        });
        let (plan, diags) = crate::links::compile_signal_propagation(&lg, &store, &reg);
        assert!(diags.is_empty(), "no cycle expected");
        assert_eq!(plan.len(), 1, "one signal feature pair");

        orch.set_port_registry(reg);
        orch.set_link_graph(lg);
        orch.set_slot_store(store);
        orch.set_signal_propagation(plan);
        orch
    }

    /// RSC-3.2 signal cutover (single-path, RSC-3.6): the signal feature must
    /// propagate `sensor.out → monitor.in` each tick through the slot-routed
    /// path, and the target slot must hold the last routed value. The former
    /// slots-on-vs-off parity arm was retired with no-slots mode (RSC-3.5f.3 —
    /// slot routing is now unconditional).
    #[test]
    fn rsc32_signal_propagation_cutover_parity() {
        let mut on = signal_cutover_orchestrator();

        // Seed the source feature value into the context (phase 1 copies it to
        // the source port feature, then phase 2-signal routes it to the
        // target). Drive a changing value each tick.
        for tick in 1..=10 {
            let driven = Value::Float(tick as f64 * 1.5);
            on.context.set("sensor.out.v".to_owned(), driven.clone());
            on.step();

            // The value propagated to the target context key.
            assert_eq!(
                on.context.get("monitor.in.v"),
                Some(&driven),
                "signal must propagate to monitor.in.v at tick {tick}"
            );
        }

        // Slot store carries the routed value.
        let store = on.slot_store();
        let sid = store.slot_by_name("monitor.in.v").expect("target slot");
        assert_eq!(
            store.get(sid),
            Some(&Value::Float(15.0)),
            "the target slot holds the last routed value"
        );
    }

    /// RSC-5.3 (D-5.0.4): a SignalLink between an `[A]` source slot and a
    /// `[mA]` target slot (same dimension, different scale) auto-converts the
    /// magnitude at the boundary — `2 A` lands as `2000 mA` in the target slot.
    /// This is the flow/link half of D-5.0.4 ("convert by scale ratio there").
    #[test]
    fn rsc53_signal_boundary_converts_cross_scale_units() {
        use crate::flows::port::{PortFeature, PortInstanceIR};
        use crate::flows::PortDirection;
        use crate::links::{LinkClass, LinkEndpoint, LinkGraph, LinkIR, LinkSourceKind};
        use crate::slots::{MeasurementRef, RuntimeId, SlotMeta, SlotStore, Variability, WriterId};
        use sysml_core::physics::classify::ClassificationConfidence;

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 10.0,
            max_ticks: 1000,
            max_time_ms: 10_000.0,
            ..Default::default()
        });

        // sensor.out (Out, feat i) -> monitor.in (In, feat i).
        let mut reg = crate::flows::PortRegistry::new();
        let mut src = PortInstanceIR::new("sensor", "out").with_direction(PortDirection::Out);
        src.add_feature(PortFeature {
            name: "i".into(),
            direction: PortDirection::Out,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        reg.register(src);
        let mut tgt = PortInstanceIR::new("monitor", "in").with_direction(PortDirection::In);
        tgt.add_feature(PortFeature {
            name: "i".into(),
            direction: PortDirection::In,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        reg.register(tgt);

        // The source slot stores amperes; the target slot stores milliamperes
        // (same ElectricCurrent dimension, scale 1.0 vs 1e-3). mRefs are sourced
        // from the unit table so the dimension matches exactly.
        let amp = crate::expressions::units::lookup_unit("A").expect("A");
        let milliamp = crate::expressions::units::lookup_unit("mA").expect("mA");
        let mut store = SlotStore::new();
        let src_meta = SlotMeta::new(
            RuntimeId::top_level(sysml_core::ElementId::from_string("decl:sensor.out.i")),
            Variability::Continuous,
            WriterId::Orchestrator,
            "sensor.out.i",
            "sensor.out.i",
        )
        .with_m_ref(Some(MeasurementRef {
            dimension: amp.dimension,
            unit: Some(std::sync::Arc::from("A")),
            scale: amp.scale,
            offset: amp.offset,
        }));
        store.intern(src_meta, Value::Float(0.0));
        let tgt_meta = SlotMeta::new(
            RuntimeId::top_level(sysml_core::ElementId::from_string("decl:monitor.in.i")),
            Variability::Continuous,
            WriterId::Orchestrator,
            "monitor.in.i",
            "monitor.in.i",
        )
        .with_m_ref(Some(MeasurementRef {
            dimension: milliamp.dimension,
            unit: Some(std::sync::Arc::from("mA")),
            scale: milliamp.scale,
            offset: milliamp.offset,
        }));
        store.intern(tgt_meta, Value::Float(0.0));

        let mut lg = LinkGraph::new();
        lg.intern(LinkIR {
            element_id: sysml_core::ElementId::from_string("link:sig"),
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: "sensor".into(),
                port: "out".into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: "monitor".into(),
                port: "in".into(),
                resolved_registry_key: None,
            },
            class: LinkClass::SignalLink,
            class_confidence: ClassificationConfidence::Declared,
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        });
        let (plan, diags) = crate::links::compile_signal_propagation(&lg, &store, &reg);
        assert!(diags.is_empty(), "no cycle expected");
        assert_eq!(plan.len(), 1, "one signal feature pair");
        assert!(
            plan.pairs()[0].convert.is_some(),
            "the A->mA pair must carry a precomputed boundary conversion"
        );

        orch.set_port_registry(reg);
        orch.set_link_graph(lg);
        orch.set_slot_store(store);
        orch.set_signal_propagation(plan);

        // Seed 2 amperes at the source; after a tick the target slot holds the
        // same current expressed in milliamperes (2 A = 2000 mA).
        orch.context
            .set("sensor.out.i".to_owned(), Value::Float(2.0));
        orch.step();

        let store = orch.slot_store();
        let tid = store.slot_by_name("monitor.in.i").expect("target slot");
        match store.get(tid) {
            Some(Value::Float(v)) => {
                assert!(
                    (v - 2000.0).abs() < 1e-9,
                    "2 A must convert to 2000 mA, got {v}"
                )
            }
            other => panic!("target slot should hold a converted Float, got {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // RSC-3.5d — propagate_port_values slot materializer (phases 1/3) +
    // phase-2 non-signal-copy load-bearing probe
    // -----------------------------------------------------------------------

    /// Build an orchestrator routing one feature `v` over a NON-signal flow
    /// `src.out → dst.in`. A single non-signal FlowUsage link is installed but
    /// NO `SignalPropagation` plan, so `mint_signal_feature_slots` never runs and
    /// the port-feature slots are absent — exactly a PowerBond/MessageChannel-shaped
    /// flow whose ONLY propagation mechanism today is the phase-2 string copy.
    /// (No subsystems; `step` still runs `propagate_port_values`, the unit under test.)
    fn nonsignal_flow_orchestrator() -> Orchestrator {
        use crate::flows::port::{PortFeature, PortInstanceIR};
        use crate::flows::PortDirection;
        use crate::links::{LinkClass, LinkEndpoint, LinkGraph, LinkIR, LinkSourceKind};
        use sysml_core::physics::classify::ClassificationConfidence;

        let mut orch = Orchestrator::new(OrchestratorConfig {
            dt_ms: 10.0,
            max_ticks: 1000,
            max_time_ms: 10_000.0,
            ..Default::default()
        });

        let mut reg = crate::flows::PortRegistry::new();
        let mut src = PortInstanceIR::new("src", "out").with_direction(PortDirection::Out);
        src.add_feature(PortFeature {
            name: "v".into(),
            direction: PortDirection::Out,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        reg.register(src);
        let mut dst = PortInstanceIR::new("dst", "in").with_direction(PortDirection::In);
        dst.add_feature(PortFeature {
            name: "v".into(),
            direction: PortDirection::In,
            type_name: Some("Real".into()),
            value: Value::Float(0.0),
        });
        reg.register(dst);
        // RSC-3.5e.5: a single non-signal (MessageChannel) FlowUsage link is the
        // mirror's input. No slot store / signal plan is installed, so phases 1/3
        // take the string fallback, the pair is absent from `signal_skip`, and
        // phase 2 (the non-signal copy) is the sole propagation path — exactly a
        // PowerBond/MessageChannel-shaped flow with no minted port-feature slots.
        let mut lg = LinkGraph::new();
        lg.intern(LinkIR {
            element_id: sysml_core::ElementId::from_string("link:power"),
            kind: LinkSourceKind::FlowUsage,
            source: LinkEndpoint {
                element_id: None,
                owner: "src".into(),
                port: "out".into(),
                resolved_registry_key: None,
            },
            target: LinkEndpoint {
                element_id: None,
                owner: "dst".into(),
                port: "in".into(),
                resolved_registry_key: None,
            },
            class: LinkClass::MessageChannel,
            class_confidence: ClassificationConfidence::Declared,
            is_succession: false,
            is_move: false,
            is_push: false,
            payload_type: None,
            source_payload_type: None,
            target_payload_type: None,
            via_interface: None,
        });
        orch.set_port_registry(reg);
        orch.set_link_graph(lg);
        orch
    }

    /// RSC-3.5d Piece A — phases 1/3 slot materializer (single-path, RSC-3.6).
    ///
    /// On the signal-cutover model (port-feature slots ARE minted), phases 1/3
    /// read/write the signal through slots: the driven source value must
    /// materialize into the target port feature's `port_values` snapshot each
    /// tick. The former slots-on-vs-off parity arm was retired with no-slots
    /// mode (RSC-3.5f.3 — slot routing is now unconditional).
    #[test]
    fn rsc35d_phases_1_3_slot_materializer_parity() {
        let mut on = signal_cutover_orchestrator();

        for tick in 1..=10 {
            let driven = Value::Float(tick as f64 * 2.25 - 0.5);
            on.context.set("sensor.out.v".to_owned(), driven.clone());
            let snap_on = on.step();

            // Phases 1/3 materialize the routed value into the target port
            // feature snapshot.
            assert_eq!(
                snap_on
                    .port_values
                    .get("monitor.in")
                    .and_then(|f| f.get("v")),
                Some(&driven),
                "phases 1/3 slot materializer: monitor.in.v in port_values at tick {tick}"
            );
        }
    }

    /// RSC-3.5d Piece A — phases 1/3 still propagate a NON-signal flow when no
    /// slots exist. The string fallback must remain intact: driving `src.out.v`
    /// from the context must reach `dst.in.v` (phase 1 string-read → phase 2
    /// copy → phase 3 string-write), and `dst.out` port_values must hold the
    /// copied value. Proves Piece A did not strand the no-slot path.
    #[test]
    fn rsc35d_phases_1_3_string_fallback_intact_no_slots() {
        let mut orch = nonsignal_flow_orchestrator();
        for tick in 1..=5 {
            let driven = Value::Float(tick as f64 * 3.0);
            orch.context.set("src.out.v".to_owned(), driven.clone());
            let snap = orch.step();
            // Phase 2 copied src.out.v → dst.in.v (registry feature), phase 3
            // wrote it back to the context string key.
            assert_eq!(
                snap.port_values.get("dst.in").and_then(|f| f.get("v")),
                Some(&driven),
                "non-signal phase-2 copy must land in dst.in port_values at tick {tick}"
            );
            assert_eq!(
                orch.context.get("dst.in.v"),
                Some(&driven),
                "phase-3 string fallback must write dst.in.v into context at tick {tick}"
            );
        }
    }

    /// RSC-3.5d Piece B — empirical GO/NO-GO for deleting the phase-2 non-signal
    /// string copy. Run the same non-signal-flow model twice — once with the
    /// copy enabled (production), once with it disabled (the proposed deletion)
    /// — and diff `port_values` per tick. If they diverge, the copy is
    /// load-bearing and the deletion is NO-GO.
    ///
    /// They DO diverge today: with the copy removed, no plane delivers the
    /// PowerBond/MessageChannel value to the target port, so `dst.in.v` stays
    /// at its initial 0.0 while the production path shows the driven value.
    /// PowerBond→physics and MessageChannel→ExchangePlane delivery are
    /// 3.5e/3.5f (held for the user), so this is the in-tree NO-GO proof.
    #[test]
    fn rsc35d_piece_b_nonsignal_copy_is_load_bearing() {
        let mut enabled = nonsignal_flow_orchestrator();
        let mut deleted = nonsignal_flow_orchestrator();
        deleted.set_nonsignal_phase2_enabled(false);

        let mut diverged = false;
        for tick in 1..=5 {
            let driven = Value::Float(tick as f64 * 4.0);
            enabled.context.set("src.out.v".to_owned(), driven.clone());
            deleted.context.set("src.out.v".to_owned(), driven.clone());
            let snap_en = enabled.step();
            let snap_del = deleted.step();

            let en_tgt = snap_en
                .port_values
                .get("dst.in")
                .and_then(|f| f.get("v"))
                .cloned();
            let del_tgt = snap_del
                .port_values
                .get("dst.in")
                .and_then(|f| f.get("v"))
                .cloned();

            // Production (copy enabled) delivers the driven value to the target.
            assert_eq!(
                en_tgt,
                Some(driven.clone()),
                "copy-enabled: dst.in.v must receive driven value at tick {tick}"
            );
            // Deletion strands the target at its initial value (no plane routes
            // a non-signal flow yet).
            assert_eq!(
                del_tgt,
                Some(Value::Float(0.0)),
                "copy-deleted: dst.in.v must remain stranded at 0.0 at tick {tick}"
            );
            if en_tgt != del_tgt {
                diverged = true;
            }
        }
        assert!(
            diverged,
            "RSC-3.5d Piece B: deleting the phase-2 non-signal copy MUST diverge \
             port_values (proving it is load-bearing → NO-GO)"
        );
    }

    /// RSC-3.5d Piece B control — when the copy is NOT load-bearing because the
    /// flow is signal-classified (the signal slot pass owns it), disabling the
    /// non-signal copy leaves `port_values` byte-identical. Confirms the probe
    /// flag isolates exactly the non-signal copy and nothing the signal pass
    /// already covers.
    #[test]
    fn rsc35d_piece_b_signal_flow_unaffected_by_nonsignal_toggle() {
        let mut with_copy = signal_cutover_orchestrator();
        let mut without_copy = signal_cutover_orchestrator();
        without_copy.set_nonsignal_phase2_enabled(false);

        for tick in 1..=10 {
            let driven = Value::Float(tick as f64 * 1.5);
            with_copy
                .context
                .set("sensor.out.v".to_owned(), driven.clone());
            without_copy.context.set("sensor.out.v".to_owned(), driven);
            let snap_w = with_copy.step();
            let snap_wo = without_copy.step();
            let w_pv: std::collections::BTreeMap<_, _> = snap_w.port_values.into_iter().collect();
            let wo_pv: std::collections::BTreeMap<_, _> = snap_wo.port_values.into_iter().collect();
            assert_eq!(
                w_pv, wo_pv,
                "signal flow is owned by the slot pass — toggling the non-signal \
                 copy must not change port_values at tick {tick}"
            );
        }
    }
}
