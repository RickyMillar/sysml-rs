//! # batch — Parent-of-many session coordination (R5.0)
//!
//! A [`BatchSession`] is a **parent** that owns N independent
//! [`RuntimeSession`](crate::execution::RuntimeSession) children. It is a
//! lightweight coordination layer: each child is a regular runtime session
//! addressable by its own opaque session id, and the parent simply tracks
//! which children belong to it plus per-child metadata (ordinal index,
//! parameter overrides applied at spawn, status as it progresses, verdicts
//! as they accrue on completion).
//!
//! The batch *kind* — [`BatchKind::Sweep`], [`BatchKind::MonteCarlo`],
//! [`BatchKind::TradeStudy`] — is **opaque to the backend**. The service
//! layer never switches on it; it is carried through to the archive on
//! completion so the frontend workflow views can render the appropriate
//! UI (tornado chart for sweep, histogram for Monte Carlo, Pareto table
//! for trade study). From the backend's perspective all three flavours
//! look identical: N children, driven forward by the ordinary
//! `sysml.sessions.*` commands, rolled up into one parent status.
//!
//! ## Wire format
//!
//! All enum discriminants serialise as lowercase snake strings so the
//! frontend contract is stable:
//!
//! - `BatchKind` → `"sweep"` / `"monte_carlo"` / `"trade_study"`
//! - `BatchStatus` → `"pending"` / `"running"` / `"complete"` / `"failed"`
//! - `ChildStatus` → `"pending"` / `"running"` / `"complete"` / `"failed"`
//! - `CompareOp` → `"lt"` / `"le"` / `"gt"` / `"ge"` / `"eq"` / `"ne"`
//!
//! ## Caps
//!
//! - [`MAX_BATCHES`] — total parent batches stored in
//!   `SysmlService::batches`.
//! - [`MAX_CHILDREN_PER_BATCH`] — children per batch. Protects against
//!   runaway sweep configurations; the frontend should paginate / stream
//!   above this threshold.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use sysml_store::ArchivedVerdict;

// ---------------------------------------------------------------------------
// Limits
// ---------------------------------------------------------------------------

/// Hard cap on the total number of concurrent `BatchSession`s tracked by
/// the service. Matches the live-session bucket caps — batches are cheap
/// metadata, but each one can fan out into many live sessions.
pub const MAX_BATCHES: usize = 50;

/// Hard cap on children per batch. A sweep over 1000 points is a
/// reasonable upper bound for the interactive UX; larger studies should
/// paginate.
pub const MAX_CHILDREN_PER_BATCH: usize = 1000;

// ---------------------------------------------------------------------------
// Enums — wire-stable, lowercase-snake serialisation
// ---------------------------------------------------------------------------

/// The flavour of batch run. Opaque to the backend — used by the frontend
/// workflow views to select the appropriate config / viewer kits, and
/// carried through to [`SessionOrigin`](sysml_store::SessionOrigin) on
/// archive so archive queries can filter by workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchKind {
    /// Parameter sweep — children enumerate a structured grid of
    /// overrides.
    Sweep,
    /// Monte Carlo — children draw from a distribution per parameter.
    MonteCarlo,
    /// Trade study — children are named alternatives being compared
    /// under the same evaluation function.
    TradeStudy,
    /// Sensitivity analysis — children form a structured parameter-space
    /// sample set (Morris trajectories or Sobol A/B/C_i matrices) used
    /// to estimate elementary-effect statistics or Sobol variance
    /// indices for an output metric. Post-processing happens via
    /// `sysml.sensitivity.analyze`.
    Sensitivity,
}

impl BatchKind {
    /// Machine-readable label (same as the `serde(rename_all)` wire
    /// format).
    pub fn as_str(self) -> &'static str {
        match self {
            BatchKind::Sweep => "sweep",
            BatchKind::MonteCarlo => "monte_carlo",
            BatchKind::TradeStudy => "trade_study",
            BatchKind::Sensitivity => "sensitivity",
        }
    }

    /// Parse a lowercase-snake wire label back into the enum. Returns
    /// `None` for unknown inputs so callers can produce a clean error
    /// message.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "sweep" => Some(BatchKind::Sweep),
            "monte_carlo" => Some(BatchKind::MonteCarlo),
            "trade_study" => Some(BatchKind::TradeStudy),
            "sensitivity" => Some(BatchKind::Sensitivity),
            _ => None,
        }
    }

    /// Map a batch kind to the archive [`SessionOrigin`](sysml_store::SessionOrigin)
    /// used for children when the batch completes.
    ///
    /// The `Sensitivity` kind currently reuses `SessionOrigin::Sweep`
    /// since the underlying children are independent runs with parameter
    /// overrides, mirroring sweep semantics from an archive standpoint.
    pub fn to_origin(self) -> sysml_store::SessionOrigin {
        match self {
            BatchKind::Sweep => sysml_store::SessionOrigin::Sweep,
            BatchKind::MonteCarlo => sysml_store::SessionOrigin::MonteCarlo,
            BatchKind::TradeStudy => sysml_store::SessionOrigin::TradeStudy,
            BatchKind::Sensitivity => sysml_store::SessionOrigin::Sweep,
        }
    }
}

