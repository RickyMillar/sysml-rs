//! Session archive — persistent record of completed runtime sessions.
//!
//! Where [`Store`](crate::Store) persists *model* snapshots keyed by project +
//! commit, [`SessionArchive`] persists *runtime session* outcomes keyed by
//! opaque session id. One archive entry per completed session captures the
//! metadata (workflow origin, workspace, timing), any overrides applied, the
//! verdicts that were emitted, and — optionally — the full snapshot history
//! so the UI can offer "replay this run" without keeping the live session
//! around.
//!
//! ## Shape
//!
//! The [`ArchivedSession`] schema is the wire contract Agent V (frontend)
//! consumes. All timestamps are Unix milliseconds (i64) for parity with the
//! live-session `created_at_ms` pattern. All fields serde-serialise cleanly
//! via derives — no custom impls.
//!
//! The archive avoids a direct dependency on `sysml-runtime` by taking
//! execution snapshots as opaque `serde_json::Value`s. The service layer is
//! responsible for converting `ExecutionSnapshot` → JSON at record time and
//! the frontend / playback path decodes back.
//!
//! ## In-memory impl
//!
//! [`InMemorySessionArchive`] is a bounded ring-buffer (capacity ~256) with
//! LRU eviction on insertion order. Sessions marked golden via
//! [`SessionArchive::mark_golden`] are *pinned* — they never get evicted
//! regardless of ring age. Thread-safe via a single `RwLock`.
//!
//! ## Durable storage
//!
//! A disk-backed impl (SQLite / RocksDB / Postgres) is out of scope for
//! R4.1 — the trait is designed so it can drop in later without changing
//! any of the service commands or frontend contract.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Deserializer, Serialize};
use thiserror::Error;

// ---------------------------------------------------------------------------
// Wire types — stable JSON shapes consumed by the frontend
// ---------------------------------------------------------------------------

/// Which workflow produced this archived session.
///
/// Matches the workflow routes in the frontend (`Run`, `Verify`, `Sweep`,
/// `MonteCarlo`, `TradeStudy`) so the archive-list view can render
/// origin-specific badges and default filters.
///
/// Wire format: snake_case string — `"run"`, `"verify"`, `"sweep"`,
/// `"monte_carlo"`, `"trade_study"`, `"external"`. Legacy `"compliance"`
/// archive rows deserialize as `Verify` for back-compat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionOrigin {
    /// Interactive Run workflow — `sysml.simulate.start` / `sysml.orchestrate.start`.
    Run,
    /// Verification workflow — `sysml.verify`.
    Verify,
    /// Parameter sweep.
    Sweep,
    /// Monte-Carlo workflow — `sysml.montecarlo.run`.
    MonteCarlo,
    /// Trade-study workflow — `sysml.trade_study`.
    TradeStudy,
    /// Externally produced verdicts ingested via
    /// `sysml.verify.record_external` (CI runner, pytest plugin, HIL rig).
    /// These entries are degenerate as *sessions* (`ticks: 0`, no
    /// snapshots — same shape as `sessions.verify`'s interim live-record
    /// upsert) but full citizens of the archive: they list, filter,
    /// golden-pin, and appear in `verify.timeline` like any other origin.
    External,
}

impl<'de> Deserialize<'de> for SessionOrigin {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).ok_or_else(|| {
            serde::de::Error::unknown_variant(
                &s,
                &["run", "verify", "sweep", "monte_carlo", "trade_study", "external"],
            )
        })
    }
}

impl SessionOrigin {
    /// Machine-readable label (same as serde `rename_all = "snake_case"`).
    pub fn as_str(self) -> &'static str {
        match self {
            SessionOrigin::Run => "run",
            SessionOrigin::Verify => "verify",
            SessionOrigin::Sweep => "sweep",
            SessionOrigin::MonteCarlo => "monte_carlo",
            SessionOrigin::TradeStudy => "trade_study",
            SessionOrigin::External => "external",
        }
    }

    /// Parse the wire-format string back into the enum. Returns `None` for
    /// unknown variants.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "run" => Some(SessionOrigin::Run),
            "verify" | "compliance" => Some(SessionOrigin::Verify),
            "sweep" => Some(SessionOrigin::Sweep),
            "monte_carlo" => Some(SessionOrigin::MonteCarlo),
            "trade_study" => Some(SessionOrigin::TradeStudy),
            "external" => Some(SessionOrigin::External),
            _ => None,
        }
    }
}

/// Back-reference from a verdict to the exact tick + element that produced it.
///
/// Mirrors the shape of `sysml_runtime::cases::EvidenceRef` but is redefined
/// here so `sysml-store` does not need a `sysml-runtime` dependency. The
/// service layer converts between the two.
// `Eq` is deliberately absent: `time_ms` is a float. Nothing keys a map or set
// on this type, so `PartialEq` is all it ever needed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArchivedEvidence {
    /// Runtime session identifier that produced the verdict (usually the
    /// parent `ArchivedSession.id`).
    pub session_id: String,
    /// Tick at which the verdict was evaluated.
    pub tick: u64,
    /// SIMULATED time at that tick, in milliseconds — the model's own clock,
    /// not wall clock.
    ///
    /// Recorded rather than derived. A reader can only get simulated time from
    /// a tick by multiplying by the session's `dt`, which is an inference
    /// about a run they are trying to inspect — and wrong outright for a
    /// variable-step or resumed session. `None` on records minted before this
    /// field: honest unknown, and the UI shows the tick alone rather than a
    /// computed stand-in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_ms: Option<f64>,
    /// Optional model element identifier (requirement, constraint, etc.).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
}

