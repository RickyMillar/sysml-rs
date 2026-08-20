//! Execution session types for the service layer.
//!
//! Wraps the raw runtime runners (`StateMachineRunner`, `ActionRunner`,
//! `Orchestrator`) with session lifecycle management (timeouts, history,
//! limits). These types are the domain logic that was previously embedded
//! in the LSP server's simulation / action / orchestrator session files.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use tokio::sync::broadcast;

use sysml_id::ElementId;
use sysml_runtime::breakpoint::{new_breakpoint_id, Breakpoint, BreakpointId};
use sysml_runtime::cases::VerdictKind;
use sysml_runtime::orchestrator::{ExecutionSnapshot, Orchestrator};

/// Bounded fan-out capacity for the per-session snapshot broadcast.
///
/// Picked to absorb short client stalls (a few hundred ms at typical 10 Hz
/// orchestrator tick rates) without letting a silently-dead subscriber pin
/// unbounded memory. Slow subscribers receive `RecvError::Lagged(N)` and
/// are expected to recover via a fresh `sessions_info` fetch — the server
/// does not queue per-receiver.
pub const SNAPSHOT_BROADCAST_CAPACITY: usize = 64;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Session idle timeout (60 minutes, per ADR-005). A session is reaped once
/// it has been idle — no step / inject / poll — for this long. This is an
/// INACTIVITY clock (`RuntimeSession::last_activity`), not age since creation,
/// so a long-running *active* session is never wrongly reaped (ADR-005's goal).
pub const SESSION_TIMEOUT: Duration = Duration::from_secs(3600);

/// Maximum history entries retained per session.
pub const MAX_HISTORY: usize = 1_000;

/// Hard ceiling on the `ticks` argument of a single `sessions.step` bulk call.
///
/// A bulk step runs the orchestrator server-side (no per-tick HTTP round-trip),
/// which is what makes a sub-millisecond-resolution run (e.g. a hybrid ODE chain,
/// ~5,856 ticks to trip at dt=100ns) reachable in a live demo. This bound keeps
/// a single request from monopolising the service thread; a request exceeding it
/// is a hard `InvalidInput` (never a silent clamp — the caller must ask for a
/// legal amount). The orchestrator's own `max_ticks`/`max_time_ms` still bound
/// the run independently (see [`RuntimeSession::step_many`]).
pub const MAX_BULK_STEP_TICKS: u64 = 20_000;

/// Default number of **orchestrator snapshots** retained per session for
/// the fork-at-tick feature (R4).
///
/// This is a ring buffer of cloned `Orchestrator` states keyed by tick,
/// used by `sysml.sessions.fork_with_overrides` when `at_tick` is supplied.
/// Distinct from `MAX_HISTORY` (which bounds the lighter-weight
/// `ExecutionSnapshot` trace used for diffing/timeline analytics).
///
/// Orchestrator clones are heavier than execution snapshots — they include
/// all subsystem executor state, so 256 is a deliberate trade-off between
/// rewind depth and memory footprint. Service-level overrides can pass a
/// different value via [`RuntimeSession::set_snapshot_retention`].
pub const DEFAULT_SNAPSHOT_RETENTION_TICKS: usize = 256;

/// Default archive cadence, in ticks (UX closeout arc #7 /
///
/// `RuntimeSession::step()` used to push a `fork_for_archive()` deep clone
/// of the orchestrator on **every** tick — profiling showed that clone is
/// ~89% of per-tick cost (10.6 ms/tick measured on a hybrid session,
/// `fork_for_archive` alone ≈ 9.45 ms), because the archive is a rewind/fork
/// buffer that discards almost every entry it clones (bounded ring of
/// [`DEFAULT_SNAPSHOT_RETENTION_TICKS`]). `step()`, the `ExecutionSnapshot`
/// build, breakpoint/crossing/trip detection, and time-series append are
/// untouched by this — they stay every-tick.
///
/// A coarse cadence archives every Nth tick instead, **plus** a forced
/// archive on any tick classified event-significant (state-machine
/// transition, a boolean context variable flipping value — the generic
/// "trip-latch flip" — or a breakpoint firing) so the one workflow the
/// archive exists for — rewind exactly to an event — stays intact. See
/// [`RuntimeSession::step`] for the forced-archive classification.
///
/// 100 is a perf-budget pick: it takes the dominant `fork_for_archive` cost
/// from ~9.45 ms/tick to ~0.09 ms/tick amortized (well within the
/// ~0.3-0.4 ms/tick target), while `DEFAULT_SNAPSHOT_RETENTION_TICKS = 256`
/// archived checkpoints at this cadence reach back 25,600 ticks — *longer*
/// rewind reach than today's every-tick archive at the same memory
/// footprint. Distinct from, and never coupled to, `dt` (a model/numerical
/// choice) or `OrchestratorConfig::snapshot_interval` (a different
/// mechanism, at a different layer, thinning the orchestrator's own
/// `trace: Vec<ExecutionSnapshot>` that feeds causation/sequence analysis —
/// reusing it here would silently thin that trace too).
pub const DEFAULT_ARCHIVE_CADENCE_TICKS: u64 = 100;

/// Fallback kind label used on `SubsystemSummary` when the session has
/// not been stepped yet and no executor snapshot is available.
pub const UNKNOWN_KIND_LABEL: &str = "unknown";

/// Per-kind concurrent session caps. Returns the hard cap for the given kind.
///
/// The buckets are independent: saturating Simulation does not block Action
/// or Orchestrator sessions.
pub fn quota_for(kind: SessionKind) -> usize {
    match kind {
        SessionKind::Simulation => 30,
        SessionKind::Action => 30,
        SessionKind::Orchestrator => 20,
    }
}

// ---------------------------------------------------------------------------
// SessionKind
// ---------------------------------------------------------------------------

/// The kind of execution a session represents.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    serde::Serialize,
    serde::Deserialize,
    schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum SessionKind {
    /// A state-machine simulation session.
    Simulation,
    /// An action execution session.
    Action,
    /// A multi-subsystem orchestrator session.
    Orchestrator,
}

impl SessionKind {
    /// Number of distinct kinds — used to size per-kind counter arrays.
    pub const VARIANT_COUNT: usize = 3;

    /// Human-readable label for logging and error messages.
    pub fn as_str(self) -> &'static str {
        match self {
            SessionKind::Simulation => "simulation",
            SessionKind::Action => "action",
            SessionKind::Orchestrator => "orchestrator",
        }
    }

    /// Dense index into per-kind counter arrays. The value is stable across
    /// the lifetime of the process; it is not part of any serialised format.
    pub fn index(self) -> usize {
        match self {
            SessionKind::Simulation => 0,
            SessionKind::Action => 1,
            SessionKind::Orchestrator => 2,
        }
    }
}

impl fmt::Display for SessionKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Generate a fresh opaque session id.
///
/// canonical-key: synthetic-session-uuid — sessions are intentionally
/// reparse-instable; same source + same target session can be started
/// multiple times and each new session must be distinguishable.
///
/// Session keys are typed [`ElementId`]s minted via `ElementId::new_v4()`.
/// They are UUID-shaped so they do not collide on `file://` URIs that
/// contain colons, and so external callers cannot derive subsystem
/// metadata by parsing the key. The typed wrapper guards against
/// concatenating URIs and names into ad-hoc string keys (S1.T7).
pub fn new_session_id() -> ElementId {
    ElementId::new_v4()
}

// ---------------------------------------------------------------------------
// SessionSummary / SessionDetail / SessionQuota — UI-facing projections
// ---------------------------------------------------------------------------

/// A serializable summary of a live session — everything the UI needs to
/// render a session card without a second round-trip.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SessionSummary {
    /// Opaque session identifier (UUID v4).
    pub id: String,
    /// Kind of execution this session wraps.
    pub kind: SessionKind,
    /// URI of the model being executed.
    pub uri: String,
    /// Name of the primary subsystem (state machine or action) driven by
    /// this session. `None` for multi-subsystem orchestrators.
    pub subsystem_name: Option<String>,
    /// User-provided display name (set via `sessions.rename`).
    pub label: Option<String>,
    /// Wall-clock creation time in unix milliseconds.
    pub created_at_ms: u64,
    /// Wall-clock milliseconds elapsed since creation.
    pub elapsed_ms: u64,
    /// Current orchestrator tick.
    pub tick: u64,
    /// Current simulation time in milliseconds.
    pub time_ms: f64,
    /// Current state of the primary subsystem, if applicable.
    pub current_state: Option<String>,
    /// Whether the session has completed execution.
    pub completed: bool,
    /// Whether the session has exceeded the inactivity timeout.
    pub is_expired: bool,
    /// Number of snapshots currently retained in history.
    pub history_len: usize,
    /// Number of subsystems in the underlying orchestrator.
    pub subsystem_count: usize,
    /// If this session was produced by `sysml.sessions.fork`, the tick at
    /// which it branched from the parent. `None` for sessions created via
    /// `*.start` or after `sessions.reset`. UX timelines should anchor
    /// compare-mode playheads on this value when present.
    pub fork_point_tick: Option<u64>,
    /// Ticks currently retained in the orchestrator archive, oldest →
    /// newest (UX closeout arc #7 §2.4 honesty flag) — exactly the set of
    /// ticks `sessions.fork_with_overrides(at_tick=…)` will accept. A
    /// client uses this to render fork/rewind affordances only at valid
    /// points instead of guessing and hitting the fail-hard
    /// `ForkAtTickError::SnapshotMissing`. Empty when the archive is
    /// disabled (`snapshot_retention_ticks == 0`) or the session hasn't
    /// stepped yet.
    #[serde(default)]
    pub forkable_ticks: Vec<u64>,
    /// Whether the session is currently halted at a breakpoint (mirrors
    /// [`RuntimeSession::is_paused`]). BP1: the client composes a halt
    /// cause from `paused` / `completed` / `ticks_advanced` — there is no
    /// separate `HaltReason` enum. `#[serde(default)]` for wire
    /// compatibility with clients that pre-date this field.
    #[serde(default)]
    pub paused: bool,
    /// The id of the breakpoint that triggered the current pause, if any
    /// (mirrors [`RuntimeSession::paused_at_breakpoint`]). `None` when
    /// `paused` is `false` or the session was never paused.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub paused_at_breakpoint: Option<BreakpointId>,
    /// Ticks actually advanced by the call that produced this summary —
    /// e.g. `sysml.sessions.step`'s real `step_many` return, which may be
    /// less than the requested `ticks` if the run halted early on a
    /// breakpoint pause or the orchestrator's configured tick/time limit.
    /// This is a **per-call** value, not persisted session state: it is
    /// always `0` on summaries built by non-stepping commands (info,
    /// reset, fork, inject, resume, …) and is only ever populated by
    /// `sysml.sessions.step` itself (see `sessions_step_internal`).
    /// `#[serde(default)]` for wire compatibility.
    #[serde(default)]
    pub ticks_advanced: u64,
    /// Model/run provenance captured at session creation (B6 remainder):
    /// workspace-graph content digest (same identity as store commits /
    /// baselines) + corroborating git state + resolved workspace root.
    /// `None` = a session predating capture, or one minted outside the
    /// service layer — never fabricated. Report exports read this into
    /// `ReportProvenance.model`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<sysml_store::SessionProvenance>,
    /// Parameter overrides applied when this session was BUILT, as
    /// `(key, value_string)` pairs in the order the caller supplied them.
    ///
    /// Scenario provenance, deliberately NOT merged with overrides applied
    /// mid-run via `sessions.step(overrides)`. These were in force at tick 0,
    /// so every snapshot in the history reflects them; a step-time override
    /// only takes effect from the tick it was applied at. A reader cannot
    /// call a run "the severe case" without knowing which of the two they are
    /// looking at — hence a distinct field.
    ///
    /// Empty for a session created with no overrides, i.e. the model's own
    /// declared defaults: the baseline scenario.
    #[serde(default)]
    pub create_overrides: Vec<(String, String)>,
}

/// A serializable summary of one subsystem within a session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SubsystemSummary {
    /// Subsystem name (as registered on the orchestrator).
    pub name: String,
    /// Kind label: "stateMachine", "action", "ode", "discrete", etc.
    pub kind_label: String,
    /// Current state name or action node ID.
    pub current_state: String,
    /// Whether this subsystem has completed.
    pub completed: bool,
    /// Transitions currently available from the current state.
    pub available_transitions: Vec<(String, String)>,
    /// Number of deferred events currently queued (SM-specific, 0 for others).
    #[serde(default)]
    pub deferred_event_count: usize,
    /// Source element id of the subsystem, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
}

/// Full per-session detail returned by `sessions.info`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SessionDetail {
    /// Flat summary projection.
    pub summary: SessionSummary,
    /// All subsystems in the underlying orchestrator.
    pub subsystems: Vec<SubsystemSummary>,
    /// Latest recorded snapshot, if any.
    pub latest_snapshot: Option<ExecutionSnapshot>,
}

/// Per-kind session budget usage.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct BucketUsage {
    /// Currently live sessions of this kind.
    pub used: usize,
    /// Hard cap for this kind.
    pub cap: usize,
}

/// Budget usage across all session kinds.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SessionQuota {
    /// Simulation bucket.
    pub simulation: BucketUsage,
    /// Action bucket.
    pub action: BucketUsage,
    /// Orchestrator bucket.
    pub orchestrator: BucketUsage,
}

// ---------------------------------------------------------------------------
// SessionDivergence — diff between two sessions (B7)
// ---------------------------------------------------------------------------

/// Per-subsystem state difference between two sessions.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SubsystemDiff {
    /// Subsystem name shared by both sides.
    pub name: String,
    /// `current_state` on side A at its latest snapshot (or `null` if the
    /// subsystem is not present or the side has no snapshot).
    pub a_state: Option<String>,
    /// `current_state` on side B at its latest snapshot.
    pub b_state: Option<String>,
    /// Source element id of the subsystem, when known (taken from whichever
    /// side carries it; side A preferred).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
}

/// Per-variable value difference between two sessions' shared contexts.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct VariableDiff {
    /// Variable name.
    pub name: String,
    /// Value on side A (serialized via `sysml-core`'s JSON encoding). `null`
    /// if the variable is absent on that side.
    pub a_value: Option<sysml_core::Value>,
    /// Value on side B.
    pub b_value: Option<sysml_core::Value>,
}

/// Latest-snapshot divergence between two sessions.
///
/// Returned by `sysml.sessions.diff`. Compares the most recent snapshot of
/// session `a` against session `b`:
/// - `current_tick_a` / `current_tick_b`: the tick of each side's latest
///   snapshot, or `0` if no snapshot has been taken yet.
/// - `subsystem_diffs`: every subsystem that either exists on only one side
///   or has a different `current_state` on the two sides.
/// - `variable_diffs`: every context variable whose value differs (including
///   variables present on only one side).
///
/// For "where did they first diverge?" questions use
/// [`SessionTimelineDivergence`] (returned by `sysml.sessions.diff_timeline`)
/// instead.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SessionDivergence {
    /// Session id of side A (echoed back for caller convenience).
    pub a_id: String,
    /// Session id of side B.
    pub b_id: String,
    /// Latest tick on side A (0 if no snapshot yet).
    pub current_tick_a: u64,
    /// Latest tick on side B.
    pub current_tick_b: u64,
    /// Subsystems whose `current_state` differs or that are unique to one side.
    pub subsystem_diffs: Vec<SubsystemDiff>,
    /// Context variables that differ.
    pub variable_diffs: Vec<VariableDiff>,
}

/// Per-tick diff within a timeline divergence report.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct TickDiff {
    /// The tick at which this diff was observed. Same tick on both sides.
    pub tick: u64,
    /// Subsystems whose `current_state` differs at this tick.
    pub subsystem_diffs: Vec<SubsystemDiff>,
    /// Context variables whose value differs at this tick.
    pub variable_diffs: Vec<VariableDiff>,
}

/// Tick-aligned divergence between two sessions' full histories.
///
/// Returned by `sysml.sessions.diff_timeline`. Walks both sessions'
/// recorded snapshots in tick order and reports **where** they first
/// diverged plus the per-tick deltas across the shared tick range.
///
/// The session history is bounded to `MAX_HISTORY` (1000 snapshots), so
/// very long runs may have evicted the fork point; `history_truncated`
/// signals that the real divergence point may lie before
/// `shared_start_tick`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct SessionTimelineDivergence {
    /// Session id of side A.
    pub a_id: String,
    /// Session id of side B.
    pub b_id: String,
    /// Earliest tick present in both histories. `None` if the two
    /// histories do not overlap (or if either side has no snapshots).
    pub shared_start_tick: Option<u64>,
    /// Latest tick present in both histories. `None` if no overlap.
    pub shared_end_tick: Option<u64>,
    /// First tick within the shared range where subsystem state or a
    /// non-bookkeeping variable differs. `None` if the sessions agree
    /// across the entire shared range.
    pub first_divergence_tick: Option<u64>,
    /// Sparse per-tick diffs within the shared range. Ordered
    /// oldest → newest; only ticks with at least one diff are included.
    pub tick_diffs: Vec<TickDiff>,
    /// `true` if either side's history has been evicted past the ideal
    /// start of the shared range. Signals that the real divergence may
    /// lie before `shared_start_tick` and the UI should display a
    /// "history truncated" hint in compare-mode timelines.
    pub history_truncated: bool,
}