/// Overall status of a [`BatchSession`].
///
/// Transitions:
///
/// ```text
///   Pending ─(first child transitions → Running)──► Running { .. }
///       │                                               │
///       ▼                                               ▼
///   Failed(err)  ◄──any child Failed──►  Running { .. }
///                                             │
///                                             ▼
///                                          Complete   (all children terminal,
///                                                      no failures)
/// ```
///
/// Wire format uses `#[serde(tag = "status")]` so the object is always
/// self-describing. The `Failed` variant carries its message under
/// `error` (serde cannot inline a tuple variant's single field when
/// `tag` is set):
///
/// - `{"status": "pending"}`
/// - `{"status": "running", "running": 3, "completed": 1}`
/// - `{"status": "complete"}`
/// - `{"status": "failed", "error": "..."}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BatchStatus {
    /// No child has started yet.
    Pending,
    /// At least one child is live.
    Running {
        /// Children currently in the `Running` child status.
        running: usize,
        /// Children that have reached the `Complete` child status.
        completed: usize,
    },
    /// All children have terminated successfully.
    Complete,
    /// The batch was aborted or a child failed fatally. The string is a
    /// short human-readable reason; callers should surface it verbatim.
    #[serde(rename = "failed")]
    Failed {
        /// Message copied from the first failed child's status.
        error: String,
    },
}

/// Per-child status reported by [`ChildDescriptor::status`].
///
/// Wire format uses a `tag`/`content` split so the `Failed` variant's
/// message lives under an `error` key and the other variants serialise
/// as bare `{"status": "..."}` objects:
///
/// - `{"status": "pending"}`
/// - `{"status": "running"}`
/// - `{"status": "complete"}`
/// - `{"status": "failed", "error": "runner crashed"}`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", content = "error", rename_all = "snake_case")]
pub enum ChildStatus {
    /// Child session was created but has not been stepped.
    Pending,
    /// Child session is live — it has been stepped at least once and has
    /// not yet terminated.
    Running,
    /// Child session terminated normally (auto-complete on final tick or
    /// an explicit `sessions.stop`).
    Complete,
    /// Child failed — the runner raised an execution error or the
    /// session was force-stopped before completion. The string is a
    /// human-readable reason.
    Failed(String),
}

impl ChildStatus {
    /// Whether this status represents a terminal state (no further
    /// transitions possible).
    pub fn is_terminal(&self) -> bool {
        matches!(self, ChildStatus::Complete | ChildStatus::Failed(_))
    }
}

/// Comparison operator used by [`ParamPredicate`] in
/// [`BatchFilter::param_predicate`]. Mirrors
/// [`sysml_runtime::breakpoint::CompareOp`](sysml_runtime::breakpoint::CompareOp)
/// but with a wider set of synonymous wire spellings so callers can use
/// `"<"`, `"lt"`, or `"less_than"` interchangeably.
///
/// Wire format: lowercase two-letter label (`"lt"` / `"le"` / `"gt"` /
/// `"ge"` / `"eq"` / `"ne"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompareOp {
    /// Less than (`x < value`).
    Lt,
    /// Less than or equal (`x <= value`).
    Le,
    /// Greater than (`x > value`).
    Gt,
    /// Greater than or equal (`x >= value`).
    Ge,
    /// Equal within `f64::EPSILON` (`x == value`).
    Eq,
    /// Not equal within `f64::EPSILON` (`x != value`).
    Ne,
}

impl CompareOp {
    /// Apply the operator to a pair of f64s.
    pub fn apply(self, lhs: f64, rhs: f64) -> bool {
        match self {
            CompareOp::Lt => lhs < rhs,
            CompareOp::Le => lhs <= rhs,
            CompareOp::Gt => lhs > rhs,
            CompareOp::Ge => lhs >= rhs,
            CompareOp::Eq => (lhs - rhs).abs() < f64::EPSILON,
            CompareOp::Ne => (lhs - rhs).abs() >= f64::EPSILON,
        }
    }
}

/// Verdict discriminant for [`BatchFilter::only_verdict`].
///
/// Deliberately mirrors the lowercase strings used by
/// [`ArchivedVerdict::verdict`](sysml_store::ArchivedVerdict) so the
/// filter compares against the wire form directly — no conversion needed
/// during the scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerdictKind {
    /// Any child with at least one `"pass"` verdict matches.
    Pass,
    /// Any child with at least one `"fail"` verdict matches.
    Fail,
    /// Any child with at least one `"inconclusive"` verdict matches.
    Inconclusive,
    /// Any child with at least one `"error"` verdict matches.
    Error,
}