/// How a verdict was produced — layer 2 of the B10 evidence taxonomy
/// archived record. TOOL-POLICY vocabulary (§2.1a(d) ruling), deliberately
/// NOT spec vocabulary — which is why it lives here and not in sysml-core
/// next to `VerdictKind`/`VerificationMethodKind`.
///
/// Wire format: lowercase string — `"static"`, `"trajectory"`,
/// `"external"` — matching the existing `VerifyResult.evaluation_mode`
/// values. Deserialization mirrors [`SessionOrigin`]: an ABSENT field
/// defaults to `Trajectory` (factually correct back-compat — the sole
/// historical archive-verdict producer, `sessions.verify`, only ever wrote
/// trajectory verdicts), but an unrecognized PRESENT string hard-rejects.
/// `#[serde(default)]` must never swallow garbage as a silent fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvaluationMode {
    /// Evaluated against static overrides/defaults (`sysml.verify`,
    /// `sysml.evaluate.verification_cases`). Never archived — static
    /// verdicts are recomputed per revision; the variant exists for the
    /// ephemeral wire surfaces that share this vocabulary.
    Static,
    /// Evaluated against a live session's simulation state
    /// (`sysml.sessions.verify`).
    Trajectory,
    /// Produced outside the tool and ingested via
    /// `sysml.verify.record_external`.
    External,
}

impl<'de> Deserialize<'de> for EvaluationMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_str(&s).ok_or_else(|| {
            serde::de::Error::unknown_variant(&s, &["static", "trajectory", "external"])
        })
    }
}

impl EvaluationMode {
    /// Machine-readable label (same as serde `rename_all = "snake_case"`).
    pub fn as_str(self) -> &'static str {
        match self {
            EvaluationMode::Static => "static",
            EvaluationMode::Trajectory => "trajectory",
            EvaluationMode::External => "external",
        }
    }

    /// Parse the wire-format string back into the enum. Returns `None` for
    /// unknown variants.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "static" => Some(EvaluationMode::Static),
            "trajectory" => Some(EvaluationMode::Trajectory),
            "external" => Some(EvaluationMode::External),
            _ => None,
        }
    }
}

/// Serde back-compat default for [`ArchivedVerdict::evaluation_mode`]:
/// records predating the field were all written by `sessions.verify`
/// (trajectory) — reconstruction of true history, not fabrication.
fn default_evaluation_mode() -> EvaluationMode {
    EvaluationMode::Trajectory
}

/// Evidence behind an EXTERNALLY produced verdict (CI runner, pytest
/// plugin, HIL rig) — layer 3 of the B10 taxonomy for
/// `EvaluationMode::External` records. Computed trajectory verdicts carry
/// [`ArchivedEvidence`] instead; the two are never both populated (enforced
/// by the [`ArchivedVerdict`] smart constructors).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEvidence {
    /// Producing tool, free-form but required (e.g. `"pytest-7.4"`,
    /// `"hil-bench-2"`). Ingestion rejects an empty string.
    pub tool: String,
    /// The client's claim of WHICH MODEL the result was produced against —
    /// a `ModelGraph::content_digest()`, same identity space as session
    /// provenance and baselines. REQUIRED at ingestion (fail-hard): an
    /// external verdict that cannot say what it tested is not evidence.
    /// A mismatch against the current model is recorded and surfaced
    /// (staleness label), never rejected — the mismatch IS the signal.
    pub declared_digest: String,
    /// Opaque run reference in the tool's own namespace (CI job URL,
    /// test-run id). Optional, never fabricated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_ref: Option<String>,
    /// Artifact pointers (log/report URIs). Opaque to the tool.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifacts: Vec<String>,
    /// Resolved `ElementId` of the verification case at ingestion time.
    /// Ingestion resolves the case name against the graph to validate it;
    /// the resolved identity is captured here rather than thrown away
    /// (steward amendment — a second producer minting name-only records
    /// would compound the pre-existing name-keyed `case_id` debt).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub element_id: Option<String>,
}

/// A single verdict captured during a session's lifetime.
///
/// One entry per `case_id` × tick pair. `verdict` is the lowercase string
/// `"pass" | "fail" | "inconclusive" | "error"` matching the frontend
/// `VerdictBadge` union.
///
/// `#[non_exhaustive]`: construction outside this crate goes through the
/// smart constructors ([`ArchivedVerdict::trajectory`] /
/// [`ArchivedVerdict::external`]) so the mode↔evidence pairing invariant
/// (trajectory ⇒ session `evidence`, external ⇒ `external` payload, never
/// both) is a compile-time API shape, not a convention a future mint site
/// can silently violate.
// `Eq` dropped with `ArchivedEvidence`'s, which now carries a float
// (simulated `time_ms`). Nothing keys a map or set on a verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ArchivedVerdict {
    /// Stable identifier of the verification / analysis case.
    pub case_id: String,
    /// The 4-valued verdict serialised as a lowercase string.
    pub verdict: String,
    /// When the verdict was recorded (Unix milliseconds).
    pub timestamp: i64,
    /// How the verdict was produced (B10 layer 2). Absent on records
    /// predating the field → `Trajectory` (see `default_evaluation_mode`).
    #[serde(default = "default_evaluation_mode")]
    pub evaluation_mode: EvaluationMode,
    /// Deep-link back to the tick + element that caused this verdict.
    /// Populated on trajectory verdicts only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<ArchivedEvidence>,
    /// Evidence behind an external verdict (B10 layer 3). Populated on
    /// `EvaluationMode::External` records only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<ExternalEvidence>,
    /// Content digest of the verification case's own subtree at the moment
    /// this verdict was recorded (P6 of the test-management model study —
    /// `ModelGraph::subtree_digest` of the case element). Lets a reader flag
    /// "this case changed since this execution" by comparing against the
    /// CURRENT subtree digest (generalizes external `matches_current_model`
    /// from the whole model to one case). ABSENT on records predating the
    /// field (`#[serde(default)]` → `None`) = honest "unknown", NEVER a
    /// fabricated stale/fresh claim; and `None` at mint whenever the case's
    /// digest could not be computed (e.g. a compile-error verdict with no
    /// resolved element).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case_digest: Option<String>,
}