/// Diff two `ExecutionSnapshot`s into subsystem-state and variable diffs.
///
/// Subsystems and variables that match on both sides are omitted. Variables
/// whose name matches [`sysml_runtime::expressions::is_internal_var`] are
/// also omitted — runtime bookkeeping fields drift by construction when two
/// sessions run on independent clocks, so surfacing them in compare-mode UIs
/// would be pure noise.
///
/// Used by both `sysml.sessions.diff` (latest-snapshot diff) and
/// `sysml.sessions.diff_timeline` (per-tick walk).
pub(crate) fn diff_snapshots(
    a: &ExecutionSnapshot,
    b: &ExecutionSnapshot,
) -> (Vec<SubsystemDiff>, Vec<VariableDiff>) {
    let mut subsystem_diffs = Vec::new();
    let mut subsystem_names: std::collections::BTreeSet<&str> =
        a.subsystem_states.keys().map(|s| s.as_str()).collect();
    subsystem_names.extend(b.subsystem_states.keys().map(|s| s.as_str()));
    for name in subsystem_names {
        let a_state = a
            .subsystem_states
            .get(name)
            .map(|s| s.current_state.clone());
        let b_state = b
            .subsystem_states
            .get(name)
            .map(|s| s.current_state.clone());
        if a_state != b_state {
            let element_id = a
                .subsystem_states
                .get(name)
                .and_then(|s| s.source_element_id.clone())
                .or_else(|| {
                    b.subsystem_states
                        .get(name)
                        .and_then(|s| s.source_element_id.clone())
                })
                .map(|id| id.to_string());
            subsystem_diffs.push(SubsystemDiff {
                name: name.to_owned(),
                a_state,
                b_state,
                element_id,
            });
        }
    }

    let mut variable_diffs = Vec::new();
    let mut var_names: std::collections::BTreeSet<&str> =
        a.variables.keys().map(|s| s.as_str()).collect();
    var_names.extend(b.variables.keys().map(|s| s.as_str()));
    for name in var_names {
        if sysml_runtime::expressions::is_internal_var(name) {
            continue;
        }
        let a_value = a.variables.get(name).cloned();
        let b_value = b.variables.get(name).cloned();
        if a_value != b_value {
            variable_diffs.push(VariableDiff {
                name: name.to_owned(),
                a_value,
                b_value,
            });
        }
    }

    (subsystem_diffs, variable_diffs)
}

// ---------------------------------------------------------------------------
// ActionTraceEntry / FlowEvent
// ---------------------------------------------------------------------------

/// A single entry in the action execution trace.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ActionTraceEntry {
    /// Current node ID after the step.
    pub node_id: String,
    /// Whether the action completed on this step.
    pub completed: bool,
    /// Trace outputs produced.
    pub outputs: Vec<String>,
    /// Flow events emitted during this step.
    pub flow_events: Vec<FlowEvent>,
    /// Diagnostics (errors/warnings) from this step.
    pub diagnostics: Vec<String>,
}

/// A flow event emitted during action execution.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct FlowEvent {
    /// The kind of flow event.
    pub kind: FlowEventKind,
    /// Human-readable description.
    pub description: String,
}

/// Kinds of flow events.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum FlowEventKind {
    /// A message was sent.
    Send,
    /// A message was received.
    Accept,
    /// A message was routed through the flow router.
    Routed,
}

/// Format a trace history into a human-readable string.
pub fn format_trace(history: &VecDeque<ActionTraceEntry>) -> String {
    let mut out = String::new();
    for (i, entry) in history.iter().enumerate() {
        out.push_str(&format!("step {}: node={}", i + 1, entry.node_id));
        if entry.completed {
            out.push_str(" [COMPLETED]");
        }
        for o in &entry.outputs {
            out.push_str(&format!(" output={}", o));
        }
        for fe in &entry.flow_events {
            out.push_str(&format!(" {:?}:{}", fe.kind, fe.description));
        }
        out.push('\n');
    }
    out
}

// ---------------------------------------------------------------------------
// ForkAtTickError — structured error for fork-at-tick failures (R4)
// ---------------------------------------------------------------------------

/// Errors produced by `sysml.sessions.fork_with_overrides` when an
/// `at_tick` argument is supplied and cannot be honoured.
///
/// Serialised as tagged JSON (serde `tag = "kind"`) so the frontend can
/// switch on `kind` and show specific copy instead of parsing opaque
/// message strings.
///
/// Shape examples:
/// ```json
/// { "kind": "FutureTick", "tick": 100, "current": 42 }
/// { "kind": "SnapshotMissing", "tick": 3, "earliest_available": 50, "valid_ticks": [50, 150, 250] }
/// ```
///
/// with the coarse archive cadence, most ticks are never archived by
/// design — `SnapshotMissing` is the FAIL-HARD response for any
/// non-archived `at_tick`, never a silent clamp to the nearest archived
/// tick. `valid_ticks` gives the caller everything needed to retry against
/// an actually-forkable tick without guessing.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind")]
pub enum ForkAtTickError {
    /// Caller asked to fork at a tick in the future (parent has not
    /// reached it yet).
    FutureTick {
        /// The requested tick.
        tick: u64,
        /// The parent session's current tick.
        current: u64,
    },
    /// The requested tick is in the past but was never archived — either
    /// evicted from the bounded ring, or simply not a cadence/event tick
    /// under the session's archive cadence (see
    /// [`RuntimeSession::set_archive_cadence`]).
    SnapshotMissing {
        /// The requested tick.
        tick: u64,
        /// The oldest tick still available in the archive, or `None`
        /// when the archive is empty.
        earliest_available: Option<u64>,
        /// Every tick currently forkable (archived), oldest → newest —
        /// see [`RuntimeSession::forkable_ticks`]. Never a "nearest"
        /// suggestion: the caller must pick one of these exact ticks.
        #[serde(default)]
        valid_ticks: Vec<u64>,
    },
}

impl fmt::Display for ForkAtTickError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForkAtTickError::FutureTick { tick, current } => write!(
                f,
                "requested fork tick {tick} is in the future (current tick is {current})"
            ),
            ForkAtTickError::SnapshotMissing {
                tick,
                earliest_available,
                valid_ticks,
            } => match earliest_available {
                Some(earliest) => write!(
                    f,
                    "tick {tick} is not archived (earliest available: {earliest}; {} forkable tick(s) retained: {valid_ticks:?})",
                    valid_ticks.len(),
                ),
                None => write!(f, "snapshot for tick {tick} not available (archive empty)"),
            },
        }
    }
}

impl std::error::Error for ForkAtTickError {}

// ---------------------------------------------------------------------------
// RuntimeSession — unified session type (Phase 4)
// ---------------------------------------------------------------------------

/// A unified runtime session wrapping an `Orchestrator`.
///
/// Replaces the three separate session types (`SimulationSession`,
/// `ActionSession`, `OrchestratorSession`) with a single type. All
/// execution sessions are orchestrators with the appropriate executor(s).
pub struct RuntimeSession {
    /// The underlying orchestrator.
    pub orchestrator: Orchestrator,
    /// URI of the model being executed.
    pub uri: String,
    /// The kind of execution this session represents.
    pub kind: SessionKind,
    /// Name of the primary subsystem (state machine or action) driven by
    /// this session, if any. Orchestrator sessions with multiple subsystems
    /// leave this `None`.
    pub subsystem_name: Option<String>,
    /// User-provided display name (set via `sessions.rename`).
    pub label: Option<String>,
    /// Wall-clock creation time in unix milliseconds. Used for summary
    /// display only; expiry is tracked separately via the monotonic
    /// `last_activity` (idle clock, per ADR-005 / the session contract).
    pub created_at_ms: u64,
    /// If this session was produced by `sysml.sessions.fork`, the tick at
    /// which it branched from the parent. `None` for sessions created via
    /// `*.start`. Used by `sessions.diff_timeline` to distinguish a true
    /// history eviction from the "fresh child has only the seed snapshot"
    /// case — a fork point below the shared start means nothing is lost.
    pub fork_point_tick: Option<u64>,
    /// The latest `sysml.sessions.verify` outcome for this session, in the
    /// per-element shape the canvas verdict sidecar joins
    /// (`sysml.diagram.verdict_overlay`). PRODUCER: `sessions_verify` ONLY —
    /// the one `VerificationRunner` path against this session's live context;
    /// never populate it from another verdict source (`verify_with_simulation`
    /// et al. — that would reopen the second-verdict-path problem Inc2b
    /// closes). Deliberately kept across subsequent steps — stale verdicts are
    /// labeled via `verified_at_tick`, not silently dropped (steward ruling
    /// 2026-07-14); there are no invalidation hooks. Fresh sessions and forks
    /// start `None` (a fork's context diverges from what was verified).
    pub latest_verification: Option<sysml_diagram::VerificationVerdicts>,
    /// Parameter overrides this session's orchestrator was BUILT with, in
    /// caller order. Surfaced as [`SessionSummary::create_overrides`] and
    /// written to the archive's `overrides` field on stop.
    ///
    /// Set by the service mint paths that accept create-time overrides; empty
    /// otherwise. Never populated from a step-time override — see the summary
    /// field's doc for why the two must stay distinguishable.
    pub create_overrides: Vec<(String, String)>,
    /// Model/run provenance captured at session creation (B6 remainder):
    /// workspace-graph content digest + corroborating git state. Set by
    /// every service mint path (`capture_session_provenance` — the four
    /// session mint paths plus B10's `verify.record_external` archive
    /// ingestion); `None` only for sessions constructed outside the
    /// service layer (unit tests).
    /// INHERITED verbatim by forks — identity field, same bucket as `uri`
    /// (the fork's orchestrator IS the parent's model state; re-capturing
    /// would record a graph the fork does not run).
    pub provenance: Option<sysml_store::SessionProvenance>,
    /// Compiled signal expressions for time-varying parameter display sync.
    signal_exprs: Vec<(String, sysml_runtime::expressions::ExprIR)>,
    /// Monotonic creation time. Drives the `elapsed_ms` summary field
    /// (age since creation) — NOT expiry. Reset by [`RuntimeSession::reset`].
    created_at: Instant,
    /// Monotonic last-activity time. Drives expiry: a session is expired
    /// once it has been idle (no step / inject / poll) for `SESSION_TIMEOUT`
    /// (idle clock, per ADR-005 + the session-backend contract — NOT age
    /// since creation, so a long *active* run is never wrongly reaped).
    /// Refreshed by [`RuntimeSession::touch`], [`RuntimeSession::step`],
    /// and [`RuntimeSession::reset`].
    last_activity: Instant,
    /// Bounded snapshot history.
    history: VecDeque<ExecutionSnapshot>,
    /// Registered breakpoints keyed by opaque id.
    ///
    /// Evaluated at step boundaries (see [`RuntimeSession::check_breakpoints`]).
    /// When a breakpoint matches, the session transitions to the paused phase
    /// and records the firing id in `paused_at_breakpoint`.
    breakpoints: HashMap<BreakpointId, Breakpoint>,
    /// Whether the session is currently paused by a breakpoint hit.
    ///
    /// When `true`, callers can inspect [`RuntimeSession::paused_at_breakpoint`]
    /// to see which breakpoint id fired. Clearing pause state is the caller's
    /// responsibility (via [`RuntimeSession::resume`]) — `step` is a no-op
    /// while paused so the client can snapshot state before continuing.
    paused: bool,
    /// Breakpoint id that triggered the current pause, if any.
    paused_at_breakpoint: Option<BreakpointId>,
    /// Ring buffer of full `Orchestrator` clones keyed by tick, powering
    /// `fork_with_overrides(at_tick = Some(t))` (R4). Populated after each
    /// successful step.
    ///
    /// Bounded by `snapshot_retention_ticks` (default
    /// `DEFAULT_SNAPSHOT_RETENTION_TICKS = 256`). Oldest entries are
    /// evicted when the buffer is full. These clones are deliberately
    /// heavier than `ExecutionSnapshot`s — they must capture every bit of
    /// subsystem-executor state needed to resume simulation.
    orchestrator_archive: VecDeque<(u64, Orchestrator)>,
    /// Maximum number of orchestrator snapshots retained. Configured via
    /// [`RuntimeSession::set_snapshot_retention`].
    snapshot_retention_ticks: usize,
    /// Archive cadence, in ticks (UX closeout arc #7). A checkpoint is
    /// unconditionally archived every `archive_cadence_ticks` ticks;
    /// `<= 1` means every tick (matching the `OrchestratorConfig
    /// ::snapshot_interval` convention this deliberately does NOT reuse —
    /// see [`DEFAULT_ARCHIVE_CADENCE_TICKS`]). Independent of cadence, a
    /// tick classified event-significant is *always* force-archived
    /// regardless of where it falls in the cadence window — see
    /// [`RuntimeSession::step`]. Configured via
    /// [`RuntimeSession::set_archive_cadence`].
    archive_cadence_ticks: u64,
    /// Remaining debounce ticks per breakpoint id. While non-zero, the
    /// breakpoint is suppressed from firing. Decremented once per
    /// [`RuntimeSession::check_breakpoints`] invocation. Used by the
    /// `debounce_ticks` field shared by `Breakpoint::ThresholdCrossing` /
    /// `Breakpoint::Conditional` (both back onto
    /// `sysml_runtime::breakpoint::CompareBreakpoint` post-BP4-collapse)
    /// to avoid re-firing every tick while the threshold condition stays
    /// true.
    ///
    /// Only breakpoints with a non-zero `debounce_ticks` get an entry.
    breakpoint_debounce: HashMap<BreakpointId, u32>,
    /// Per-session canonical columnar numeric store (ADR-011 §6 /
    /// S3.T9). Populated on every successful step from
    /// `snapshot_view::normalize(&snapshot).scalar_vars` — the same
    /// scalar-variable projection the FE consumes.
    ///
    /// This is the per-session **canonical** numeric store: future
    /// consumers (WS time-series stream, sessions.diff_timeline,
    /// MCP `sysml.timeseries.*` if added) should read from here
    /// instead of re-normalising every snapshot on demand. The
    /// snapshot `history` field stays the source of truth for tick
    /// events and state-machine state — only variable values move
    /// here.
    ///
    /// Sized via the default 100 MB memory budget; capacity is
    /// derived per-session from the variable count at the first
    /// step (back-fills new series with NaN to keep columns aligned).
    /// Cleared on `reset()`. Forked sessions receive a fresh empty
    /// buffer (matches the snapshot-history lineage rule for
    /// forks).
    time_series: sysml_runtime::timeseries::TimeSeriesBuffer,
    /// Per-session broadcast sender for push-based snapshot fan-out.
    ///
    /// The orchestrator is installed with an observer that clones this
    /// sender and forwards every produced [`ExecutionSnapshot`] into it.
    /// Subscribers obtain a [`broadcast::Receiver`] via
    /// [`RuntimeSession::subscribe_snapshots`] and read ticks directly,
    /// replacing the 33 ms poll the WebSocket layer used in Stage 4.
    ///
    /// Dropped together with the session — closing the channel and
    /// propagating `Closed` to any live subscriber.
    snapshot_tx: broadcast::Sender<Arc<ExecutionSnapshot>>,
}

/// Install the snapshot observer that forwards every produced
/// [`ExecutionSnapshot`] into the given broadcast sender.
///
/// Uses `Sender::send`, which ignores the `Ok(usize)` / `Err(_)` return
/// value — a send failure means no live receivers, which is fine: the
/// orchestrator keeps stepping and the next subscriber picks up from the
/// next tick. `Arc::new` once per tick keeps per-subscriber memory cost
/// to a single allocation regardless of fan-out.
///
/// live-perf B2a: skip the clone+send entirely when `receiver_count() == 0`
/// (no WS client attached — headless/API-driven runs, before a client
/// connects, or after it disconnects). `ExecutionSnapshot::clone()` is not
/// free — `variables` is `Arc`-shared, but `subsystem_states`, `messages`,
/// `constraint_results`, `guard_diagnoses`, `causation_links`,
/// `port_values`, `derivatives`, `resolved_refs`, and `flow_drop_warnings`
/// are all deep-cloned `HashMap`/`Vec` fields — so every tick without a
/// subscriber was paying that cost for a value nobody could ever observe.
/// This only gates the broadcast fan-out: the caller's own `snapshot`
/// (returned from `Orchestrator::step`) is unaffected, so `history` /
/// `time_series` in [`RuntimeSession::step`] are built exactly as before
/// regardless of subscriber count. A client that *is* subscribed still
/// gets every frame — `receiver_count()` only reaches 0 when there is
/// truly no one to send to.
fn install_snapshot_observer(
    orchestrator: &mut Orchestrator,
    tx: &broadcast::Sender<Arc<ExecutionSnapshot>>,
) {
    let tx = tx.clone();
    orchestrator.set_snapshot_observer(Arc::new(move |snapshot: &ExecutionSnapshot| {
        if tx.receiver_count() == 0 {
            return;
        }
        let _ = tx.send(Arc::new(snapshot.clone()));
    }));
}