impl VerdictKind {
    /// The lowercase wire label (matches `ArchivedVerdict.verdict`).
    pub fn as_str(self) -> &'static str {
        match self {
            VerdictKind::Pass => "pass",
            VerdictKind::Fail => "fail",
            VerdictKind::Inconclusive => "inconclusive",
            VerdictKind::Error => "error",
        }
    }
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

/// Filter input for `sysml.batch.slice`.
///
/// Fields are additive (AND semantics). All three are optional; the empty
/// filter returns every child in the batch.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchFilter {
    /// Keep only children whose `status.status` discriminant equals this
    /// value (the `"pending"` / `"running"` / `"complete"` / `"failed"`
    /// wire label, parsed into [`ChildStatusKind`]). The `failed`
    /// variant's inner message is ignored for filtering — matching is
    /// tag-only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_status: Option<ChildStatusKind>,

    /// Keep only children that have at least one [`ArchivedVerdict`]
    /// whose `verdict` matches this kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub only_verdict: Option<VerdictKind>,

    /// Keep only children whose `params[name]` value compares true
    /// against `value` under `op`. The parameter value is coerced via
    /// [`param_value_as_f64`] — non-numeric values are skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param_predicate: Option<ParamPredicate>,
}

/// Tag-only child-status filter used by [`BatchFilter::only_status`].
///
/// Uses the same serde labels as [`ChildStatus`] but drops the `Failed`
/// inner string so the filter is trivially serialisable as a plain enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildStatusKind {
    /// Matches any child in `ChildStatus::Pending`.
    Pending,
    /// Matches any child in `ChildStatus::Running`.
    Running,
    /// Matches any child in `ChildStatus::Complete`.
    Complete,
    /// Matches any child in `ChildStatus::Failed(_)`, regardless of
    /// message.
    Failed,
}

impl ChildStatusKind {
    /// Project a full [`ChildStatus`] down to the tag-only kind used by
    /// the filter.
    pub fn from_status(status: &ChildStatus) -> Self {
        match status {
            ChildStatus::Pending => ChildStatusKind::Pending,
            ChildStatus::Running => ChildStatusKind::Running,
            ChildStatus::Complete => ChildStatusKind::Complete,
            ChildStatus::Failed(_) => ChildStatusKind::Failed,
        }
    }
}

/// Numeric predicate applied against a child's `params` map.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParamPredicate {
    /// Name of the parameter to test. Must match a key in
    /// [`ChildDescriptor::params`].
    pub param: String,
    /// Operator applied to `(child.params[param], value)`.
    pub op: CompareOp,
    /// Right-hand side of the comparison.
    pub value: f64,
}

// ---------------------------------------------------------------------------
// Batch + Child types
// ---------------------------------------------------------------------------

/// Per-child metadata held by a [`BatchSession`].
///
/// The child's *runtime state* lives in the ordinary
/// `SysmlService::sessions` map keyed by `session_id` — this descriptor
/// is lightweight tracking so the batch-status snapshot can be built
/// without touching the live sessions.
/// One requested outcome, read off a child at the end of its run.
///
/// A sweep's *inputs* are what the user varied; its *outcomes* are what the
/// user asked to measure. The inputs live in [`ChildDescriptor::params`] and
/// have always been on the wire; this is the matching half for the outputs.
///
/// Three states, deliberately distinguishable:
///
/// - `value = Some(v)` — the variable was recorded and ended at `v`.
/// - `value = None`, `error = Some(why)` — the outcome was requested but could
///   not be read (the model never produced that variable, or it produced no
///   finite sample). Consumers must render this as unavailable.
/// - the outcome key is absent entirely — it was never requested.
///
/// `value` is `None` rather than `0.0` on failure on purpose: a variable that
/// was never recorded and a variable that settled at zero are different facts,
/// and a chart that cannot tell them apart invents a data point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OutcomeReading {
    /// Final finite value of the variable, or `None` when unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<f64>,
    /// Model time (ms) the value was sampled at. `None` alongside `value`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<f64>,
    /// Unit symbol (`"K"`, `"mA"`) when the slot declared one. Absent for
    /// dimensionless or type-only quantities — not an error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
    /// Why this outcome could not be read. Mutually exclusive with `value`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// A decimated `(time_ms, value)` trace of how this outcome got to
    /// `value`, oldest first. Empty when the outcome could not be read.
    ///
    /// The final value alone cannot answer "did this run settle, or did it
    /// stop mid-transient?" — the question every reading near a model's
    /// initial condition should provoke. The shape is the cheapest honest
    /// answer, so it is captured at the same moment as the value, from the
    /// same buffer, and decimated so that keeping it costs a fraction of the
    /// snapshot history already retained.
    ///
    /// Decimated with LTTB, which preserves visual extrema — the peaks and
    /// knees a sparkline is read for — rather than sampling every Nth point
    /// and flattening them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<(f64, f64)>,
}