impl ArchivedVerdict {
    /// A verdict computed against a live session's simulation state
    /// (`sysml.sessions.verify`). `evidence` is `Option` because a case
    /// that fails to COMPILE still archives an honest error verdict with
    /// no element to point at (and pre-contract historical records also
    /// lack it) — but a trajectory record can never carry `external`.
    pub fn trajectory(
        case_id: impl Into<String>,
        verdict: impl Into<String>,
        timestamp: i64,
        evidence: Option<ArchivedEvidence>,
        case_digest: Option<String>,
    ) -> Self {
        ArchivedVerdict {
            case_id: case_id.into(),
            verdict: verdict.into(),
            timestamp,
            evaluation_mode: EvaluationMode::Trajectory,
            evidence,
            external: None,
            case_digest,
        }
    }

    /// A verdict produced outside the tool and ingested via
    /// `sysml.verify.record_external`. The external evidence payload is
    /// REQUIRED — ingestion without it is rejected upstream, and an
    /// external record can never carry session `evidence`.
    pub fn external(
        case_id: impl Into<String>,
        verdict: impl Into<String>,
        timestamp: i64,
        external: ExternalEvidence,
        case_digest: Option<String>,
    ) -> Self {
        ArchivedVerdict {
            case_id: case_id.into(),
            verdict: verdict.into(),
            timestamp,
            evaluation_mode: EvaluationMode::External,
            evidence: None,
            external: Some(external),
            case_digest,
        }
    }
}

/// Golden-session marker metadata.
///
/// Present on any entry that has been tagged as a reference run via
/// [`SessionArchive::mark_golden`]. Golden entries are pinned in the ring —
/// they never get evicted even if the cap is exceeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoldenMetadata {
    /// Human-readable label (e.g. `"v1.3 baseline"`, `"spec-passing run"`).
    pub label: String,
    /// When the session was marked golden (Unix milliseconds).
    pub marked_at: i64,
}

/// Breakdown of verdict outcomes recorded during a session.
///
/// Mirrors the 4-valued verdict union used by `VerifyResult.verdict` and the
/// frontend `VerdictBadge`. Any unknown/unmapped verdict string is silently
/// dropped rather than bucketed into `error` so we do not over-count.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerdictCounts {
    /// Number of verdicts that passed.
    pub pass: usize,
    /// Number of verdicts that failed.
    pub fail: usize,
    /// Number of verdicts that were inconclusive.
    pub inconclusive: usize,
    /// Number of verdicts that errored.
    pub error: usize,
}

impl VerdictCounts {
    /// Count a batch of verdicts by their lowercase string discriminant.
    pub fn from_verdicts(verdicts: &[ArchivedVerdict]) -> Self {
        let mut out = Self::default();
        for v in verdicts {
            match v.verdict.as_str() {
                "pass" => out.pass += 1,
                "fail" => out.fail += 1,
                "inconclusive" => out.inconclusive += 1,
                "error" => out.error += 1,
                _ => {}
            }
        }
        out
    }

    /// Total number of verdicts across all buckets.
    pub fn total(&self) -> usize {
        self.pass + self.fail + self.inconclusive + self.error
    }
}

/// Summary projection of an [`ArchivedSession`] — everything the list view
/// needs without the potentially large `snapshots` or `verdicts` payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedSessionSummary {
    /// Opaque session id (same key used by `SysmlService::sessions`).
    pub id: String,
    /// User-visible label, if the session was renamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Which workflow produced this session.
    pub origin: SessionOrigin,
    /// Workspace root URI (or file:// URI for single-file runs).
    pub workspace_uri: String,
    /// Session creation time (Unix milliseconds).
    pub created_at: i64,
    /// Session end time (Unix milliseconds).
    pub ended_at: i64,
    /// Total ticks the orchestrator advanced during this run.
    pub ticks: u64,
    /// Per-outcome breakdown of verdicts recorded on this session.
    pub verdict_counts: VerdictCounts,
    /// Number of snapshots retained (may be less than `ticks` if the session
    /// was truncated by `MAX_HISTORY`).
    pub snapshot_count: usize,
    /// Golden marker if this session was pinned as a reference run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub golden: Option<GoldenMetadata>,
}