impl RuntimeSession {
    /// Create a new runtime session.
    pub fn new(
        mut orchestrator: Orchestrator,
        uri: String,
        kind: SessionKind,
        subsystem_name: Option<String>,
    ) -> Self {
        let created_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_else(|e| {
                tracing::warn!(
                    ?e,
                    "system clock before UNIX_EPOCH; session created_at_ms set to 0"
                );
                0
            });
        let (snapshot_tx, _) = broadcast::channel(SNAPSHOT_BROADCAST_CAPACITY);
        install_snapshot_observer(&mut orchestrator, &snapshot_tx);
        Self {
            orchestrator,
            uri,
            kind,
            subsystem_name,
            label: None,
            created_at_ms,
            fork_point_tick: None,
            latest_verification: None,
            provenance: None,
            // Set by the caller after construction when the orchestrator was
            // built with create-time overrides (`sessions.create`), since only
            // the caller knows what it passed to the builder.
            create_overrides: Vec::new(),
            signal_exprs: Vec::new(),
            created_at: Instant::now(),
            last_activity: Instant::now(),
            history: VecDeque::new(),
            breakpoints: HashMap::new(),
            paused: false,
            paused_at_breakpoint: None,
            orchestrator_archive: VecDeque::new(),
            snapshot_retention_ticks: DEFAULT_SNAPSHOT_RETENTION_TICKS,
            archive_cadence_ticks: DEFAULT_ARCHIVE_CADENCE_TICKS,
            breakpoint_debounce: HashMap::new(),
            time_series: sysml_runtime::timeseries::TimeSeriesBuffer::new(),
            snapshot_tx,
        }
    }

    /// Subscribe to push-notified snapshots for this session. Each call
    /// returns an independent [`broadcast::Receiver`]; slow subscribers
    /// receive `RecvError::Lagged(N)` on overflow and are expected to
    /// resync via `sessions_info` — the server does not queue.
    pub fn subscribe_snapshots(&self) -> broadcast::Receiver<Arc<ExecutionSnapshot>> {
        self.snapshot_tx.subscribe()
    }

    /// Override the orchestrator snapshot retention for this session.
    ///
    /// A value of `0` disables the archive entirely (fork-at-past-tick
    /// will always fail with `SnapshotMissing`). Shrinking the retention
    /// truncates the archive immediately from the oldest end.
    pub fn set_snapshot_retention(&mut self, ticks: usize) {
        self.snapshot_retention_ticks = ticks;
        while self.orchestrator_archive.len() > ticks {
            self.orchestrator_archive.pop_front();
        }
    }

    /// Returns the current orchestrator snapshot retention window (in ticks).
    pub fn snapshot_retention_ticks(&self) -> usize {
        self.snapshot_retention_ticks
    }

    /// Returns the number of orchestrator snapshots currently archived.
    pub fn archived_snapshot_count(&self) -> usize {
        self.orchestrator_archive.len()
    }

    /// Returns the oldest tick currently retained in the archive, or
    /// `None` if the archive is empty.
    pub fn earliest_archived_tick(&self) -> Option<u64> {
        self.orchestrator_archive.front().map(|(t, _)| *t)
    }

    /// Returns `true` if the archive currently holds a snapshot for the
    /// given tick.
    pub fn has_archived_tick(&self, tick: u64) -> bool {
        self.orchestrator_archive
            .iter()
            .any(|(t, _)| *t == tick)
    }

    /// Returns every tick currently retained in the archive, oldest →
    /// newest (UX closeout arc #7 §2.4 honesty flag).
    ///
    /// This is the exact set of ticks `fork_child_at` will accept — a
    /// client uses it to render fork/rewind affordances only at valid
    /// points instead of guessing and hitting the fail-hard
    /// [`ForkAtTickError::SnapshotMissing`]. Extends the existing
    /// `archived_snapshot_count()` / `earliest_archived_tick()` vocabulary
    /// rather than inventing a new one.
    pub fn forkable_ticks(&self) -> Vec<u64> {
        self.orchestrator_archive.iter().map(|(t, _)| *t).collect()
    }

    /// Override the archive cadence for this session (UX closeout arc #7).
    ///
    /// A checkpoint is unconditionally archived every `ticks` ticks (`<= 1`
    /// means every tick); a tick classified event-significant (SM
    /// transition, boolean variable flip, breakpoint fire) is always
    /// force-archived regardless of cadence phase. Does not retroactively
    /// affect already-archived ticks.
    pub fn set_archive_cadence(&mut self, ticks: u64) {
        self.archive_cadence_ticks = ticks;
    }

    /// Returns the current archive cadence, in ticks.
    pub fn archive_cadence_ticks(&self) -> u64 {
        self.archive_cadence_ticks
    }

    /// Build a flat `SessionSummary` projection for this session.
    ///
    /// The caller provides the session id (held by `SysmlService::sessions`,
    /// not stored in the session itself).
    pub fn summary(&self, id: String) -> SessionSummary {
        let latest = self.history.back();
        let (tick, time_ms, completed) = match latest {
            Some(snap) => (snap.tick, snap.time_ms, snap.completed),
            None => (0, 0.0, false),
        };
        let current_state = latest.and_then(|snap| {
            // Prefer the named primary subsystem; fall back to the first
            // subsystem for orchestrator-kind sessions.
            if let Some(name) = &self.subsystem_name {
                snap.subsystem_states
                    .get(name)
                    .map(|s| s.current_state.clone())
            } else {
                snap.subsystem_states
                    .values()
                    .next()
                    .map(|s| s.current_state.clone())
            }
        });

        SessionSummary {
            id,
            kind: self.kind,
            uri: self.uri.clone(),
            subsystem_name: self.subsystem_name.clone(),
            label: self.label.clone(),
            created_at_ms: self.created_at_ms,
            elapsed_ms: self.created_at.elapsed().as_millis() as u64,
            tick,
            time_ms,
            current_state,
            completed,
            is_expired: self.is_expired(),
            history_len: self.history.len(),
            subsystem_count: self.orchestrator.subsystems().len(),
            fork_point_tick: self.fork_point_tick,
            forkable_ticks: self.forkable_ticks(),
            paused: self.paused,
            paused_at_breakpoint: self.paused_at_breakpoint.clone(),
            // Per-call value — always 0 here; `sessions_step_internal` is
            // the one caller that overwrites it with the real `step_many`
            // return before responding. See the field doc on
            // `SessionSummary::ticks_advanced`.
            ticks_advanced: 0,
            provenance: self.provenance.clone(),
            create_overrides: self.create_overrides.clone(),
        }
    }

    /// Current orchestrator tick — cheap accessor used by the
    /// session-events WS handler to skip the full snapshot clone when
    /// no tick has advanced since the last observation.
    pub fn current_tick(&self) -> u64 {
        self.orchestrator.tick()
    }

    /// Whether the orchestrator has completed.
    pub fn is_completed(&self) -> bool {
        self.orchestrator.is_completed()
    }

    /// Build a full `SessionDetail` projection for this session.
    pub fn detail(&self, id: String) -> SessionDetail {
        let latest = self.history.back();
        // Iterate orchestrator subsystems to preserve insertion order, looking
        // up each by name in the latest snapshot.
        let subsystems = self
            .orchestrator
            .subsystems()
            .iter()
            .map(|sub| {
                let snap_state = latest.and_then(|snap| snap.subsystem_states.get(&sub.name));
                SubsystemSummary {
                    name: sub.name.clone(),
                    kind_label: snap_state
                        .map(|s| s.kind.to_owned())
                        .unwrap_or_else(|| UNKNOWN_KIND_LABEL.to_owned()),
                    current_state: snap_state
                        .map(|s| s.current_state.clone())
                        .unwrap_or_default(),
                    completed: snap_state.map(|s| s.completed).unwrap_or(false),
                    available_transitions: snap_state
                        .map(|s| s.available_transitions.clone())
                        .unwrap_or_default(),
                    deferred_event_count: snap_state
                        .map(|s| s.deferred_event_count)
                        .unwrap_or(0),
                    element_id: sub.source_element_id.as_ref().map(|id| id.to_string()),
                }
            })
            .collect();

        SessionDetail {
            summary: self.summary(id),
            subsystems,
            latest_snapshot: latest.cloned(),
        }
    }

    /// Test-only: force this session into the expired state.
    #[cfg(test)]
    pub fn test_mark_expired(&mut self) {
        // Expiry is idle-based — push the last-activity clock past the timeout.
        self.last_activity = Instant::now()
            .checked_sub(SESSION_TIMEOUT + Duration::from_secs(1))
            .unwrap_or_else(Instant::now);
    }

    /// Set signal expressions for time-varying parameter display sync.
    pub fn set_signals(&mut self, signals: Vec<(String, sysml_runtime::expressions::ExprIR)>) {
        self.signal_exprs = signals;
    }

    /// Step the orchestrator and record the snapshot.
    ///
    /// If the session is paused at a breakpoint, this call is a no-op and
    /// returns the latest snapshot unchanged — clients must call
    /// [`RuntimeSession::resume`] first. After each successful step the
    /// snapshot is checked against registered breakpoints; if any match,
    /// the session transitions to the paused phase and subsequent steps
    /// are blocked until `resume()` is called.
    ///
    /// ## Archive cadence (UX closeout arc #7)
    ///
    /// `step()`, the `ExecutionSnapshot` build, breakpoint/crossing/trip
    /// detection, and the time-series append all stay every-tick — only
    /// the `orchestrator_archive` push (the expensive `fork_for_archive()`
    /// deep clone, ~89% of a production tick) is decimated. A checkpoint is
    /// archived when EITHER is true:
    /// - it lands on the [`RuntimeSession::archive_cadence_ticks`] cadence
    ///   (every Nth tick), or
    /// - it is classified **event-significant**: a state-machine
    ///   transition, a boolean context variable flipping value (the
    ///   generic "trip-latch flip" — not tied to any one model's variable
    ///   name), or a breakpoint firing.
    ///
    /// This guarantees the one workflow the archive exists for — rewind
    /// exactly to an event tick — stays intact even though most ticks are
    ///
    /// ### The ordering trap (design doc §3)
    ///
    /// The archive-push decision needs to know whether a breakpoint fired
    /// THIS tick, but `check_breakpoints`'s debounce-aware match has real
    /// side effects (arms debounce windows, sets `paused`/
    /// `paused_at_breakpoint`, decrements counters). This executor picked
    /// **option (b)**: `check_breakpoints` now returns `bool` (did a
    /// breakpoint fire this tick), and is called — with its full side
    /// effects — *before* the archive-push decision rather than after.
    /// This is safe because nothing `check_breakpoints` mutates
    /// (`paused`, `paused_at_breakpoint`, `breakpoint_debounce`) is read by
    /// `fork_for_archive()` or by the archive-cadence decision itself; the
    /// two were only ever in their old relative order by historical
    /// accident, not because of a real dependency. Swapping them gives the
    /// archive decision the information it needs without duplicating the
    /// breakpoint-match logic in a second "pure" evaluator.
    pub fn step(&mut self) -> ExecutionSnapshot {
        // Stepping is activity — refresh the idle clock so an actively-driven
        // run is never reaped mid-flight (even past SESSION_TIMEOUT).
        self.last_activity = Instant::now();
        // While paused, report the last snapshot and refuse to advance.
        // This lets callers poll state without implicitly resuming.
        if self.paused {
            if let Some(last) = self.history.back() {
                return last.clone();
            }
        }

        let snapshot = self.orchestrator.step();

        // Sync signal values to orchestrator context so the UI sees current values.
        if !self.signal_exprs.is_empty() {
            let t = snapshot.time_ms / 1000.0;
            let evaluator = sysml_runtime::expressions::ExpressionEvaluator::new();
            // Cull-arc W3: alias_live preserves the pre-cull `.clone()` exactly.
            // NB the base is the orchestrator MASTER context (live slot handle);
            // `ctx.set("t", …)` here is a speculative probe whose result is
            // written back explicitly via `self.orchestrator.context.set` below,
            // so this is a scratch-shaped use on a handle-bearing base — flagged
            // for the REQ-1 leak-vs-misclassification review, behaviour preserved.
            let mut ctx = self.orchestrator.context.alias_live();
            ctx.set("t".to_owned(), sysml_core::Value::Float(t));
            for (name, expr) in &self.signal_exprs {
                if let Ok(sysml_core::Value::Float(f)) = evaluator.eval(expr, &ctx) {
                    self.orchestrator.context.set(name.clone(), sysml_core::Value::Float(f));
                }
            }
        }

        // UX closeout arc #7: classify event-significance against the
        // PRIOR tick's snapshot (still at the back of `history`) before it
        // is displaced by this tick's push below. Borrowed, not cloned —
        // both comparisons are read-only walks over `snapshot`'s already-
        // built maps.
        let sm_transitioned = self
            .history
            .back()
            .map(|prev| subsystem_state_transitioned(prev, &snapshot))
            .unwrap_or(false);
        let bool_flipped = self
            .history
            .back()
            .map(|prev| bool_variable_flipped(prev, &snapshot))
            .unwrap_or(false);

        if self.history.len() >= MAX_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(snapshot.clone());

        // ADR-011 §6 / S3.T9: route scalar variable values into the
        // canonical TimeSeriesBuffer. `append_snapshot` de-duplicates
        // stale ticks (same/older time_ms) automatically, matching the
        // history's monotonic-time assumption.
        //
        // Task #8 (steward ruling 2026-07-07): supply the session's live slot
        // store so the projection is gated to meta spellings only. Without it,
        // add_alias spellings (e.g. an ODE's qualified `{ode}.duty`) would
        // double every aliased state var in `scalar_vars` and hence in
        // `timeseries_names`. The store's name→meta classification is immutable
        // post-mint, so any tick's store gives the same answer. The read guard
        // is scoped to the (owned) `normalized` build and dropped before the
        // `&mut self.time_series` append.
        let normalized = {
            let store = self.orchestrator.slot_store();
            sysml_runtime::snapshot_view::normalize_with(
                &snapshot,
                sysml_runtime::snapshot_view::NormalizeOptions {
                    slots: Some(&store),
                    ..Default::default()
                },
            )
        };
        self.time_series.append_snapshot(&normalized);

        // Check breakpoints against the new snapshot. If any fire, pause.
        //
        // Ordering trap (design doc §3): this MUST run before the
        // archive-push decision below so `breakpoint_fired` reflects this
        // tick's real (debounce-aware) match — see the `step` doc comment
        // for why running it here, ahead of the push, is safe.
        let breakpoint_fired = self.check_breakpoints(&snapshot);
        let event_significant = breakpoint_fired || sm_transitioned || bool_flipped;

        // UX closeout arc #7: archive a deep clone of the orchestrator
        // keyed by tick so `fork_with_overrides(at_tick)` can rewind. Skip
        // entirely when retention is 0 — the archive is disabled. Evict
        // the oldest entries when we exceed the configured retention
        // window. A checkpoint is pushed when the tick lands on the
        // archive cadence OR was just classified event-significant — a
        // forced archive on the exact tick a trip/transition/breakpoint
        // happens is what keeps "rewind to the event" working even though
        // most ticks are no longer archived (see the `step` doc comment
        //
        // `fork_for_archive` strips the trace from each clone — the
        // archive's job is "branch from tick N", not "replay N's
        // history" (the forked session builds its own trace). Without
        // this, the archive retains archive_cap × trace_cap copies of
        // every snapshot, which on large-workspace-class workloads
        // ran to multi-GB per session.
        if self.snapshot_retention_ticks > 0 {
            let tick = snapshot.tick;
            let is_cadence_tick =
                self.archive_cadence_ticks <= 1 || tick.is_multiple_of(self.archive_cadence_ticks);
            if is_cadence_tick || event_significant {
                self.orchestrator_archive
                    .push_back((tick, self.orchestrator.fork_for_archive()));
                while self.orchestrator_archive.len() > self.snapshot_retention_ticks {
                    self.orchestrator_archive.pop_front();
                }
            }
        }

        snapshot
    }

    /// Advance up to `max_ticks` ticks in one call, returning the number of
    /// ticks ACTUALLY advanced (never padded to `max_ticks`).
    ///
    /// The single-tick [`step`](Self::step) is the `max_ticks == 1` case; a bulk
    /// call runs the loop server-side so a fine-dt run reaches a far-off event
    /// (e.g. a firmware trip ~5,856 ticks out) without one HTTP round-trip per tick.
    /// Every advanced tick goes through `step`, so `time_series`/`history`
    /// accumulate exactly as they do for single-stepping — the chart stays
    /// complete server-side and the caller need only return the final summary.
    ///
    /// Two early-exit conditions:
    ///  1. a breakpoint pause (`step` auto-pauses on a match; we stop so the
    ///     client can inspect the paused state), and
    ///  2. the orchestrator's [`can_run_more`](sysml_runtime::orchestrator::Orchestrator::can_run_more)
    ///     predicate going false — raw `step` has no internal guard, so a bulk
    ///     loop would otherwise blow past the configured `max_ticks`/
    ///     `max_time_ms` safety limits. Gating the loop CONTINUATION on it keeps
    ///     the fail-hard boundary intact.
    ///
    /// The FIRST tick is deliberately unconditional (a paused session aside):
    /// `max_ticks == 1` must stay byte-identical to the classic single
    /// `sessions.step`, which advances exactly one tick even for a settled /
    /// `is_completed()` session (the session contract relies on stepping a
    /// settled sim to build snapshot history). Only ticks 2..N are gated on
    /// `can_run_more`, which is where a runaway loop would actually occur.
    pub fn step_many(&mut self, max_ticks: u64) -> u64 {
        let mut advanced = 0;
        while advanced < max_ticks {
            // A breakpoint pause halts every tick, including the first (`step`
            // would no-op while paused anyway).
            if self.paused {
                break;
            }
            // Safety guard applies from the second tick on, so the first tick
            // preserves classic single-step semantics (see above). Bounds-only
            // (`within_run_bounds`), deliberately NOT `can_run_more`: an
            // explicit `ticks: N` request is the same user intent the contract
            // honours for single-stepping a settled sim, and a continuous-only
            // (plain-ODE) workspace reports `is_completed()` vacuously true —
            // gating on it stopped bulk runs after one tick (live-caught on
            // examples/oscillator-tuning-study).
            if advanced > 0 && !self.orchestrator.within_run_bounds() {
                break;
            }
            let _ = self.step();
            advanced += 1;
            // `step` may have hit a breakpoint and paused us; stop here so the
            // returned count reflects what actually ran.
            if self.paused {
                break;
            }
        }
        advanced
    }