/// Points retained per outcome per child.
///
/// Sized for a sparkline a few dozen pixels wide, not for reanalysis: the
/// full series stays available from the live session until it stops, and the
/// snapshot history covers the tail. At ~200 points this costs a few KB per
/// child against the ~18 MB its archive record already occupies.
pub const OUTCOME_SERIES_POINTS: usize = 200;

impl OutcomeReading {
    /// A successful reading, with the trace that produced it.
    pub fn read(
        value: f64,
        time_ms: f64,
        unit: Option<String>,
        series: Vec<(f64, f64)>,
    ) -> Self {
        Self { value: Some(value), time_ms: Some(time_ms), unit, error: None, series }
    }

    /// A failed reading carrying the reason it failed.
    pub fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            value: None,
            time_ms: None,
            unit: None,
            error: Some(reason.into()),
            series: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChildDescriptor {
    /// Opaque session id of the child runtime session. Identical in
    /// format to any other session id (UUID v4); individually
    /// addressable through `sysml.sessions.*`.
    pub session_id: String,
    /// Zero-based ordinal within the batch. Stable across status
    /// snapshots so the frontend can key table rows on it.
    pub index: usize,
    /// Overrides applied when the child was spawned, as a sorted map so
    /// the wire order is deterministic regardless of the input ordering.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub params: BTreeMap<String, serde_json::Value>,
    /// Current lifecycle status.
    pub status: ChildStatus,
    /// Verdicts harvested when the child completed. Empty while the
    /// child is Pending / Running; populated atomically with the
    /// `Complete` / `Failed` transition.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verdicts: Vec<ArchivedVerdict>,
    /// Final readings for the outcomes this batch was asked to measure,
    /// keyed by variable name. Captured from the child's own time series
    /// at the moment it stops — before the live session is released —
    /// and populated with the `Complete` / `Failed` transition, exactly
    /// like `verdicts`.
    ///
    /// Empty when the batch requested no outcomes. A requested outcome
    /// that could not be read is PRESENT with an `error`, never omitted:
    /// silence would be indistinguishable from "not asked for".
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outcomes: BTreeMap<String, OutcomeReading>,
}

impl ChildDescriptor {
    /// Whether at least one verdict in this child matches the supplied
    /// [`VerdictKind`]. Used by [`BatchFilter::only_verdict`].
    pub fn has_verdict(&self, kind: VerdictKind) -> bool {
        let label = kind.as_str();
        self.verdicts.iter().any(|v| v.verdict == label)
    }

    /// Project this child's `params[name]` into an `f64` if it is
    /// numeric. Returns `None` for missing keys or non-numeric values.
    /// Used by [`ParamPredicate`] evaluation during slice filtering.
    pub fn param_f64(&self, name: &str) -> Option<f64> {
        let v = self.params.get(name)?;
        param_value_as_f64(v)
    }
}

/// Best-effort coercion of a `serde_json::Value` into `f64`.
///
/// - Number → direct cast (including u64 / i64 losing precision at the
///   top of the range, same as every other piece of JSON handling we do).
/// - String → `parse::<f64>` (permissive: numeric literals wrapped in
///   quotes still work).
/// - Bool / null / array / object → `None`.
pub fn param_value_as_f64(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Render a JSON parameter value as the string form
/// [`sysml_runtime::compiler::apply_overrides`] expects when spawning a
/// batch child.
///
/// Numbers, booleans, and strings round-trip through their native
/// textual representation; arrays / objects fall back to the full JSON
/// encoding so the downstream override applier sees a single token it
/// can parse or pass through.
pub fn param_value_to_override_string(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Null => "null".to_owned(),
        other => other.to_string(),
    }
}

/// Parent session owning N children.
///
/// The frontend addresses children through their individual
/// `session_id`s via the ordinary `sysml.sessions.*` commands — this
/// type is the coordination metadata layer on top. Mutation is serialised
/// through the `Arc<RwLock<BatchSession>>` held in
/// `SysmlService::batches`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchSession {
    /// Opaque batch identifier (UUID v4). Distinct namespace from
    /// session ids — a batch id is NEVER also a session id and vice
    /// versa.
    pub id: String,
    /// Workflow flavour.
    pub kind: BatchKind,
    /// Optional free-form display label; surfaced on the workflow view
    /// tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Wall-clock creation time (Unix milliseconds), matching the
    /// convention used by `SessionSummary::created_at_ms`.
    pub created_at_ms: u64,
    /// Ordered children (index stable, matches `ChildDescriptor::index`).
    pub children: Vec<ChildDescriptor>,
    /// Aggregated status of the batch.
    pub status: BatchStatus,
    /// Variable names this batch was asked to measure, in the order the
    /// caller listed them. Recorded on the batch (not re-sent per child)
    /// because it is a property of the study, and read at child-stop time
    /// to decide what to capture.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outcomes: Vec<String>,
}