/// A completed runtime session, retained in the archive for later inspection,
/// replay, or comparison.
///
/// This is the full payload returned by [`SessionArchive::get`]. For list
/// queries use [`ArchivedSessionSummary`] which omits `snapshots` + `overrides`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchivedSession {
    /// Opaque session id (same key used by `SysmlService::sessions`).
    pub id: String,
    /// User-visible label, if the session was renamed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Which workflow produced this session.
    pub origin: SessionOrigin,
    /// Workspace root URI (or file:// URI for single-file runs).
    pub workspace_uri: String,
    /// Session creation time (Unix milliseconds).
    pub created_at: i64,
    /// Session end time (Unix milliseconds).
    pub ended_at: i64,
    /// Total ticks the orchestrator advanced during this run.
    pub ticks: u64,
    /// Overrides applied at session start, as `(key, value_string)` pairs.
    /// Empty if no overrides were used.
    #[serde(default)]
    pub overrides: Vec<(String, String)>,
    /// Verdicts emitted during the session, in record order.
    #[serde(default)]
    pub verdicts: Vec<ArchivedVerdict>,
    /// Full execution-snapshot history (as opaque JSON values so
    /// `sysml-store` does not need a `sysml-runtime` dependency). Ordered
    /// oldest → newest. Empty for sessions where snapshot persistence was
    /// skipped (e.g. very long runs).
    #[serde(default)]
    pub snapshots: Vec<serde_json::Value>,
    /// The run's `value_units` measurement table, stored ONCE rather than
    /// once per snapshot.
    ///
    /// This table maps a variable name to its dimension and (where the slot
    /// declared one) its unit. It is fixed for the whole run — the runtime
    /// builds it at first tick and `Arc`-shares the same allocation into every
    /// `ExecutionSnapshot`. Serialising snapshots individually used to undo
    /// that sharing and write a full copy per tick, which dominated the
    /// archive's memory by roughly two orders of magnitude (see
    /// `SysmlService::archive_session_entry`). Hoisting it here keeps the
    /// information and drops the duplication.
    ///
    /// `None` for records written before this field existed, and for runs
    /// whose slots carry no measurement metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snapshot_value_units: Option<serde_json::Value>,
    /// Golden marker if this session was pinned as a reference run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub golden: Option<GoldenMetadata>,
    /// Model/run provenance captured at session creation (B6 remainder).
    /// `None` = a record predating capture — never fabricated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<crate::SessionProvenance>,
}

impl ArchivedSession {
    /// Project a full archive entry down to its list-view summary.
    pub fn to_summary(&self) -> ArchivedSessionSummary {
        ArchivedSessionSummary {
            id: self.id.clone(),
            label: self.label.clone(),
            origin: self.origin,
            workspace_uri: self.workspace_uri.clone(),
            created_at: self.created_at,
            ended_at: self.ended_at,
            ticks: self.ticks,
            verdict_counts: VerdictCounts::from_verdicts(&self.verdicts),
            snapshot_count: self.snapshots.len(),
            golden: self.golden.clone(),
        }
    }
}

/// Query filter for [`SessionArchive::list`].
///
/// All fields are additive: if multiple are set, entries must match every
/// condition. Omitting a field means "don't filter on it".
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchiveFilter {
    /// Restrict to sessions whose `workspace_uri` matches exactly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_uri: Option<String>,
    /// Restrict to sessions produced by this workflow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<SessionOrigin>,
    /// Unix-millisecond lower bound on `created_at`. Inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since: Option<i64>,
    /// If `true`, only sessions tagged golden are returned.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub only_golden: bool,
}

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Errors returned by [`SessionArchive`] operations.
#[derive(Debug, Error)]
pub enum ArchiveError {
    /// Session id not found in the archive.
    #[error("archived session not found: {0}")]
    NotFound(String),
    /// Internal lock poisoning or impl-specific failure.
    #[error("archive error: {0}")]
    Internal(String),
}

/// Persistent record of completed runtime sessions.
///
/// Implementations must be `Send + Sync` so the service can hold the archive
/// behind `Arc<dyn SessionArchive>` alongside its other shared state.
pub trait SessionArchive: Send + Sync {
    /// Insert or update an archive entry. Idempotent on `entry.id` — calling
    /// twice with the same id replaces the existing entry rather than
    /// pushing a duplicate.
    fn record(&self, entry: ArchivedSession) -> Result<(), ArchiveError>;

    /// Fetch the full archived session by id. Returns `None` if the session
    /// was never recorded or has since been evicted from the ring.
    fn get(&self, id: &str) -> Option<ArchivedSession>;

    /// List summaries matching `filter`, ordered newest-first by
    /// `created_at`.
    fn list(&self, filter: ArchiveFilter) -> Vec<ArchivedSessionSummary>;

    /// Tag an archived session as golden.
    ///
    /// Creates the [`GoldenMetadata`] with the given label + current
    /// timestamp. If the session is already golden, the label is updated
    /// and `marked_at` is refreshed. Golden sessions are never evicted from
    /// the ring.
    fn mark_golden(&self, id: &str, label: String) -> Result<(), ArchiveError>;

    /// Remove the golden tag from an archived session.
    ///
    /// The session itself remains in the archive but is no longer pinned
    /// and becomes eligible for LRU eviction.
    fn unmark_golden(&self, id: &str) -> Result<(), ArchiveError>;
}

// ---------------------------------------------------------------------------
// In-memory impl
// ---------------------------------------------------------------------------

/// Hard cap on the number of non-golden sessions retained in the in-memory
/// ring. Once this is exceeded, the oldest non-golden entry (by insertion
/// order) is evicted on the next [`InMemorySessionArchive::record`] call.
pub const MAX_ARCHIVED_SESSIONS: usize = 256;