    /// Inject an event into a named subsystem and step.
    pub fn inject_event(&mut self, subsystem: &str, event: &str) -> ExecutionSnapshot {
        self.orchestrator.inject_event(subsystem, event);
        self.step()
    }

    /// Check whether the session has expired.
    ///
    /// Idle-based: a session is expired once it has gone `SESSION_TIMEOUT`
    /// without any activity (step / inject / poll / reset). This is NOT age
    /// since creation — an actively-driven or actively-polled session never
    /// expires, matching ADR-005's intent (support long analysis runs) and
    /// the session-backend contract ("10/60 min of wall-clock inactivity").
    pub fn is_expired(&self) -> bool {
        self.last_activity.elapsed() > SESSION_TIMEOUT
    }

    /// Refresh the idle clock. Called whenever a client interacts with the
    /// session (step, inject, info poll) so that "active" sessions are never
    /// reaped and the idle countdown starts only once interaction stops.
    pub fn touch(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Reset the orchestrator and session state.
    ///
    /// Clears the fork lineage too — after a reset, the session is
    /// indistinguishable from a freshly-started one of the same kind.
    /// Breakpoints themselves are preserved (they are registered debug
    /// metadata) but pause state is cleared so execution can resume.
    pub fn reset(&mut self) {
        self.orchestrator.reset();
        self.created_at = Instant::now();
        self.last_activity = Instant::now();
        self.history.clear();
        self.orchestrator_archive.clear();
        self.fork_point_tick = None;
        self.paused = false;
        self.paused_at_breakpoint = None;
        // Drop any in-flight debounce counters — fresh execution should
        // be able to fire breakpoints again immediately.
        self.breakpoint_debounce.clear();
        // S3.T9: the canonical time-series store is per-session
        // execution data. Reset wipes it alongside the orchestrator's
        // step counter.
        self.time_series = sysml_runtime::timeseries::TimeSeriesBuffer::new();
    }

    /// Borrow the session's canonical [`TimeSeriesBuffer`] (S3.T9).
    /// Populated on every successful step from
    /// `snapshot_view::normalize(&snapshot).scalar_vars` — the
    /// authoritative columnar numeric store for the session's
    /// scalar variables.
    pub fn time_series(&self) -> &sysml_runtime::timeseries::TimeSeriesBuffer {
        &self.time_series
    }

    /// The `(subsystem_name → current_state)` map at the latest snapshot.
    /// Returns an empty map if the session has not been stepped yet.
    pub fn subsystem_states_latest(&self) -> std::collections::HashMap<String, String> {
        let mut out = std::collections::HashMap::new();
        if let Some(snap) = self.history.back() {
            for (name, state) in &snap.subsystem_states {
                out.insert(name.clone(), state.current_state.clone());
            }
        }
        out
    }

    /// The tick of the latest recorded snapshot, or 0 if none.
    pub fn latest_tick(&self) -> u64 {
        self.history.back().map(|s| s.tick).unwrap_or(0)
    }

    /// A clone of the current orchestrator context variables. Convenience for
    /// `sessions.diff` — prefer `orchestrator.context` for anything else.
    ///
    /// The runtime stores `variables` as `Arc<HashMap>` for copy-on-write
    /// forking; this helper dereferences and deep-clones to keep the public
    /// API shape (owned `HashMap`) intact.
    pub fn context_snapshot(&self) -> std::collections::HashMap<String, sysml_core::Value> {
        (*self.orchestrator.context.variables).clone()
    }

    /// Borrow the session's recorded snapshots oldest → newest. Convenience
    /// for `sessions.diff_timeline` — prefer `history()` for anything that
    /// just needs the deque.
    pub fn snapshots_slice(&self) -> Vec<&ExecutionSnapshot> {
        self.history.iter().collect()
    }

    /// Produce an independent child session from this one.
    ///
    /// The child's orchestrator is a deep copy at the current tick (via
    /// `Orchestrator::fork`). History is reseeded with the parent's most
    /// recent snapshot so the child's summary still reports a sensible
    /// `tick`/`current_state` before its first step. The child gets a
    /// fresh monotonic `created_at` (its expiry clock restarts).
    ///
    /// Used by `sysml.sessions.fork`.
    pub fn fork_child(&self) -> Self {
        let created_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_else(|e| {
                tracing::warn!(
                    ?e,
                    "system clock before UNIX_EPOCH; fork child created_at_ms set to 0"
                );
                0
            });
        let mut history = VecDeque::new();
        let fork_point_tick = self.history.back().map(|s| s.tick);
        if let Some(latest) = self.history.back().cloned() {
            history.push_back(latest);
        }
        // Fork produces an orchestrator with no observer (see
        // SnapshotObserver clone semantics). Give the child its own
        // broadcast channel so its ticks don't leak into the parent's
        // subscribers, then install a matching observer.
        let mut orchestrator = self.orchestrator.fork();
        let (snapshot_tx, _) = broadcast::channel(SNAPSHOT_BROADCAST_CAPACITY);
        install_snapshot_observer(&mut orchestrator, &snapshot_tx);
        Self {
            orchestrator,
            uri: self.uri.clone(),
            kind: self.kind,
            subsystem_name: self.subsystem_name.clone(),
            label: self.label.clone(),
            created_at_ms,
            fork_point_tick,
            // NOT inherited: the parent's verification judged the parent's
            // run; the fork's trajectory diverges from here. Verify the child
            // to get its own verdicts.
            latest_verification: None,
            // Inherited verbatim: identity field (see field doc).
            provenance: self.provenance.clone(),
            // Inherited verbatim: the child's context came from a parent
            // BUILT with these, so they are still in force at every tick the
            // child holds. A `fork_with_overrides` override is a step-time
            // change on top and is not folded in here.
            create_overrides: self.create_overrides.clone(),
            signal_exprs: self.signal_exprs.clone(),
            created_at: Instant::now(),
            last_activity: Instant::now(),
            history,
            // Forked children inherit the parent's breakpoint set. They
            // start un-paused so the caller can drive them independently.
            breakpoints: self.breakpoints.clone(),
            paused: false,
            paused_at_breakpoint: None,
            // Children start with an empty archive; their own `step()`
            // calls build up their own retention window. Inheriting the
            // parent's archive would let the child rewind past its own
            // birth which has no sensible semantics.
            orchestrator_archive: VecDeque::new(),
            snapshot_retention_ticks: self.snapshot_retention_ticks,
            archive_cadence_ticks: self.archive_cadence_ticks,
            // Clone debounce counters so a child fork with an already-
            // suppressed breakpoint continues to respect the suppression
            // window. This matches how pause-state is handled: children
            // get independent execution but preserved debug metadata.
            breakpoint_debounce: self.breakpoint_debounce.clone(),
            // S3.T9: child starts with an empty numeric store. Same
            // rule as `orchestrator_archive` — inheriting would let the
            // child read variable history from before its own birth,
            // which has no sensible semantics.
            time_series: sysml_runtime::timeseries::TimeSeriesBuffer::new(),
            snapshot_tx,
        }
    }

    /// Produce an independent child session rewound to a specific tick
    /// (R4: fork-at-tick).
    ///
    /// The child's orchestrator is a deep copy of the parent at the
    /// requested tick — sourced from the parent's orchestrator archive.
    /// The child's history is reseeded with the matching execution
    /// snapshot (if retained) so its summary reports the correct
    /// `tick`/`current_state` before its first step.
    ///
    /// # Errors
    ///
    /// Returns [`ForkAtTickError::FutureTick`] when `at_tick` exceeds the
    /// parent's latest tick, and [`ForkAtTickError::SnapshotMissing`]
    /// when the requested tick has been evicted from the archive (or was
    /// never captured because the session hasn't stepped yet / retention
    /// is disabled).
    ///
    /// Used by `sysml.sessions.fork_with_overrides` when `at_tick` is
    /// supplied.
    pub fn fork_child_at(&self, at_tick: u64) -> Result<Self, ForkAtTickError> {
        let current = self.latest_tick();
        if at_tick > current {
            return Err(ForkAtTickError::FutureTick {
                tick: at_tick,
                current,
            });
        }
        let archived = self
            .orchestrator_archive
            .iter()
            .find(|(t, _)| *t == at_tick)
            .map(|(_, o)| o.clone())
            .ok_or_else(|| ForkAtTickError::SnapshotMissing {
                tick: at_tick,
                earliest_available: self.earliest_archived_tick(),
                valid_ticks: self.forkable_ticks(),
            })?;

        let created_at_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_else(|e| {
                tracing::warn!(
                    ?e,
                    "system clock before UNIX_EPOCH; fork child created_at_ms set to 0"
                );
                0
            });

        // Reseed child history with the matching execution snapshot so
        // the child's summary reports the correct pre-step state.
        let mut history = VecDeque::new();
        if let Some(snap) = self
            .history
            .iter()
            .find(|s| s.tick == at_tick)
            .cloned()
        {
            history.push_back(snap);
        }

        // Same rationale as `fork_child`: a fresh broadcast channel on
        // the child so its ticks don't leak to the parent's subscribers.
        // The archived orchestrator's observer slot is already empty
        // (SnapshotObserver clears on clone), so we can safely re-install.
        let mut orchestrator = archived;
        let (snapshot_tx, _) = broadcast::channel(SNAPSHOT_BROADCAST_CAPACITY);
        install_snapshot_observer(&mut orchestrator, &snapshot_tx);
        Ok(Self {
            orchestrator,
            uri: self.uri.clone(),
            kind: self.kind,
            subsystem_name: self.subsystem_name.clone(),
            label: self.label.clone(),
            created_at_ms,
            fork_point_tick: Some(at_tick),
            // NOT inherited — same rationale as `fork_child`.
            latest_verification: None,
            // Inherited verbatim — same rationale as `fork_child`.
            provenance: self.provenance.clone(),
            create_overrides: self.create_overrides.clone(),
            signal_exprs: self.signal_exprs.clone(),
            created_at: Instant::now(),
            last_activity: Instant::now(),
            history,
            breakpoints: self.breakpoints.clone(),
            paused: false,
            paused_at_breakpoint: None,
            orchestrator_archive: VecDeque::new(),
            snapshot_retention_ticks: self.snapshot_retention_ticks,
            archive_cadence_ticks: self.archive_cadence_ticks,
            // Fresh fork: the child manages its own debounce windows.
            // Copying the parent's counters would suppress breakpoints that
            // the child has not yet observed.
            breakpoint_debounce: HashMap::new(),
            // S3.T9: child starts with an empty time-series store. The
            // parent's history before the fork-point tick is recorded
            // there but doesn't transfer to the child (matches
            // `orchestrator_archive` and the snapshot-history seeding
            // rule — child has visibility into the fork-point snapshot
            // only).
            time_series: sysml_runtime::timeseries::TimeSeriesBuffer::new(),
            snapshot_tx,
        })
    }

    /// Access the snapshot history.
    pub fn history(&self) -> &VecDeque<ExecutionSnapshot> {
        &self.history
    }

    // ----------------------------------------------------------------------
    // Breakpoint management (R1.2)
    // ----------------------------------------------------------------------

    /// Register a breakpoint on this session. Returns an opaque id the caller
    /// can later pass to [`RuntimeSession::clear_breakpoint`].
    pub fn set_breakpoint(&mut self, breakpoint: Breakpoint) -> BreakpointId {
        let id = new_breakpoint_id();
        self.breakpoints.insert(id.clone(), breakpoint);
        id
    }

    /// Remove a breakpoint by id. Returns the removed breakpoint, if any.
    pub fn clear_breakpoint(&mut self, id: &str) -> Option<Breakpoint> {
        let removed = self.breakpoints.remove(id);
        // Drop any pending debounce window for this breakpoint — it no
        // longer exists, so there's nothing to suppress.
        self.breakpoint_debounce.remove(id);
        // If we cleared the breakpoint that is currently pausing us, also
        // drop the pause state — nothing left to hold it.
        if removed.is_some()
            && self
                .paused_at_breakpoint
                .as_deref()
                .map(|p| p == id)
                .unwrap_or(false)
        {
            self.paused = false;
            self.paused_at_breakpoint = None;
        }
        removed
    }

    /// List all currently-registered breakpoints as `(id, breakpoint)` pairs.
    ///
    /// Returned in deterministic order (sorted by id) so UIs get stable
    /// row ordering across polls.
    pub fn list_breakpoints(&self) -> Vec<(BreakpointId, Breakpoint)> {
        let mut out: Vec<(BreakpointId, Breakpoint)> = self
            .breakpoints
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    /// Returns `true` if the session is currently paused at a breakpoint.
    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// The id of the breakpoint that triggered the current pause, if any.
    pub fn paused_at_breakpoint(&self) -> Option<&str> {
        self.paused_at_breakpoint.as_deref()
    }

    /// Clear the pause flag so subsequent [`RuntimeSession::step`] calls
    /// advance the orchestrator again. Breakpoints remain registered and
    /// can fire again on later steps.
    pub fn resume(&mut self) {
        self.paused = false;
        self.paused_at_breakpoint = None;
    }

    /// Evaluate registered breakpoints against the given snapshot.
    ///
    /// Sets `paused` / `paused_at_breakpoint` if any breakpoint matches.
    /// The check covers:
    /// - `StateEntry` / `TransitionFire` / `ActionInvoke`: matches against
    ///   the subsystem `current_state` (which for the action runner is the
    ///   current node id) — cheap and covers the common cases.
    /// - `ConstraintViolation`: matches a constraint result whose name
    ///   equals the breakpoint's element id and whose `satisfied` is
    ///   `false`.
    /// - `ThresholdCrossing` / `Conditional` (BP4: unified onto one
    ///   `CompareBreakpoint` field set — `snapshot.<variable> <op> <value>`):
    ///   fires when the compare is true and the breakpoint is `enabled`
    ///   (always `true` for the historical unscoped `ThresholdCrossing`
    ///   tag). The variable's `Value` is coerced to `f64` via
    ///   `snapshot_view::value_to_scalar` — Int/Float pass through
    ///   unchanged and **`Bool` coerces to `1.0`/`0.0`** (BP3: a bool
    ///   context var flipping to `true` now fires an `op: Eq, value: 1.0`
    ///   breakpoint instead of silently never matching, since `Bool` used
    ///   to fall through the old inline Float/Int-only match to `None`).
    ///   When `debounce_ticks > 0` a firing arms a suppression window so
    ///   the breakpoint doesn't fire every tick while the condition stays
    ///   true; the counter decrements once per tick.
    ///
    /// The first breakpoint that matches wins (deterministic id order).
    /// If multiple breakpoints fire on the same tick, richer selection
    /// (hit count / priority) is deferred to later rounds.
    ///
    /// Returns `true` if a breakpoint fired this tick (`false` if none
    /// matched, none are registered, or the session was already paused).
    /// UX closeout arc #7 (`step`) reads this to decide whether the tick
    /// must be force-archived — see the ordering-trap note on `step`.
    fn check_breakpoints(&mut self, snapshot: &ExecutionSnapshot) -> bool {
        if self.breakpoints.is_empty() || self.paused {
            // Advance debounce counters even while paused / without
            // breakpoints, so a subsequent resume or clear-then-set
            // doesn't inherit stale suppression from a prior tick.
            self.breakpoint_debounce.retain(|_, remaining| {
                *remaining = remaining.saturating_sub(1);
                *remaining > 0
            });
            return false;
        }

        // Walk breakpoints in deterministic id order so multi-match behaviour
        // is stable across polls.
        let mut ids: Vec<&str> = self.breakpoints.keys().map(|s| s.as_str()).collect();
        ids.sort();

        for id in ids {
            let Some(bp) = self.breakpoints.get(id) else {
                continue;
            };
            // Debounced breakpoints remain armed across polls but are
            // suppressed until their countdown elapses.
            if self.breakpoint_debounce.contains_key(id) {
                continue;
            }
            let (hit, debounce_window) = match bp {
                Breakpoint::StateEntry { element_id }
                | Breakpoint::TransitionFire { element_id }
                | Breakpoint::ActionInvoke { element_id } => (
                    snapshot
                        .subsystem_states
                        .values()
                        .any(|s| s.current_state == *element_id),
                    0,
                ),
                // A *violation* is a decided `Fail` — the run evaluated the
                // constraint and it did not hold. An `Inconclusive` row (an
                // unbound parameter, say) has established nothing to break
                // on, and used to trip this breakpoint only because the
                // verdict was flattened into `!satisfied`.
                Breakpoint::ConstraintViolation { element_id } => (
                    snapshot
                        .constraint_results
                        .iter()
                        .any(|c| c.name == *element_id && c.verdict == VerdictKind::Fail),
                    0,
                ),
                // BP4: `ThresholdCrossing` and `Conditional` now share one
                // `CompareBreakpoint` field set, so both variants compare
                // the same way — one arm instead of two near-identical
                // copies. BP3: the scalar pull goes through
                // `snapshot_view::value_to_scalar`, which (unlike the old
                // inline Float/Int-only match here) also coerces
                // `Value::Bool` to `1.0`/`0.0` — matching the convention
                // already used by `constraints.rs` and `snapshot_view.rs`
                // itself — so a bool context var flipping to `true` fires
                // an `op: Eq, value: 1.0` breakpoint instead of silently
                // never matching. A disabled breakpoint (`enabled: false`,
                // only reachable via the historical `Conditional` tag)
                // never fires.
                Breakpoint::ThresholdCrossing(f) | Breakpoint::Conditional(f) => {
                    let matched = f.enabled
                        && snapshot
                            .variables
                            .get(&f.variable)
                            .and_then(sysml_runtime::snapshot_view::value_to_scalar)
                            .map(|current| f.op.apply(current, f.value))
                            .unwrap_or(false);
                    (matched, f.debounce_ticks)
                }
            };
            if hit {
                // Arm a debounce window (if any) so this breakpoint
                // doesn't re-fire every tick while the condition holds.
                if debounce_window > 0 {
                    self.breakpoint_debounce
                        .insert(id.to_owned(), debounce_window);
                }
                self.paused = true;
                self.paused_at_breakpoint = Some(id.to_owned());
                return true;
            }
        }

        // No breakpoint fired this tick. Advance all pre-existing debounce
        // counters — a counter armed *this* tick (by a fire that already
        // returned above) is never decremented on its arming tick, so the
        // first full suppression happens on the very next tick.
        self.breakpoint_debounce.retain(|_, remaining| {
            *remaining = remaining.saturating_sub(1);
            *remaining > 0
        });
        false
    }
}

// ---------------------------------------------------------------------------
// Event-significance classification (UX closeout arc #7)
// ---------------------------------------------------------------------------

/// Executor kinds whose `SubsystemState::current_state` is a genuine
/// discrete state — a state machine's current state name, or an action
/// graph's current node id.
///
/// `TickOutput::current_state`'s own doc comment spells out the three
/// shapes it carries: "state name, node ID, **or formatted primary
/// value**". That third shape is what continuous-dynamics executors
/// (`ode`, `ode45`, `bdf`, `discrete` [`DiscreteStateSolver`], `physics`,
/// `continuousDynamicsHybrid`, `modeSelectionHybrid`, …) put there — e.g.
/// the ODE's dominant state variable rendered for display — and it changes
/// on *every* tick by construction. Treating that as an "SM transition"
/// would force-archive every single tick on any physics-bearing model
/// (confirmed against the reference hybrid example during this arc's own perf
/// measurement), defeating archive cadence entirely for exactly the
/// workloads it exists to help. Only `stateMachine` and `action` executors
/// carry a real discrete state in this field.
fn is_discrete_subsystem_kind(kind: &str) -> bool {
    matches!(kind, "stateMachine" | "action")
}

/// Returns `true` if any **discrete** subsystem's `current_state` differs
/// between the previous and current snapshot — a state-machine (or
/// action-runner) transition happened this tick. Continuous-dynamics
/// subsystems (ODE/physics/hybrid — see [`is_discrete_subsystem_kind`])
/// are excluded from this comparison; their `current_state` is a
/// continuously-varying display value, not a discrete transition.
fn subsystem_state_transitioned(prev: &ExecutionSnapshot, cur: &ExecutionSnapshot) -> bool {
    fn discrete_states(snap: &ExecutionSnapshot) -> HashMap<&str, &str> {
        snap.subsystem_states
            .iter()
            .filter(|(_, s)| is_discrete_subsystem_kind(s.kind))
            .map(|(name, s)| (name.as_str(), s.current_state.as_str()))
            .collect()
    }
    let prev_discrete = discrete_states(prev);
    let cur_discrete = discrete_states(cur);
    if prev_discrete.len() != cur_discrete.len() {
        return true;
    }
    cur_discrete
        .iter()
        .any(|(name, state)| prev_discrete.get(name) != Some(state))
}

/// Returns `true` if any `Value::Bool` context variable changed value
/// between the previous and current snapshot.
///
/// This is a generic "trip-latch flip" detector: the design doc's event
/// class is about a discrete boolean flag flipping (e.g. a firmware
/// trip latch), but nothing in the runtime names that variable — any
/// boolean-typed variable flipping counts, model-agnostically. A boolean
/// variable appearing for the first time this tick (no prior value to
/// compare against, e.g. lazily materialized) is not treated as a flip.
fn bool_variable_flipped(prev: &ExecutionSnapshot, cur: &ExecutionSnapshot) -> bool {
    cur.variables.iter().any(|(name, value)| match value {
        sysml_core::Value::Bool(b) => matches!(
            prev.variables.get(name),
            Some(sysml_core::Value::Bool(pb)) if pb != b
        ),
        _ => false,
    })
}

impl fmt::Debug for RuntimeSession {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RuntimeSession")
            .field("uri", &self.uri)
            .field("kind", &self.kind)
            .field("tick", &self.orchestrator.tick())
            .field("history_len", &self.history.len())
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;
    use sysml_runtime::orchestrator::Orchestrator;
    use sysml_runtime::statemachine::StateMachineRunner;
    use sysml_runtime::{StateIR, StateMachineIR, TransitionIR};

    fn simple_sm_ir() -> StateMachineIR {
        StateMachineIR::new("TestSM", "idle")
            .with_state(StateIR::new("idle"))
            .with_state(StateIR::new("running"))
            .with_state(StateIR::new("done").final_state())
            .with_transition(TransitionIR::new("idle", "running").with_event("start"))
            .with_transition(TransitionIR::new("running", "done").with_event("stop"))
    }

    fn make_sim_session(ir: StateMachineIR, sm_name: &str) -> RuntimeSession {
        let runner = StateMachineRunner::new(ir);
        let mut orchestrator = Orchestrator::new(Default::default());
        orchestrator.add_state_machine(sm_name, runner);
        RuntimeSession::new(
            orchestrator,
            "test".to_owned(),
            SessionKind::Simulation,
            Some(sm_name.to_owned()),
        )
    }

    fn current_state<'a>(snap: &'a ExecutionSnapshot, name: &str) -> &'a str {
        snap.subsystem_states
            .get(name)
            .map(|s| s.current_state.as_str())
            .unwrap_or("")
    }