impl BatchSession {
    /// Build a fresh batch session with N child placeholders.
    ///
    /// Each child descriptor is in [`ChildStatus::Pending`] with the
    /// supplied `session_id` (already-spawned runtime session) and the
    /// overrides that were applied. `status` is computed via
    /// [`Self::recompute_status`] so a zero-child batch reports
    /// `Pending` and a fully-populated batch is ready for per-child
    /// progression.
    pub fn new(
        kind: BatchKind,
        label: Option<String>,
        children: Vec<ChildDescriptor>,
        outcomes: Vec<String>,
    ) -> Self {
        let mut batch = Self {
            id: Self::new_id(),
            kind,
            label,
            created_at_ms: Self::now_ms(),
            children,
            status: BatchStatus::Pending,
            outcomes,
        };
        batch.recompute_status();
        batch
    }

    /// Generate a fresh opaque batch id. Distinct from
    /// `execution::new_session_id` by call site only — the wire shape
    /// (UUID v4 lowercase) is identical; the separate namespace is
    /// enforced by which map the caller looks up.
    ///
    /// canonical-key: synthetic-session-uuid — batches share the same
    /// "intentionally reparse-instable, fresh-per-invocation" rationale
    /// as sessions.
    pub fn new_id() -> String {
        uuid::Uuid::new_v4().to_string()
    }

    /// Current unix-millisecond timestamp; degrades to 0 on clock skew
    /// (matches [`RuntimeSession::new`](crate::execution::RuntimeSession::new)
    /// behaviour so batch + session timestamps cannot mysteriously
    /// disagree on systems with broken clocks).
    pub fn now_ms() -> u64 {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or_else(|e| {
                tracing::warn!(
                    ?e,
                    "system clock before UNIX_EPOCH; batch created_at_ms set to 0"
                );
                0
            })
    }

    /// Recompute `self.status` from the current set of children.
    ///
    /// This is the single source of truth for parent rollup — call it
    /// after every child state change. The logic:
    ///
    /// - If any child is `Failed`, the batch is `Failed`.
    /// - Else if any child is `Running` or the batch still has `Pending`
    ///   children while at least one is already `Complete`, the batch is
    ///   `Running { running, completed }`.
    /// - Else if *every* child is `Complete`, the batch is `Complete`.
    /// - Else (all `Pending`) the batch stays `Pending`.
    pub fn recompute_status(&mut self) {
        let mut running = 0usize;
        let mut completed = 0usize;
        let mut pending = 0usize;
        let mut first_failure: Option<String> = None;

        for child in &self.children {
            match &child.status {
                ChildStatus::Pending => pending += 1,
                ChildStatus::Running => running += 1,
                ChildStatus::Complete => completed += 1,
                ChildStatus::Failed(msg) => {
                    if first_failure.is_none() {
                        first_failure = Some(msg.clone());
                    }
                }
            }
        }

        if let Some(msg) = first_failure {
            // A single failed child poisons the batch. The frontend
            // still sees every child in `children`, so the fail-then-
            // drill workflow works exactly the same as a successful
            // batch.
            self.status = BatchStatus::Failed { error: msg };
            return;
        }

        let total = self.children.len();
        if total > 0 && completed == total {
            self.status = BatchStatus::Complete;
            return;
        }

        if running > 0 || (completed > 0 && pending > 0) {
            self.status = BatchStatus::Running { running, completed };
            return;
        }

        self.status = BatchStatus::Pending;
    }

    /// Locate the descriptor for a child by its session id. Linear scan
    /// — children are small (≤ MAX_CHILDREN_PER_BATCH) and this is
    /// called on session-termination which is already expensive.
    pub fn child_mut_by_session_id(
        &mut self,
        session_id: &str,
    ) -> Option<&mut ChildDescriptor> {
        self.children
            .iter_mut()
            .find(|c| c.session_id == session_id)
    }

    /// Apply [`BatchFilter`] to return a cloned subset of children.
    /// The resulting order matches the original `children` order
    /// (ascending by `index`), which is what the frontend table uses.
    pub fn filter_children(&self, filter: &BatchFilter) -> Vec<ChildDescriptor> {
        self.children
            .iter()
            .filter(|c| matches_filter(c, filter))
            .cloned()
            .collect()
    }
}