/// Bounded in-memory [`SessionArchive`].
///
/// - Stores up to [`MAX_ARCHIVED_SESSIONS`] non-golden sessions + unlimited
///   golden sessions.
/// - Eviction is FIFO over insertion order, skipping any entries that are
///   currently golden.
/// - All operations take a single `RwLock` to keep the implementation
///   trivially correct under concurrency. Read paths (`get`, `list`) take a
///   read lock; mutators take a write lock.
pub struct InMemorySessionArchive {
    inner: RwLock<Inner>,
}

struct Inner {
    /// Ordered insertion list of session ids for LRU semantics.
    order: VecDeque<String>,
    /// Session payloads keyed by id. `Arc` so cloning on read is cheap.
    entries: HashMap<String, Arc<ArchivedSession>>,
}

impl InMemorySessionArchive {
    /// Construct an empty in-memory archive.
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner {
                order: VecDeque::new(),
                entries: HashMap::new(),
            }),
        }
    }

    /// Number of entries currently in the archive (golden + non-golden).
    pub fn len(&self) -> usize {
        self.inner
            .read()
            .map(|g| g.entries.len())
            .unwrap_or(0)
    }

    /// Returns true if the archive contains no entries.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for InMemorySessionArchive {
    fn default() -> Self {
        Self::new()
    }
}

fn lock_err<T>(err: std::sync::PoisonError<T>) -> ArchiveError {
    ArchiveError::Internal(format!("lock poisoned: {err}"))
}

/// Current unix-millisecond timestamp; falls back to 0 on clock skew.
fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

impl SessionArchive for InMemorySessionArchive {
    fn record(&self, entry: ArchivedSession) -> Result<(), ArchiveError> {
        let mut guard = self.inner.write().map_err(lock_err)?;

        let id = entry.id.clone();
        let was_present = guard.entries.contains_key(&id);

        // Idempotent replace: if the id already exists, overwrite in place
        // and keep its position in the order queue (re-recording an id does
        // not bump it to the back of the LRU window).
        guard.entries.insert(id.clone(), Arc::new(entry));
        if !was_present {
            guard.order.push_back(id);
        }

        // Evict oldest non-golden entries while we exceed the cap. Golden
        // sessions stay pinned — we skip them in the scan and keep walking.
        while guard.entries.len() > MAX_ARCHIVED_SESSIONS {
            // Find the oldest non-golden id.
            let victim_idx = guard.order.iter().position(|candidate| {
                guard
                    .entries
                    .get(candidate)
                    .map(|e| e.golden.is_none())
                    .unwrap_or(false)
            });
            match victim_idx {
                Some(idx) => {
                    let victim = guard
                        .order
                        .remove(idx)
                        .expect("victim_idx in bounds by construction");
                    guard.entries.remove(&victim);
                }
                None => {
                    // Every entry is golden — nothing to evict. We let the
                    // archive grow past the cap rather than drop golden data.
                    break;
                }
            }
        }
        Ok(())
    }

    fn get(&self, id: &str) -> Option<ArchivedSession> {
        let guard = self.inner.read().ok()?;
        guard.entries.get(id).map(|arc| (**arc).clone())
    }

    fn list(&self, filter: ArchiveFilter) -> Vec<ArchivedSessionSummary> {
        let Ok(guard) = self.inner.read() else {
            return Vec::new();
        };

        let mut out: Vec<ArchivedSessionSummary> = guard
            .entries
            .values()
            .filter(|entry| {
                if let Some(ws) = filter.workspace_uri.as_ref() {
                    if entry.workspace_uri != *ws {
                        return false;
                    }
                }
                if let Some(origin) = filter.origin {
                    if entry.origin != origin {
                        return false;
                    }
                }
                if let Some(since) = filter.since {
                    if entry.created_at < since {
                        return false;
                    }
                }
                if filter.only_golden && entry.golden.is_none() {
                    return false;
                }
                true
            })
            .map(|e| e.to_summary())
            .collect();

        // Newest first so UI list shows recent runs at the top.
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        out
    }

    fn mark_golden(&self, id: &str, label: String) -> Result<(), ArchiveError> {
        let mut guard = self.inner.write().map_err(lock_err)?;
        let Some(arc) = guard.entries.get(id) else {
            return Err(ArchiveError::NotFound(id.to_owned()));
        };
        // Clone underlying entry, mutate, reinsert. We hold an Arc of
        // immutable data so this is the cheapest correct path.
        let mut updated = (**arc).clone();
        updated.golden = Some(GoldenMetadata {
            label,
            marked_at: now_ms(),
        });
        guard.entries.insert(id.to_owned(), Arc::new(updated));
        Ok(())
    }