    #[test]
    fn test_simulation_lifecycle() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");

        let snap = session.step();
        assert_eq!(current_state(&snap, "sm"), "idle");

        let snap = session.inject_event("sm", "start");
        assert_eq!(current_state(&snap, "sm"), "running");

        let snap = session.inject_event("sm", "stop");
        assert_eq!(current_state(&snap, "sm"), "done");
        assert!(snap.subsystem_states.get("sm").unwrap().completed);

        assert_eq!(session.history().len(), 3);
    }

    #[test]
    fn test_session_expiry() {
        let session = make_sim_session(simple_sm_ir(), "sm");

        // A freshly-created session should not be expired.
        assert!(!session.is_expired());
    }

    /// Expiry is IDLE-based, not age-based: advancing the session (step)
    /// refreshes the idle clock, so an actively-driven run is never reaped —
    /// even one that had crossed the timeout while idle. Under the old
    /// age-based `created_at` clock, `step()` would NOT un-expire a session.
    #[test]
    fn test_expiry_is_idle_based_step_refreshes_clock() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");

        // Force the idle clock past the timeout.
        session.test_mark_expired();
        assert!(session.is_expired(), "session should be expired while idle");

        // Stepping is activity — it must refresh the idle clock.
        let _ = session.step();
        assert!(
            !session.is_expired(),
            "stepping must un-expire (idle clock refreshed)"
        );

        // An explicit touch (e.g. a future keepalive) does the same.
        session.test_mark_expired();
        assert!(session.is_expired());
        session.touch();
        assert!(!session.is_expired(), "touch() must refresh the idle clock");
    }

    fn make_action_session(name: &str) -> RuntimeSession {
        use sysml_runtime::actions::{ActionGraphIR, ActionNodeIR, ActionRunner};

        // Build a minimal action graph: initial -> merge -> final
        let mut ir = ActionGraphIR::new(name, "TestAction");
        let merge_node = ActionNodeIR::Merge {
            id: "m1".to_string(),
        };
        ir.add_node(merge_node);
        let initial = ir.initial_node_id.clone();
        let final_id = ir.final_node_ids[0].clone();
        ir.add_edge(&initial, "m1");
        ir.add_edge("m1", &final_id);

        let runner = ActionRunner::new(ir);
        let mut orchestrator = Orchestrator::new(Default::default());
        orchestrator.add_action(name, runner);
        RuntimeSession::new(
            orchestrator,
            "test".to_owned(),
            SessionKind::Action,
            Some(name.to_owned()),
        )
    }

    // ---- T9 — TimeSeriesBuffer canonical numeric store --------------------

    #[test]
    fn time_series_starts_empty() {
        let session = make_sim_session(simple_sm_ir(), "sm");
        assert!(session.time_series().is_empty());
    }

    #[test]
    fn time_series_accumulates_steps() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        let initial_len = session.time_series().len();
        session.step();
        session.step();
        session.step();
        assert_eq!(session.time_series().len(), initial_len + 3);
    }

    #[test]
    fn time_series_clears_on_reset() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session.step();
        session.step();
        assert!(!session.time_series().is_empty());
        session.reset();
        assert!(session.time_series().is_empty());
    }

    #[test]
    fn time_series_fresh_per_fork() {
        let mut parent = make_sim_session(simple_sm_ir(), "sm");
        parent.step();
        parent.step();
        assert!(!parent.time_series().is_empty());
        let child = parent.fork_child();
        // Child inherits no time-series; its own steps build it up.
        assert!(child.time_series().is_empty());
        // Parent's buffer is unaffected by the fork.
        assert_eq!(parent.time_series().len(), 2);
    }

    #[test]
    fn snapshot_observer_broadcasts_step_snapshots() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        let mut rx = session.subscribe_snapshots();

        // Each step must push exactly one snapshot into the broadcast.
        let stepped = session.step();
        let received = rx.try_recv().expect("first snapshot must be delivered");
        assert_eq!(received.tick, stepped.tick);
        assert_eq!(current_state(&received, "sm"), current_state(&stepped, "sm"));

        let stepped = session.inject_event("sm", "start");
        let received = rx.try_recv().expect("second snapshot must be delivered");
        assert_eq!(received.tick, stepped.tick);
        assert_eq!(current_state(&received, "sm"), "running");

        // No additional traffic without another step.
        assert!(matches!(
            rx.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn snapshot_observer_skips_broadcast_with_zero_subscribers() {
        // live-perf B2a: no subscriber has ever been created, so
        // `snapshot_tx.receiver_count()` is 0 for the whole test. Stepping
        // must not panic, and `history`/`time_series` — which are populated
        // from the value `Orchestrator::step` returns to the caller, not
        // from the broadcast observer — must still accumulate exactly as
        // they would with a subscriber attached.
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        assert!(session.history().is_empty());
        assert!(session.time_series().is_empty());

        let stepped = session.step();
        assert_eq!(session.history().len(), 1);
        assert_eq!(session.history().back().unwrap().tick, stepped.tick);
        assert_eq!(session.time_series().len(), 1);

        let stepped = session.inject_event("sm", "start");
        assert_eq!(session.history().len(), 2);
        assert_eq!(current_state(&stepped, "sm"), "running");
        assert_eq!(session.time_series().len(), 2);

        // Now attach a subscriber after the fact and confirm the frame
        // still arrives once `receiver_count()` is non-zero again — the
        // guard only skips the send when there is truly no one listening,
        // it never permanently disables the broadcast.
        let mut rx = session.subscribe_snapshots();
        let stepped = session.step();
        let received = rx.try_recv().expect("frame must arrive once subscribed");
        assert_eq!(received.tick, stepped.tick);
    }

    #[test]
    fn snapshot_observer_is_independent_per_fork() {
        let mut parent = make_sim_session(simple_sm_ir(), "sm");
        let mut parent_rx = parent.subscribe_snapshots();
        // Seed the parent's history so fork_child has a reseed snapshot.
        parent.step();
        let _ = parent_rx.try_recv();

        let mut child = parent.fork_child();
        let mut child_rx = child.subscribe_snapshots();

        // A child step must reach the child's subscribers but NOT the parent's.
        child.step();
        assert!(child_rx.try_recv().is_ok(), "child receiver sees child ticks");
        assert!(
            matches!(
                parent_rx.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ),
            "child ticks must not leak onto parent receivers",
        );

        // And vice versa.
        parent.step();
        assert!(parent_rx.try_recv().is_ok(), "parent receiver sees parent ticks");
        assert!(
            matches!(
                child_rx.try_recv(),
                Err(tokio::sync::broadcast::error::TryRecvError::Empty)
            ),
            "parent ticks must not leak onto child receivers",
        );
    }

    #[test]
    fn test_action_session() {
        let mut session = make_action_session("test_action");

        // Step until completion or a few iterations.
        for _ in 0..5 {
            let snap = session.step();
            if snap.subsystem_states.get("test_action").unwrap().completed {
                break;
            }
        }

        assert!(!session.history().is_empty());
    }

    // -----------------------------------------------------------------------
    // Breakpoint primitives (R1.2)
    // -----------------------------------------------------------------------

    #[test]
    fn breakpoint_set_clear_list_round_trip() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        assert!(session.list_breakpoints().is_empty());
        assert!(!session.is_paused());

        let id1 = session.set_breakpoint(Breakpoint::state_entry("running"));
        let id2 = session.set_breakpoint(Breakpoint::transition_fire("t1"));
        let listed = session.list_breakpoints();
        assert_eq!(listed.len(), 2);
        let names: Vec<&BreakpointId> = listed.iter().map(|(id, _)| id).collect();
        assert!(names.iter().any(|id| **id == id1));
        assert!(names.iter().any(|id| **id == id2));

        // Deterministic order: sorted by id.
        let sorted_names: Vec<&BreakpointId> = {
            let mut s = names.clone();
            s.sort();
            s
        };
        assert_eq!(names, sorted_names, "list_breakpoints must return sorted ids");

        // Clear by id.
        let removed = session.clear_breakpoint(&id1);
        assert_eq!(removed, Some(Breakpoint::state_entry("running")));
        assert_eq!(session.list_breakpoints().len(), 1);

        // Clearing an unknown id returns None and doesn't panic.
        assert!(session.clear_breakpoint("no-such-id").is_none());
    }

    #[test]
    fn breakpoint_state_entry_pauses_session() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        let id = session.set_breakpoint(Breakpoint::state_entry("running"));

        // Tick 1: enter initial state "idle" — no breakpoint.
        let snap = session.step();
        assert_eq!(current_state(&snap, "sm"), "idle");
        assert!(!session.is_paused(), "should not pause on idle");

        // Inject "start" → session should transition to "running" and pause.
        let snap = session.inject_event("sm", "start");
        assert_eq!(current_state(&snap, "sm"), "running");
        assert!(session.is_paused(), "expected pause on state-entry breakpoint");
        assert_eq!(session.paused_at_breakpoint(), Some(id.as_str()));

        // While paused, step() must be a no-op — still reports "running".
        let before_len = session.history().len();
        let snap2 = session.step();
        assert_eq!(current_state(&snap2, "sm"), "running");
        assert_eq!(
            session.history().len(),
            before_len,
            "paused step must not advance history"
        );

        // resume() clears pause so subsequent injects work.
        session.resume();
        assert!(!session.is_paused());
        assert!(session.paused_at_breakpoint().is_none());
        let snap3 = session.inject_event("sm", "stop");
        assert_eq!(current_state(&snap3, "sm"), "done");
    }

    #[test]
    fn breakpoint_clear_active_breakpoint_unpauses() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        let id = session.set_breakpoint(Breakpoint::state_entry("running"));

        let _ = session.step(); // idle
        let _ = session.inject_event("sm", "start");
        assert!(session.is_paused());

        // Clearing the breakpoint that is currently pausing should release
        // the pause so the session can continue.
        let removed = session.clear_breakpoint(&id);
        assert!(removed.is_some());
        assert!(!session.is_paused());
        assert!(session.paused_at_breakpoint().is_none());
    }

    #[test]
    fn breakpoint_reset_clears_pause_preserves_breakpoints() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        let _id = session.set_breakpoint(Breakpoint::state_entry("running"));
        let _ = session.step();
        let _ = session.inject_event("sm", "start");
        assert!(session.is_paused());

        session.reset();

        assert!(!session.is_paused());
        assert!(session.paused_at_breakpoint().is_none());
        // Breakpoints survive reset — they're debug metadata.
        assert_eq!(session.list_breakpoints().len(), 1);
    }

    #[test]
    fn breakpoint_fork_inherits_and_diverges() {
        let mut parent = make_sim_session(simple_sm_ir(), "sm");
        let _ = parent.set_breakpoint(Breakpoint::state_entry("running"));
        let _ = parent.step();

        let mut child = parent.fork_child();
        assert_eq!(child.list_breakpoints().len(), 1);
        // Child starts un-paused even if the parent is paused.
        assert!(!child.is_paused());

        // Mutating the child's breakpoint set should not affect the parent.
        child.clear_breakpoint(&child.list_breakpoints()[0].0);
        assert!(child.list_breakpoints().is_empty());
        assert_eq!(parent.list_breakpoints().len(), 1);
    }

    // -----------------------------------------------------------------------
    // Conditional + debounced-threshold breakpoints (R4.4)
    // -----------------------------------------------------------------------

    #[test]
    fn conditional_breakpoint_fires_when_condition_true() {
        use sysml_runtime::breakpoint::CompareOp;

        let mut session = make_sim_session(simple_sm_ir(), "sm");
        // Seed a variable on the orchestrator context so the first step's
        // snapshot carries `voltage = 15.0`.
        session
            .orchestrator
            .context
            .set("voltage".to_owned(), sysml_core::Value::Float(15.0));

        let id = session.set_breakpoint(Breakpoint::conditional(
            "circuit1",
            "voltage",
            CompareOp::Gt,
            12.0,
        ));

        let _ = session.step();
        assert!(session.is_paused(), "conditional should fire on voltage > 12");
        assert_eq!(session.paused_at_breakpoint(), Some(id.as_str()));
    }

    #[test]
    fn conditional_breakpoint_does_not_fire_when_condition_false() {
        use sysml_runtime::breakpoint::CompareOp;

        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session
            .orchestrator
            .context
            .set("voltage".to_owned(), sysml_core::Value::Float(5.0));

        let _id = session.set_breakpoint(Breakpoint::conditional(
            "circuit1",
            "voltage",
            CompareOp::Gt,
            12.0,
        ));

        let _ = session.step();
        assert!(
            !session.is_paused(),
            "conditional must not fire when the predicate is false"
        );
    }

    #[test]
    fn conditional_breakpoint_ignored_when_disabled() {
        use sysml_runtime::breakpoint::CompareOp;

        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session
            .orchestrator
            .context
            .set("voltage".to_owned(), sysml_core::Value::Float(15.0));

        let disabled = Breakpoint::Conditional(sysml_runtime::breakpoint::CompareBreakpoint {
            variable: "voltage".to_owned(),
            op: CompareOp::Gt,
            value: 12.0,
            debounce_ticks: 0,
            enabled: false,
            target: Some("circuit1".to_owned()),
            label: None,
        });
        session.set_breakpoint(disabled);

        let _ = session.step();
        assert!(
            !session.is_paused(),
            "disabled conditional must never fire"
        );
    }

    #[test]
    fn conditional_breakpoint_handles_negative_and_zero_values() {
        use sysml_runtime::breakpoint::CompareOp;

        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session
            .orchestrator
            .context
            .set("x".to_owned(), sysml_core::Value::Float(-3.5));

        let id = session.set_breakpoint(Breakpoint::conditional(
            "p",
            "x",
            CompareOp::Lt,
            0.0,
        ));
        let _ = session.step();
        assert!(
            session.is_paused(),
            "conditional should fire for -3.5 < 0.0"
        );
        assert_eq!(session.paused_at_breakpoint(), Some(id.as_str()));
    }

    #[test]
    fn conditional_breakpoint_skips_nan_safely() {
        use sysml_runtime::breakpoint::CompareOp;

        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session
            .orchestrator
            .context
            .set("x".to_owned(), sysml_core::Value::Float(f64::NAN));

        session.set_breakpoint(Breakpoint::conditional(
            "p",
            "x",
            CompareOp::Gt,
            0.0,
        ));
        let _ = session.step();
        assert!(
            !session.is_paused(),
            "NaN should not satisfy Gt — breakpoint must stay un-fired"
        );
    }

    #[test]
    fn threshold_breakpoint_with_debounce_fires_once_then_suppresses() {
        use sysml_runtime::breakpoint::CompareOp;

        let mut session = make_sim_session(simple_sm_ir(), "sm");
        // Seed variable permanently > threshold so the predicate stays
        // true across multiple ticks — without debounce this would pause
        // every tick.
        session
            .orchestrator
            .context
            .set("i_total".to_owned(), sysml_core::Value::Float(50.0));

        let id = session.set_breakpoint(Breakpoint::threshold_crossing_with_debounce(
            "i_total",
            CompareOp::Gt,
            32.0,
            3, // suppress for 3 ticks after firing
        ));

        // Tick 1: fires, arms the debounce window.
        let _ = session.step();
        assert!(session.is_paused(), "expected first fire");
        assert_eq!(session.paused_at_breakpoint(), Some(id.as_str()));

        // Manually resume, then step 3 more times. The breakpoint must
        // stay suppressed for the entirety of the debounce window.
        session.resume();
        for tick in 0..3 {
            let _ = session.step();
            assert!(
                !session.is_paused(),
                "debounce tick {tick}: breakpoint must stay suppressed"
            );
        }
        // Next step: debounce counter has elapsed, it should fire again.
        let _ = session.step();
        assert!(
            session.is_paused(),
            "breakpoint should re-fire after the debounce window elapses"
        );
    }

    #[test]
    fn threshold_breakpoint_without_debounce_fires_every_tick() {
        use sysml_runtime::breakpoint::CompareOp;

        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session
            .orchestrator
            .context
            .set("i_total".to_owned(), sysml_core::Value::Float(50.0));

        // Legacy behaviour — debounce_ticks = 0 (default).
        session.set_breakpoint(Breakpoint::threshold_crossing(
            "i_total",
            CompareOp::Gt,
            32.0,
        ));

        let _ = session.step();
        assert!(session.is_paused());
        // Resume then re-step: without debounce the condition still
        // holds and must re-fire immediately.
        session.resume();
        let _ = session.step();
        assert!(
            session.is_paused(),
            "zero-debounce threshold should re-fire on the next tick"
        );
    }

    #[test]
    fn bool_context_var_flip_fires_eq_one_breakpoint() {
        // BP3: a bool context var flipping to `true` must fire an
        // `op: Eq, value: 1.0` breakpoint on that tick — before this fix,
        // `check_breakpoints`'s inline Float/Int-only scalar match
        // silently dropped `Value::Bool`, so a boolean condition (e.g. a
        // firmware "tripped" latch) could never satisfy any breakpoint.
        use sysml_runtime::breakpoint::CompareOp;

        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session
            .orchestrator
            .context
            .set("tripped".to_owned(), sysml_core::Value::Bool(false));

        let id = session.set_breakpoint(Breakpoint::conditional(
            "circuit1",
            "tripped",
            CompareOp::Eq,
            1.0,
        ));

        // Still false: must not fire.
        let _ = session.step();
        assert!(!session.is_paused(), "tripped=false must not satisfy Eq 1.0");

        // Flip to true and step again: 1.0 Eq 1.0 must fire.
        session
            .orchestrator
            .context
            .set("tripped".to_owned(), sysml_core::Value::Bool(true));
        let _ = session.step();
        assert!(session.is_paused(), "tripped=true (-> 1.0) must satisfy Eq 1.0");
        assert_eq!(session.paused_at_breakpoint(), Some(id.as_str()));
    }

    // -----------------------------------------------------------------------
    // Tests migrated from sysml-lsp-server/src/simulation.rs
    // -----------------------------------------------------------------------

    /// Build a simple traffic light state machine IR.
    fn traffic_light_ir() -> StateMachineIR {
        StateMachineIR::new("TrafficLight", "Red")
            .with_state(StateIR::new("Red"))
            .with_state(StateIR::new("Green"))
            .with_state(StateIR::new("Yellow"))
            .with_transition(TransitionIR::new("Red", "Green").with_event("timer"))
            .with_transition(TransitionIR::new("Green", "Yellow").with_event("timer"))
            .with_transition(TransitionIR::new("Yellow", "Red").with_event("timer"))
    }

    /// Build a graph with a state machine for compilation testing.
    fn traffic_light_graph() -> sysml_core::ModelGraph {
        use sysml_core::{Element, ElementKind, ModelGraph, Relationship, RelationshipKind};

        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition).with_name("TrafficLight");
        let sm_id = graph.add_element(sm);

        let red = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Red")
            .with_owner(sm_id.clone())
            .with_prop("initial", sysml_core::Value::Bool(true));
        let red_id = graph.add_element(red);

        let green = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Green")
            .with_owner(sm_id.clone());
        let green_id = graph.add_element(green);

        let yellow = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Yellow")
            .with_owner(sm_id.clone());
        let yellow_id = graph.add_element(yellow);

        // Red -> Green on "timer"
        let mut t1 = Relationship::new(
            RelationshipKind::Transition,
            red_id.clone(),
            green_id.clone(),
        );
        t1.props
            .insert("event".into(), sysml_core::Value::String("timer".into()));
        graph.add_relationship(t1);

        // Green -> Yellow on "timer"
        let mut t2 = Relationship::new(
            RelationshipKind::Transition,
            green_id.clone(),
            yellow_id.clone(),
        );
        t2.props
            .insert("event".into(), sysml_core::Value::String("timer".into()));
        graph.add_relationship(t2);

        // Yellow -> Red on "timer"
        let mut t3 = Relationship::new(
            RelationshipKind::Transition,
            yellow_id.clone(),
            red_id.clone(),
        );
        t3.props
            .insert("event".into(), sysml_core::Value::String("timer".into()));
        graph.add_relationship(t3);

        graph
    }

    #[test]
    fn session_start_step_lifecycle() {
        let mut session = make_sim_session(traffic_light_ir(), "tl");

        let snap = session.step();
        assert_eq!(current_state(&snap, "tl"), "Red");
        assert_eq!(session.history().len(), 1);

        // timer event: Red -> Green
        let snap = session.inject_event("tl", "timer");
        assert_eq!(current_state(&snap, "tl"), "Green");
        assert_eq!(session.history().len(), 2);

        // timer event: Green -> Yellow
        let snap = session.inject_event("tl", "timer");
        assert_eq!(current_state(&snap, "tl"), "Yellow");
        assert_eq!(session.history().len(), 3);

        // timer event: Yellow -> Red
        let snap = session.inject_event("tl", "timer");
        assert_eq!(current_state(&snap, "tl"), "Red");
        assert_eq!(session.history().len(), 4);
    }

    #[test]
    fn session_compile_from_graph() {
        use std::sync::Arc;

        let graph = traffic_light_graph();
        let snap = sysml_ide_db::Snapshot::new(Arc::new(graph));
        let ir = snap
            .compile_state_machine("TrafficLight")
            .expect("should compile state machine from graph");

        let mut session = make_sim_session(ir, "tl");
        let snap = session.step();
        assert_eq!(current_state(&snap, "tl"), "Red");

        let snap = session.inject_event("tl", "timer");
        assert_eq!(current_state(&snap, "tl"), "Green");
    }

    #[test]
    fn session_guard_evaluation() {
        // State machine with a guarded transition
        let ir = StateMachineIR::new("Guarded", "Idle")
            .with_state(StateIR::new("Idle"))
            .with_state(StateIR::new("Active"))
            .with_transition(
                TransitionIR::new("Idle", "Active")
                    .with_event("go")
                    .with_guard("speed > 0"),
            );

        let mut session = make_sim_session(ir, "g");
        let snap = session.step();
        assert_eq!(current_state(&snap, "g"), "Idle");

        // Without setting speed, guard won't pass (undefined var falls back)
        let snap = session.inject_event("g", "go");
        assert_eq!(current_state(&snap, "g"), "Idle");

        // Set speed via the orchestrator's shared context; sync_context_in
        // merges it into the runner's eval_ctx on the next tick.
        session
            .orchestrator
            .context
            .set("speed", sysml_core::Value::Int(10));
        let snap = session.inject_event("g", "go");
        assert_eq!(current_state(&snap, "g"), "Active");
    }

    #[test]
    fn session_expiry_and_reset() {
        let mut session = make_sim_session(traffic_light_ir(), "tl");

        // Fresh session should not be expired
        assert!(!session.is_expired());

        // Step a few times
        session.step(); // initial: Red
        session.inject_event("tl", "timer"); // -> Green
        session.inject_event("tl", "timer"); // -> Yellow
        assert_eq!(session.history().len(), 3);
        let last = session.history().back().unwrap();
        assert_eq!(current_state(last, "tl"), "Yellow");

        // Reset should return to initial state
        session.reset();
        assert!(session.history().is_empty());
        assert!(!session.is_expired());
        let snap = session.step();
        assert_eq!(current_state(&snap, "tl"), "Red");
    }

    // -----------------------------------------------------------------------
    // Tests migrated from sysml-lsp-server/src/action_session.rs
    // -----------------------------------------------------------------------

    /// Build a simple action IR with an assign node.
    fn simple_action_ir() -> sysml_runtime::actions::ActionGraphIR {
        use sysml_runtime::actions::{ActionGraphIR, ActionNodeIR};

        let mut ir = ActionGraphIR::new("test", "TestAction");
        // Add an assign node between initial and final
        let assign_id = ir.add_node(ActionNodeIR::Assign {
            id: "assign_x".into(),
            target: "x".into(),
            value: sysml_runtime::expressions::ExprIR::LiteralInt(42),
        });
        ir.add_edge(&ir.initial_node_id.clone(), &assign_id);
        ir.add_edge(&assign_id, &ir.final_node_ids[0].clone());
        ir
    }

    fn make_action_runtime_session(name: &str) -> RuntimeSession {
        use sysml_runtime::actions::ActionRunner;
        let runner = ActionRunner::new(simple_action_ir());
        let mut orchestrator = Orchestrator::new(Default::default());
        orchestrator.add_action(name, runner);
        RuntimeSession::new(
            orchestrator,
            "test".to_owned(),
            SessionKind::Action,
            Some(name.to_owned()),
        )
    }

    #[test]
    fn test_action_session_new() {
        let session = make_action_runtime_session("act");
        assert!(session.history().is_empty());
    }

    #[test]
    fn test_action_session_step() {
        let mut session = make_action_runtime_session("act");

        // Step until the action completes or we've taken a handful of ticks.
        for _ in 0..5 {
            let snap = session.step();
            assert!(!snap
                .subsystem_states
                .get("act")
                .unwrap()
                .current_state
                .is_empty());
            if snap.subsystem_states.get("act").unwrap().completed {
                break;
            }
        }

        assert!(!session.history().is_empty());
    }

    #[test]
    fn test_action_session_reset() {
        let mut session = make_action_runtime_session("act");

        session.step();
        session.step();
        assert!(!session.history().is_empty());

        session.reset();
        assert!(session.history().is_empty());
    }

    #[test]
    fn test_format_trace() {
        // `format_trace` operates on `ActionTraceEntry` records — exercise it
        // directly with synthetic entries rather than through a deprecated
        // session type.
        let mut history: VecDeque<ActionTraceEntry> = VecDeque::new();
        history.push_back(ActionTraceEntry {
            node_id: "n1".into(),
            completed: false,
            outputs: vec![],
            flow_events: vec![],
            diagnostics: vec![],
        });
        history.push_back(ActionTraceEntry {
            node_id: "n2".into(),
            completed: true,
            outputs: vec!["out".into()],
            flow_events: vec![],
            diagnostics: vec![],
        });

        let trace = format_trace(&history);
        assert!(trace.contains("step 1"));
        assert!(trace.contains("step 2"));
        assert!(trace.contains("[COMPLETED]"));
    }

    #[test]
    fn test_action_session_expiry() {
        let session = make_action_runtime_session("act");
        assert!(!session.is_expired());
    }

    // -----------------------------------------------------------------------
    // B1 — Session catalog / summary / detail / quota / expiry
    // -----------------------------------------------------------------------

    #[test]
    fn test_quota_for_each_kind() {
        assert_eq!(quota_for(SessionKind::Simulation), 30);
        assert_eq!(quota_for(SessionKind::Action), 30);
        assert_eq!(quota_for(SessionKind::Orchestrator), 20);
    }

    #[test]
    fn test_summary_on_fresh_session() {
        let session = make_sim_session(simple_sm_ir(), "sm");
        let summary = session.summary("abc-123".to_owned());
        assert_eq!(summary.id, "abc-123");
        assert_eq!(summary.kind, SessionKind::Simulation);
        assert_eq!(summary.subsystem_name.as_deref(), Some("sm"));
        assert_eq!(summary.label, None);
        assert_eq!(summary.tick, 0);
        assert_eq!(summary.time_ms, 0.0);
        assert_eq!(summary.current_state, None); // no snapshot yet
        assert!(!summary.completed);
        assert!(!summary.is_expired);
        assert_eq!(summary.history_len, 0);
        assert_eq!(summary.subsystem_count, 1);
        assert!(summary.created_at_ms > 0);
    }

    #[test]
    fn test_summary_after_step_reflects_state() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        let _ = session.step();
        let summary = session.summary("id".to_owned());
        assert_eq!(summary.history_len, 1);
        assert_eq!(summary.current_state.as_deref(), Some("idle"));
        assert!(summary.tick >= 1);
    }

    #[test]
    fn test_detail_enumerates_subsystems() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        let _ = session.step();
        let detail = session.detail("id".to_owned());
        assert_eq!(detail.subsystems.len(), 1);
        assert_eq!(detail.subsystems[0].name, "sm");
        assert_eq!(detail.subsystems[0].current_state, "idle");
        assert_eq!(detail.subsystems[0].kind_label, "stateMachine");
        assert!(detail.latest_snapshot.is_some());
    }

    #[test]
    fn test_mark_expired_helper_flips_flag() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        assert!(!session.is_expired());
        session.test_mark_expired();
        assert!(session.is_expired());
    }

    // -----------------------------------------------------------------------
    // Topology — domain inference, humanize, topology building
    // -----------------------------------------------------------------------

    #[test]
    fn test_infer_subsystem_domain_thermal() {
        assert_eq!(super::infer_subsystem_domain("stationThermalModel", "ode"), "thermal");
        assert_eq!(super::infer_subsystem_domain("enclosureThermal", "ode"), "thermal");
        assert_eq!(super::infer_subsystem_domain("heatSink", "ode"), "thermal");
        assert_eq!(super::infer_subsystem_domain("tempSensor", "sm"), "thermal");
    }

    #[test]
    fn test_infer_subsystem_domain_protection() {
        assert_eq!(super::infer_subsystem_domain("ProtectionRace", "sm"), "protection");
        assert_eq!(super::infer_subsystem_domain("ThermalShutdown", "sm"), "thermal"); // thermal wins over shutdown
        assert_eq!(super::infer_subsystem_domain("BreakerRelayCoordinator", "sm"), "protection");
        assert_eq!(super::infer_subsystem_domain("tripLogic", "sm"), "protection");
    }

    #[test]
    fn test_infer_subsystem_domain_electrical() {
        assert_eq!(super::infer_subsystem_domain("faultCurrentModel", "ode"), "electrical");
        assert_eq!(super::infer_subsystem_domain("currentSensor", "sm"), "electrical");
        assert_eq!(super::infer_subsystem_domain("powerMonitor", "ode"), "electrical");
        assert_eq!(super::infer_subsystem_domain("currentLimiter", "sm"), "electrical");
    }

    #[test]
    fn test_infer_subsystem_domain_signal() {
        assert_eq!(super::infer_subsystem_domain("firmwareController", "sm"), "signal");
        assert_eq!(super::infer_subsystem_domain("sensorHub", "sm"), "signal");
    }

    #[test]
    fn test_infer_subsystem_domain_hydraulic() {
        assert_eq!(super::infer_subsystem_domain("pumpController", "sm"), "hydraulic");
        assert_eq!(super::infer_subsystem_domain("pressureSensor", "ode"), "hydraulic");
        assert_eq!(super::infer_subsystem_domain("tankLevel", "ode"), "hydraulic");
    }

    #[test]
    fn test_infer_subsystem_domain_mechanical() {
        assert_eq!(super::infer_subsystem_domain("motorDriver", "sm"), "mechanical_translational");
        assert_eq!(super::infer_subsystem_domain("actuatorControl", "ode"), "mechanical_translational");
    }

    #[test]
    fn test_infer_subsystem_domain_fallback_by_kind() {
        // No domain keywords — falls back to kind-based heuristic
        assert_eq!(super::infer_subsystem_domain("myODE", "ode"), "electrical");
        assert_eq!(super::infer_subsystem_domain("someLogic", "sm"), "protection");
        assert_eq!(super::infer_subsystem_domain("unknown", "discrete"), "uncategorized");
    }

    #[test]
    fn test_humanize_module_name_with_digits() {
        assert_eq!(super::humanize_module_name("circuit1"), "Circuit 1");
        assert_eq!(super::humanize_module_name("circuit_7"), "Circuit 7");
        assert_eq!(super::humanize_module_name("zone3"), "Zone 3");
        assert_eq!(super::humanize_module_name("module10"), "Module 10");
    }

    #[test]
    fn test_humanize_module_name_no_digits() {
        assert_eq!(super::humanize_module_name("thermal"), "Thermal");
        assert_eq!(super::humanize_module_name("root"), "Root");
    }

    #[test]
    fn test_topology_single_subsystem() {
        let session = make_sim_session(simple_sm_ir(), "sm");
        let topo = session.topology();

        assert!(!topo.root_label.is_empty());
        // Single SM with no var_prefix → goes into "root" module
        assert_eq!(topo.modules.len(), 1);
        assert_eq!(topo.modules[0].id, "root");
        assert_eq!(topo.modules[0].subsystems.len(), 1);
        assert_eq!(topo.modules[0].subsystems[0].name, "sm");
        assert_eq!(topo.modules[0].subsystems[0].kind, "sm");
        assert_eq!(topo.modules[0].subsystems[0].health.status, "nominal");
    }

    #[test]
    fn test_topology_after_step_has_state() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        let _ = session.step();
        let topo = session.topology();

        assert_eq!(topo.modules[0].subsystems[0].current_state, "idle");
    }

    #[test]
    fn test_topology_multi_subsystem_with_prefixes() {
        let runner1 = StateMachineRunner::new(simple_sm_ir());
        let runner2 = StateMachineRunner::new(simple_sm_ir());
        let mut orchestrator = Orchestrator::new(Default::default());
        orchestrator.add_state_machine_prefixed("circuit1.protection", runner1, "circuit1");
        orchestrator.add_state_machine_prefixed("circuit2.protection", runner2, "circuit2");

        let session = RuntimeSession::new(
            orchestrator,
            "test".to_owned(),
            SessionKind::Orchestrator,
            None,
        );
        let topo = session.topology();

        // Two modules: circuit1 and circuit2
        assert_eq!(topo.modules.len(), 2);
        let names: Vec<&str> = topo.modules.iter().map(|m| m.id.as_str()).collect();
        assert!(names.contains(&"circuit1"));
        assert!(names.contains(&"circuit2"));

        // Each module has 1 subsystem
        for module in &topo.modules {
            assert_eq!(module.subsystems.len(), 1);
            assert!(module.subsystems[0].name.contains("protection"));
        }
    }

    #[test]
    fn test_topology_domain_summaries() {
        let runner1 = StateMachineRunner::new(simple_sm_ir());
        let runner2 = StateMachineRunner::new(simple_sm_ir());
        let mut orchestrator = Orchestrator::new(Default::default());
        orchestrator.add_state_machine("thermalShutdown", runner1);
        orchestrator.add_state_machine("protectionRace", runner2);

        let session = RuntimeSession::new(
            orchestrator,
            "test".to_owned(),
            SessionKind::Orchestrator,
            None,
        );
        let topo = session.topology();

        // Both subsystems go to "root" module (no prefix)
        assert_eq!(topo.modules.len(), 1);
        assert_eq!(topo.modules[0].subsystems.len(), 2);

        // Domain summaries should include thermal and protection
        let domains: Vec<&str> = topo.domain_summaries.iter().map(|d| d.domain.as_str()).collect();
        assert!(domains.contains(&"thermal"), "expected thermal domain, got: {:?}", domains);
        assert!(domains.contains(&"protection"), "expected protection domain, got: {:?}", domains);
    }

    #[test]
    fn test_topology_module_labels_humanized() {
        let runner = StateMachineRunner::new(simple_sm_ir());
        let mut orchestrator = Orchestrator::new(Default::default());
        orchestrator.add_state_machine_prefixed("circuit7.breaker", runner, "circuit7");

        let session = RuntimeSession::new(
            orchestrator,
            "test".to_owned(),
            SessionKind::Orchestrator,
            None,
        );
        let topo = session.topology();

        assert_eq!(topo.modules[0].label, "Circuit 7");
    }

    #[test]
    fn test_topology_serializes_to_json() {
        let session = make_sim_session(simple_sm_ir(), "sm");
        let topo = session.topology();

        let json = serde_json::to_value(&topo).expect("topology should serialize");
        assert!(json.get("root_label").is_some());
        assert!(json.get("modules").is_some());
        assert!(json.get("domain_summaries").is_some());

        let modules = json["modules"].as_array().unwrap();
        assert_eq!(modules.len(), 1);
        assert!(modules[0].get("id").is_some());
        assert!(modules[0].get("subsystems").is_some());
        assert!(modules[0].get("health").is_some());
    }

    #[test]
    fn test_topology_health_rollup() {
        // Health is always nominal since we don't have threshold-based computation yet
        let session = make_sim_session(simple_sm_ir(), "sm");
        let topo = session.topology();

        assert_eq!(topo.modules[0].health.status, "nominal");
        assert!(topo.modules[0].health.message.is_none());
        for ds in &topo.domain_summaries {
            assert_eq!(ds.status, "nominal");
        }
    }

    #[test]
    fn test_topology_sparkline_empty_without_steps() {
        let session = make_sim_session(simple_sm_ir(), "sm");
        let topo = session.topology();

        // No history → empty sparkline
        assert!(topo.modules[0].subsystems[0].sparkline.is_empty());
    }

    #[test]
    fn test_topology_element_id_from_compiled_graph() {
        use std::sync::Arc;
        use sysml_core::{Element, ElementKind, ModelGraph, Relationship, RelationshipKind};

        // Build a graph with a state machine whose ElementId we can verify.
        let mut graph = ModelGraph::new();

        let sm = Element::new_with_kind(ElementKind::StateDefinition)
            .with_name("TrafficLight");
        let sm_id = graph.add_element(sm);
        let sm_id_str = sm_id.to_string();

        let red = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Red")
            .with_owner(sm_id.clone())
            .with_prop("initial", sysml_core::Value::Bool(true));
        let red_id = graph.add_element(red);

        let green = Element::new_with_kind(ElementKind::StateUsage)
            .with_name("Green")
            .with_owner(sm_id.clone());
        let green_id = graph.add_element(green);

        // Red -> Green on "timer"
        let mut t1 = Relationship::new(
            RelationshipKind::Transition,
            red_id.clone(),
            green_id.clone(),
        );
        t1.props
            .insert("trigger".into(), sysml_core::Value::String("timer".into()));
        graph.add_relationship(t1);

        let graph_arc = Arc::new(graph);
        let base_ctx = sysml_ide_db::eval_context_seed::context_from_graph(&graph_arc);
        let precompiled = Arc::new(sysml_runtime::constraints::extract_and_precompile(&graph_arc));
        let port_flow = Arc::new(sysml_runtime::flows::build_port_flow_resources(&graph_arc));
        let gated = Arc::new(sysml_runtime::compiler::build_gated_expressions(&graph_arc));
        let snap = sysml_ide_db::Snapshot::new(graph_arc);
        let orchestrator = snap
            .build_workspace_orchestrator(
                base_ctx,
                Some(precompiled),
                Some(port_flow),
                Some(gated),
                None,
                &[],
                None,
                None,
            )
            .expect("should build orchestrator from graph");

        let session = RuntimeSession::new(
            orchestrator,
            "test-uri".to_owned(),
            SessionKind::Orchestrator,
            Some("TrafficLight".to_owned()),
        );

        let topo = session.topology();
        assert!(!topo.modules.is_empty(), "topology should have at least one module");

        // Find the subsystem compiled from TrafficLight and verify its element_id.
        let tl_sub = topo.modules.iter()
            .flat_map(|m| &m.subsystems)
            .find(|s| s.name == "TrafficLight")
            .expect("should find TrafficLight subsystem");

        assert_eq!(
            tl_sub.element_id.as_deref(),
            Some(sm_id_str.as_str()),
            "subsystem element_id should match the source StateDefinition's ElementId",
        );
    }

    // -----------------------------------------------------------------------
    // R4 — fork-at-tick: orchestrator archive + fork_child_at rewind
    // -----------------------------------------------------------------------

    /// Build a sim session and seed a per-tick context variable `v = tick`
    /// so we can later verify the child was rewound to the correct tick.
    ///
    /// The variable is written BEFORE each step so it is captured in the
    /// archived orchestrator snapshot for that tick.
    fn make_sim_session_with_tick_variable() -> RuntimeSession {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        // These fork-at-tick tests exercise the RETENTION/eviction contract,
        // not the archive-CADENCE feature (UX closeout arc #7) — force
        // every-tick archiving so the two orthogonal knobs don't interact.
        session.set_archive_cadence(1);
        for i in 1..=5u64 {
            // Write before step() so the archived orchestrator clone for
            // tick `i` carries `v == i`.
            session
                .orchestrator
                .context
                .set("v".to_owned(), sysml_core::Value::Int(i as i64));
            session.step();
        }
        session
    }

    #[test]
    fn test_fork_at_tick_archive_populated_on_step() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        // Testing retention, not cadence — see the comment on
        // `make_sim_session_with_tick_variable`.
        session.set_archive_cadence(1);
        assert_eq!(session.archived_snapshot_count(), 0);
        for _ in 0..3 {
            session.step();
        }
        assert_eq!(
            session.archived_snapshot_count(),
            3,
            "archive should contain one orchestrator clone per step"
        );
        assert_eq!(session.earliest_archived_tick(), Some(1));
        assert!(session.has_archived_tick(2));
    }

    #[test]
    fn test_fork_at_tick_happy_path_restores_variables() {
        let parent = make_sim_session_with_tick_variable();
        assert_eq!(parent.latest_tick(), 5);
        // Verify the parent's current context reflects the final tick.
        assert_eq!(
            parent.orchestrator.context.get("v"),
            Some(&sysml_core::Value::Int(5)),
        );

        // Rewind the child to tick 2 — archived snapshot's context must
        // show `v == 2`, not the parent's current `v == 5`.
        let child = parent
            .fork_child_at(2)
            .expect("fork at retained past tick must succeed");
        assert_eq!(child.fork_point_tick, Some(2));
        assert_eq!(
            child.orchestrator.context.get("v"),
            Some(&sysml_core::Value::Int(2)),
            "child's orchestrator context must be the tick-2 snapshot",
        );
        // Child begins with a clean archive of its own.
        assert_eq!(child.archived_snapshot_count(), 0);
        // Child history is reseeded with the matching execution snapshot.
        assert_eq!(child.history().len(), 1);
        assert_eq!(child.history().back().unwrap().tick, 2);
    }

    #[test]
    fn test_fork_at_tick_future_returns_future_tick_error() {
        let parent = make_sim_session_with_tick_variable();
        let current = parent.latest_tick();
        let err = parent
            .fork_child_at(current + 10)
            .expect_err("fork at a future tick must fail");
        match err {
            ForkAtTickError::FutureTick { tick, current: c } => {
                assert_eq!(tick, current + 10);
                assert_eq!(c, current);
            }
            other => panic!("expected FutureTick, got {other:?}"),
        }
    }

    #[test]
    fn test_fork_at_tick_evicted_returns_snapshot_missing() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        // Testing retention/eviction, not cadence.
        session.set_archive_cadence(1);
        // Shrink the retention window so we can force eviction cheaply.
        session.set_snapshot_retention(3);
        for _ in 0..5 {
            session.step();
        }
        // Oldest retained tick is 3 (ticks 1 and 2 evicted).
        assert_eq!(session.earliest_archived_tick(), Some(3));

        let err = session
            .fork_child_at(1)
            .expect_err("fork at evicted tick must fail");
        match err {
            ForkAtTickError::SnapshotMissing {
                tick,
                earliest_available,
                valid_ticks,
            } => {
                assert_eq!(tick, 1);
                assert_eq!(earliest_available, Some(3));
                assert_eq!(valid_ticks, vec![3, 4, 5]);
            }
            other => panic!("expected SnapshotMissing, got {other:?}"),
        }
    }

    #[test]
    fn test_fork_at_tick_retention_override_shrinks_archive() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        // Testing retention, not cadence.
        session.set_archive_cadence(1);
        for _ in 0..10 {
            session.step();
        }
        assert_eq!(session.archived_snapshot_count(), 10);
        // Shrinking retention must truncate immediately.
        session.set_snapshot_retention(3);
        assert_eq!(session.archived_snapshot_count(), 3);
        // Oldest retained tick should be 8 (ticks 1..=7 trimmed).
        assert_eq!(session.earliest_archived_tick(), Some(8));
    }

    #[test]
    fn test_fork_at_tick_error_serialization_is_structured() {
        let future = ForkAtTickError::FutureTick {
            tick: 100,
            current: 42,
        };
        let v = serde_json::to_value(&future).unwrap();
        assert_eq!(v["kind"], "FutureTick");
        assert_eq!(v["tick"], 100);
        assert_eq!(v["current"], 42);

        let missing = ForkAtTickError::SnapshotMissing {
            tick: 3,
            earliest_available: Some(50),
            valid_ticks: vec![50, 150, 250],
        };
        let v = serde_json::to_value(&missing).unwrap();
        assert_eq!(v["kind"], "SnapshotMissing");
        assert_eq!(v["tick"], 3);
        assert_eq!(v["earliest_available"], 50);
        assert_eq!(v["valid_ticks"], serde_json::json!([50, 150, 250]));
    }

    #[test]
    fn test_fork_at_tick_disabled_archive_returns_snapshot_missing() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session.set_snapshot_retention(0);
        for _ in 0..3 {
            session.step();
        }
        assert_eq!(session.archived_snapshot_count(), 0);
        // latest_tick is driven by history (not the archive), so requesting
        // a past tick must produce SnapshotMissing, not FutureTick.
        let err = session
            .fork_child_at(1)
            .expect_err("fork with disabled archive must fail");
        assert!(matches!(err, ForkAtTickError::SnapshotMissing { .. }));
    }

    #[test]
    fn test_fork_at_tick_reset_clears_archive() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        // Testing retention, not cadence.
        session.set_archive_cadence(1);
        for _ in 0..5 {
            session.step();
        }
        assert!(session.archived_snapshot_count() > 0);
        session.reset();
        assert_eq!(session.archived_snapshot_count(), 0);
    }

    // -----------------------------------------------------------------------
    // UX closeout arc #7 — archive cadence
    // -----------------------------------------------------------------------

    /// With no SM transitions, no boolean-variable flips, and no
    /// breakpoints, nothing is event-significant — only cadence ticks land
    /// in the archive.
    #[test]
    fn test_archive_cadence_only_cadence_ticks_archived() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session.set_archive_cadence(5);
        for _ in 0..12 {
            session.step();
        }
        assert_eq!(session.forkable_ticks(), vec![5, 10]);
    }

    /// Anti-pattern guard (design doc §4): the shipped default must not
    /// silently degrade to "archive every tick" — a real cadence window
    /// must elapse before anything below it is archived.
    #[test]
    fn test_archive_cadence_default_is_not_every_tick() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        assert_eq!(session.archive_cadence_ticks(), DEFAULT_ARCHIVE_CADENCE_TICKS);
        for _ in 0..(DEFAULT_ARCHIVE_CADENCE_TICKS - 1) {
            session.step();
        }
        assert_eq!(
            session.archived_snapshot_count(),
            0,
            "ticks below the cadence window must not be archived by default"
        );
    }

    /// A state-machine transition must be force-archived on its exact tick
    /// even when that tick falls nowhere near the cadence window.
    #[test]
    fn test_archive_cadence_forces_archive_on_sm_transition() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session.set_archive_cadence(100);
        session.step(); // tick 1: idle, no transition
        session.step(); // tick 2: idle, no transition
        session.inject_event("sm", "start"); // tick 3: idle -> running
        assert!(
            session.has_archived_tick(3),
            "SM-transition tick must be force-archived even off-cadence"
        );
        assert!(
            !session.has_archived_tick(1) && !session.has_archived_tick(2),
            "non-event, non-cadence ticks must NOT be archived"
        );
    }

    /// A boolean context variable flipping value — the generic "trip-latch
    /// flip" event class — must be force-archived on its exact tick.
    #[test]
    fn test_archive_cadence_forces_archive_on_bool_variable_flip() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session.set_archive_cadence(100);
        session
            .orchestrator
            .context
            .set("flag".to_owned(), sysml_core::Value::Bool(false));
        session.step(); // tick 1: flag stays false, no flip
        session
            .orchestrator
            .context
            .set("flag".to_owned(), sysml_core::Value::Bool(true));
        session.step(); // tick 2: flag flips false -> true
        assert!(
            session.has_archived_tick(2),
            "the tick a boolean context variable flips must be force-archived"
        );
        assert!(!session.has_archived_tick(1));
    }

    /// Breakpoint fire must still happen on the EXACT tick (event detection
    /// stays every-tick, unchanged by this arc) and that tick must be
    /// force-archived despite being off-cadence. Also exercises the §3
    /// ordering-trap fix: the archive decision must see the fresh
    /// (debounce-aware) match computed by `check_breakpoints` this same
    /// tick, not a stale/late one.
    #[test]
    fn test_archive_cadence_forces_archive_on_breakpoint_fire_at_exact_tick() {
        let mut session = make_sim_session(simple_sm_ir(), "sm");
        session.set_archive_cadence(100);
        session.set_breakpoint(Breakpoint::StateEntry {
            element_id: "running".to_owned(),
        });

        session.step(); // tick 1: idle
        session.step(); // tick 2: idle
        assert!(!session.is_paused());

        session.inject_event("sm", "start"); // tick 3: idle -> running, breakpoint fires
        assert!(
            session.is_paused(),
            "breakpoint must fire on the exact transition tick"
        );
        assert_eq!(session.latest_tick(), 3);
        assert!(
            session.has_archived_tick(3),
            "breakpoint-fire tick must be force-archived even off-cadence"
        );
        assert!(!session.has_archived_tick(1) && !session.has_archived_tick(2));
    }

    /// The key non-regression proof (design doc §6): forking at a
    /// coarse-archived tick must reproduce the dense-archive
    /// (retention=256, cadence=1) baseline's forward trajectory exactly.
    /// Cadence only changes WHICH ticks get archived — `fork_for_archive`
    /// is always a full `Orchestrator` deep clone, so the content archived
    /// at any given tick is identical regardless of cadence.
    #[test]
    fn test_byte_identical_continuation_coarse_vs_dense_archive() {
        let mut dense = make_sim_session(simple_sm_ir(), "sm");
        dense.set_snapshot_retention(DEFAULT_SNAPSHOT_RETENTION_TICKS);
        dense.set_archive_cadence(1);

        let mut coarse = make_sim_session(simple_sm_ir(), "sm");
        coarse.set_snapshot_retention(DEFAULT_SNAPSHOT_RETENTION_TICKS);
        coarse.set_archive_cadence(4);

        // Drive both sessions through an identical event schedule up to the
        // fork point (tick 4 — on the coarse cadence, so both sides have it).
        for tick in 1..=4u64 {
            if tick == 3 {
                dense.inject_event("sm", "start");
                coarse.inject_event("sm", "start");
            } else {
                dense.step();
                coarse.step();
            }
        }
        assert!(dense.has_archived_tick(4));
        assert!(coarse.has_archived_tick(4));

        let mut dense_child = dense
            .fork_child_at(4)
            .expect("dense fork at tick 4 must succeed");
        let mut coarse_child = coarse
            .fork_child_at(4)
            .expect("coarse fork at tick 4 must succeed");

        // Continue both children through an identical forward schedule and
        // assert the trajectories never diverge.
        for tick in 5..=9u64 {
            let d = if tick == 7 {
                dense_child.inject_event("sm", "stop")
            } else {
                dense_child.step()
            };
            let c = if tick == 7 {
                coarse_child.inject_event("sm", "stop")
            } else {
                coarse_child.step()
            };
            assert_eq!(d.tick, c.tick, "tick {tick}: tick counter diverged");
            assert_eq!(d.time_ms, c.time_ms, "tick {tick}: time_ms diverged");
            assert_eq!(
                *d.variables, *c.variables,
                "tick {tick}: context variables diverged"
            );
            assert_eq!(
                d.subsystem_states.get("sm").map(|s| &s.current_state),
                c.subsystem_states.get("sm").map(|s| &s.current_state),
                "tick {tick}: subsystem current_state diverged"
            );
            assert_eq!(
                d.subsystem_states.get("sm").map(|s| s.completed),
                c.subsystem_states.get("sm").map(|s| s.completed),
                "tick {tick}: subsystem completed diverged"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Topology types (multi-physics simulation UX)
// ---------------------------------------------------------------------------

/// Top-level topology of a running simulation, derived from the orchestrator
/// subsystem tree and (optionally) physics domain classification.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SystemTopology {
    /// Root label (e.g., "ProductionCell").
    pub root_label: String,
    /// Structural module groups (circuits, thermal network, etc.).
    pub modules: Vec<ModuleNode>,
    /// Per-domain health summaries.
    pub domain_summaries: Vec<DomainSummary>,
}

/// A structural group of subsystems (e.g., one station in the production cell).
#[derive(Debug, Clone, serde::Serialize)]
pub struct ModuleNode {
    pub id: String,
    pub label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[allow(unused)]
    pub rating: Option<String>,
    /// The `ElementId` of the owning `PartUsage` that the `var_prefix` was
    /// derived from. Used by the frontend to map overlay data to diagram
    /// nodes without a separate lookup (ADR-006).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    pub domain: String,
    pub subsystems: Vec<SubsystemNode>,
    pub health: HealthInfo,
}

/// A single subsystem within a module.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SubsystemNode {
    pub name: String,
    pub kind: String,
    pub domain: String,
    /// The `ElementId` of the source model element this subsystem was compiled
    /// from (e.g., `StateDefinition`, ODE owner). Propagated from
    /// `Subsystem::source_element_id` (ADR-006).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
    pub current_state: String,
    pub sparkline: Vec<f64>,
    pub health: HealthInfo,
}

/// Per-domain health summary.
#[derive(Debug, Clone, serde::Serialize)]
pub struct DomainSummary {
    pub domain: String,
    pub status: String,
    pub message: String,
    pub key_metrics: Vec<DomainMetric>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct DomainMetric {
    pub label: String,
    pub value: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HealthInfo {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

impl HealthInfo {
    pub fn nominal() -> Self {
        Self { status: "nominal".to_owned(), message: None }
    }
    pub fn warning(msg: impl Into<String>) -> Self {
        Self { status: "warning".to_owned(), message: Some(msg.into()) }
    }
    pub fn critical(msg: impl Into<String>) -> Self {
        Self { status: "critical".to_owned(), message: Some(msg.into()) }
    }
}

impl RuntimeSession {
    /// Build a `SystemTopology` from this session's orchestrator state.
    ///
    /// Groups subsystems by their `var_prefix` (instance scope) into modules.
    /// Subsystems without a prefix go into a "root" module.
    pub fn topology(&self) -> SystemTopology {
        use std::collections::HashMap;
        use sysml_runtime::orchestrator::ExecutionPhase;

        let latest = self.history.back();
        let subsystems = self.orchestrator.subsystems();

        // Group subsystems by var_prefix → modules
        let mut module_map: HashMap<String, Vec<SubsystemNode>> = HashMap::new();

        for sub in subsystems {
            let kind = match sub.executor.phase() {
                ExecutionPhase::ContinuousDynamics => "ode",
                ExecutionPhase::DiscreteDynamics => "discrete",
                ExecutionPhase::StateMachine => "sm",
                ExecutionPhase::Action => "action",
                ExecutionPhase::Physics => "physics",
            };

            // Infer domain from subsystem name/kind heuristics
            let domain = infer_subsystem_domain(&sub.name, kind);

            let current_state = latest
                .and_then(|snap| snap.subsystem_states.get(&sub.name))
                .map(|s| s.current_state.clone())
                .unwrap_or_default();

            // Build sparkline from history (last 20 values for numeric states)
            let sparkline: Vec<f64> = self.history.iter()
                .filter_map(|snap| {
                    snap.subsystem_states.get(&sub.name)
                        .and_then(|s| s.current_state.parse::<f64>().ok())
                })
                .rev()
                .take(20)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();

            let health = HealthInfo::nominal(); // TODO: derive from thresholds

            let node = SubsystemNode {
                name: sub.name.clone(),
                kind: kind.to_owned(),
                domain: domain.clone(),
                element_id: sub.source_element_id.as_ref().map(|id| id.to_string()),
                current_state,
                sparkline,
                health,
            };

            let module_key = sub.var_prefix.clone().unwrap_or_else(|| "root".to_owned());
            module_map.entry(module_key).or_default().push(node);
        }

        // Convert module map to sorted vec
        let mut modules: Vec<ModuleNode> = module_map
            .into_iter()
            .map(|(key, subs)| {
                // Primary domain = most common domain among subsystems
                let primary_domain = subs.iter()
                    .fold(HashMap::new(), |mut counts, s| {
                        *counts.entry(s.domain.clone()).or_insert(0usize) += 1;
                        counts
                    })
                    .into_iter()
                    .max_by_key(|(_, count)| *count)
                    .map(|(domain, _)| domain)
                    .unwrap_or_else(|| "uncategorized".to_owned());

                // Health = worst of children
                let worst_health = subs.iter()
                    .fold(HealthInfo::nominal(), |worst, s| {
                        if s.health.status == "critical" || worst.status == "critical" {
                            HealthInfo::critical(s.health.message.clone().unwrap_or_default())
                        } else if s.health.status == "warning" || worst.status == "warning" {
                            HealthInfo::warning(s.health.message.clone().unwrap_or_default())
                        } else {
                            worst
                        }
                    });

                let label = if key == "root" {
                    self.subsystem_name.clone().unwrap_or_else(|| "Root".to_owned())
                } else {
                    humanize_module_name(&key)
                };

                ModuleNode {
                    id: key,
                    label,
                    rating: None,
                    element_id: None,
                    domain: primary_domain,
                    subsystems: subs,
                    health: worst_health,
                }
            })
            .collect();
        modules.sort_by(|a, b| a.id.cmp(&b.id));

        // Build domain summaries
        let mut domain_counts: HashMap<String, (usize, usize, usize)> = HashMap::new(); // (total, warning, critical)
        for module in &modules {
            for sub in &module.subsystems {
                let entry = domain_counts.entry(sub.domain.clone()).or_default();
                entry.0 += 1;
                if sub.health.status == "warning" { entry.1 += 1; }
                if sub.health.status == "critical" { entry.2 += 1; }
            }
        }

        let domain_summaries: Vec<DomainSummary> = domain_counts
            .into_iter()
            .map(|(domain, (total, warn, crit))| {
                let status = if crit > 0 { "critical" }
                    else if warn > 0 { "warning" }
                    else { "nominal" };
                let nominal = total - warn - crit;
                let message = if warn + crit == 0 {
                    format!("{total} subsystems nominal")
                } else {
                    format!("{nominal}/{total} nominal, {warn} warning, {crit} critical")
                };
                DomainSummary {
                    domain,
                    status: status.to_owned(),
                    message,
                    key_metrics: Vec::new(),
                }
            })
            .collect();

        let root_label = self.subsystem_name.clone()
            .or_else(|| {
                // Try to extract a meaningful root name from the URI
                self.uri.rsplit('/').next()
                    .and_then(|f| f.strip_suffix(".sysml"))
                    .map(|s| s.to_owned())
            })
            .unwrap_or_else(|| "System".to_owned());

        SystemTopology {
            root_label,
            modules,
            domain_summaries,
        }
    }
}

/// Infer physics domain from subsystem name and kind using heuristics.
fn infer_subsystem_domain(name: &str, kind: &str) -> String {
    let lower = name.to_lowercase();

    // Thermal domain
    if lower.contains("thermal") || lower.contains("temp") || lower.contains("heat")
        || lower.contains("cooling") || lower.contains("enclosure")
    {
        return "thermal".to_owned();
    }

    // Protection domain (state machines that manage safety)
    if lower.contains("protection") || lower.contains("race") || lower.contains("trip")
        || lower.contains("breaker") || lower.contains("relay") || lower.contains("shutdown")
        || lower.contains("lockout")
    {
        return "protection".to_owned();
    }

    // Electrical domain
    if lower.contains("fault")
        || lower.contains("current") || lower.contains("voltage") || lower.contains("power")
        || lower.contains("electrical") || lower.contains("kcl")
    {
        return "electrical".to_owned();
    }

    // Hydraulic domain (before signal — "pressureSensor" should be hydraulic, not signal)
    if lower.contains("hydraulic") || lower.contains("pressure") || lower.contains("pump")
        || lower.contains("valve") || lower.contains("pipe") || lower.contains("tank")
    {
        return "hydraulic".to_owned();
    }

    // Mechanical domain (before signal — "motorDriver" should be mechanical, not signal)
    if lower.contains("motor") || lower.contains("actuator") || lower.contains("gear")
        || lower.contains("shaft") || lower.contains("torque") || lower.contains("velocity")
    {
        return "mechanical_translational".to_owned();
    }

    // Signal domain (sensors, firmware — checked last among specific domains)
    if lower.contains("sensor") || lower.contains("firmware") || lower.contains("command")
        || lower.contains("signal")
    {
        return "signal".to_owned();
    }

    // Default: ODE subsystems are likely physics, SM subsystems are likely protection/control
    match kind {
        "ode" => "electrical".to_owned(),
        "physics" => "electrical".to_owned(),
        "sm" | "action" => "protection".to_owned(),
        _ => "uncategorized".to_owned(),
    }
}

/// Convert a var_prefix like "circuit1" or "circuit_7" to "Circuit 1" or "Circuit 7".
fn humanize_module_name(prefix: &str) -> String {
    // Try to split at the boundary between letters and digits
    if let Some(pos) = prefix.find(|c: char| c.is_ascii_digit()) {
        let (word, num) = prefix.split_at(pos);
        let word = word.trim_end_matches('_');
        let mut capitalized = String::new();
        for (i, ch) in word.chars().enumerate() {
            if i == 0 {
                capitalized.extend(ch.to_uppercase());
            } else {
                capitalized.push(ch);
            }
        }
        format!("{} {}", capitalized, num)
    } else {
        let mut s = String::new();
        for (i, ch) in prefix.chars().enumerate() {
            if i == 0 {
                s.extend(ch.to_uppercase());
            } else {
                s.push(ch);
            }
        }
        s
    }
}