/// Evaluate a [`BatchFilter`] against a single child. Exposed at module
/// scope so unit tests can cover the combinators without a full batch.
pub fn matches_filter(child: &ChildDescriptor, filter: &BatchFilter) -> bool {
    if let Some(want) = filter.only_status {
        if ChildStatusKind::from_status(&child.status) != want {
            return false;
        }
    }
    if let Some(want) = filter.only_verdict {
        if !child.has_verdict(want) {
            return false;
        }
    }
    if let Some(pred) = &filter.param_predicate {
        match child.param_f64(&pred.param) {
            Some(lhs) => {
                if !pred.op.apply(lhs, pred.value) {
                    return false;
                }
            }
            None => return false,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Command I/O wrappers
// ---------------------------------------------------------------------------

/// Response of `sysml.batch.create`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchCreateResult {
    /// Opaque batch id — pass to every subsequent `sysml.batch.*` call.
    pub batch_id: String,
    /// Child session ids in ordinal order. Each id is addressable
    /// through `sysml.sessions.*` exactly like a top-level session.
    pub child_session_ids: Vec<String>,
}

/// Response of `sysml.batch.status`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchStatusResult {
    /// Full parent-session snapshot, children included.
    pub batch: BatchSession,
}

/// Response of `sysml.batch.results`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchResultsResult {
    /// Flat per-child descriptors. When `include_verdicts=false`
    /// (default) the per-descriptor `verdicts` vector is cleared to
    /// keep payloads small; the frontend rehydrates via
    /// `sysml.sessions.archive.get` if needed.
    pub children: Vec<ChildDescriptor>,
}