    fn unmark_golden(&self, id: &str) -> Result<(), ArchiveError> {
        let mut guard = self.inner.write().map_err(lock_err)?;
        let Some(arc) = guard.entries.get(id) else {
            return Err(ArchiveError::NotFound(id.to_owned()));
        };
        let mut updated = (**arc).clone();
        updated.golden = None;
        guard.entries.insert(id.to_owned(), Arc::new(updated));
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    fn sample_entry(id: &str, origin: SessionOrigin, created_at: i64) -> ArchivedSession {
        ArchivedSession {
            id: id.to_owned(),
            label: None,
            origin,
            workspace_uri: "file:///workspace".to_owned(),
            created_at,
            ended_at: created_at + 1_000,
            ticks: 42,
            overrides: Vec::new(),
            verdicts: Vec::new(),
            snapshots: Vec::new(),
            snapshot_value_units: None,
            golden: None,
            provenance: None,
        }
    }

    #[test]
    fn record_and_get_roundtrip() {
        let archive = InMemorySessionArchive::new();
        let entry = sample_entry("s-1", SessionOrigin::Run, 100);
        archive.record(entry.clone()).unwrap();
        let got = archive.get("s-1").unwrap();
        assert_eq!(got.id, "s-1");
        assert_eq!(got.origin, SessionOrigin::Run);
        assert_eq!(got.created_at, 100);
    }

    #[test]
    fn record_is_idempotent_on_id() {
        let archive = InMemorySessionArchive::new();
        let mut entry = sample_entry("s-1", SessionOrigin::Run, 100);
        archive.record(entry.clone()).unwrap();
        entry.ticks = 999;
        archive.record(entry).unwrap();
        assert_eq!(archive.len(), 1, "same id must not push a second entry");
        assert_eq!(archive.get("s-1").unwrap().ticks, 999);
    }

    #[test]
    fn get_returns_none_for_unknown_id() {
        let archive = InMemorySessionArchive::new();
        assert!(archive.get("nope").is_none());
    }

    #[test]
    fn list_newest_first() {
        let archive = InMemorySessionArchive::new();
        archive.record(sample_entry("s-1", SessionOrigin::Run, 100)).unwrap();
        archive.record(sample_entry("s-2", SessionOrigin::Run, 200)).unwrap();
        archive.record(sample_entry("s-3", SessionOrigin::Run, 150)).unwrap();
        let entries = archive.list(ArchiveFilter::default());
        assert_eq!(entries.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
                   vec!["s-2", "s-3", "s-1"]);
    }

    #[test]
    fn list_filters_by_workspace() {
        let archive = InMemorySessionArchive::new();
        let mut a = sample_entry("s-a", SessionOrigin::Run, 100);
        a.workspace_uri = "file:///a".to_owned();
        let mut b = sample_entry("s-b", SessionOrigin::Run, 200);
        b.workspace_uri = "file:///b".to_owned();
        archive.record(a).unwrap();
        archive.record(b).unwrap();
        let only_a = archive.list(ArchiveFilter {
            workspace_uri: Some("file:///a".to_owned()),
            ..Default::default()
        });
        assert_eq!(only_a.len(), 1);
        assert_eq!(only_a[0].id, "s-a");
    }

    #[test]
    fn list_filters_by_origin_and_since_combined() {
        let archive = InMemorySessionArchive::new();
        archive.record(sample_entry("s-run-old", SessionOrigin::Run, 100)).unwrap();
        archive.record(sample_entry("s-run-new", SessionOrigin::Run, 300)).unwrap();
        archive.record(sample_entry("s-verify", SessionOrigin::Verify, 300)).unwrap();
        let filtered = archive.list(ArchiveFilter {
            origin: Some(SessionOrigin::Run),
            since: Some(200),
            ..Default::default()
        });
        assert_eq!(filtered.iter().map(|e| e.id.as_str()).collect::<Vec<_>>(),
                   vec!["s-run-new"]);
    }

    #[test]
    fn list_filters_only_golden() {
        let archive = InMemorySessionArchive::new();
        archive.record(sample_entry("s-1", SessionOrigin::Run, 100)).unwrap();
        archive.record(sample_entry("s-2", SessionOrigin::Run, 200)).unwrap();
        archive.mark_golden("s-2", "ref".to_owned()).unwrap();
        let goldens = archive.list(ArchiveFilter {
            only_golden: true,
            ..Default::default()
        });
        assert_eq!(goldens.len(), 1);
        assert_eq!(goldens[0].id, "s-2");
        assert!(goldens[0].golden.is_some());
    }

    #[test]
    fn mark_golden_missing_errors() {
        let archive = InMemorySessionArchive::new();
        let err = archive.mark_golden("nope", "x".to_owned()).unwrap_err();
        assert!(matches!(err, ArchiveError::NotFound(_)));
    }

    #[test]
    fn mark_golden_then_unmark() {
        let archive = InMemorySessionArchive::new();
        archive.record(sample_entry("s-1", SessionOrigin::Run, 100)).unwrap();
        archive.mark_golden("s-1", "baseline".to_owned()).unwrap();
        assert!(archive.get("s-1").unwrap().golden.is_some());
        archive.unmark_golden("s-1").unwrap();
        assert!(archive.get("s-1").unwrap().golden.is_none());
    }

    #[test]
    fn mark_golden_updates_label() {
        let archive = InMemorySessionArchive::new();
        archive.record(sample_entry("s-1", SessionOrigin::Run, 100)).unwrap();
        archive.mark_golden("s-1", "first".to_owned()).unwrap();
        archive.mark_golden("s-1", "second".to_owned()).unwrap();
        assert_eq!(
            archive.get("s-1").unwrap().golden.unwrap().label,
            "second"
        );
    }

    #[test]
    fn ring_evicts_oldest_non_golden() {
        let archive = InMemorySessionArchive::new();
        // Fill to the cap.
        for i in 0..MAX_ARCHIVED_SESSIONS {
            archive
                .record(sample_entry(&format!("s-{i}"), SessionOrigin::Run, i as i64))
                .unwrap();
        }
        assert_eq!(archive.len(), MAX_ARCHIVED_SESSIONS);

        // One more: oldest (s-0) must be evicted.
        archive
            .record(sample_entry("s-new", SessionOrigin::Run, 1_000_000))
            .unwrap();
        assert_eq!(archive.len(), MAX_ARCHIVED_SESSIONS);
        assert!(archive.get("s-0").is_none(), "oldest non-golden must be evicted");
        assert!(archive.get("s-new").is_some());
    }

    #[test]
    fn golden_sessions_are_pinned() {
        let archive = InMemorySessionArchive::new();
        archive
            .record(sample_entry("pinned", SessionOrigin::Run, 0))
            .unwrap();
        archive.mark_golden("pinned", "keep".to_owned()).unwrap();

        // Fill + overflow. Non-golden entries get evicted, 'pinned' must
        // still be findable.
        for i in 0..MAX_ARCHIVED_SESSIONS + 10 {
            archive
                .record(sample_entry(&format!("s-{i}"), SessionOrigin::Run, (i + 1) as i64))
                .unwrap();
        }
        assert!(archive.get("pinned").is_some(), "golden must survive eviction");
    }

    #[test]
    fn summary_serde_shape_stable() {
        // Frontend Agent V reads this shape; lock it down.
        let entry = ArchivedSession {
            id: "s-1".to_owned(),
            label: Some("baseline".to_owned()),
            origin: SessionOrigin::Verify,
            workspace_uri: "file:///w".to_owned(),
            created_at: 1_700_000_000_000,
            ended_at: 1_700_000_001_000,
            ticks: 3,
            overrides: vec![("speed".to_owned(), "10".to_owned())],
            verdicts: vec![ArchivedVerdict::trajectory(
                "CaseA",
                "pass",
                1_700_000_000_500,
                Some(ArchivedEvidence {
                    time_ms: None,
                    session_id: "s-1".to_owned(),
                    tick: 2,
                    element_id: Some("Req::X".to_owned()),
                }),
                None,
            )],
            snapshots: vec![serde_json::json!({ "tick": 0 })],
            snapshot_value_units: None,
            golden: Some(GoldenMetadata {
                label: "ref".to_owned(),
                marked_at: 1_700_000_002_000,
            }),
            provenance: None,
        };
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["id"], "s-1");
        assert_eq!(json["origin"], "verify");
        assert_eq!(json["verdicts"][0]["verdict"], "pass");
        assert_eq!(json["verdicts"][0]["evidence"]["tick"], 2);
        assert_eq!(json["golden"]["label"], "ref");
    }

    #[test]
    fn summary_omits_snapshots_and_verdicts() {
        let archive = InMemorySessionArchive::new();
        let mut entry = sample_entry("s-1", SessionOrigin::Run, 100);
        entry.snapshots = vec![serde_json::json!({ "tick": 0 }); 50];
        entry.overrides = vec![("k".to_owned(), "v".to_owned()); 20];
        entry.verdicts = vec![ArchivedVerdict::trajectory("C", "pass", 0, None, None); 3];
        archive.record(entry).unwrap();
        let list = archive.list(ArchiveFilter::default());
        assert_eq!(list.len(), 1);
        let as_json = serde_json::to_value(&list[0]).unwrap();
        assert!(as_json.get("snapshots").is_none(), "summary must not serialise snapshots");
        assert!(as_json.get("verdicts").is_none(), "summary must not serialise raw verdicts");
        assert!(as_json.get("overrides").is_none(), "summary must not serialise overrides");
        assert_eq!(as_json["snapshot_count"], 50);
        assert_eq!(as_json["verdict_counts"]["pass"], 3);
        assert_eq!(as_json["verdict_counts"]["fail"], 0);
    }

    #[test]
    fn verdict_counts_aggregation() {
        let verdicts = vec![
            ArchivedVerdict::trajectory("a", "pass", 0, None, None),
            ArchivedVerdict::trajectory("b", "pass", 0, None, None),
            ArchivedVerdict::trajectory("c", "fail", 0, None, None),
            ArchivedVerdict::trajectory("d", "inconclusive", 0, None, None),
            ArchivedVerdict::trajectory("e", "error", 0, None, None),
            ArchivedVerdict::trajectory("f", "unknown", 0, None, None),
        ];
        let counts = VerdictCounts::from_verdicts(&verdicts);
        assert_eq!(counts.pass, 2);
        assert_eq!(counts.fail, 1);
        assert_eq!(counts.inconclusive, 1);
        assert_eq!(counts.error, 1);
        // Unknown verdict discriminants are silently dropped.
        assert_eq!(counts.total(), 5);
    }

    #[test]
    fn origin_snake_case_wire_format() {
        // Multi-word variants use snake_case on the wire.
        assert_eq!(serde_json::to_value(SessionOrigin::MonteCarlo).unwrap(), "monte_carlo");
        assert_eq!(serde_json::to_value(SessionOrigin::TradeStudy).unwrap(), "trade_study");
        // Single-word variants unchanged.
        assert_eq!(serde_json::to_value(SessionOrigin::Run).unwrap(), "run");
        assert_eq!(serde_json::to_value(SessionOrigin::Verify).unwrap(), "verify");
        // Round-trip.
        let parsed: SessionOrigin = serde_json::from_str("\"monte_carlo\"").unwrap();
        assert_eq!(parsed, SessionOrigin::MonteCarlo);
        // Legacy compliance archive rows fold into Verify.
        let legacy: SessionOrigin = serde_json::from_str("\"compliance\"").unwrap();
        assert_eq!(legacy, SessionOrigin::Verify);
        // from_str helper mirrors serde.
        assert_eq!(SessionOrigin::from_str("trade_study"), Some(SessionOrigin::TradeStudy));
        assert_eq!(SessionOrigin::from_str("compliance"), Some(SessionOrigin::Verify));
        assert_eq!(SessionOrigin::from_str("unknown"), None);
    }

    // -- B10 evidence taxonomy (verification-evidence-taxonomy.md) --------