/// Response of `sysml.batch.slice`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchSliceResult {
    /// Children matching every filter clause. Verdicts are always
    /// included in the slice response since slicing is typically used
    /// for drill-down, not bulk transport.
    pub children: Vec<ChildDescriptor>,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn child(
        idx: usize,
        status: ChildStatus,
        params: Vec<(&str, serde_json::Value)>,
        verdicts: Vec<&str>,
    ) -> ChildDescriptor {
        let mut map = BTreeMap::new();
        for (k, v) in params {
            map.insert(k.to_owned(), v);
        }
        ChildDescriptor {
            outcomes: BTreeMap::new(),
            session_id: format!("child-{idx}"),
            index: idx,
            params: map,
            status,
            verdicts: verdicts
                .into_iter()
                .map(|v| ArchivedVerdict::trajectory(format!("Case{idx}"), v, 0, None, None))
                .collect(),
        }
    }

    // -- enum wire format ------------------------------------------------

    #[test]
    fn batch_kind_serialises_as_lowercase_snake() {
        assert_eq!(
            serde_json::to_value(BatchKind::Sweep).unwrap(),
            serde_json::json!("sweep")
        );
        assert_eq!(
            serde_json::to_value(BatchKind::MonteCarlo).unwrap(),
            serde_json::json!("monte_carlo")
        );
        assert_eq!(
            serde_json::to_value(BatchKind::TradeStudy).unwrap(),
            serde_json::json!("trade_study")
        );
        assert_eq!(
            serde_json::to_value(BatchKind::Sensitivity).unwrap(),
            serde_json::json!("sensitivity")
        );
    }

    #[test]
    fn batch_kind_round_trips_from_wire() {
        for raw in ["sweep", "monte_carlo", "trade_study", "sensitivity"] {
            let parsed = BatchKind::from_str(raw).expect("known label");
            assert_eq!(parsed.as_str(), raw);
        }
        assert!(BatchKind::from_str("Sweep").is_none()); // strict case
        assert!(BatchKind::from_str("").is_none());
    }

    #[test]
    fn child_status_running_serialises_as_tag_object() {
        let v = serde_json::to_value(ChildStatus::Running).unwrap();
        assert_eq!(v, serde_json::json!({"status": "running"}));
    }

    #[test]
    fn child_status_failed_carries_message() {
        let v = serde_json::to_value(ChildStatus::Failed("boom".into())).unwrap();
        // tag + content split puts the error message under `error`.
        assert_eq!(v.get("status").and_then(|s| s.as_str()), Some("failed"));
        assert_eq!(v.get("error").and_then(|s| s.as_str()), Some("boom"));
    }

    #[test]
    fn batch_status_running_carries_counts() {
        let v = serde_json::to_value(BatchStatus::Running {
            running: 2,
            completed: 3,
        })
        .unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "status": "running",
                "running": 2,
                "completed": 3,
            })
        );
    }

    // -- recompute_status ------------------------------------------------

    fn batch_with_children(children: Vec<ChildDescriptor>) -> BatchSession {
        BatchSession {
            outcomes: Vec::new(),
            id: "batch-1".into(),
            kind: BatchKind::Sweep,
            label: None,
            created_at_ms: 0,
            children,
            status: BatchStatus::Pending,
        }
    }

    #[test]
    fn recompute_all_pending_is_pending() {
        let mut b = batch_with_children(vec![
            child(0, ChildStatus::Pending, vec![], vec![]),
            child(1, ChildStatus::Pending, vec![], vec![]),
        ]);
        b.recompute_status();
        assert_eq!(b.status, BatchStatus::Pending);
    }

    #[test]
    fn recompute_one_running_one_complete_is_running_with_counts() {
        let mut b = batch_with_children(vec![
            child(0, ChildStatus::Running, vec![], vec![]),
            child(1, ChildStatus::Complete, vec![], vec![]),
            child(2, ChildStatus::Pending, vec![], vec![]),
        ]);
        b.recompute_status();
        assert_eq!(
            b.status,
            BatchStatus::Running {
                running: 1,
                completed: 1,
            }
        );
    }

    #[test]
    fn recompute_pending_with_one_complete_is_running() {
        // No child actually running, but mixed pending/complete is
        // still "in progress" so the parent must advertise progress.
        let mut b = batch_with_children(vec![
            child(0, ChildStatus::Pending, vec![], vec![]),
            child(1, ChildStatus::Complete, vec![], vec![]),
        ]);
        b.recompute_status();
        assert_eq!(
            b.status,
            BatchStatus::Running {
                running: 0,
                completed: 1,
            }
        );
    }

    #[test]
    fn recompute_all_complete_is_complete() {
        let mut b = batch_with_children(vec![
            child(0, ChildStatus::Complete, vec![], vec![]),
            child(1, ChildStatus::Complete, vec![], vec![]),
        ]);
        b.recompute_status();
        assert_eq!(b.status, BatchStatus::Complete);
    }

    #[test]
    fn recompute_any_failed_is_failed_with_message() {
        let mut b = batch_with_children(vec![
            child(0, ChildStatus::Complete, vec![], vec![]),
            child(1, ChildStatus::Failed("runner crashed".into()), vec![], vec![]),
            child(2, ChildStatus::Running, vec![], vec![]),
        ]);
        b.recompute_status();
        assert_eq!(
            b.status,
            BatchStatus::Failed {
                error: "runner crashed".into(),
            }
        );
    }

    #[test]
    fn recompute_empty_batch_stays_pending() {
        let mut b = batch_with_children(vec![]);
        b.recompute_status();
        // No children → no signal either way; Pending is the safe
        // identity. Anything else would claim success / failure for a
        // batch that hasn't populated yet.
        assert_eq!(b.status, BatchStatus::Pending);
    }

    // -- filter_children -------------------------------------------------

    fn three_child_batch() -> BatchSession {
        batch_with_children(vec![
            child(
                0,
                ChildStatus::Complete,
                vec![("mass", serde_json::json!(1.0))],
                vec!["pass"],
            ),
            child(
                1,
                ChildStatus::Complete,
                vec![("mass", serde_json::json!(2.0))],
                vec!["fail"],
            ),
            child(
                2,
                ChildStatus::Running,
                vec![("mass", serde_json::json!(3.0))],
                vec![],
            ),
        ])
    }

    #[test]
    fn filter_by_status_only() {
        let b = three_child_batch();
        let out = b.filter_children(&BatchFilter {
            only_status: Some(ChildStatusKind::Complete),
            ..Default::default()
        });
        let idxs: Vec<usize> = out.iter().map(|c| c.index).collect();
        assert_eq!(idxs, vec![0, 1]);
    }

    #[test]
    fn filter_by_verdict_only() {
        let b = three_child_batch();
        let out = b.filter_children(&BatchFilter {
            only_verdict: Some(VerdictKind::Fail),
            ..Default::default()
        });
        let idxs: Vec<usize> = out.iter().map(|c| c.index).collect();
        assert_eq!(idxs, vec![1]);
    }

    #[test]
    fn filter_by_param_predicate_numeric() {
        let b = three_child_batch();
        let out = b.filter_children(&BatchFilter {
            param_predicate: Some(ParamPredicate {
                param: "mass".into(),
                op: CompareOp::Ge,
                value: 2.0,
            }),
            ..Default::default()
        });
        let idxs: Vec<usize> = out.iter().map(|c| c.index).collect();
        assert_eq!(idxs, vec![1, 2]);
    }

    #[test]
    fn filter_combines_clauses_with_and() {
        let b = three_child_batch();
        let out = b.filter_children(&BatchFilter {
            only_status: Some(ChildStatusKind::Complete),
            only_verdict: Some(VerdictKind::Pass),
            param_predicate: Some(ParamPredicate {
                param: "mass".into(),
                op: CompareOp::Lt,
                value: 2.0,
            }),
        });
        let idxs: Vec<usize> = out.iter().map(|c| c.index).collect();
        assert_eq!(idxs, vec![0]);
    }

    #[test]
    fn filter_missing_param_excludes_child() {
        let b = batch_with_children(vec![child(
            0,
            ChildStatus::Complete,
            vec![],
            vec![],
        )]);
        let out = b.filter_children(&BatchFilter {
            param_predicate: Some(ParamPredicate {
                param: "missing".into(),
                op: CompareOp::Eq,
                value: 0.0,
            }),
            ..Default::default()
        });
        assert!(out.is_empty());
    }

    #[test]
    fn filter_non_numeric_param_excludes_child() {
        let b = batch_with_children(vec![child(
            0,
            ChildStatus::Complete,
            vec![("label", serde_json::json!("hello"))],
            vec![],
        )]);
        let out = b.filter_children(&BatchFilter {
            param_predicate: Some(ParamPredicate {
                param: "label".into(),
                op: CompareOp::Eq,
                value: 0.0,
            }),
            ..Default::default()
        });
        assert!(out.is_empty());
    }

    #[test]
    fn filter_param_value_as_f64_accepts_string_numbers() {
        let v = serde_json::json!("3.14");
        assert_eq!(param_value_as_f64(&v), Some(3.14));
    }

    // -- child helpers ---------------------------------------------------

    #[test]
    fn child_status_kind_round_trips() {
        for (status, kind) in [
            (ChildStatus::Pending, ChildStatusKind::Pending),
            (ChildStatus::Running, ChildStatusKind::Running),
            (ChildStatus::Complete, ChildStatusKind::Complete),
            (
                ChildStatus::Failed("any".into()),
                ChildStatusKind::Failed,
            ),
        ] {
            assert_eq!(ChildStatusKind::from_status(&status), kind);
        }
    }

    #[test]
    fn child_has_verdict_scans_all_entries() {
        let c = child(
            0,
            ChildStatus::Complete,
            vec![],
            vec!["pass", "inconclusive"],
        );
        assert!(c.has_verdict(VerdictKind::Pass));
        assert!(c.has_verdict(VerdictKind::Inconclusive));
        assert!(!c.has_verdict(VerdictKind::Fail));
        assert!(!c.has_verdict(VerdictKind::Error));
    }

    #[test]
    fn compare_op_applies_across_all_variants() {
        assert!(CompareOp::Lt.apply(1.0, 2.0));
        assert!(CompareOp::Le.apply(2.0, 2.0));
        assert!(CompareOp::Gt.apply(3.0, 2.0));
        assert!(CompareOp::Ge.apply(2.0, 2.0));
        assert!(CompareOp::Eq.apply(2.0, 2.0));
        assert!(CompareOp::Ne.apply(2.0, 3.0));
    }

    #[test]
    fn batch_kind_to_origin_maps_cleanly() {
        assert_eq!(
            BatchKind::Sweep.to_origin(),
            sysml_store::SessionOrigin::Sweep
        );
        assert_eq!(
            BatchKind::MonteCarlo.to_origin(),
            sysml_store::SessionOrigin::MonteCarlo
        );
        assert_eq!(
            BatchKind::TradeStudy.to_origin(),
            sysml_store::SessionOrigin::TradeStudy
        );
        // Sensitivity reuses Sweep origin (children are parameter-override
        // runs, not archived under a distinct workflow today).
        assert_eq!(
            BatchKind::Sensitivity.to_origin(),
            sysml_store::SessionOrigin::Sweep
        );
    }

    #[test]
    fn caps_are_nonzero_and_sensible() {
        assert!(MAX_BATCHES > 0);
        assert!(MAX_CHILDREN_PER_BATCH > 0);
        assert!(MAX_CHILDREN_PER_BATCH >= MAX_BATCHES);
    }

    #[test]
    fn child_descriptor_roundtrips_json() {
        let c = child(
            0,
            ChildStatus::Complete,
            vec![("mass", serde_json::json!(1.5))],
            vec!["pass"],
        );
        let v = serde_json::to_value(&c).unwrap();
        let back: ChildDescriptor = serde_json::from_value(v).unwrap();
        assert_eq!(back.session_id, c.session_id);
        assert_eq!(back.index, c.index);
        assert_eq!(back.params, c.params);
        assert_eq!(back.verdicts.len(), c.verdicts.len());
    }

    #[test]
    fn batch_session_roundtrips_json() {
        let b = three_child_batch();
        let v = serde_json::to_value(&b).unwrap();
        let back: BatchSession = serde_json::from_value(v).unwrap();
        assert_eq!(back.id, b.id);
        assert_eq!(back.kind, b.kind);
        assert_eq!(back.children.len(), b.children.len());
    }
}