    #[test]
    fn origin_external_wire_format() {
        assert_eq!(serde_json::to_value(SessionOrigin::External).unwrap(), "external");
        let parsed: SessionOrigin = serde_json::from_str("\"external\"").unwrap();
        assert_eq!(parsed, SessionOrigin::External);
        assert_eq!(SessionOrigin::from_str("external"), Some(SessionOrigin::External));
    }

    #[test]
    fn verdict_evaluation_mode_back_compat_absent_field_is_trajectory() {
        // Every archived verdict predating the field was written by the
        // sole historical producer (`sessions.verify`) — trajectory. The
        // default reconstructs true history, never fabricates.
        let old_json = r#"{
            "case_id": "CaseA",
            "verdict": "pass",
            "timestamp": 1700000000500,
            "evidence": { "session_id": "s-1", "tick": 2 }
        }"#;
        let v: ArchivedVerdict = serde_json::from_str(old_json).unwrap();
        assert_eq!(v.evaluation_mode, EvaluationMode::Trajectory);
        assert!(v.external.is_none());
        assert_eq!(v.evidence.as_ref().unwrap().tick, 2);
    }

    #[test]
    fn verdict_evaluation_mode_rejects_unknown_string() {
        // `#[serde(default)]` covers only an ABSENT field; a garbage
        // PRESENT string must hard-reject, never silently fall back.
        let bad_json = r#"{
            "case_id": "CaseA",
            "verdict": "pass",
            "timestamp": 0,
            "evaluation_mode": "vibes"
        }"#;
        let err = serde_json::from_str::<ArchivedVerdict>(bad_json).unwrap_err();
        assert!(err.to_string().contains("vibes"), "error names the bad variant: {err}");
    }

    #[test]
    fn smart_constructors_enforce_mode_evidence_pairing() {
        // Trajectory: session evidence slot only, never the external one.
        let t = ArchivedVerdict::trajectory(
            "CaseA",
            "pass",
            10,
            Some(ArchivedEvidence { session_id: "s-1".into(), tick: 3, time_ms: None, element_id: None }),
            Some("case-subtree-digest-a".to_owned()),
        );
        assert_eq!(t.evaluation_mode, EvaluationMode::Trajectory);
        assert!(t.evidence.is_some());
        assert!(t.external.is_none());
        assert_eq!(t.case_digest.as_deref(), Some("case-subtree-digest-a"));

        // External: external payload required, session evidence never set.
        let e = ArchivedVerdict::external(
            "CaseB",
            "fail",
            11,
            ExternalEvidence {
                tool: "pytest-7.4".into(),
                declared_digest: "digest-x".into(),
                run_ref: Some("ci://job/42".into()),
                artifacts: vec!["file:///report.xml".into()],
                element_id: Some("elem-1".into()),
            },
            Some("case-subtree-digest-b".to_owned()),
        );
        assert_eq!(e.evaluation_mode, EvaluationMode::External);
        assert!(e.evidence.is_none());
        assert_eq!(e.case_digest.as_deref(), Some("case-subtree-digest-b"));
        let ext = e.external.as_ref().unwrap();
        assert_eq!(ext.tool, "pytest-7.4");
        assert_eq!(ext.declared_digest, "digest-x");
    }

    #[test]
    fn external_verdict_serde_round_trip() {
        let e = ArchivedVerdict::external(
            "CaseB",
            "fail",
            11,
            ExternalEvidence {
                tool: "hil-bench-2".into(),
                declared_digest: "digest-y".into(),
                run_ref: None,
                artifacts: Vec::new(),
                element_id: None,
            },
            Some("case-digest-z".to_owned()),
        );
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["evaluation_mode"], "external");
        assert_eq!(json["external"]["tool"], "hil-bench-2");
        assert_eq!(json["external"]["declared_digest"], "digest-y");
        assert_eq!(json["case_digest"], "case-digest-z");
        // Optional externals are omitted, not nulled.
        assert!(json["external"].get("run_ref").is_none());
        assert!(json["external"].get("artifacts").is_none());
        assert!(json.get("evidence").is_none());
        let back: ArchivedVerdict = serde_json::from_value(json).unwrap();
        assert_eq!(back, e);
    }

    /// Serde back-compat pin (P6): an archived verdict JSON predating the
    /// `case_digest` field deserializes with `case_digest: None` — an
    /// honest "unknown", never a fabricated stale/fresh claim. `None` is
    /// omitted (not nulled) on re-serialization.
    #[test]
    fn verdict_case_digest_back_compat_absent_field_is_none() {
        let old_json = r#"{
            "case_id": "CaseA",
            "verdict": "pass",
            "timestamp": 1700000000500,
            "evaluation_mode": "trajectory",
            "evidence": { "session_id": "s-1", "tick": 2 }
        }"#;
        let v: ArchivedVerdict = serde_json::from_str(old_json).unwrap();
        assert!(v.case_digest.is_none(), "absent case_digest = honest unknown");
        // A present digest round-trips and is omitted when None.
        let with = ArchivedVerdict::trajectory("CaseA", "pass", 0, None, Some("d".to_owned()));
        let back: ArchivedVerdict =
            serde_json::from_value(serde_json::to_value(&with).unwrap()).unwrap();
        assert_eq!(back.case_digest.as_deref(), Some("d"));
        let none = ArchivedVerdict::trajectory("CaseA", "pass", 0, None, None);
        assert!(serde_json::to_value(&none).unwrap().get("case_digest").is_none());
    }
}
