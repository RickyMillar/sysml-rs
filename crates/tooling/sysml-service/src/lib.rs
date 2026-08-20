//! # sysml-service — Unified Service Layer
//!
//! Consolidates all SysML v2 domain state and operations into one crate.
//! Transports (CLI, LSP, REST, MCP) become thin protocol adapters.
//!
//! ## Construction
//!
//! ```no_run
//! use sysml_service::SysmlService;
//! use std::path::Path;
//!
//! // Single-file mode (CLI)
//! let service = SysmlService::from_file(Path::new("model.sysml")).unwrap();
//!
//! // Workspace mode (LSP, MCP)
//! let service = SysmlService::from_workspace(Path::new("./my-project")).unwrap();
//!
//! // Empty (REST, testing)
//! let service = SysmlService::empty();
//! ```

pub mod aggregation;
pub mod batch;
pub mod bounds;
pub mod code_actions;
pub mod command_meta;
pub mod command_trait;
pub mod completion;
pub mod constraint_monitor;
pub mod diagnostics;
pub mod diagram;
pub mod diagram_edit;
pub mod diagram_manager;
pub mod error;
pub mod evaluation;
pub mod execution;
pub mod executions;
pub mod expression_ast;
pub mod field_edit;
pub mod formatting;
pub mod fs;
mod git_provenance;
pub mod goto_definition;
pub mod hover;
pub mod inspect;
pub mod library_cache;
pub mod model_tree_query;
pub mod open_context;
pub mod project_discovery;
pub mod outline;
pub mod position;
pub mod progress;
pub mod project_registry;
pub mod query;
pub mod workflow;
pub mod readiness;
pub mod references;
pub mod rename;
pub mod scenario;
pub mod scope;
pub mod sensitivity;
pub mod session_events;
pub mod session_reaper;
pub mod storage;
pub mod text_edit;
pub mod types;
pub mod verify_timeline;
pub mod visualization;
pub mod whatif;
pub mod workspace_verify;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use dashmap::DashMap;
use sysml_id::{CommitId, ProjectId};
use sysml_ide_db::{Analysis, AnalysisHost, ProjectFileSet, SourceFile};
use sysml_ide_db::{file_source_query, workspace_capabilities as ws_caps};
// Re-export `ElementId` from sysml-id under the same name as the
// `crate::types::*` glob. Using `pub use` (rather than a private `use`)
// avoids the hidden_glob_reexports lint and keeps `sysml_service::ElementId`
// addressable both inside this module and from downstream crates after the
// S1.T7 session-key migration.
pub use sysml_id::ElementId;
use sysml_runtime::expressions::EvalContext;
// Wire-facing archive types (`ArchiveFilter`, `ArchivedSession`,
// `SessionOrigin`, …) are re-exported through `crate::types::*` below to
// keep one canonical import path for consumers; we path-qualify them via
// `sysml_store::` inside this file to avoid shadowing that public
// re-export. Only archive helpers that are *not* re-exported through
// `types.rs` (the constructor + trait) are pulled into the local scope.
use sysml_store::{InMemorySessionArchive, SessionArchive, SnapshotMeta};

pub use crate::error::ServiceError;
pub use crate::scope::{GraphScope, WORKSPACE_URI};
pub use crate::types::*;

// Re-export macro and inventory types for downstream consumers.
pub use sysml_service_macros::{service_command, service_impl};
pub use command_trait::{
    CommandRegistration, ServiceCommand, execute_command, registered_command_metas,
    registered_commands,
};

/// Normalize a workspace file URI to a provenance-manifest path: relative
/// to `root` when the file lives under it (so no absolute path leaks into
/// downloaded reports or blessed baselines), else the canonical URI
/// verbatim (honest fallback for a file outside the root, or a capture
/// with no known root). Uses the codebase's `strip_prefix("file://")`
/// idiom rather than pulling in a `url` dependency — a spelling mismatch
/// simply falls through to the canonical-URI branch. See
/// [`SysmlService::workspace_file_manifest`].
fn provenance_relative_path(uri: &str, root: Option<&std::path::Path>) -> String {
    if let Some(root) = root {
        let fs_path = std::path::Path::new(uri.strip_prefix("file://").unwrap_or(uri));
        if let Ok(rel) = fs_path.strip_prefix(root) {
            return rel.to_string_lossy().into_owned();
        }
    }
    sysml_ide_db::canonical_uri(uri)
}

fn source_span_matches_uri(span: Option<&sysml_span::Span>, uri: &str) -> bool {
    let file_basename = std::path::Path::new(uri)
        .file_name()
        .and_then(|f| f.to_str())
        .unwrap_or(uri);
    span.map(|sp| sp.file == file_basename || sp.file.ends_with(file_basename))
        .unwrap_or(false)
}

fn view_summaries_from_query_rows(
    summaries: Vec<sysml_query::ElementSummary>,
) -> Vec<sysml_core::ViewSummary> {
    summaries
        .into_iter()
        .filter_map(|summary| match summary.expansion {
            Some(sysml_query::SummaryExpansion::View(view)) => Some(view),
            None => None,
        })
        .collect()
}

/// Convert an `ExecutionSnapshot` to a `StepResult` for SM backward compatibility.
fn snapshot_to_step_result(
    snapshot: &sysml_runtime::orchestrator::ExecutionSnapshot,
    sm_name: &str,
) -> sysml_runtime::StepResult {
    if let Some(state) = snapshot.subsystem_states.get(sm_name) {
        sysml_runtime::StepResult {
            state: state.current_state.clone(),
            outputs: state.outputs.clone(),
            sends: state.sends.clone(),
            port_sends: Vec::new(),
            completed: state.completed,
            available_transitions: state.available_transitions.clone(),
            incoming_trigger: state.incoming_transition_trigger.clone(),
        }
    } else {
        // Fallback: take first subsystem state or default
        let first = snapshot.subsystem_states.values().next();
        sysml_runtime::StepResult {
            state: first.map(|s| s.current_state.clone()).unwrap_or_default(),
            outputs: first.map(|s| s.outputs.clone()).unwrap_or_default(),
            sends: first.map(|s| s.sends.clone()).unwrap_or_default(),
            port_sends: Vec::new(),
            completed: snapshot.completed,
            available_transitions: first.map(|s| s.available_transitions.clone()).unwrap_or_default(),
            incoming_trigger: first.and_then(|s| s.incoming_transition_trigger.clone()),
        }
    }
}

/// Convert an `ExecutionSnapshot` to an `ActionTraceEntry` for action backward compatibility.
fn snapshot_to_action_trace(
    snapshot: &sysml_runtime::orchestrator::ExecutionSnapshot,
    action_name: &str,
) -> execution::ActionTraceEntry {
    let state = snapshot.subsystem_states.get(action_name);
    execution::ActionTraceEntry {
        node_id: state.map(|s| s.current_state.clone()).unwrap_or_default(),
        completed: snapshot.completed,
        outputs: state.map(|s| s.outputs.clone()).unwrap_or_default(),
        flow_events: Vec::new(),
        diagnostics: Vec::new(),
    }
}

/// Extract numeric/string values from a snapshot into a JSON map.
fn snapshot_values_json(snapshot: &sysml_runtime::orchestrator::ExecutionSnapshot) -> serde_json::Map<String, serde_json::Value> {
    let mut vals = serde_json::Map::new();
    for (k, v) in snapshot.variables.iter() {
        match v {
            sysml_core::Value::Float(f) => { vals.insert(k.clone(), serde_json::json!(f)); }
            sysml_core::Value::Int(i) => { vals.insert(k.clone(), serde_json::json!(i)); }
            sysml_core::Value::Bool(b) => { vals.insert(k.clone(), serde_json::json!(b)); }
            sysml_core::Value::String(s) => { vals.insert(k.clone(), serde_json::json!(s)); }
            // RSC-5.4 (D-5.0.7 / B4): a Quantity's magnitude is its scalar value.
            // Previously dropped via `_ => {}`, which hid every unit-bearing slot
            // from the FE; the unit/dimension surface beside it (see
            // `snapshot_unit_dimension_json`).
            sysml_core::Value::Quantity { value, .. } => { vals.insert(k.clone(), serde_json::json!(value)); }
            _ => {}
        }
    }
    vals
}

/// RSC-5.4 (D-5.0.7): unit + dimension JSON maps for a snapshot's variables,
/// sourced from the slot-derived `value_units` and keyed to match the `values`
/// map. Dimension is present for every ISQ-typed slot; unit only where the
/// slot's m_ref carries one (explicit `[unit]`). Both empty (and omitted from
/// the wire) when no slot carries an m_ref — byte-identical for unit-free sims.
fn snapshot_unit_dimension_json(
    snapshot: &sysml_runtime::orchestrator::ExecutionSnapshot,
) -> (
    serde_json::Map<String, serde_json::Value>,
    serde_json::Map<String, serde_json::Value>,
) {
    let mut units = serde_json::Map::new();
    let mut dims = serde_json::Map::new();
    if snapshot.value_units.is_empty() {
        return (units, dims);
    }
    for k in snapshot.variables.keys() {
        if let Some(vm) = snapshot.value_units.get(k) {
            dims.insert(k.clone(), serde_json::json!(vm.dimension.to_string()));
            if let Some(u) = &vm.unit {
                units.insert(k.clone(), serde_json::json!(u));
            }
        }
    }
    (units, dims)
}

/// Build a simulation result JSON object from a snapshot.
fn snapshot_sim_json(
    snapshot: &sysml_runtime::orchestrator::ExecutionSnapshot,
    sm_name: &str,
) -> serde_json::Value {
    let final_state = snapshot.subsystem_states.get(sm_name)
        .map(|s| s.current_state.clone())
        .unwrap_or_else(|| "unknown".to_owned());
    let (units, dimensions) = snapshot_unit_dimension_json(snapshot);
    let mut obj = serde_json::Map::new();
    obj.insert("time_ms".to_owned(), serde_json::json!(snapshot.time_ms));
    obj.insert("completed".to_owned(), serde_json::json!(snapshot.completed));
    obj.insert("final_state".to_owned(), serde_json::json!(final_state));
    obj.insert(
        "values".to_owned(),
        serde_json::Value::Object(snapshot_values_json(snapshot)),
    );
    // RSC-5.4 (D-5.0.7): unit/dimension beside the magnitude, omitted when empty
    // (no m_ref-bearing slots) so unit-free simulation results stay byte-identical.
    if !units.is_empty() {
        obj.insert("units".to_owned(), serde_json::Value::Object(units));
    }
    if !dimensions.is_empty() {
        obj.insert("dimensions".to_owned(), serde_json::Value::Object(dimensions));
    }
    serde_json::Value::Object(obj)
}

/// Map a live `SessionKind` to the archive's `SessionOrigin` on stop.
///
/// Live sessions carry the three runtime kinds (`Simulation`, `Action`,
/// `Orchestrator`); the archive's origin is workflow-facing
/// (`Run` / `Verify` / `Sweep` / `MonteCarlo` / `TradeStudy`).
/// `sessions_stop` is reached by every *.start kind plus
/// workflow-specific codepaths that may re-assign origin after the fact
/// via a richer API. Until those exist, Simulation / Action / Orchestrator
/// all fold into `SessionOrigin::Run` — the interactive Run workflow.
fn session_kind_origin(_kind: execution::SessionKind) -> SessionOrigin {
    SessionOrigin::Run
}

/// Parse the string form of a `SessionOrigin` (wire: snake_case) into the
/// typed enum. Produces a `ServiceError::InvalidInput` on unknown variants
/// so MCP / REST callers get a clear error rather than a deserialise
/// failure at the inventory dispatch layer.
fn parse_session_origin(s: &str) -> Result<SessionOrigin, ServiceError> {
    SessionOrigin::from_str(s).ok_or_else(|| {
        ServiceError::InvalidInput(format!(
            "unknown session origin '{s}' (expected one of: run, verify, sweep, monte_carlo, trade_study)"
        ))
    })
}

/// Lift a [`sysml_store::ArchiveError`] into a [`ServiceError`].
fn archive_err_to_service(e: sysml_store::ArchiveError) -> ServiceError {
    match e {
        sysml_store::ArchiveError::NotFound(id) => {
            ServiceError::ElementNotFound(format!("no archived session: {id}"))
        }
        sysml_store::ArchiveError::Internal(msg) => ServiceError::Internal(msg),
    }
}

/// Pull a scalar output metric from a batch child for sensitivity
/// analysis (R7.4).
///
/// Resolution order (falls through to the next source if the current
/// one doesn't carry the key as a real number):
///
/// 1. Verdict-specific signals: for the special metric names
///    `"fail_count"` and `"pass_count"` we count matching verdicts on
///    the child. `"verdict_numeric"` maps `pass → 1.0`, `fail → 0.0`,
///    `inconclusive/error → NaN` for the first verdict. These cover
///    the common "did the batch child pass / how many constraints
///    failed" cases without needing the model to bake a KPI.
/// 2. `child.params[metric]` — lets the caller thread a pre-computed
///    KPI back through overrides when the model doesn't produce it as
///    a verdict. This is also the slot the frontend uses for
///    Ishigami-style pure-math test harnesses where the "output" is
///    just a function of the inputs.
///
/// Returns `f64::NAN` when neither source carries the key as a real
/// number. The Morris/Sobol index helpers treat NaN outputs as
/// "missing sample" and skip the corresponding elementary effect /
/// contribution pair.
/// Match a focus filename against a fully-qualified URI for `inspect`'s
/// workspace mode. Accepts either the full URI ending in `focus` (path
/// suffix match) or the URI's file_name component being exactly equal to
/// `focus`.
fn uri_matches_focus(uri: &str, focus: &str) -> bool {
    if uri.ends_with(focus) {
        return true;
    }
    std::path::Path::new(uri)
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == focus)
}

fn extract_child_metric(
    child: &crate::batch::ChildDescriptor,
    metric: &str,
) -> f64 {
    // 1. Verdict-derived pseudo-metrics.
    match metric {
        "fail_count" => {
            return child.verdicts.iter().filter(|v| v.verdict == "fail").count() as f64;
        }
        "pass_count" => {
            return child.verdicts.iter().filter(|v| v.verdict == "pass").count() as f64;
        }
        "verdict_numeric" => {
            if let Some(v) = child.verdicts.first() {
                return match v.verdict.as_str() {
                    "pass" => 1.0,
                    "fail" => 0.0,
                    _ => f64::NAN,
                };
            }
            return f64::NAN;
        }
        _ => {}
    }
    // 2. Echoed params map — the common slot for pre-computed KPIs and
    // the one the pure-math Ishigami harness uses.
    if let Some(v) = child.params.get(metric) {
        if let Some(n) = crate::batch::param_value_as_f64(v) {
            return n;
        }
    }
    f64::NAN
}

/// Central service owning all domain state.
///
/// Thread-safe: all interior state is behind `Arc` / concurrent maps.
/// Use [`snapshot()`](SysmlService::snapshot) for immutable read views.
pub struct SysmlService {
    /// Optional store backend for persistence (RwLock for interior mutability).
    store: Option<Arc<RwLock<dyn sysml_store::Store + Send + Sync>>>,

    /// Workflow sidecar (append-only review/process event log). One
    /// instance per process, set at construction like `archive`; the
    /// shell binary supplies a durable backend, tests default in-memory.
    workflow_store: Arc<dyn sysml_store::WorkflowStore>,

    /// Workspace root (if constructed via `from_workspace`).
    workspace_root: Option<PathBuf>,

    /// Active runtime sessions (unified: simulations, actions, orchestrators).
    ///
    /// Keyed by [`ElementId`] (S1.T7) — formerly an opaque `String`.
    /// `ElementId` serializes transparently as a UUID-shaped string, so
    /// the externally visible session-id wire format is unchanged.
    sessions: DashMap<ElementId, execution::RuntimeSession>,

    /// Per-kind live-session counts, indexed by [`execution::SessionKind::index`].
    /// Maintained by [`insert_session`], [`remove_session`] and
    /// [`retain_sessions`]; `session_count(kind)` reads this array instead
    /// of scanning the sessions map on every quota check.
    session_counts: [AtomicUsize; execution::SessionKind::VARIANT_COUNT],

    /// Loaded project manifests for source file I/O.
    project_registry: std::sync::RwLock<project_registry::ProjectRegistry>,

    /// Open diagram state: view types, expanded nodes, cached graphs.
    diagram_manager: diagram_manager::DiagramManager,

    /// Cached external/library source text for hover docs.
    hover_source_cache: std::sync::RwLock<std::collections::HashMap<String, Arc<String>>>,

    /// Archive of completed runtime sessions (R4.1). Populated by
    /// `sessions_stop` and queryable via the `sysml.sessions.archive.*`
    /// command family. Defaults to [`InMemorySessionArchive`]; callers can
    /// supply an alternative implementation (e.g. disk-backed) via
    /// [`SysmlService::with_archive`].
    archive: Arc<dyn SessionArchive>,

    /// Live batch sessions (R5.0). A [`batch::BatchSession`] is a parent
    /// that owns N runtime sessions as independent children; each child
    /// remains individually addressable via `sysml.sessions.*`. Keyed by
    /// the batch's opaque id (a UUID v4 distinct from any session id).
    batches: DashMap<String, Arc<RwLock<batch::BatchSession>>>,

    /// Process-local result cache for the unified `sysml.query` primitive.
    /// Keyed by `(uri, QuerySpec JSON, graph revision)`; graph changes mint a
    /// new revision and naturally miss stale entries.
    query_cache: DashMap<String, sysml_query::QueryResult>,

    /// Per-revision memo of the verification-case verdict map feeding
    /// `sysml.workspace.requirement_rows` (B2). Evaluating verification
    /// cases compiles-and-runs a `VerificationRunner` per case — paging
    /// through the requirement table must not re-run verification per page.
    /// `evaluate_verification_cases(graph)` is a pure function of the graph,
    /// so revision keying is sound.
    requirement_verdict_cache:
        DashMap<u64, Arc<HashMap<ElementId, sysml_query::RequirementVerificationState>>>,

    /// Salsa-backed incremental database (S2.T1 / S2.T2, ADR-010).
    ///
    /// Wraps [`AnalysisHost`] in `Arc<std::sync::Mutex>`. Salsa's internal
    /// storage is `!Sync`, so a Mutex is required regardless. We use the
    /// std (sync) Mutex rather than tokio's because all `SysmlService`
    /// commands are sync — switching to tokio's Mutex would force
    /// `.await` plumbing through every read site for no benefit. Lock
    /// acquisitions are brief: clone the DB Arc inside `analysis()`, or
    /// mutate inputs, then drop the guard immediately. The LSP server
    /// uses tokio's Mutex separately for its own AnalysisHost — this is
    /// the service-owned host, accessed only from sync service methods.
    host: Arc<std::sync::Mutex<AnalysisHost>>,

    /// Broadcast channel for lifecycle events (P-RA4). Capacity 256;
    /// lagged subscribers drop messages, never block the publisher. No
    /// subscribers is normal (CLI/MCP synchronous flows) — `send`
    /// returning `Err` is silently ignored.
    progress_bus: tokio::sync::broadcast::Sender<progress::ProgressEvent>,

    /// Service-tracked lifecycle override for `Readiness.library`
    /// (P-RA4). [`readiness::Readiness::from_host`] only knows
    /// `Loaded`/`Unloaded` (derived from `host.library_graph().is_some()`).
    /// The transient states `Loading` and `Failed(...)` are stashed here
    /// by `publish_progress(ProgressEvent::LibraryLoad { .. })` and
    /// surfaced via `readiness_for`. When the bus reports `Loaded`/`Failed`
    /// the override is preserved so callers see the lifecycle terminus;
    /// `Unloaded` resets the override so the host-derived state wins
    /// again.
    library_lifecycle: std::sync::RwLock<Option<readiness::LibraryReadiness>>,
}

#[sysml_service_macros::service_impl]
impl SysmlService {
    /// Create an empty service with no files loaded.
    pub fn empty() -> Self {
        Self {
            store: None,
            workflow_store: Arc::new(sysml_store::InMemoryWorkflowStore::new()),
            workspace_root: None,
            sessions: DashMap::new(),
            session_counts: Self::new_session_counts(),
            project_registry: std::sync::RwLock::new(project_registry::ProjectRegistry::new()),
            diagram_manager: diagram_manager::DiagramManager::new(),
            hover_source_cache: std::sync::RwLock::new(std::collections::HashMap::new()),
            archive: Arc::new(InMemorySessionArchive::new()),
            batches: DashMap::new(),
            query_cache: DashMap::new(),
            requirement_verdict_cache: DashMap::new(),
            host: Arc::new(std::sync::Mutex::new(AnalysisHost::new())),
            progress_bus: tokio::sync::broadcast::channel(256).0,
            library_lifecycle: std::sync::RwLock::new(None),
        }
    }

    /// Create a service with an optional store backend.
    pub fn with_store(store: Arc<RwLock<dyn sysml_store::Store + Send + Sync>>) -> Self {
        Self {
            store: Some(store),
            workflow_store: Arc::new(sysml_store::InMemoryWorkflowStore::new()),
            workspace_root: None,
            sessions: DashMap::new(),
            session_counts: Self::new_session_counts(),
            project_registry: std::sync::RwLock::new(project_registry::ProjectRegistry::new()),
            diagram_manager: diagram_manager::DiagramManager::new(),
            hover_source_cache: std::sync::RwLock::new(std::collections::HashMap::new()),
            archive: Arc::new(InMemorySessionArchive::new()),
            batches: DashMap::new(),
            query_cache: DashMap::new(),
            requirement_verdict_cache: DashMap::new(),
            host: Arc::new(std::sync::Mutex::new(AnalysisHost::new())),
            progress_bus: tokio::sync::broadcast::channel(256).0,
            library_lifecycle: std::sync::RwLock::new(None),
        }
    }

    /// Create a service with a custom [`SessionArchive`] implementation.
    ///
    /// R4.1 only ships with [`InMemorySessionArchive`]; this constructor
    /// exists so a disk-backed archive can drop in later without touching
    /// any of the service commands.
    pub fn with_archive(archive: Arc<dyn SessionArchive>) -> Self {
        Self {
            store: None,
            workflow_store: Arc::new(sysml_store::InMemoryWorkflowStore::new()),
            workspace_root: None,
            sessions: DashMap::new(),
            session_counts: Self::new_session_counts(),
            project_registry: std::sync::RwLock::new(project_registry::ProjectRegistry::new()),
            diagram_manager: diagram_manager::DiagramManager::new(),
            hover_source_cache: std::sync::RwLock::new(std::collections::HashMap::new()),
            archive,
            batches: DashMap::new(),
            query_cache: DashMap::new(),
            requirement_verdict_cache: DashMap::new(),
            host: Arc::new(std::sync::Mutex::new(AnalysisHost::new())),
            progress_bus: tokio::sync::broadcast::channel(256).0,
            library_lifecycle: std::sync::RwLock::new(None),
        }
    }

    /// Access the configured session archive.
    pub fn archive(&self) -> &Arc<dyn SessionArchive> {
        &self.archive
    }

    /// Swap in a workflow-store backend (builder-style, composes with
    /// any constructor). The shell binary uses this to supply the
    /// durable JSONL store; the default everywhere else is in-memory.
    #[must_use]
    pub fn with_workflow_store(mut self, workflow_store: Arc<dyn sysml_store::WorkflowStore>) -> Self {
        self.workflow_store = workflow_store;
        self
    }

    /// Access the configured workflow sidecar store.
    pub fn workflow_store(&self) -> &Arc<dyn sysml_store::WorkflowStore> {
        &self.workflow_store
    }

    /// Single-file mode: parse one file and make it available.
    pub fn from_file(path: &Path) -> Result<Self, ServiceError> {
        let service = Self::empty();
        service.load_file(path)?;
        Ok(service)
    }

    /// Workspace mode: discover and parse all `.sysml` files under `root`.
    ///
    /// Post-P3, this is a thin wrapper over [`Self::open_context`] with
    /// an [`OpenTarget::Folder`]. The legacy per-file loop is gone; the
    /// new path stdlib-enables in one shot and respects nested
    /// `sysml.toml` boundaries (cargo-style isolation) as a side effect.
    pub fn from_workspace(root: &Path) -> Result<Self, ServiceError> {
        use sysml_project::discovery::OpenTarget;
        let root = root.canonicalize().map_err(|e| ServiceError::io(root, e))?;

        let service = Self {
            store: None,
            workflow_store: Arc::new(sysml_store::InMemoryWorkflowStore::new()),
            workspace_root: Some(root.clone()),
            sessions: DashMap::new(),
            session_counts: Self::new_session_counts(),
            project_registry: std::sync::RwLock::new(project_registry::ProjectRegistry::new()),
            diagram_manager: diagram_manager::DiagramManager::new(),
            hover_source_cache: std::sync::RwLock::new(std::collections::HashMap::new()),
            archive: Arc::new(InMemorySessionArchive::new()),
            batches: DashMap::new(),
            query_cache: DashMap::new(),
            requirement_verdict_cache: DashMap::new(),
            host: Arc::new(std::sync::Mutex::new(AnalysisHost::new())),
            progress_bus: tokio::sync::broadcast::channel(256).0,
            library_lifecycle: std::sync::RwLock::new(None),
        };

        service.open_context(OpenTarget::Folder(root))?;
        Ok(service)
    }

    /// Subscribe to the [`progress::ProgressEvent`] broadcast stream
    /// (P-RA4). Returns a fresh receiver; existing events are not
    /// replayed. If the publisher lags ahead of a slow subscriber, the
    /// subscriber's `recv()` returns `RecvError::Lagged` and the
    /// channel skips messages — broadcast capacity is 256.
    pub fn subscribe_progress(
        &self,
    ) -> tokio::sync::broadcast::Receiver<progress::ProgressEvent> {
        self.progress_bus.subscribe()
    }

    /// Non-blocking publish on the progress bus (P-RA4). Returns the
    /// number of receivers the event reached, or `0` if no subscribers
    /// are attached. The error case (`SendError`) is unreachable on
    /// `broadcast::Sender::send` — it only errors when there are no
    /// receivers, which we treat as success (CLI/MCP sync flows never
    /// subscribe).
    ///
    /// Side effect: `LibraryLoad { phase, detail, .. }` events update
    /// the service-tracked `library_lifecycle` override so subsequent
    /// `readiness_for` calls see `Loading` / `Failed(detail)` even
    /// before `host.library_graph()` flips to `Some`.
    pub fn publish_progress(&self, event: progress::ProgressEvent) {
        // Thread library-load lifecycle into the readiness override
        // before broadcasting; readers that consume the broadcast will
        // see a consistent `readiness_for` snapshot once they receive
        // the event.
        if let progress::ProgressEvent::LibraryLoad { phase, detail, .. } = &event {
            let next = match phase {
                progress::LibraryPhase::Loading => Some(readiness::LibraryReadiness::Loading),
                progress::LibraryPhase::Loaded => Some(readiness::LibraryReadiness::Loaded),
                progress::LibraryPhase::Failed => Some(readiness::LibraryReadiness::Failed(
                    std::sync::Arc::from(detail.as_str()),
                )),
            };
            if let Ok(mut guard) = self.library_lifecycle.write() {
                *guard = next;
            }
        }
        // `send` returns `Err(SendError)` only when there are zero
        // receivers. That's the normal case for any synchronous
        // transport, so swallow the error rather than propagate.
        let _ = self.progress_bus.send(event);
    }

    /// Reset the service-tracked library-lifecycle override so
    /// `readiness_for` falls back to the host-derived `Loaded`/`Unloaded`
    /// state. Use after `cache.rebuild` or before a fresh load to clear
    /// stale `Failed` / `Loading` lifecycle.
    pub fn reset_library_lifecycle(&self) {
        if let Ok(mut guard) = self.library_lifecycle.write() {
            *guard = None;
        }
    }

    /// Borrow the shared `AnalysisHost`. Returned `Arc` allows transports
    /// (LSP, MCP, REST, CLI) to share the same salsa DB so all loaded files
    /// — editor-driven and service-driven — live in one place.
    ///
    /// **Locking discipline**: hold this `Mutex` only briefly: clone an
    /// `Analysis` snapshot, set inputs, or look up a `SourceFile`, then
    /// drop the guard. Never `.await` while holding the guard — the
    /// `MutexGuard` is `!Send` and the future will not compile.
    pub fn host_arc(&self) -> &Arc<std::sync::Mutex<sysml_ide_db::AnalysisHost>> {
        &self.host
    }

    // -- Graph utilities --

    /// ProjectId reserved for the service's workspace ProjectFileSet.
    /// Must be > 9 (stdlib reserves 0..9) and distinct from any user project.
    const SERVICE_WORKSPACE_PROJECT_ID: u32 = 100;

    /// Acquire an immutable [`Analysis`] snapshot of the host. Brief lock —
    /// clones the DB Arc, drops the guard, returns the snapshot for
    /// concurrent reads.
    ///
    /// **LOCK-ORDER INVARIANT (2026-07-17 wedge):** never acquire the host
    /// lock — directly or via a helper like [`Self::source_file_for`] —
    /// while an `Analysis` is alive on the same thread. An `Analysis` is a
    /// salsa db clone; a concurrent mutation (`load_workspace`, LSP
    /// `did_change`) holds the Mutex inside a salsa setter and blocks until
    /// every clone drops, while this thread blocks on the Mutex — a
    /// permanent deadlock that wedges the whole server ("accepting
    /// connections, never responding"). When a code path needs both a
    /// snapshot and host lookups, take them under ONE lock acquisition via
    /// [`Self::locked_analysis_with`].
    fn host_analysis(&self) -> Analysis {
        self.host.lock().unwrap().analysis()
    }

    /// Run `read` under the host lock and return its result together with
    /// an [`Analysis`] snapshot taken under the SAME lock acquisition —
    /// the one safe way to combine "look something up on the host" with
    /// "hold a snapshot" (see the lock-order invariant on
    /// [`Self::host_analysis`]).
    fn locked_analysis_with<R>(
        &self,
        read: impl FnOnce(&sysml_ide_db::AnalysisHost) -> R,
    ) -> (Analysis, R) {
        let host = self.host.lock().unwrap();
        let r = read(&host);
        (host.analysis(), r)
    }

    /// Snapshot + workspace [`sysml_ide_db::ProjectFileSet`] under ONE brief
    /// lock acquisition, guard dropped before the caller runs any salsa
    /// query. Fails hard when no workspace is loaded.
    ///
    /// This is the one home for the workspace-accessor prologue
    /// (`eval_context`, `workspace_*`, `cached_smodel` / `cached_view_model`).
    /// Never run a salsa query while the host guard is alive — a cold
    /// elaboration inside the guard serializes every other host user for
    /// seconds (same lock, opposite direction from the 2026-07-17 wedge;
    /// see the lock-order invariant on `host_analysis`).
    fn workspace_analysis(
        &self,
    ) -> Result<(Analysis, sysml_ide_db::ProjectFileSet), ServiceError> {
        let (analysis, pfs_opt) = self.locked_analysis_with(|host| {
            host.project_file_set(sysml_project::ProjectHandle(
                Self::SERVICE_WORKSPACE_PROJECT_ID,
            ))
        });
        let Some(pfs) = pfs_opt else {
            return Err(ServiceError::ElementNotFound(
                "workspace not loaded; call open_context first".to_owned(),
            ));
        };
        Ok((analysis, pfs))
    }

    /// Look up the [`SourceFile`] salsa input for a URI by going through the
    /// host's [`FileSet`]. Returns `None` if the URI hasn't been loaded.
    fn source_file_for(&self, uri: &str) -> Option<SourceFile> {
        let host = self.host.lock().unwrap();
        let id = host.file_id(uri)?;
        host.source_file(id)
    }

    /// Ensure the workspace [`ProjectFileSet`] exists on the host and that
    /// it tracks every currently-loaded file. Created lazily; updated in
    /// place once present (Salsa Setter triggers downstream invalidation).
    /// Called from `load_file*` / `load_source` / `load_workspace` whenever
    /// the host's tracked files change.
    fn ensure_workspace_pfs(&self) -> ProjectFileSet {
        use salsa::Setter;
        use sysml_project::ProjectHandle as SysmlProjectId;
        let mut host = self.host.lock().unwrap();
        let pid = SysmlProjectId(Self::SERVICE_WORKSPACE_PROJECT_ID);
        // Collect every user-owned SourceFile the host currently tracks, in
        // URI order so the merge inside elaborate_workspace is deterministic
        // across runs. Files registered with a project_id (e.g. stdlib files
        // under STDLIB_BUNDLE_PROJECT_ID) are excluded — they participate in
        // resolution via `LibraryGraph`, not the user workspace PFS.
        let mut entries: Vec<(String, SourceFile)> = host
            .files()
            .user_file_ids()
            .filter_map(|fid| {
                let uri = host.files().uri(fid)?.to_string();
                let sf = host.files().source_file(fid)?;
                Some((uri, sf))
            })
            .collect();
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        let files: Vec<SourceFile> = entries.into_iter().map(|(_, sf)| sf).collect();
        let files_arc = std::sync::Arc::new(files);
        match host.project_file_set(pid) {
            Some(pfs) => {
                pfs.set_files(host.db_mut()).to(files_arc);
                pfs
            }
            None => {
                let pfs = ProjectFileSet::new(
                    host.db(),
                    pid.0,
                    files_arc,
                    sysml_ide_db::project_inputs::PROJECT_KIND_DISCOVERED,
                );
                host.add_project_file_set(pfs);
                pfs
            }
        }
    }

    // -- File management --

    /// Discover and load all `.sysml` files under a workspace directory.
    #[service_command(
        name = "sysml.load_workspace",
        category = FileManagement,
        description = "Discover and load all .sysml files under a directory (recursive). Returns loaded URIs and any errors.",
        returns = "WorkspaceLoadResult { loaded_uris, error_count, errors }",
    )]
    pub fn load_workspace(
        &self,
        #[doc = "Root directory of the workspace to scan"] root: &Path,
    ) -> Result<types::WorkspaceLoadResult, ServiceError> {
        let root = root.canonicalize().map_err(|e| ServiceError::io(root, e))?;

        // Scope the workspace to `root`: drop every host file that sits
        // outside the new workspace. Makes load_workspace idempotent for
        // the same root and prevents state from accumulating across
        // switches (previously two successive load_workspace calls with
        // different roots left both workspaces' docs merged into
        // __workspace__). open_context itself does NOT drop old files;
        // scoping is load_workspace's contract, so we apply it BEFORE the
        // open_context call.
        //
        // The prefix compare must use the host's canonical URI form
        // (`file://` URLs — see sysml_ide_db::canonical_uri). Before
        // 2026-07-16 this compared against the raw path form, which never
        // matched, so every call wiped EVERY user file — reloads worked
        // only by accident of that over-removal (full re-parse + full
        // re-elaboration each time, and LSP overlay flags silently
        // destroyed). With the canonical compare, in-root files keep
        // their FileId/SourceFile identity across reloads and salsa can
        // backdate unchanged files.
        let root_str = root.to_string_lossy().to_string();
        let canonical_root = sysml_ide_db::canonical_uri(&root_str);
        let scope_prefix = if canonical_root.ends_with('/') {
            canonical_root
        } else {
            format!("{canonical_root}/")
        };
        {
            let mut host = self.host.lock().unwrap();
            let to_remove: Vec<String> = host
                .files()
                .user_file_ids()
                .filter_map(|fid| {
                    let uri = host.files().uri(fid)?.to_string();
                    if uri.starts_with(&scope_prefix) {
                        None
                    } else {
                        Some(uri)
                    }
                })
                .collect();
            for uri in to_remove {
                host.remove_file(&uri);
            }
        }

        // Route every file-entry path through the canonical loader
        // (`open_context`). The Folder target drives discovery, stdlib
        // enablement, ProjectFileSet construction, and project-tagging
        // in one place — no more parallel discover + manual load loop.
        //
        // DiskAuthoritative (steward-ruled 2026-07-16): load_workspace is
        // an explicit, disk-authoritative reload of its root — every
        // discovered file's content is re-read from disk and any editor
        // overlay on it is cleared, and in-root files deleted from disk
        // drop out of the host. It is the one command that asserts "the
        // filesystem is truth for this root right now." Implicit
        // file-entry paths (LSP did_open/did_change, the LSP indexer's
        // Folder rescans, load_file) keep overlay-preserving behavior.
        let ctx = self.open_context_with(
            sysml_project::discovery::OpenTarget::Folder(root.clone()),
            crate::open_context::OverlayPolicy::DiskAuthoritative,
        )?;

        // Surface error-severity parse diagnostics as workspace load
        // errors. (Bucket 5-followup, 2026-05-05.) Pulled from Salsa's
        // memoized parse_file results. Discovery-time diagnostics from
        // open_context (cap warnings, stdlib-unavailable) are folded in.
        let mut errors: Vec<String> = ctx
            .diagnostics
            .iter()
            .filter(|d| d.severity.is_error())
            .map(|d| {
                let line = d.span.as_ref().and_then(|s| s.line).unwrap_or(0);
                format!("{}:{line}: {}", root.display(), d.message)
            })
            .collect();
        {
            // Snapshot + SourceFiles under ONE lock acquisition (lock-order
            // invariant on `host_analysis` — re-locking per uri while the
            // snapshot is alive deadlocks against a concurrent mutation,
            // e.g. a second load_workspace: the 2026-07-17 wedge).
            let (analysis, files) = self.locked_analysis_with(|host| {
                ctx.loaded_uris
                    .iter()
                    .filter_map(|uri| {
                        let sf = host.file_id(uri).and_then(|id| host.source_file(id))?;
                        Some((uri.clone(), sf))
                    })
                    .collect::<Vec<(String, SourceFile)>>()
            });
            for (uri, sf) in files {
                let parsed = analysis.parse_file(sf);
                for d in parsed.diagnostics().iter().filter(|d| d.severity.is_error()) {
                    let line = d.span.as_ref().and_then(|s| s.line).unwrap_or(0);
                    errors.push(format!("{uri}:{line}: {}", d.message));
                }
            }
        }

        tracing::info!(
            root = %root.display(),
            loaded = ctx.loaded_uris.len(),
            errors = errors.len(),
            stdlib = ctx.library.is_some(),
            "workspace loaded"
        );

        // Include the workspace sentinel in the result so callers know to
        // use it (see `scope::WORKSPACE_URI`).
        let mut loaded_uris = ctx.loaded_uris;
        loaded_uris.push(WORKSPACE_URI.to_string());

        Ok(types::WorkspaceLoadResult {
            loaded_uris,
            error_count: errors.len(),
            errors,
        })
    }

    /// Parse and load a single file into the service.
    ///
    /// Routes through [`Self::open_context`] so callers transparently
    /// get sibling visibility (when an ancestor `sysml.toml` is
    /// present) and stdlib resolution. Returns the URI under which the
    /// file is addressable in the salsa host — for `OpenTarget::File`
    /// that's exactly `path.to_string_lossy()`, regardless of whether
    /// mode resolved to Strict or DiscoveredViaManifest, because
    /// `discover()` preserves the input root's path form.
    ///
    /// (P3 cut-over. Unblocked by the `try_eval_unresolved` recursion
    /// guard landed in the same commit set — without that guard, the
    /// pid-100-tagged + stdlib-loaded combination tripped an infinite
    /// `Value::Ref` cycle in the ISQ power-factor evaluation under
    /// fixtures like contract_orchestrate's valve-gating model.)
    #[service_command(
        name = "sysml.load_file",
        category = FileManagement,
        description = "Parse and load a SysML file using the PEG batch parser",
        returns = "string (URI key for the loaded file)",
    )]
    pub fn load_file(
        &self,
        #[doc = "File system path to the .sysml file"] path: &Path,
    ) -> Result<String, ServiceError> {
        use sysml_project::discovery::OpenTarget;
        let _ctx = self.open_context(OpenTarget::File(path.to_path_buf()))?;
        Ok(path.to_string_lossy().to_string())
    }

    /// Load source text directly (no file I/O).
    #[service_command(
        name = "sysml.load_source",
        category = FileManagement,
        description = "Parse and load SysML source text directly (no file I/O)",
        returns = "()",
    )]
    pub fn load_source(
        &self,
        #[doc = "Identifier for the source (used as lookup key)"] uri: &str,
        #[doc = "SysML v2 source text"] source: &str,
    ) -> Result<(), ServiceError> {
        {
            let mut host = self.host.lock().unwrap();
            host.set_file_content(uri, source.to_owned());
        }
        self.ensure_workspace_pfs();
        Ok(())
    }

    /// Load source text and attribute the file to the service workspace
    /// project so workspace-aware queries pick it up.
    ///
    /// Like [`load_source`] but registers the file under the service's
    /// workspace `ProjectId`. `compute_full_diagnostics` and the
    /// `resolve_file_best` salsa query gate the workspace
    /// resolution path on the file having a project_id; without it,
    /// every per-file diagnostic call falls back to file-only
    /// resolution and cross-file imports stay unresolved. Use this from
    /// Rust callers (CLI inspect workspace mode, eventually REST) that
    /// populate the host one file at a time. Intentionally not a
    /// `#[service_command]` — exposed for direct Rust use, not for the
    /// transport-erased dispatcher.
    pub fn load_workspace_source(&self, uri: &str, source: &str) -> Result<(), ServiceError> {
        {
            let mut host = self.host.lock().unwrap();
            host.set_file_content_in_project(
                uri,
                source.to_owned(),
                sysml_project::ProjectHandle(Self::SERVICE_WORKSPACE_PROJECT_ID),
            );
        }
        self.ensure_workspace_pfs();
        Ok(())
    }

    /// Unload a previously-loaded URI.
    ///
    /// Removes the file from the salsa host so subsequent reads no
    /// longer see it, then refreshes the workspace `ProjectFileSet` so
    /// `workspace_aware_graph` callers see the new file set on the
    /// next access.
    ///
    /// Returns `true` if the URI was loaded (and is now unloaded),
    /// `false` if it was already absent.
    #[service_command(
        name = "sysml.unload_file",
        category = FileManagement,
        description = "Remove a previously-loaded URI from the analysis host",
        returns = "bool",
    )]
    pub fn unload_file(
        &self,
        #[doc = "URI of the loaded model to remove"] uri: &str,
    ) -> Result<bool, ServiceError> {
        let removed = {
            let mut host = self.host.lock().unwrap();
            host.remove_file(uri).is_some()
        };
        if removed {
            self.ensure_workspace_pfs();
        }
        Ok(removed)
    }

    /// Per-URI readiness predicate (Phase P-RA1).
    ///
    /// Returns a [`readiness::Readiness`] snapshot derived from the
    /// current [`AnalysisHost`] state — no new locks, no new state.
    /// Any transport (LSP / MCP / REST / CLI) can call this to decide
    /// whether a particular class of diagnostics or features is
    /// answerable yet, instead of duplicating its own tracking.
    ///
    #[service_command(
        name = "sysml.readiness",
        category = Analysis,
        description = "Per-URI readiness predicate: is this question answerable yet (library/project/file)?",
        returns = "Readiness { library, project, file, project_kind }",
    )]
    pub fn readiness_for(
        &self,
        #[doc = "URI of the file to query"] uri: &str,
    ) -> readiness::Readiness {
        let host = self.host.lock().unwrap();
        let mut r = readiness::Readiness::from_host(&host, uri);
        // Overlay any service-tracked lifecycle (Loading / Failed) on
        // top of the host-derived Loaded/Unloaded view (P-RA4). The
        // override is set by `publish_progress(LibraryLoad{..})` and
        // is the only way callers can observe transient lifecycle
        // states after the LSP-side `LibraryState` enum retired.
        if let Some(over) = self
            .library_lifecycle
            .read()
            .ok()
            .and_then(|g| g.clone())
        {
            // `Loaded` from the override only wins if the host also
            // says `Loaded` — that keeps the override from claiming
            // success before the graph is actually registered. The
            // other variants override unconditionally.
            match over {
                readiness::LibraryReadiness::Loaded
                    if matches!(r.library, readiness::LibraryReadiness::Loaded) =>
                {
                    // host already says Loaded — nothing to do
                }
                readiness::LibraryReadiness::Loaded => {
                    // host hasn't yet seen the graph; report Loading
                    r.library = readiness::LibraryReadiness::Loading;
                }
                other => r.library = other,
            }
        }
        r
    }

    /// List all loaded URIs.
    #[service_command(
        name = "sysml.loaded_uris",
        category = FileManagement,
        description = "List all currently loaded model URIs",
        returns = "Vec<string>",
    )]
    pub fn loaded_uris(&self) -> Vec<String> {
        let host = self.host.lock().unwrap();
        host.files()
            .user_file_ids()
            .filter_map(|fid| host.files().uri(fid).map(ToString::to_string))
            .collect()
    }

    // -- Parse & Analysis --

    /// Parse a file and return diagnostics (does not store the graph).
    #[service_command(
        name = "sysml.parse",
        category = Analysis,
        description = "Parse a SysML file and return the model graph with diagnostics (does not store)",
        returns = "(ModelGraph, Vec<Diagnostic>)",
    )]
    pub fn parse(
        &self,
        #[doc = "File system path to the .sysml file"] path: &Path,
    ) -> Result<(ModelGraph, Vec<Diagnostic>), ServiceError> {
        // No-store parse: we don't want this file content in the salsa
        // host because the service contract for `sysml.parse` is
        // explicitly "doesn't store the graph". TS-3.4 flipped this from
        // PestParser to TreeSitterParser; the TS parser is the canonical
        // batch + IDE parser now that TS-1.x closed the relationship
        // gaps and TS-3.3 made TS the strict diagnostic oracle.
        use sysml_parser_incremental::TreeSitterParser;
        use sysml_parser_trait::{Parser, SysmlFile};
        let source = std::fs::read_to_string(path).map_err(|e| ServiceError::io(path, e))?;
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "input.sysml".to_owned());
        let result = TreeSitterParser::new().parse(&[SysmlFile::new(&filename, &source)]);
        Ok((result.graph, result.diagnostics))
    }

    /// Compute the full diagnostic pipeline for a loaded URI.
    ///
    /// Runs parse → resolve → validate → 8 health checks, with phase
    /// gating, library-type suppression, scope-aware suppression, dedupe,
    /// priority sort, and total cap. Constraint monitoring lands in S2.T8
    /// commit 2. The result is sysml-span diagnostics ready for any
    /// transport-specific shape conversion.
    #[service_command(
        name = "sysml.diagnostics",
        category = Analysis,
        description = "Get the full diagnostic pipeline (parse + resolve + validate + health) for a loaded URI",
        returns = "Vec<Diagnostic>",
    )]
    pub fn diagnostics(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
    ) -> Result<Vec<Diagnostic>, ServiceError> {
        if self.source_file_for(uri).is_none() {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        }
        Ok(diagnostics::compute_full_diagnostics(&*self.host, uri))
    }

    /// Compute the inspect-pipeline result (diagnostics + semantic tokens)
    /// for one URI or for every loaded user URI.
    ///
    /// Replaces the CLI's inline parse → resolve → elaborate → validate →
    /// health pipeline (X6). The diagnostics come from
    /// `diagnostics::compute_full_diagnostics` (same pipeline the LSP uses);
    /// the tokens come from `Analysis::semantic_tokens`. Both are
    /// salsa-memoized.
    ///
    /// When `workspace` is true the call iterates every user-owned URI
    /// loaded into the host (stdlib URIs excluded) and returns a per-file
    /// result; `focus_file` optionally restricts the result to entries
    /// whose URI ends with the given filename. When `workspace` is false a
    /// `uri` is required and the response contains exactly one file entry.
    #[service_command(
        name = "sysml.inspect",
        category = Analysis,
        description = "Compute the inspect-pipeline result (diagnostics + semantic tokens) for one URI or every loaded user URI",
        returns = "InspectResponse { files: Vec<InspectFileResult> }",
    )]
    pub fn inspect(
        &self,
        #[doc = "URI of the loaded model; required when workspace=false"]
        uri: Option<&str>,
        #[doc = "If true, return inspect results for every loaded user URI"]
        workspace: bool,
        #[doc = "Optional filename filter (workspace mode); keeps only URIs ending with this suffix"]
        focus_file: Option<&str>,
    ) -> Result<inspect::InspectResponse, ServiceError> {
        if workspace {
            let entries: Vec<(String, SourceFile)> = {
                let host = self.host.lock().unwrap();
                host.files()
                    .user_file_ids()
                    .filter_map(|fid| {
                        let uri = host.files().uri(fid)?.to_string();
                        let sf = host.files().source_file(fid)?;
                        Some((uri, sf))
                    })
                    .collect()
            };
            let mut files = Vec::with_capacity(entries.len());
            for (file_uri, sf) in entries {
                if let Some(focus) = focus_file {
                    if !uri_matches_focus(&file_uri, focus) {
                        continue;
                    }
                }
                files.push(inspect::compute_inspect_file(&self.host, &file_uri, sf));
            }
            files.sort_by(|a, b| a.uri.cmp(&b.uri));
            return Ok(inspect::InspectResponse { files });
        }

        let uri = uri.ok_or_else(|| {
            ServiceError::InvalidInput(
                "inspect: `uri` is required when `workspace` is false".to_owned(),
            )
        })?;
        let sf = self.source_file_for(uri).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no graph for URI: {uri}"))
        })?;
        let file = inspect::compute_inspect_file(&self.host, uri, sf);
        Ok(inspect::InspectResponse { files: vec![file] })
    }

    // -- Query delegates --

    /// Build the document outline (nested symbol tree) for a loaded URI.
    ///
    /// Routed through the salsa-memoized `document_symbols` query. Returns
    /// byte-offset ranges; the LSP shell converts to line/col `Range`
    /// using the file's content. Returns an empty vec when the file has
    /// no parseable elements (caller may apply a transport-specific
    /// fallback).
    #[service_command(
        name = "sysml.outline",
        category = Query,
        description = "Build the document outline (nested symbol tree) for a loaded URI",
        returns = "Vec<OutlineNode>",
    )]
    pub fn outline(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
    ) -> Result<Vec<outline::OutlineNode>, ServiceError> {
        if self.source_file_for(uri).is_none() {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        }
        Ok(outline::compute_outline(&self.host, uri))
    }

    /// Resolve goto-definition target for the cursor at `(uri, line, col)`.
    ///
    /// Follows the relationship-following ladder (FeatureTyping → type,
    /// Specialization → general, etc.) and prefers a typed-usage's type
    /// definition over the usage itself. Returns `None` when the cursor
    /// doesn't resolve to an element — the LSP shell can then fall back
    /// to its word-based workspace lookup.
    #[service_command(
        name = "sysml.goto_definition",
        category = Query,
        description = "Resolve the goto-definition target for the cursor position",
        returns = "GotoTarget?",
    )]
    pub fn goto_definition(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Cursor line (0-indexed)"] line: u32,
        #[doc = "Cursor column (UTF-16 code units, 0-indexed)"] col: u32,
    ) -> Result<Option<goto_definition::GotoTarget>, ServiceError> {
        if self.source_file_for(uri).is_none() {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        }
        Ok(goto_definition::compute_goto_definition(
            &self.host, uri, line, col,
        ))
    }

    /// Find every reference to the element at `(uri, line, col)`.
    ///
    /// In-file refs come from the salsa-cached `position_map`; cross-file
    /// refs come from a name-match walk of every other file known to the
    /// host. Line/column follow the LSP convention (UTF-16 code units,
    /// 0-indexed). Returns an empty vec when the cursor doesn't resolve to
    /// an element.
    #[service_command(
        name = "sysml.references",
        category = Query,
        description = "Find every reference to the element at the given cursor position",
        returns = "Vec<RefHit>",
    )]
    pub fn references(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Cursor line (0-indexed)"] line: u32,
        #[doc = "Cursor column (UTF-16 code units, 0-indexed)"] col: u32,
    ) -> Result<Vec<references::RefHit>, ServiceError> {
        if self.source_file_for(uri).is_none() {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        }
        Ok(references::compute_references(&self.host, uri, line, col))
    }

    /// Compute whitespace-only formatting edits for a document.
    ///
    /// Replaces the LSP-side `format_document` body. The LSP shell becomes a
    /// thin shim that converts `lsp_types::FormattingOptions` to
    /// `FormatOptions` and reshapes the shared `TextEdit` into
    /// `lsp_types::TextEdit`.
    #[service_command(
        name = "sysml.format.document",
        category = Query,
        description = "Compute whitespace-only formatting edits for a loaded document",
        returns = "Vec<TextEdit>",
    )]
    pub fn format_document(
        &self,
        #[doc = "URI of the loaded document"] uri: &str,
        #[doc = "Indent width in characters (default 4)"] tab_size: Option<u32>,
        #[doc = "Insert spaces (true) or tabs (false); default true"] insert_spaces: Option<bool>,
    ) -> Result<Vec<text_edit::TextEdit>, ServiceError> {
        let sf = self.source_file_for(uri).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no graph for URI: {uri}"))
        })?;
        let analysis = self.host_analysis();
        let content = analysis.file_text(sf).to_owned();
        let options = formatting::FormatOptions {
            tab_size: tab_size.unwrap_or(4),
            insert_spaces: insert_spaces.unwrap_or(true),
        };
        Ok(formatting::compute_format_edits(&content, &options))
    }

    /// Compute the workspace edit for a diagram-driven action (create /
    /// delete / editLabel / addSequenceMessage / addSequenceLifeline).
    ///
    /// Replaces the LSP-side `handle_diagram_edit` body. The LSP shell now
    /// dispatches by calling this command, then applies the returned
    /// workspace edit via `client.apply_edit(...)`.
    #[service_command(
        name = "sysml.diagram.edit",
        category = Query,
        description = "Compute the workspace edit for a diagram-driven create/delete/editLabel/addSequenceMessage/addSequenceLifeline action",
        returns = "DiagramEditComputed",
    )]
    pub fn diagram_edit(
        &self,
        #[doc = "Diagram edit request (matches DiagramEditRequest JSON schema: { uri, action, ...payload })"]
        request: &serde_json::Value,
    ) -> Result<diagram_edit::DiagramEditComputed, ServiceError> {
        diagram_edit::compute_diagram_edit(&self.host, request)
    }

    /// Compute the full code-action list (quick-fixes + refactorings + source
    /// actions) for a `(uri, range)` selection, given the diagnostics that
    /// are visible in the editor.
    ///
    /// Replaces the LSP-side `generate_code_actions` body. The LSP shell
    /// becomes a thin shim that converts `lsp_types::Range` and `lsp_types::Diagnostic`
    /// into `CodeActionDiagnostic` and reshapes `CodeAction` into
    /// `lsp_types::CodeActionOrCommand`.
    #[service_command(
        name = "sysml.code_action.list",
        category = Query,
        description = "Compute every code action (quick-fixes, refactorings, source actions) at a range",
        returns = "Vec<CodeAction>",
    )]
    pub fn code_action_list(
        &self,
        #[doc = "URI of the loaded document"] uri: &str,
        #[doc = "Selection start line (0-indexed)"] range_start_line: u32,
        #[doc = "Selection start column (UTF-16 code units, 0-indexed)"] range_start_col: u32,
        #[doc = "Selection end line (0-indexed)"] range_end_line: u32,
        #[doc = "Selection end column (UTF-16 code units, 0-indexed)"] range_end_col: u32,
        #[doc = "Diagnostics visible at the selection (array of CodeActionDiagnostic)"]
        diagnostics: &serde_json::Value,
    ) -> Result<Vec<code_actions::CodeAction>, ServiceError> {
        let parsed: Vec<code_actions::CodeActionDiagnostic> = if diagnostics.is_null() {
            Vec::new()
        } else {
            serde_json::from_value(diagnostics.clone()).map_err(|e| {
                ServiceError::InvalidInput(format!("invalid diagnostics array: {e}"))
            })?
        };
        code_actions::compute_code_actions(
            &self.host,
            uri,
            range_start_line,
            range_start_col,
            range_end_line,
            range_end_col,
            &parsed,
        )
    }

    /// Compute prepare-rename info or apply-rename edits at a cursor
    /// position.
    ///
    /// When `new_name` is `None`, returns `RenameResponse { prepare: Some(_) }`
    /// with the placeholder text + identifier range. When `new_name` is
    /// `Some(_)`, validates the identifier, walks every host file by name to
    /// collect cross-file refs, and returns
    /// `RenameResponse { apply: Some(workspace_edit) }`.
    ///
    /// Replaces the LSP-side `prepare_rename` + `rename` handlers — the LSP
    /// shell becomes a thin shim that converts `lsp_types::Position` ↔
    /// `(line, col)` and packs `RenameResponse` into `WorkspaceEdit` /
    /// `PrepareRenameResponse`.
    #[service_command(
        name = "sysml.rename",
        category = Query,
        description = "Compute prepare-rename info or apply-rename workspace edits at a cursor position",
        returns = "RenameResponse { prepare? | apply? }",
    )]
    pub fn rename(
        &self,
        #[doc = "URI of the file at the cursor"] uri: &str,
        #[doc = "Cursor line (0-indexed)"] line: u32,
        #[doc = "Cursor column (UTF-16 code units, 0-indexed)"] col: u32,
        #[doc = "When set, returns apply-rename edits; otherwise returns prepare-rename info"]
        new_name: Option<&str>,
    ) -> Result<rename::RenameResponse, ServiceError> {
        if self.source_file_for(uri).is_none() {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        }
        rename::compute_rename(&self.host, uri, line, col, new_name)
    }

    /// Render hover content for the cursor at `(uri, line, col)`.
    ///
    /// Resolves the element under the cursor, follows relationships
    /// (`FeatureTyping → type` etc.), walks the host for cross-file type
    /// definitions where applicable, then renders the markdown signature +
    /// supertypes + doc comment + physics classification + evaluated value.
    /// Returns `None` when the cursor is not over a model element — the LSP
    /// shell falls through to its keyword fallback / import-segment paths.
    #[service_command(
        name = "sysml.hover",
        category = Query,
        description = "Render hover content (markdown + range) for the cursor position",
        returns = "HoverInfo?",
    )]
    pub fn hover(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Cursor line (0-indexed)"] line: u32,
        #[doc = "Cursor column (UTF-16 code units, 0-indexed)"] col: u32,
    ) -> Result<Option<hover::HoverInfo>, ServiceError> {
        if self.source_file_for(uri).is_none() {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        }
        Ok(hover::compute_hover(&self.host, uri, line, col))
    }

    /// Compute completion candidates for the cursor at `(uri, line, col)`.
    ///
    /// The LSP shell classifies the tree-sitter cursor context (which it
    /// already has from `salsa_tree`) into a `SyntaxCtxSummary` and passes it
    /// through. Service then picks one of four routes (NamespaceMembers /
    /// TypeReferences / FeatureChain / General), enumerates candidates from
    /// the current file, workspace, and library, and ranks them.
    #[service_command(
        name = "sysml.completion",
        category = Query,
        description = "Compute completion candidates for the cursor position",
        returns = "Vec<CompletionCandidate>",
    )]
    pub fn completion(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Cursor line (0-indexed)"] line: u32,
        #[doc = "Cursor column (UTF-16 code units, 0-indexed)"] col: u32,
        #[doc = "Trigger character (LSP triggerCharacter)"] trigger: Option<&str>,
        #[doc = "syntax_ctx.in_import (LSP CST classification)"] ctx_in_import: bool,
        #[doc = "syntax_ctx.in_comment_or_string"] ctx_in_comment_or_string: bool,
        #[doc = "syntax_ctx.in_feature_chain"] ctx_in_feature_chain: bool,
        #[doc = "syntax_ctx.in_type_ref"] ctx_in_type_ref: bool,
    ) -> Result<Vec<completion::CompletionCandidate>, ServiceError> {
        if self.source_file_for(uri).is_none() {
            return Err(ServiceError::ElementNotFound(format!(
                "no graph for URI: {uri}"
            )));
        }
        let syntax_ctx = completion::SyntaxCtxSummary {
            in_import: ctx_in_import,
            in_comment_or_string: ctx_in_comment_or_string,
            in_feature_chain: ctx_in_feature_chain,
            in_type_ref: ctx_in_type_ref,
        };
        Ok(completion::compute_completion(
            &self.host,
            uri,
            line,
            col,
            trigger,
            Some(&syntax_ctx),
        ))
    }

    /// Resolve-time enrichment for a completion item (doc-comment + type detail).
    #[service_command(
        name = "sysml.completion.resolve",
        category = Query,
        description = "Enrich a completion item with documentation and type detail",
        returns = "CompletionDetails?",
    )]
    pub fn completion_resolve(
        &self,
        #[doc = "Preferred URI to resolve in first (the document where completion was triggered)"]
        uri: Option<&str>,
        #[doc = "ElementId of the completion candidate to resolve"] element_id: &ElementId,
    ) -> Result<Option<completion::CompletionDetails>, ServiceError> {
        Ok(completion::compute_completion_resolve(
            &self.host, uri, element_id,
        ))
    }

    /// Find elements by name pattern across a loaded graph.
    #[service_command(
        name = "sysml.find",
        category = Query,
        description = "Legacy wrapper over sysml.query: find elements by name pattern (substring match), optionally filtered by element kind",
        returns = "Vec<Element>",
    )]
    pub fn find(
        &self,
        #[doc = "URI of the loaded model to search"] uri: &str,
        #[doc = "Name pattern to search for (substring match)"] pattern: &str,
        #[doc = "Optional element kind filter (e.g. 'PartUsage', 'RequirementUsage')"] kind: Option<&ElementKind>,
    ) -> Result<Vec<Element>, ServiceError> {
        let mut filters = vec![sysml_query::Filter::NameMatch {
            name_match: sysml_query::NameMatch {
                contains: Some(pattern.to_owned()),
                ci: false,
                ..sysml_query::NameMatch::default()
            },
        }];
        if let Some(kind) = kind {
            filters.push(sysml_query::Filter::Kind {
                kinds: vec![kind.clone()],
            });
        }
        self.query_all_elements(
            uri,
            sysml_query::QuerySpec {
                filter: sysml_query::Filter::All { filters },
                projection: sysml_query::Projection::Elements,
                ..sysml_query::QuerySpec::default()
            },
        )
    }

    /// Unified structured element-list query primitive.
    #[service_command(
        name = "sysml.query",
        category = Query,
        description = "Run a structured, paged query over model elements",
        returns = "QueryResult",
    )]
    pub fn query(
        &self,
        #[doc = "URI of the loaded model to query; use '__workspace__' for the merged workspace graph"] uri: &str,
        #[doc = "Structured QuerySpec JSON"] spec: &serde_json::Value,
    ) -> Result<sysml_query::QueryResult, ServiceError> {
        let spec: sysml_query::QuerySpec = serde_json::from_value(spec.clone())
            .map_err(|e| ServiceError::InvalidInput(e.to_string()))?;
        self.execute_query_spec(uri, &spec, sysml_query::QueryProfile::Service)
    }

    /// Get a single element by ID.
    #[service_command(
        name = "sysml.element",
        category = Query,
        description = "Get a single element by its ID",
        returns = "Element?",
    )]
    pub fn element(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "The element's unique identifier"] id: &ElementId,
    ) -> Result<Option<Element>, ServiceError> {
        let graph = self.require_graph(uri)?;

        Ok(graph.get_element(id).cloned())
    }

    /// Get the source slice for a single element (S4.T2).
    ///
    /// Thin wrapper over the salsa-tracked
    /// [`file_source_query::file_source_at`] query: returns the element's
    /// declaration text and the byte/line span it covers in the file.
    /// Powers Monaco sneak-peeks in the simulation-app — diagram clicks
    /// and hover popups resolve `(uri, ElementId)` and ask the service
    /// for the slice to render in a read-only editor.
    ///
    /// Returns `Ok(None)` when the element isn't present in the file's
    /// parsed graph or carries no usable span — the caller's "open at
    /// this element" UX should fall back to scrolling to file head.
    #[service_command(
        name = "sysml.get_source",
        category = Query,
        description = "Get the source text and span covering one element's declaration",
        returns = "GetSourceResult?",
    )]
    pub fn get_source(
        &self,
        #[doc = "URI of the loaded model that owns the element"] uri: &str,
        #[doc = "The element's unique identifier"] id: &ElementId,
    ) -> Result<Option<GetSourceResult>, ServiceError> {
        let sf = self
            .source_file_for(uri)
            .ok_or_else(|| ServiceError::ElementNotFound(format!("no graph for URI: {uri}")))?;
        let analysis = self.host_analysis();
        let slice = file_source_query::file_source_at(analysis.db(), sf, id.clone());
        Ok(slice.map(|s| {
            let span = s.span();
            GetSourceResult {
                text: s.text().to_owned(),
                start: span.start,
                end: span.end,
                line: span.line,
                col: span.col,
            }
        }))
    }

    /// Get children of an element.
    #[service_command(
        name = "sysml.children",
        category = Query,
        description = "Legacy wrapper over sysml.query: get direct children of an element (ownership hierarchy)",
        returns = "Vec<Element>",
    )]
    pub fn children(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Parent element ID"] id: &ElementId,
    ) -> Result<Vec<Element>, ServiceError> {
        self.query_all_elements(
            uri,
            sysml_query::QuerySpec {
                filter: sysml_query::Filter::Owner {
                    owner: sysml_query::OwnerFilter {
                        id: Some(id.clone()),
                        kind: None,
                        transitive: false,
                    },
                },
                projection: sysml_query::Projection::Elements,
                ..sysml_query::QuerySpec::default()
            },
        )
    }

    /// Compute model statistics.
    #[service_command(
        name = "sysml.stats",
        category = Query,
        description = "Compute element and relationship count statistics for a model",
        returns = "ElementStats",
    )]
    pub fn stats(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
    ) -> Result<ElementStats, ServiceError> {
        let graph = self.require_graph(uri)?;

        Ok(query::stats(&graph))
    }

    /// Build a model tree.
    #[service_command(
        name = "sysml.model.tree",
        category = Query,
        description = "Build a hierarchical tree of root elements and their children (for tree views)",
        returns = "Vec<TreeNode>",
    )]
    pub fn model_tree(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Maximum depth to recurse (omit for unlimited)"] max_depth: Option<usize>,
        #[doc = "View to project: \"user_facing\" (default — mirrors what the simulation UI sees, drops spec-mandated wrappers + currently-hidden kinds) or \"full\" (every element kind preserved, for AI agents inspecting the raw graph)."]
        view: Option<&str>,
    ) -> Result<Vec<TreeNode>, ServiceError> {
        let tree_view = query::TreeView::from_query(view);
        // PFS + scope SourceFile + snapshot under ONE lock acquisition
        // (lock-order invariant on `host_analysis` — the 2026-07-17 wedge).
        let scope = GraphScope::parse(uri);
        let (analysis, (pfs_opt, scope_sf)) = self.locked_analysis_with(|host| {
            let pfs = host.project_file_set(sysml_project::ProjectHandle(
                Self::SERVICE_WORKSPACE_PROJECT_ID,
            ));
            let sf = match &scope {
                GraphScope::File(f) => host.file_id(f).and_then(|id| host.source_file(id)),
                GraphScope::Workspace => None,
            };
            (pfs, sf)
        });

        match scope {
            // Workspace scope: source-of-roots and resolver are both the
            // (library-merged) workspace graph.
            GraphScope::Workspace => {
                let pfs = pfs_opt.ok_or_else(|| {
                    ServiceError::ElementNotFound(format!("no graph for URI: {uri}"))
                })?;
                let cached = model_tree_query::workspace_model_tree_best(
                    analysis.db(),
                    pfs,
                    analysis.library_graph(),
                    max_depth,
                    tree_view,
                );
                Ok(cached.to_vec())
            }
            // File scope: roots come from the file's parse, resolver
            // climbs out to the workspace if a PFS is loaded so
            // cross-file typed-definition children still inline. Falls
            // back to single-file mode when no workspace is loaded.
            GraphScope::File(file_uri) => {
                let sf = scope_sf.ok_or_else(|| {
                    ServiceError::ElementNotFound(format!("no graph for URI: {file_uri}"))
                })?;
                let cached = model_tree_query::file_in_workspace_model_tree_best(
                    analysis.db(),
                    sf,
                    pfs_opt,
                    analysis.library_graph(),
                    max_depth,
                    tree_view,
                );
                Ok(cached.to_vec())
            }
        }
    }

    /// Find unverified requirements.
    #[service_command(
        name = "sysml.unverified",
        category = Query,
        description = "Legacy wrapper over sysml.query: find all requirements that have no Verify relationship targeting them",
        returns = "Vec<Element>",
    )]
    pub fn unverified(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
    ) -> Result<Vec<Element>, ServiceError> {
        self.query_all_elements(
            uri,
            sysml_query::QuerySpec {
                filter: sysml_query::Filter::UnverifiedRequirement,
                projection: sysml_query::Projection::Elements,
                ..sysml_query::QuerySpec::default()
            },
        )
    }

    /// Get ancestors of an element.
    #[service_command(
        name = "sysml.ancestors",
        category = Query,
        description = "Walk the ownership chain upward from an element to the root",
        returns = "Vec<Element>",
    )]
    pub fn ancestors(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Element ID to start from"] id: &ElementId,
    ) -> Result<Vec<Element>, ServiceError> {
        let graph = self.require_graph(uri)?;

        Ok(query::ancestors(&graph, id)
            .into_iter()
            .cloned()
            .collect())
    }

    /// Get descendants of an element.
    #[service_command(
        name = "sysml.descendants",
        category = Query,
        description = "Recursively collect all descendant elements in the ownership tree",
        returns = "Vec<Element>",
    )]
    pub fn descendants(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Root element ID"] id: &ElementId,
    ) -> Result<Vec<Element>, ServiceError> {
        // Match the prior `require_graph(uri)` semantics: workspace-merged
        // graph for workspace scope (and a PFS is loaded); per-file graph
        // otherwise. The snapshot baseline encodes the per-file shape
        // (props are pre-elaboration), so don't promote workspace lookup
        // for arbitrary file URIs.
        //
        // PFS + scope SourceFile + snapshot under ONE lock acquisition
        // (lock-order invariant on `host_analysis` — the 2026-07-17 wedge).
        let scope = GraphScope::parse(uri);
        let (analysis, (pfs_opt, scope_sf)) = self.locked_analysis_with(|host| {
            let pfs = host.project_file_set(sysml_project::ProjectHandle(
                Self::SERVICE_WORKSPACE_PROJECT_ID,
            ));
            let sf = match &scope {
                GraphScope::File(f) => host.file_id(f).and_then(|id| host.source_file(id)),
                GraphScope::Workspace => None,
            };
            (pfs, sf)
        });
        match scope {
            GraphScope::Workspace => {
                let pfs = pfs_opt.ok_or_else(|| {
                    ServiceError::ElementNotFound(format!("no graph for URI: {uri}"))
                })?;
                let cached = sysml_ide_db::workspace_descendants_best(
                    analysis.db(),
                    pfs,
                    analysis.library_graph(),
                    id.clone(),
                );
                Ok(cached.to_vec())
            }
            GraphScope::File(file_uri) => {
                let sf = scope_sf.ok_or_else(|| {
                    ServiceError::ElementNotFound(format!("no graph for URI: {file_uri}"))
                })?;
                Ok(analysis.file_descendants(sf, id.clone()).to_vec())
            }
        }
    }

    /// Generate a trace matrix.
    #[service_command(
        name = "sysml.trace_matrix",
        category = Query,
        description = "Generate a traceability matrix between element kinds via a relationship kind",
        returns = "Vec<TraceMatrixRow>",
    )]
    pub fn trace_matrix(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Kind of source elements (e.g. 'PartUsage')"] source_kind: &ElementKind,
        #[doc = "Relationship kind to trace (e.g. 'Satisfy')"] rel_kind: &RelationshipKind,
        #[doc = "Kind of target elements (e.g. 'RequirementUsage')"] target_kind: &ElementKind,
    ) -> Result<Vec<TraceMatrixRow>, ServiceError> {
        // Match the prior `require_graph(uri)` semantics: workspace-merged
        // graph for workspace scope; per-file graph otherwise.
        //
        // PFS + scope SourceFile + snapshot under ONE lock acquisition
        // (lock-order invariant on `host_analysis` — the 2026-07-17 wedge).
        let scope = GraphScope::parse(uri);
        let (analysis, (pfs_opt, scope_sf)) = self.locked_analysis_with(|host| {
            let pfs = host.project_file_set(sysml_project::ProjectHandle(
                Self::SERVICE_WORKSPACE_PROJECT_ID,
            ));
            let sf = match &scope {
                GraphScope::File(f) => host.file_id(f).and_then(|id| host.source_file(id)),
                GraphScope::Workspace => None,
            };
            (pfs, sf)
        });
        match scope {
            GraphScope::Workspace => {
                let pfs = pfs_opt.ok_or_else(|| {
                    ServiceError::ElementNotFound(format!("no graph for URI: {uri}"))
                })?;
                let cached = sysml_ide_db::workspace_trace_matrix_best(
                    analysis.db(),
                    pfs,
                    analysis.library_graph(),
                    source_kind.clone(),
                    rel_kind.clone(),
                    target_kind.clone(),
                );
                Ok(cached.to_vec())
            }
            GraphScope::File(file_uri) => {
                let sf = scope_sf.ok_or_else(|| {
                    ServiceError::ElementNotFound(format!("no graph for URI: {file_uri}"))
                })?;
                Ok(analysis
                    .file_trace_matrix(
                        sf,
                        source_kind.clone(),
                        rel_kind.clone(),
                        target_kind.clone(),
                    )
                    .to_vec())
            }
        }
    }

    /// The Requirements workbench table read (B2). One row per non-library
    /// `Requirement{Definition,Usage}` across the elaborated workspace graph,
    /// in document order, with statement text, outline depth, maturity,
    /// satisfy/verify/derive/refine link rollups, and the three-state
    /// verification classification (fail / incomplete / pass).
    ///
    /// The row shape has ONE home: `sysml_query::requirement_rows`. This
    /// command is a thin dispatch that supplies the two caller-computed
    /// inputs — the elaborated `__workspace__` graph and the per-case
    /// verdict map (see [`Self::requirement_case_verdicts`]).
    #[service_command(
        name = "sysml.workspace.requirement_rows",
        category = Query,
        description = "Requirements-workbench table rows: document-ordered Requirement{Definition,Usage} rows over the elaborated workspace, with statement text, outline depth, StatusInfo maturity, satisfy/verify/derive/refine links, and a three-state verification rollup. Paged via limit/cursor in the spec ({} for defaults).",
        returns = "sysml_query::RequirementRowsResult { rows, total_estimate, cursor, cursor_invalidated, revision }",
    )]
    pub fn workspace_requirement_rows(
        &self,
        #[doc = "RequirementRowsSpec JSON: { filter?, limit?, cursor? } — pass {} for defaults"]
        spec: &serde_json::Value,
    ) -> Result<sysml_query::RequirementRowsResult, ServiceError> {
        let spec: sysml_query::RequirementRowsSpec = serde_json::from_value(spec.clone())
            .map_err(|e| ServiceError::InvalidInput(format!("invalid requirement-rows spec: {e}")))?;
        let graph = self.workspace_aware_graph()?;
        let revision = sysml_query::graph_revision(&graph);
        let verdicts = self.requirement_case_verdicts(&graph, revision);
        sysml_query::requirement_rows(
            &graph,
            &spec,
            &verdicts,
            // requirement_case_verdicts = evaluate_verification_cases over
            // the workspace graph at rest — a STATIC evaluation (§2.1a
            // ruling (d) label; the trajectory path is sessions.verify).
            "static",
            revision,
            sysml_query::QueryProfile::Service,
        )
        .map_err(|e| ServiceError::InvalidInput(e.to_string()))
    }

    /// The per-element "evaluated contract" read (R18 / B2.1): everything a
    /// requirement's verdict is computed FROM — subject, assume/require
    /// constraints, referenced attribute values — plus the narrative context
    /// (actors, stakeholders, framed concerns, rationale) as separate
    /// buckets. Deliberately NOT part of `requirement_rows`: rows are walked
    /// to exhaustion, the contract payload only the selected row surfaces
    /// (the `sysml.get_source` (uri, id)-slice precedent).
    ///
    /// The shape has ONE home: `sysml_query::requirement_detail`. Live
    /// session attribute values are a future caller-supplied input (the
    /// `case_verdicts` precedent) — this command serves the static contract.
    #[service_command(
        name = "sysml.workspace.requirement_detail",
        category = Query,
        description = "Evaluated contract of one requirement: subject, assumed/required constraints (inline text is verbatim source; reference-form links resolve when unambiguous), owned attribute values, plus narrative buckets (actors, stakeholders, framed concerns, rationale). Verdict inputs vs narrative separation is binding — render constraints/values next to the verified chip, roles in a generic detail bucket. Fails on unknown ids and non-requirement elements.",
        returns = "sysml_query::RequirementDetail { id, subject, assumed_constraints, required_constraints, framed_concerns, actors, stakeholders, referenced_attributes, rationale }",
    )]
    pub fn workspace_requirement_detail(
        &self,
        #[doc = "The requirement element id (a row id from requirement_rows)"]
        element_id: &ElementId,
    ) -> Result<sysml_query::RequirementDetail, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        sysml_query::requirement_detail(&graph, element_id, &HashMap::new())
            .map_err(|e| ServiceError::InvalidInput(e.to_string()))
    }

    /// Compute a guarded text edit replacing the BODY of an element's first
    /// `doc /* … */` comment (workbench design §7.2 buffer-writeback: the
    /// service computes, the client applies to the BUFFER after verifying
    /// `expected_old_text`, the editor owns save).
    #[service_command(
        name = "sysml.workspace.edit_requirement_doc",
        category = Query,
        description = "Compute a guarded text edit replacing an element's doc-comment body (first doc in document order). Buffer-writeback: the client applies the edit only if the buffer slice equals expected_old_text, else fails loudly. Fails hard when no doc comment exists (adding one is a creation action).",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_edit_requirement_doc(
        &self,
        #[doc = "Element id owning the doc comment (a requirement row id)"]
        element_id: &ElementId,
        #[doc = "New doc body text (comment delimiters are preserved)"]
        new_text: &str,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, element_id)?;
        field_edit::compute_doc_edit(&graph, &content, element_id, new_text)
    }

    /// Compute a guarded text edit replacing an attribute usage's inline
    /// `= value` (workbench design §7.2).
    #[service_command(
        name = "sysml.workspace.edit_attribute_value",
        category = Query,
        description = "Compute a guarded text edit replacing an attribute usage's inline `= value` expression. Buffer-writeback with an expected_old_text guard. Fails hard when the declaration has no inline value (adding one is a creation action).",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_edit_attribute_value(
        &self,
        #[doc = "The attribute usage's element id"]
        element_id: &ElementId,
        #[doc = "New value expression (single line, no `;`)"]
        new_value: &str,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, element_id)?;
        field_edit::compute_attribute_value_edit(&graph, &content, element_id, new_value)
    }

    /// Compute a guarded text edit replacing the `status` value of an
    /// element's `@StatusInfo` metadata (content maturity — the MODEL-side
    /// workflow field; approval lives in the sidecar, never in source).
    #[service_command(
        name = "sysml.workspace.edit_requirement_maturity",
        category = Query,
        description = "Compute a guarded text edit setting @StatusInfo status to StatusKind::<status> (closed vocabulary: open|tbd|tbr|tbc|done|closed). Buffer-writeback with an expected_old_text guard. Fails hard when the element has no @StatusInfo metadata (adding one is a creation action).",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_edit_requirement_maturity(
        &self,
        #[doc = "Element id carrying the @StatusInfo metadata (a requirement row id)"]
        element_id: &ElementId,
        #[doc = "New maturity: one of open|tbd|tbr|tbc|done|closed"]
        status: &str,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, element_id)?;
        field_edit::compute_maturity_edit(&graph, &content, element_id, status)
    }

    /// Compute a guarded insertion edit creating a new requirement member
    /// inside a package or requirement (workbench design §7.3 — shape from
    /// `sysml_core::member_print`, never a client-side template).
    #[service_command(
        name = "sysml.workspace.create_requirement",
        category = Query,
        description = "Compute a guarded text edit inserting a new requirement (optional <'short name'> reqId and doc body) at the end of a package's or requirement's body. Buffer-writeback: the client applies the edit only if the buffer slice equals expected_old_text. The parent must be a package or requirement.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_create_requirement(
        &self,
        #[doc = "Parent element id (a package or requirement row id)"]
        parent_id: &ElementId,
        #[doc = "Requirement name (identifier)"]
        name: &str,
        #[doc = "Optional requirement ID (the spec's declaredShortName, rendered <'…'>)"]
        short_name: Option<String>,
        #[doc = "Optional statement text (rendered as a doc comment)"]
        doc: Option<String>,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, parent_id)?;
        field_edit::compute_create_requirement(
            &graph,
            &content,
            parent_id,
            short_name.as_deref(),
            name,
            doc.as_deref(),
        )
    }

    /// Compute a guarded insertion edit adding a `doc /* … */` member to an
    /// element that has none (editing an existing one is
    /// `sysml.workspace.edit_requirement_doc`).
    #[service_command(
        name = "sysml.workspace.add_requirement_doc",
        category = Query,
        description = "Compute a guarded text edit adding a doc comment to an element that has none. Fails when a doc comment already exists (use edit_requirement_doc). Buffer-writeback with an expected_old_text guard.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_add_requirement_doc(
        &self,
        #[doc = "Element id gaining the doc comment (a requirement row id)"]
        element_id: &ElementId,
        #[doc = "Doc body text"]
        new_text: &str,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, element_id)?;
        field_edit::compute_add_doc(&graph, &content, element_id, new_text)
    }

    /// Compute a guarded insertion edit adding `@StatusInfo` maturity
    /// metadata to an element that has none (editing an existing one is
    /// `sysml.workspace.edit_requirement_maturity`).
    #[service_command(
        name = "sysml.workspace.add_requirement_maturity",
        category = Query,
        description = "Compute a guarded text edit adding @StatusInfo { status = StatusKind::<status> } to an element that has none (closed vocabulary: open|tbd|tbr|tbc|done|closed). Fails when @StatusInfo already exists (use edit_requirement_maturity). Buffer-writeback with an expected_old_text guard.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_add_requirement_maturity(
        &self,
        #[doc = "Element id gaining the maturity metadata (a requirement row id)"]
        element_id: &ElementId,
        #[doc = "Maturity: one of open|tbd|tbr|tbc|done|closed"]
        status: &str,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, element_id)?;
        field_edit::compute_add_maturity(&graph, &content, element_id, status)
    }

    /// Compute a guarded insertion edit adding a `<keyword> <name> : <Type>;`
    /// typed parameter membership (subject / actor / stakeholder / framed
    /// concern) to a requirement (workbench design §7.7). The target kind is
    /// validated per role (subject = any definition; actor/stakeholder = part
    /// definition; concern = concern definition); subject is singleton.
    #[service_command(
        name = "sysml.workspace.add_requirement_role",
        category = Query,
        description = "Compute a guarded text edit adding a `<keyword> <name> : <Type>;` member (role = \"subject\"|\"actor\"|\"stakeholder\"|\"concern\") to a requirement. type_id references the definition: subject accepts any definition, actor/stakeholder a part definition, concern a concern definition. Subject is singleton (fails when one exists). name is an identifier. Buffer-writeback with an expected_old_text guard.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_add_requirement_role(
        &self,
        #[doc = "Requirement element id gaining the role"]
        requirement_id: &ElementId,
        #[doc = "Role: \"subject\" | \"actor\" | \"stakeholder\" | \"concern\""]
        role: &str,
        #[doc = "Element id of the referenced definition"]
        type_id: &ElementId,
        #[doc = "Local parameter name (identifier)"]
        name: &str,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let role = field_edit::RequirementRoleKind::parse(role).ok_or_else(|| {
            ServiceError::InvalidInput(format!(
                "role must be \"subject\", \"actor\", \"stakeholder\", or \"concern\", got '{role}'"
            ))
        })?;
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, requirement_id)?;
        field_edit::compute_add_requirement_role(&graph, &content, requirement_id, role, type_id, name)
    }

    /// Compute a guarded insertion edit adding an `assume/require constraint
    /// [name] { <expr> }` member to a requirement (workbench design §7.7).
    /// The expression is spliced verbatim; editing an existing constraint
    /// expression is deferred (§7.4).
    #[service_command(
        name = "sysml.workspace.add_constraint",
        category = Query,
        description = "Compute a guarded text edit adding an `assume/require constraint [name] { <expr> }` member to a requirement. kind is \"assume\" or \"require\"; expr is a single-line boolean expression (no braces or `;`); name (optional) is an identifier. Buffer-writeback with an expected_old_text guard.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_add_constraint(
        &self,
        #[doc = "Requirement element id gaining the constraint"]
        element_id: &ElementId,
        #[doc = "Constraint kind: \"assume\" or \"require\""]
        kind: &str,
        #[doc = "Single-line boolean expression (no braces or `;`)"]
        expr: &str,
        #[doc = "Optional constraint name (identifier)"]
        name: Option<String>,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let is_assume = match kind.trim() {
            "assume" => true,
            "require" => false,
            other => {
                return Err(ServiceError::InvalidInput(format!(
                    "constraint kind must be \"assume\" or \"require\", got '{other}'"
                )))
            }
        };
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, element_id)?;
        field_edit::compute_add_constraint(
            &graph,
            &content,
            element_id,
            is_assume,
            name.as_deref(),
            expr,
        )
    }

    /// Compute a guarded insertion edit adding an `attribute <name> [= <value>];`
    /// member to an element (workbench design §7.7). Editing an existing value
    /// is `sysml.workspace.edit_attribute_value`.
    #[service_command(
        name = "sysml.workspace.add_attribute",
        category = Query,
        description = "Compute a guarded text edit adding an `attribute <name> [= <value>];` member to an element. Name must be a valid identifier; value (optional) is a single-line expression (no `;`). Fails when an attribute of that name already exists (edit its value instead). Buffer-writeback with an expected_old_text guard.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_add_attribute(
        &self,
        #[doc = "Element id gaining the attribute (a requirement row id)"]
        element_id: &ElementId,
        #[doc = "Attribute name (identifier)"]
        name: &str,
        #[doc = "Optional value expression (single line, no `;`)"]
        value: Option<String>,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, element_id)?;
        field_edit::compute_add_attribute(&graph, &content, element_id, name, value.as_deref())
    }

    /// Compute a guarded insertion edit adding a `@Rationale { text = "…" }`
    /// metadata member to an element (workbench design §7.7). Add-only — a
    /// requirement may carry several rationale annotations.
    #[service_command(
        name = "sysml.workspace.add_rationale",
        category = Query,
        description = "Compute a guarded text edit adding a @Rationale { text = \"…\" } metadata member to an element. Add-only (a requirement may carry several rationale annotations; the read side joins them). Text must be single-line non-blank; embedded quotes/backslashes are escaped. Buffer-writeback with an expected_old_text guard.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_add_rationale(
        &self,
        #[doc = "Element id gaining the rationale (a requirement row id)"]
        element_id: &ElementId,
        #[doc = "Rationale text (single line)"]
        text: &str,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, element_id)?;
        field_edit::compute_add_rationale(&graph, &content, element_id, text)
    }

    /// Compute a guarded insertion edit writing `satisfy <req>;` into the
    /// picked subject's body (workbench design §7.6 — the elaborator reads
    /// the statement's owner as the satisfyingFeature).
    #[service_command(
        name = "sysml.workspace.add_satisfy_link",
        category = Query,
        description = "Compute a guarded text edit inserting `satisfy <requirement>;` at the end of the picked subject element's body (the subject's file — possibly a different file than the requirement's). The reference is the requirement's simple name when it is a sibling scope member, its fully qualified name otherwise. Fails hard when the satisfy link already exists. Buffer-writeback with an expected_old_text guard.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_add_satisfy_link(
        &self,
        #[doc = "The satisfied requirement's element id (a requirement row id)"]
        requirement_id: &ElementId,
        #[doc = "The satisfying element's id (typically a part usage — the insertion target)"]
        subject_id: &ElementId,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, subject_id)?;
        field_edit::compute_add_satisfy_link(&graph, &content, requirement_id, subject_id)
    }

    /// Compute a guarded insertion edit writing `verify <req>;` into the
    /// picked verification case's `objective` body — the only spec-legal
    /// home; a case without an objective gains the whole
    /// `objective { verify …; }` block in one insertion (design §7.6).
    #[service_command(
        name = "sysml.workspace.add_verify_link",
        category = Query,
        description = "Compute a guarded text edit inserting `verify <requirement>;` into the picked verification case's objective body (the case's file). A case with no objective gets the whole `objective { verify <requirement>; }` block. Fails hard when the case already verifies the requirement, or when the target is not a verification case. Buffer-writeback with an expected_old_text guard.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_add_verify_link(
        &self,
        #[doc = "The verified requirement's element id (a requirement row id)"]
        requirement_id: &ElementId,
        #[doc = "The verifying case's element id (a verification case def/usage — the insertion target)"]
        case_id: &ElementId,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, case_id)?;
        field_edit::compute_add_verify_link(&graph, &content, requirement_id, case_id)
    }

    /// Compute a guarded insertion edit writing a `#derivation connection`
    /// block (Requirement Derivation domain library — the core grammar has
    /// no derive keyword) into the DERIVED requirement's owning package;
    /// prepends the load-bearing `private import RequirementDerivation::*;`
    /// when the owning-package chain lacks one (design §7.6).
    #[service_command(
        name = "sysml.workspace.add_derive_link",
        category = Query,
        description = "Compute a guarded text edit inserting `#derivation connection { end #original ::> <original>; end #derive ::> <derived>; }` at the end of the derived requirement's owning package. Prepends `private import RequirementDerivation::*;` in the same insertion when the owning-package chain lacks one (the import is load-bearing for Derive elaboration). Fails hard when the derive link already exists. requirement_id is the DERIVED end; a 'derived to' add swaps the roles client-side. Buffer-writeback with an expected_old_text guard.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_add_derive_link(
        &self,
        #[doc = "The DERIVED requirement's element id (the row being edited)"]
        requirement_id: &ElementId,
        #[doc = "The ORIGINAL requirement's element id (what it derives from)"]
        original_id: &ElementId,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, requirement_id)?;
        field_edit::compute_add_derive_link(&graph, &content, requirement_id, original_id)
    }

    /// Compute a guarded insertion edit writing a `dependency … { @Refinement; }`
    /// into the REFINING requirement's owning package (workbench design §7.6 —
    /// refine is a KerML Dependency + ModelingMetadata::Refinement, not a
    /// keyword); prepends the load-bearing `private import ModelingMetadata::*;`
    /// when the owning-package chain lacks one.
    #[service_command(
        name = "sysml.workspace.add_refine_link",
        category = Query,
        description = "Compute a guarded text edit inserting `dependency from <refining> to <refined> { @Refinement; }` at the end of the refining requirement's owning package. Prepends `private import ModelingMetadata::*;` in the same insertion when the owning-package chain lacks one (the import is load-bearing for Refine elaboration). requirement_id is the REFINING end (the row's outgoing `refines`). Fails hard when the refine link already exists. Buffer-writeback with an expected_old_text guard.",
        returns = "FieldEditComputed { uri, element_id, field, edit: TextEdit }",
    )]
    pub fn workspace_add_refine_link(
        &self,
        #[doc = "The REFINING requirement's element id (the row being edited)"]
        requirement_id: &ElementId,
        #[doc = "The REFINED requirement's element id (what it refines)"]
        refined_id: &ElementId,
    ) -> Result<field_edit::FieldEditComputed, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let content = self.file_text_for_element(&graph, requirement_id)?;
        field_edit::compute_add_refine_link(&graph, &content, requirement_id, refined_id)
    }

    /// Current source text of the file that declares `element_id` — resolved
    /// via the element's first span, fetched from the salsa host (the same
    /// text the spans were computed against).
    fn file_text_for_element(
        &self,
        graph: &sysml_core::ModelGraph,
        element_id: &ElementId,
    ) -> Result<String, ServiceError> {
        let elem = graph.get_element(element_id).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("element '{element_id}' not found"))
        })?;
        let uri = elem
            .spans
            .first()
            .map(|s| s.file.clone())
            .ok_or_else(|| {
                ServiceError::InvalidInput(format!(
                    "element '{element_id}' has no source span — not an editable source element"
                ))
            })?;
        let guard = self.host.lock().unwrap();
        let file_id = guard.file_id(&uri).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no loaded file for URI: {uri}"))
        })?;
        let sf = guard.source_file(file_id).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no loaded file for URI: {uri}"))
        })?;
        let analysis = guard.analysis();
        let content = sysml_ide_db::Cancelled::catch(std::panic::AssertUnwindSafe(|| {
            analysis.file_text(sf).to_owned()
        }))
        .map_err(|_| ServiceError::Internal("salsa cancelled".into()))?;
        Ok(content)
    }

    /// Evaluate every verification case in the graph and classify each into
    /// the three-state verdict class `sysml_query` consumes: `Fail|Error →
    /// Fail`, `Inconclusive → Incomplete`, `Pass → Pass`.
    ///
    /// Memoized per graph revision — `evaluate_verification_cases`
    /// compiles-and-runs a `VerificationRunner` per case, and paging through
    /// the requirement table must not re-run verification for every page.
    fn requirement_case_verdicts(
        &self,
        graph: &sysml_core::ModelGraph,
        revision: u64,
    ) -> Arc<HashMap<ElementId, sysml_query::RequirementVerificationState>> {
        if let Some(cached) = self.requirement_verdict_cache.get(&revision) {
            return cached.clone();
        }
        use sysml_runtime::cases::VerdictKind;
        let map: HashMap<ElementId, sysml_query::RequirementVerificationState> =
            evaluation::evaluate_verification_cases(graph)
                .into_iter()
                .map(|result| {
                    let state = match result.verdict {
                        VerdictKind::Pass => sysml_query::RequirementVerificationState::Pass,
                        VerdictKind::Fail | VerdictKind::Error => {
                            sysml_query::RequirementVerificationState::Fail
                        }
                        VerdictKind::Inconclusive => {
                            sysml_query::RequirementVerificationState::Incomplete
                        }
                    };
                    (result.element_id, state)
                })
                .collect();
        let map = Arc::new(map);
        self.requirement_verdict_cache.insert(revision, map.clone());
        map
    }

    /// Multi-URI peer of `sysml.model.tree`. Walks every loaded user file,
    /// asks the per-file `model_tree_query` for its tree projection,
    /// resolves spans to LSP `Position` ranges at the service boundary,
    /// and returns one `{uri, nodes}` entry per loaded URI.
    ///
    /// Bucket B / B4 — the multi-URI driver lives here so all transports
    /// share it; the LSP handler collapses to a thin marshal that
    /// flattens to the editor's existing flat-array wire shape.
    #[service_command(
        name = "sysml.workspace.model_tree",
        category = Query,
        description = "Walk all loaded user files; emit per-URI tree projections with line/character ranges (LSP Position semantics). Deterministic URI ordering for cache stability.",
        returns = "Vec<WorkspaceModelTreeFile>",
    )]
    pub fn workspace_model_tree(
        &self,
        #[doc = "Maximum depth to recurse (omit for unlimited)"] max_depth: Option<usize>,
        #[doc = "Tree view: \"full\" (default, every element kind) or \"user_facing\""]
        view: Option<&str>,
    ) -> Result<Vec<WorkspaceModelTreeFile>, ServiceError> {
        const SYNTHETIC_FILE: &str = "<synthetic>";

        // Lock the host briefly to snapshot (uri, content) for every
        // user-loaded file. Stdlib bundle files are excluded by
        // `user_file_ids`.
        let files_info: Vec<(String, String)> = {
            let host = self.host.lock().unwrap();
            let analysis_for_text = host.analysis();
            host.files()
                .user_file_ids()
                .filter_map(|fid| {
                    let uri = host.files().uri(fid)?.to_string();
                    let sf = host.files().source_file(fid)?;
                    let content = analysis_for_text.file_text(sf).to_owned();
                    Some((uri, content))
                })
                .collect()
        };

        let view_arg = view.unwrap_or("full");
        let mut groups: Vec<WorkspaceModelTreeFile> =
            Vec::with_capacity(files_info.len());

        for (uri, content) in &files_info {
            // Skip URIs where either the tree projection or the graph
            // can't be produced — mirrors the prior LSP "skip URIs that
            // aren't loaded" semantic without swallowing typed errors
            // for callers that ask via the typed surface.
            let Ok(graph) = self.require_graph(uri) else {
                continue;
            };
            let tree = match self.model_tree(uri, max_depth, Some(view_arg)) {
                Ok(t) => t,
                Err(_) => continue,
            };

            let mut nodes: Vec<TreeNodeWithRange> = Vec::with_capacity(tree.len());
            for node in &tree {
                // Skip synthetic-only top-level roots (mirrors prior
                // LSP-69 filter — these are spec-mandated wrappers
                // without a real source span).
                let element_id = node.element_id.clone().unwrap_or_else(|| node.id.clone());
                if let Some(element) = graph.elements.get(&element_id) {
                    if element.spans.iter().all(|s| s.file == SYNTHETIC_FILE) {
                        continue;
                    }
                }
                nodes.push(build_tree_node_with_range(node, &graph, uri, content));
            }

            groups.push(WorkspaceModelTreeFile {
                uri: uri.clone(),
                nodes,
            });
        }

        // Deterministic per-URI ordering for cacheability.
        groups.sort_by(|a, b| a.uri.cmp(&b.uri));

        Ok(groups)
    }

    /// Project expression ASTs from the model graph for a single element
    /// (by id) or every expression-bearing element (when `element_id` is
    /// `None`). Pure structural projection — no evaluation.
    #[service_command(
        name = "sysml.expression.ast",
        category = Query,
        description = "Project expression element subtrees as JSON ASTs for rendering (KaTeX) and inspection",
        returns = "Vec<ExpressionAstResult>",
    )]
    pub fn expression_ast(
        &self,
        #[doc = "Optional element ID to project a single owner (constraint, calc, attribute). Omit to project all."] element_id: Option<&ElementId>,
    ) -> Result<Vec<ExpressionAstResult>, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let results = match element_id {
            Some(id) => expression_ast::project_one(&graph, id).into_iter().collect(),
            None => expression_ast::project_all(&graph),
        };
        Ok(results)
    }

    // -- Eval context --

    /// Build an evaluation context from a loaded graph.
    ///
    /// Uses the workspace graph when available so cross-file values resolve.
    /// Goes through salsa-tracked `workspace_eval_context*` /
    /// `file_eval_context` queries (S2.T17) — repeated calls against an
    /// unchanged graph return a cached `Arc<EvalContext>` instead of
    /// re-walking every named element.
    pub fn eval_context(&self) -> Result<EvalContext, ServiceError> {
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_eval_context_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        Ok(cached.ctx().alias_live())
    }

    /// Build an evaluation context with overrides applied.
    pub fn eval_context_with_overrides(
        &self,
        overrides: &[(String, String)],
    ) -> Result<EvalContext, ServiceError> {
        let mut ctx = self.eval_context()?;
        sysml_runtime::compiler::apply_overrides(&mut ctx, overrides);
        Ok(ctx)
    }

    /// Precompile the workspace's constraint set (salsa-cached).
    ///
    /// Routes through the tracked
    /// `workspace_precompiled_constraints_with_library` query in
    /// `sysml-ide-db`. Subsequent calls against an unchanged graph return
    /// the same `Arc<PrecompiledConstraintSet>` — the extract-and-
    /// precompile walk pays once per graph revision per ADR-011 §3.
    ///
    /// Callers that build a workspace orchestrator and want continuous
    /// constraint monitoring should pass this `Arc` into
    /// `Snapshot::build_workspace_orchestrator`.
    pub fn workspace_precompiled_constraints(
        &self,
    ) -> Result<Arc<sysml_runtime::constraints::PrecompiledConstraintSet>, ServiceError> {
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_precompiled_constraints_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        Ok(cached.arc())
    }

    /// Compile the workspace's port+flow IR bundle (salsa-cached).
    ///
    /// Routes through the tracked
    /// `workspace_port_flow_runtime_with_library` query in `sysml-ide-db`.
    /// Subsequent calls against an unchanged graph return the same
    /// `Arc<PortFlowResources>` — the `compile_ports` + `compile_flows`
    /// graph walks pay once per graph revision per ADR-011 §3
    /// (RT-17 / RT-18 / RT-19, bundled).
    ///
    /// Callers that build a workspace orchestrator should pass this
    /// `Arc` into `Snapshot::build_workspace_orchestrator` so the
    /// orchestrator reuses the cached registry + connections instead of
    /// re-walking the graph.
    pub fn workspace_port_flow_resources(
        &self,
    ) -> Result<Arc<sysml_runtime::flows::PortFlowResources>, ServiceError> {
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_port_flow_runtime_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        Ok(cached.arc())
    }

    /// Compile the workspace's gated-expression bundle (salsa-cached).
    ///
    /// Routes through the tracked
    /// `workspace_gated_expressions_with_library` query in `sysml-ide-db`.
    /// Subsequent calls against an unchanged graph return the same
    /// `Arc<Vec<(String, ExprIR)>>` — `detect_computed_expressions` and
    /// the instance-scoped pair extraction pay once per graph revision
    /// per ADR-011 §3 row `cached_gated_expressions` (RT-16), and the
    /// per-expression `ode_builder::parse_derivative` calls land inside
    /// the cache too.
    ///
    /// Callers that build a workspace orchestrator should pass this
    /// `Arc` into `Snapshot::build_workspace_orchestrator` so the
    /// orchestrator replays the cached entries instead of re-walking
    /// the graph and re-parsing each RHS.
    pub fn workspace_gated_expressions(
        &self,
    ) -> Result<Arc<Vec<sysml_runtime::compiler::GatedExprSpec>>, ServiceError> {
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_gated_expressions_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        Ok(cached.arc())
    }

    /// Compile the workspace's signal-expression table (salsa-cached).
    ///
    /// Routes through the tracked
    /// `workspace_signal_expr_table_with_library` query in `sysml-ide-db`.
    /// Subsequent calls against an unchanged graph return the same
    /// `Arc<Vec<(String, ExprIR)>>` — the ODE detection walk plus the
    /// per-entry `ode_builder::parse_derivative` calls land inside the
    /// cache per ADR-011 §3 row `cached_signal_expr_table` (RT-35).
    ///
    /// Mirrors the in-place behaviour of `simulate.continuous.auto`: returns
    /// the first detected ODE's signal expressions; an empty `Vec` when no ODE
    /// is detected. Callers branch on `is_empty()` and pass the (cloned) `Vec`
    /// into `RuntimeSession::set_signals`.
    pub fn workspace_signal_expr_table(
        &self,
    ) -> Result<Arc<Vec<(String, sysml_runtime::expressions::ExprIR)>>, ServiceError> {
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_signal_expr_table_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        Ok(cached.arc())
    }

    /// Resolve the snapshot-scoped `Arc<Mutex<RefResolveCache>>` for the
    /// workspace `uri`. Returns the salsa-cached handle so every
    /// orchestrator built against the same elaborated-graph revision
    /// shares one populated cache (ADR-011 §6 / S3.T14, replacing the
    /// per-session `Orchestrator.ref_resolve_cache` field, RT-23).
    ///
    /// The salsa query body is just `Arc::new(Mutex::new(HashMap::new()))`
    /// — the value is empty on first call and accumulates compile
    /// results during simulation. When the elaborated graph changes
    /// (file edit), salsa returns a new Arc to a fresh empty cache;
    /// the old populated Arc drops along with its entries.
    ///
    /// Library overlay handling, fallback chain
    /// (`workspace_ref_resolve_cache_with_library` →
    /// `workspace_ref_resolve_cache` → `file_ref_resolve_cache`) mirrors
    /// the other workspace helpers in this file. Helper is not a
    /// `#[service_command]` (no MCP surface needed), so the MCP
    /// coverage gate is unaffected.
    pub fn workspace_ref_resolve_cache(
        &self,
    ) -> Result<
        Arc<std::sync::Mutex<sysml_runtime::expressions::RefResolveCache>>,
        ServiceError,
    > {
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_ref_resolve_cache_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        Ok(cached.arc())
    }

    /// Resolve the salsa-cached `PhysicsDomainRegistry` for the
    /// workspace `uri`. Routes through the tracked
    /// `workspace_physics_registry_with_library` query in `sysml-ide-db`
    /// (ADR-011 §3 / S3.T11). Each call against an unchanged
    /// elaborated-graph revision returns the same
    /// `Arc<PhysicsDomainRegistry>`, so per-port `classify_port_definition`
    /// calls in hover and code-lens land on a populated registry without
    /// re-walking the workspace.
    ///
    /// Helper is not a `#[service_command]` (no MCP surface needed); the
    /// MCP coverage gate is unaffected. Library overlay handling +
    /// fallback chain (with_library → no-library → file-scope) mirrors
    /// the other workspace helpers in this file.
    pub fn workspace_physics_registry(
        &self,
    ) -> Result<Arc<sysml_core::physics::PhysicsDomainRegistry>, ServiceError> {
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_physics_registry_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        Ok(cached.arc())
    }

    /// Render a `ViewRequest` against the graph for `uri`, going
    /// through the salsa `diagram_query` cache (ADR-011 §3 / S3.T5)
    /// when the request is cacheable and the view type produces an
    /// SGraph payload.
    ///
    /// Dispatch:
    /// 1. View types that do *not* produce SGraph (Grid / Geometry /
    ///    Browser) bypass the T5 SGraph cache and fall through to
    ///    `smodel::to_payload_with_filter_cache` (tier 2 below).
    /// 2. Requests carrying `filter` / `hints` / `overlays` (i.e.
    ///    `cache_key()` returns `None`) bypass the cache.
    /// 3. Otherwise the request hits `cached_smodel`, and the SGraph
    ///    is wrapped in `DiagramPayload::Graph` before JSON
    ///    serialisation — the canonical `{"kind":"graph","data":{…}}`
    ///    wire shape (see `graph_payload_json`).
    ///
    /// This helper is what every diagram-rendering service command
    /// (`views_render`, future `diagram_open` etc.) should call.
    pub fn diagram_with_cached(
        &self,
        uri: &str,
        request: &sysml_diagram::ViewRequest,
    ) -> Result<serde_json::Value, ServiceError> {
        // Tier 1: full T5 cache — request must be cacheable AND produce SGraph.
        if visualization::view_type_produces_sgraph(request.view_type) {
            if let Some(sgraph) = self.cached_smodel(request)? {
                return Ok(visualization::graph_payload_json(&sgraph));
            }
        }
        // Tier 2: filter-bearing requests (or hint/overlay-bearing
        // requests) bypass the T5 SGraph cache. Thread the T6b
        // precompiled-filter-expression cache so per-element filter
        // evaluation skips the compile step. Routes through the
        // `view_filter_exprs_best` dispatcher — fail-hard, no soft
        // fallback to uncached `diagram_with()` (Q3 of the resolution-
        //
        // For `__workspace__` (which has no `SourceFile`), pass any
        // tracked user file solely as the dispatcher key — the
        // workspace arms only consult `pfs`, the `source_file` argument
        // is unused.
        let graph = self.workspace_aware_graph()?;
        let project_id = Some(sysml_project::ProjectHandle(
            Self::SERVICE_WORKSPACE_PROJECT_ID,
        ));
        // ONE lock acquisition for snapshot + SourceFile (lock-order
        // invariant on `host_analysis` — the 2026-07-17 wedge).
        let (analysis, sf) = self.locked_analysis_with(|host| {
            host.file_id(uri)
                .and_then(|id| host.source_file(id))
                .or_else(|| {
                    host.files()
                        .user_file_ids()
                        .next()
                        .and_then(|id| host.source_file(id))
                })
        });
        let sf = sf.ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no graph for URI: {uri}"))
        })?;
        let filter_cache = analysis.view_filter_exprs_best(sf, project_id).arc();
        let payload =
            sysml_diagram::smodel::to_payload_with_filter_cache(&graph, request, &filter_cache);
        Ok(serde_json::to_value(&payload).unwrap_or_default())
    }

    /// Resolve the salsa-cached `SGraph` for a diagram render on the
    /// workspace `uri`, when the request shape is cacheable.
    ///
    /// Routes through the tracked `workspace_diagram_with_library`
    /// query in `sysml-ide-db` (ADR-011 §3 / S3.T5). Returns `None`
    /// when the request carries `filter` / `hints` / `overlays` —
    /// those bypass the cache because the key newtype
    /// ([`sysml_diagram::DiagramRequestKey`]) deliberately captures
    /// only `(view_type, expanded_ids, expose)`. Library overlay
    /// handling + fallback chain (with_library → no-library →
    /// file-scope) mirrors the other workspace helpers.
    ///
    /// Returns `Ok(Some(Arc<SGraph>))` when the cache produced or
    /// returned a value; `Ok(None)` when the request bypasses the
    /// cache (caller must fall back to direct `to_smodel_with`);
    /// `Err(...)` when the URI doesn't resolve to a known graph.
    ///
    /// Helper is not a `#[service_command]`; MCP coverage gate
    /// unaffected.
    pub fn cached_smodel(
        &self,
        request: &sysml_diagram::ViewRequest,
    ) -> Result<Option<Arc<sysml_diagram::smodel::SGraph>>, ServiceError> {
        let Some(key) = request.cache_key() else {
            return Ok(None);
        };
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_diagram_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
            key,
        );
        Ok(Some(cached.arc()))
    }

    /// Resolve the salsa-cached [`sysml_diagram::ViewModel`] for a render on the
    /// workspace `uri`, when the request shape is cacheable.
    ///
    /// The ViewModel is the renderer-agnostic wire artifact (Bucket 1) — the
    /// promoted `DiagramIR` scene plus its addenda (design tokens, the
    /// `ElementId↔Span` text-map, interaction descriptors). Routes through the
    /// tracked `workspace_view_model_best` query in `sysml-ide-db`, with the same
    /// library-overlay → no-library → file-scope fallback chain as
    /// [`Self::cached_smodel`]. Returns `Ok(None)` when the request carries
    /// `filter`/`hints`/`overlays` (those bypass the cache key).
    ///
    /// Helper is not a `#[service_command]`; MCP coverage gate unaffected.
    pub fn cached_view_model(
        &self,
        request: &sysml_diagram::ViewRequest,
    ) -> Result<Option<Arc<sysml_diagram::ViewModel>>, ServiceError> {
        let Some(key) = request.cache_key() else {
            return Ok(None);
        };
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_view_model_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
            key,
        );
        Ok(Some(cached.arc()))
    }

    /// Resolve the [`sysml_diagram::ViewModel`] JSON for a scoped render, with the
    /// same two-tier dispatch as [`Self::diagram_with_cached`]:
    ///
    /// 1. Cacheable request → [`Self::cached_view_model`] (the full ViewModel —
    ///    scene + tokens + text-map + interactions + frame).
    /// 2. Filter-bearing request (cache_key `None`) → the uncached pure builder
    ///    `to_view_model_with_filter_cache`, plus the **frame** addendum resolved
    ///    here from the view index (a declared filter-view is a framed-view too,
    ///    spec §8.2.3.26 — without this, filter-only views rendered frameless).
    ///    **Caveat:** the remaining salsa-attached addenda (text-map /
    ///    interactions) are still Tier-1-only. This mirrors the cache-key design
    ///    (`DiagramRequestKey` captures only `(view_type, expanded_ids, expose)`);
    ///    attaching those to the filtered path is a tracked follow-up, not part
    ///    of this command.
    ///
    /// Helper, not a `#[service_command]`; MCP coverage gate unaffected.
    pub fn view_model_with_cached(
        &self,
        uri: &str,
        request: &sysml_diagram::ViewRequest,
    ) -> Result<serde_json::Value, ServiceError> {
        if let Some(view_model) = self.cached_view_model(request)? {
            return Ok(serde_json::to_value(&*view_model).unwrap_or_default());
        }
        // Tier 2: filter-bearing request bypasses the cache. Thread the precompiled
        // filter-expression cache, same as `diagram_with_cached`.
        let graph = self.workspace_aware_graph()?;
        let project_id = Some(sysml_project::ProjectHandle(
            Self::SERVICE_WORKSPACE_PROJECT_ID,
        ));
        // ONE lock acquisition for snapshot + SourceFile (lock-order
        // invariant on `host_analysis` — the 2026-07-17 wedge).
        let (analysis, sf) = self.locked_analysis_with(|host| {
            host.file_id(uri)
                .and_then(|id| host.source_file(id))
                .or_else(|| {
                    host.files()
                        .user_file_ids()
                        .next()
                        .and_then(|id| host.source_file(id))
                })
        });
        let sf = sf.ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no graph for URI: {uri}"))
        })?;
        let filter_cache = analysis.view_filter_exprs_best(sf, project_id).arc();
        let mut view_model =
            sysml_diagram::to_view_model_with_filter_cache(&graph, request, &filter_cache);
        // Frame addendum (§8.2.3.26 / §F-10): the Tier-1 path resolves this in
        // `frame_for_key`; the bypass must attach it too — a declared view is a
        // framed-view whether it scopes by expose or by filter.
        if view_model.frame.is_none() {
            if let Some(view_id) = request.view_id.as_ref() {
                if let Some(summary) = self
                    .workspace_view_index()?
                    .iter()
                    .find(|s| &s.id == view_id)
                {
                    view_model.frame = Some(sysml_diagram::view_frame_from_summary(
                        &graph,
                        summary,
                        request.view_type,
                    ));
                }
            }
        }
        Ok(serde_json::to_value(&view_model).unwrap_or_default())
    }

    /// Resolve the salsa-cached `Vec<ViewSummary>` for the workspace
    /// `uri`. Routes through the tracked
    /// `workspace_view_index_with_library` query in `sysml-ide-db`
    /// (ADR-011 §3 / S3.T6a). Each call against an unchanged
    /// elaborated-graph revision returns the same
    /// `Arc<Vec<ViewSummary>>`, so the FE views panel and every
    /// `views_render` / `views_by_viewpoint` dispatch share the
    /// same materialised list.
    ///
    /// Helper is not a `#[service_command]` (no MCP surface needed);
    /// the MCP coverage gate is unaffected. Library overlay handling
    /// + fallback chain (with_library → no-library → file-scope)
    /// mirrors the other workspace helpers.
    pub fn workspace_view_index(
        &self,
    ) -> Result<Arc<Vec<sysml_core::ViewSummary>>, ServiceError> {
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_view_index_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        Ok(cached.arc())
    }

    /// Resolve the salsa-cached physics health diagnostics for the
    /// workspace `uri`. Routes through the tracked
    /// `workspace_physics_health_with_library` query in `sysml-ide-db`
    /// (ADR-011 §3 / S3.T11). Returns
    /// `Arc<Vec<Diagnostic>>` so callers can filter / convert without
    /// paying the per-edit re-walk of the graph each time.
    ///
    /// Helper is not a `#[service_command]`. Library overlay handling
    /// + fallback chain mirrors the other workspace helpers.
    pub fn workspace_physics_health(
        &self,
    ) -> Result<Arc<Vec<sysml_span::Diagnostic>>, ServiceError> {
        // Guard-drop discipline: snapshot + PFS under one brief lock, then
        // run the salsa query lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let cached = sysml_ide_db::workspace_physics_health_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        Ok(cached.arc())
    }

    // -- Execution operations --

    /// Start a state machine simulation session.
    ///
    /// Compiles the named state machine from the graph at `uri`, creates a
    /// `SimulationSession`, and returns the session key plus the initial step result.
    ///
    /// Per-kind primitive composed by [`sessions_create`], which is the unified
    /// client entry point (server-side kind inference, one `SessionSummary`
    /// shape). New clients should call `sessions.create`; this command remains
    /// for transport stability and as the simulation building block.
    #[service_command(
        name = "sysml.simulate.start",
        category = Execution,
        description = "Compile a state machine from the model and start a simulation session",
        returns = "(session_key: string, StepResult)",
        stateful = true,
    )]
    pub fn simulate_start(
        &self,
        #[doc = "URI of the loaded model containing the state machine"] uri: &str,
        #[doc = "Name of the state machine definition to simulate"] sm_name: &str,
    ) -> Result<(ElementId, sysml_runtime::StepResult), ServiceError> {
        // Use workspace graph so imported types/values from other files resolve.
        let graph = self.workspace_aware_graph()?;

        self.cap_check(execution::SessionKind::Simulation)?;

        // Resolve the SM's source ElementId — the same state-definition (or
        // usage) `compile_state_machine` selects by name — so the simulation
        // overlay (`sysml.diagram.sim_overlay`, Bucket 1.8) can join the active
        // subsystem to its scene node by id (ADR-006 topology-overlay hook).
        // Resolved before `graph` is moved into the snapshot below.
        let sm_element_id = {
            let ids = graph.lookup_by_name(sm_name);
            ids.iter()
                .find(|id| {
                    graph
                        .get_element(id)
                        .is_some_and(|e| e.kind == sysml_core::ElementKind::StateDefinition)
                })
                .or_else(|| {
                    ids.iter().find(|id| {
                        graph
                            .get_element(id)
                            .is_some_and(|e| e.kind == sysml_core::ElementKind::StateUsage)
                    })
                })
                .cloned()
        };

        let snap = self.execution_snapshot(uri)?;
        // Ledger L44: `build_sm_orchestrator` mints/binds slots (the bare
        // `add_state_machine` this used to call directly never did), so a
        // transition effect's attribute assignment routes through a real
        // slot and survives into every snapshot/diff instead of silently
        // vanishing.
        let mut orchestrator = snap
            .build_sm_orchestrator(sm_name, None, None)
            .map_err(|e| ServiceError::Execution(e.message))?;
        if let Some(id) = sm_element_id {
            orchestrator.set_last_source_element_id(id);
        }
        let mut session = execution::RuntimeSession::new(
            orchestrator,
            uri.to_owned(),
            execution::SessionKind::Simulation,
            Some(sm_name.to_owned()),
        );
        session.provenance = Some(self.capture_session_provenance()?);
        let snapshot = session.step();

        // Convert snapshot to StepResult for backward compatibility
        let initial = snapshot_to_step_result(&snapshot, sm_name);

        let key = execution::new_session_id();
        self.insert_session(key.clone(), session);
        Ok((key, initial))
    }

    /// Step a simulation session forward.
    #[service_command(
        name = "sysml.simulate.step",
        category = Execution,
        description = "Advance a simulation session by one step with an optional event",
        returns = "StepResult",
        stateful = true,
    )]
    pub fn simulate_step(
        &self,
        #[doc = "Session key returned by simulate.start"] session_key: &str,
        #[doc = "Optional event to inject (e.g. 'timer', 'buttonPress')"] event: Option<&str>,
    ) -> Result<sysml_runtime::StepResult, ServiceError> {
        let key = ElementId::from_string(session_key);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no simulation session: {session_key}"))
        })?;
        let sm_name = entry
            .subsystem_name
            .clone()
            .ok_or_else(|| {
                ServiceError::Execution(format!(
                    "session {session_key} has no state machine subsystem"
                ))
            })?;
        if let Some(ev) = event {
            entry.orchestrator.inject_event(&sm_name, ev);
        }
        let snapshot = entry.step();
        Ok(snapshot_to_step_result(&snapshot, &sm_name))
    }

    /// Stop and remove a simulation session.
    ///
    /// Thin wrapper over `sessions_stop` kept for transport stability with
    /// clients that still call `sysml.simulate.stop`.
    #[service_command(
        name = "sysml.simulate.stop",
        category = Execution,
        description = "Terminate a simulation session and release its resources",
        returns = "()",
        stateful = true,
    )]
    pub fn simulate_stop(
        &self,
        #[doc = "Session key returned by simulate.start"] session_key: &str,
    ) -> Result<(), ServiceError> {
        self.sessions_stop(session_key)
    }

    /// Start an action execution session.
    ///
    /// Compiles the named action from the graph at `uri` and returns the session key.
    ///
    /// Per-kind primitive composed by [`sessions_create`] (the unified client
    /// entry point). New clients should call `sessions.create`; this remains for
    /// transport stability and as the action building block.
    #[service_command(
        name = "sysml.action.start",
        category = Execution,
        description = "Compile an action from the model and start an execution session",
        returns = "string (session_key)",
        stateful = true,
    )]
    pub fn action_start(
        &self,
        #[doc = "URI of the loaded model containing the action"] uri: &str,
        #[doc = "Name of the action definition to execute"] action_name: &str,
    ) -> Result<ElementId, ServiceError> {
        self.cap_check(execution::SessionKind::Action)?;

        let snap = self.execution_snapshot(uri)?;
        let action_ir = snap.compile_action(action_name)
            .map_err(|e| ServiceError::Execution(e.message))?;

        let runner = sysml_runtime::actions::ActionRunner::new(action_ir);
        let mut orchestrator = sysml_runtime::orchestrator::Orchestrator::new(Default::default());
        orchestrator.add_action(action_name, runner);
        let mut session = execution::RuntimeSession::new(
            orchestrator,
            uri.to_owned(),
            execution::SessionKind::Action,
            Some(action_name.to_owned()),
        );
        session.provenance = Some(self.capture_session_provenance()?);
        let key = execution::new_session_id();
        self.insert_session(key.clone(), session);
        Ok(key)
    }

    /// Step an action session forward.
    #[service_command(
        name = "sysml.action.step",
        category = Execution,
        description = "Step an action session forward, returning the trace entry for the executed node",
        returns = "ActionTraceEntry",
        stateful = true,
    )]
    pub fn action_step(
        &self,
        #[doc = "Session key returned by action.start"] session_key: &str,
    ) -> Result<execution::ActionTraceEntry, ServiceError> {
        let key = ElementId::from_string(session_key);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no action session: {session_key}"))
        })?;
        let action_name = entry
            .subsystem_name
            .clone()
            .ok_or_else(|| {
                ServiceError::Execution(format!(
                    "session {session_key} has no action subsystem"
                ))
            })?;
        let snapshot = entry.step();
        Ok(snapshot_to_action_trace(&snapshot, &action_name))
    }

    /// Start an orchestrator session for a model URI.
    ///
    /// **Deprecated** — a weaker duplicate of [`orchestrate_workspace_start`].
    /// It used to open-code its own `build_workspace_orchestrator` call with an
    /// uncached `context_from_graph` seed and no `ref_resolve_cache`, skipping
    /// the salsa-cached seeds ADR-011 §3 mandates. It now forwards to
    /// `orchestrate.workspace.start` so there is a single orchestrator-build
    /// home (principle #4 / #5); the response shape `(session_key,
    /// ExecutionSnapshot)` is unchanged. New callers should use
    /// `sessions.create` (server-side kind inference, one `SessionSummary`
    /// shape).
    #[service_command(
        name = "sysml.orchestrate.start",
        category = Execution,
        description = "Start a multi-subsystem orchestrator session from the model",
        returns = "(session_key: string, ExecutionSnapshot)",
        stateful = true,
        deprecated = true,
    )]
    pub fn orchestrate_start(
        &self,
        #[doc = "URI of the loaded model with multiple subsystems"] uri: &str,
    ) -> Result<(ElementId, sysml_runtime::orchestrator::ExecutionSnapshot), ServiceError> {
        self.orchestrate_workspace_start(uri, None, None, None)
    }

    /// Step an orchestrator session forward.
    #[service_command(
        name = "sysml.orchestrate.step",
        category = Execution,
        description = "Advance all subsystems in the orchestrator by one tick",
        returns = "ExecutionSnapshot",
        stateful = true,
    )]
    pub fn orchestrate_step(
        &self,
        #[doc = "Session key returned by orchestrate.start"] session_key: &str,
    ) -> Result<sysml_runtime::orchestrator::ExecutionSnapshot, ServiceError> {
        let key = ElementId::from_string(session_key);
        let mut entry =
            self.sessions
                .get_mut(&key)
                .ok_or_else(|| {
                    ServiceError::ElementNotFound(format!(
                        "no orchestrator session: {session_key}"
                    ))
                })?;
        Ok(entry.step())
    }

    /// Inject an event into an orchestrator subsystem and step.
    #[service_command(
        name = "sysml.orchestrate.inject",
        category = Execution,
        description = "Inject an event into a specific subsystem and advance the orchestrator",
        returns = "ExecutionSnapshot",
        stateful = true,
    )]
    pub fn orchestrate_inject(
        &self,
        #[doc = "Session key returned by orchestrate.start"] session_key: &str,
        #[doc = "Name of the subsystem to inject the event into"] subsystem: &str,
        #[doc = "Event to inject"] event: &str,
    ) -> Result<sysml_runtime::orchestrator::ExecutionSnapshot, ServiceError> {
        let key = ElementId::from_string(session_key);
        let mut entry =
            self.sessions
                .get_mut(&key)
                .ok_or_else(|| {
                    ServiceError::ElementNotFound(format!(
                        "no orchestrator session: {session_key}"
                    ))
                })?;
        Ok(entry.inject_event(subsystem, event))
    }

    /// Start a workspace-level orchestrator session.
    ///
    /// Discovers ALL state machines and ODE subsystems in the workspace graph
    /// and wires them into a single orchestrator. This is the entry point for
    /// large multi-file, multi-subsystem models.
    ///
    /// The orchestrator primitive composed by [`sessions_create`] for no-target
    /// and ODE-coupled runs. New clients should call `sessions.create`; this
    /// remains for transport stability and as the orchestrator building block.
    #[service_command(
        name = "sysml.orchestrate.workspace.start",
        category = Execution,
        description = "Compile all subsystems from the workspace and start an orchestrator session",
        returns = "(session_key: string, ExecutionSnapshot)",
        stateful = true,
    )]
    pub fn orchestrate_workspace_start(
        &self,
        #[doc = "URI of the loaded workspace (or __workspace__)"] uri: &str,
        #[doc = "Time step in milliseconds (default 1.0)"] dt_ms: Option<f64>,
        #[doc = "Maximum simulation time in milliseconds (default 60000.0)"] max_time_ms: Option<f64>,
        #[doc = "Parameter overrides applied while BUILDING the orchestrator, so they are in force at tick 0 (scenario setup). An unknown target is a hard error, never a silent no-op. For changing a parameter mid-run use sessions.step's overrides instead."]
        overrides: Option<&[(String, String)]>,
    ) -> Result<(ElementId, sysml_runtime::orchestrator::ExecutionSnapshot), ServiceError> {
        self.cap_check(execution::SessionKind::Orchestrator)?;

        // Salsa-cached seed + constraint precompile: workspace graph +
        // library overlay per ADR-011 §3. Both walks free after the
        // first call. Constraint set is wired into the orchestrator so
        // per-tick `evaluate_constraints` surfaces violations in
        // ExecutionSnapshot.
        let base_ctx = self.eval_context_with_overrides(&[])?;
        let precompiled = self.workspace_precompiled_constraints()?;
        let port_flow = self.workspace_port_flow_resources()?;
        let gated = self.workspace_gated_expressions()?;
        let ref_cache = self.workspace_ref_resolve_cache()?;
        let snap = self.execution_snapshot(uri)?;

        let orchestrator = snap
            .build_workspace_orchestrator(
                base_ctx,
                Some(precompiled),
                Some(port_flow),
                Some(gated),
                Some(ref_cache),
                overrides.unwrap_or(&[]),
                dt_ms,
                max_time_ms,
            )
            // An override naming nothing in the model is caller input, not an
            // internal fault: `build_workspace_orchestrator` validates every
            // target against the ODE parameters, state variables and context
            // variables and fails hard with the legal target list. Map it to
            // InvalidInput (400) so a typo'd scenario key reads as a typo
            // rather than a server error. Same reasoning as the step path's
            // RS002 mapping.
            .map_err(|e| {
                if overrides.is_some_and(|o| !o.is_empty())
                    && e.message.contains("unknown override target")
                {
                    ServiceError::InvalidInput(e.message)
                } else {
                    ServiceError::Execution(e.message)
                }
            })?;

        let mut session = execution::RuntimeSession::new(
            orchestrator,
            uri.to_owned(),
            execution::SessionKind::Orchestrator,
            None,
        );
        session.provenance = Some(self.capture_session_provenance()?);
        // Scenario provenance. Recorded BEFORE the seed step below, so the
        // very first snapshot a client sees already carries the scenario that
        // produced it.
        session.create_overrides = overrides.unwrap_or(&[]).to_vec();
        let snapshot = session.step();

        let key = execution::new_session_id();
        self.insert_session(key.clone(), session);
        Ok((key, snapshot))
    }

    /// Stop and remove an orchestrator session.
    #[service_command(
        name = "sysml.orchestrate.stop",
        category = Execution,
        description = "Terminate an orchestrator session and release its resources",
        returns = "()",
        stateful = true,
    )]
    pub fn orchestrate_stop(
        &self,
        #[doc = "Session key returned by orchestrate.start"] session_key: &str,
    ) -> Result<(), ServiceError> {
        // Thin wrapper over `sessions_stop` kept for transport stability.
        self.sessions_stop(session_key)
    }

    // `sysml.simulate.continuous.start` was DELETED in the execution-entry
    // unification arc (execution-entry-unification-plan.md P5, Decision 6). It
    // took ODE derivative/signal expressions as raw strings from the caller,
    // bypassing the model's own declared `StateSpaceRepresentation` —
    // model-bypassing invented semantics (principle #3). Its only non-explicit
    // behaviour (empty `ode_state_vars`) merely duplicated `continuous_auto`.
    // The spec-faithful continuous path is model-driven: `continuous_auto`
    // (auto-discovery from `@ToolExecution`-annotated metadata) or the workspace
    // orchestrator (`sessions.create` / `orchestrate.workspace.start`).

    /// Start a continuous simulation by auto-discovering ODE configuration from model metadata.
    ///
    /// Scans the loaded model for elements annotated with
    /// `@ToolExecution { toolName = "builtin:ode-rk4" }` and extracts state variables,
    /// initial values, and ODE parameters from the element's attributes. No explicit
    /// ODE parameters need to be provided by the client.
    #[service_command(
        name = "sysml.simulate.continuous.auto",
        category = Execution,
        description = "Start a continuous simulation auto-discovering ODE config from model metadata",
        returns = "json",
        stateful = true,
    )]
    pub fn continuous_auto(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Name of the state machine definition to simulate"] sm_name: &str,
        #[doc = "Time step in milliseconds (default 100.0)"] dt_ms: Option<f64>,
        #[doc = "Maximum simulation time in milliseconds (default 30000.0)"] max_time_ms: Option<f64>,
    ) -> Result<serde_json::Value, ServiceError> {
        self.cap_check(execution::SessionKind::Orchestrator)?;

        let snap = self.execution_snapshot(uri)?;
        let orchestrator = snap.build_orchestrator(sm_name, &[], dt_ms, max_time_ms)
            .map_err(|e| ServiceError::Execution(e.message))?;

        let mut session = execution::RuntimeSession::new(
            orchestrator,
            uri.to_owned(),
            execution::SessionKind::Orchestrator,
            Some(sm_name.to_owned()),
        );
        session.provenance = Some(self.capture_session_provenance()?);

        // Wire signal expressions for display sync (time-varying params show live values).
        // Salsa-cached per ADR-011 §3 RT-35 cached_signal_expr_table.
        let signals = self.workspace_signal_expr_table()?;
        if !signals.is_empty() {
            session.set_signals((*signals).clone());
        }

        let key = execution::new_session_id();
        self.insert_session(key.clone(), session);

        Ok(serde_json::json!({
            "session_key": key,
            "time_ms": 0.0,
        }))
    }

    // ------------------------------------------------------------------
    // Session catalog commands — the UX-facing contract.
    // ------------------------------------------------------------------

    /// Create an execution session, inferring its kind from the model.
    ///
    /// The unified creation entry point: the server resolves `target` against
    /// the workspace graph and dispatches to the right kind so the client never
    /// has to know the `simulate.start` / `action.start` /
    /// `orchestrate.workspace.start` taxonomy (the FE's old `commandForTarget`
    /// decision tree moves here). Always returns a `SessionSummary` — one shape,
    /// vs the divergent tuple/string/object shapes the kind-specific `*.start`
    /// commands return.
    ///
    /// Resolution:
    /// - `target` omitted, empty, or `__workspace__` ⇒ multi-subsystem
    ///   orchestrator over the whole workspace.
    /// - `target` names a state machine ⇒ a single-SM **simulation**, unless the
    ///   workspace has more than one state machine or multiple subsystems, in
    ///   which case the whole orchestrator runs so every subsystem advances in
    ///   lockstep (matching the FE's prior multi-SM rule).
    /// - `target` names an action ⇒ an **action** session.
    /// - `target` names any other element (part, case, …) ⇒ orchestrator.
    /// - `target` names nothing in the model ⇒ hard `ElementNotFound` (fail hard;
    ///   no silent fallback to the workspace orchestrator).
    #[service_command(
        name = "sysml.sessions.create",
        category = Execution,
        description = "Create an execution session, inferring the kind (simulation / action / orchestrator) from the model and optional target. Unified entry point subsuming the *.start commands.",
        returns = "SessionSummary",
        stateful = true,
    )]
    pub fn sessions_create(
        &self,
        #[doc = "URI of the loaded model, or __workspace__ for the merged workspace"] uri: &str,
        #[doc = "Optional target: a state-machine or action name. Omit (or pass __workspace__) to run the whole multi-subsystem workspace orchestrator."]
        target: Option<&str>,
        #[doc = "Time step in milliseconds (orchestrator sessions; default 1.0)"] dt_ms: Option<f64>,
        #[doc = "Maximum simulation time in milliseconds (orchestrator sessions; default 60000.0)"]
        max_time_ms: Option<f64>,
        #[doc = "Scenario overrides applied while BUILDING the session, so they hold from tick 0. Orchestrator sessions only. An unknown target is a hard error. Recorded on the session as create_overrides and archived on stop; use sessions.step's overrides to change a parameter mid-run instead."]
        overrides: Option<&[(String, String)]>,
    ) -> Result<execution::SessionSummary, ServiceError> {
        enum Pick {
            Simulation(String),
            Action(String),
            Orchestrator,
        }

        let named = target
            .map(str::trim)
            .filter(|t| !t.is_empty() && *t != WORKSPACE_URI);

        let pick = match named {
            None => Pick::Orchestrator,
            Some(name) => {
                use sysml_core::ElementKind as K;
                let graph = self.workspace_aware_graph()?;
                let kinds: Vec<K> = graph
                    .lookup_by_name(name)
                    .iter()
                    .filter_map(|id| graph.get_element(id).map(|e| e.kind.clone()))
                    .collect();
                if kinds.is_empty() {
                    return Err(ServiceError::ElementNotFound(format!(
                        "no model element named '{name}' to run; pass __workspace__ \
                         or omit `target` to run the whole workspace"
                    )));
                }
                let is_state = kinds.iter().any(|k| {
                    matches!(k, K::StateDefinition | K::StateUsage | K::ExhibitStateUsage)
                });
                let is_action =
                    kinds.iter().any(|k| matches!(k, K::ActionDefinition | K::ActionUsage));
                if is_state {
                    // A lone state machine runs as a lightweight simulation; one
                    // coupled to continuous dynamics (or living in a multi-file
                    // workspace) must run under the orchestrator so the ODE and
                    // every other subsystem advance in lockstep — otherwise an
                    // SM-only session would ignore the physics it's driven by.
                    // `has_ode_dynamics` is the reliable signal here (the
                    // library-overlaid SM *count* is not — it includes stdlib
                    // state definitions).
                    if self.workspace_capabilities()?.has_ode_dynamics {
                        Pick::Orchestrator
                    } else {
                        Pick::Simulation(name.to_string())
                    }
                } else if is_action {
                    Pick::Action(name.to_string())
                } else {
                    Pick::Orchestrator
                }
            }
        };

        // Create-time overrides are an orchestrator-build capability
        // (`build_workspace_orchestrator` seeds and validates them). The
        // single-SM and action builders have no equivalent, so asking for one
        // there is refused outright rather than accepted and silently dropped
        // — a dropped scenario override is the worst possible failure here:
        // the run looks configured and is not.
        let create_overrides: &[(String, String)] = overrides.unwrap_or(&[]);
        if !create_overrides.is_empty() && !matches!(pick, Pick::Orchestrator) {
            return Err(ServiceError::InvalidInput(
                "create-time overrides are supported for orchestrator sessions only; \
                 the named target resolves to a single state machine or action, whose \
                 builder has no override seeding. Omit `target` (or pass __workspace__) \
                 to run the whole workspace with this scenario, or apply the override \
                 with sessions.step once the session exists"
                    .to_owned(),
            ));
        }

        let id = match pick {
            Pick::Simulation(sm) => self.simulate_start(uri, &sm)?.0,
            Pick::Action(act) => self.action_start(uri, &act)?,
            Pick::Orchestrator => {
                self.orchestrate_workspace_start(uri, dt_ms, max_time_ms, overrides)?
                    .0
            }
        };

        let entry = self.sessions.get(&id).ok_or_else(|| {
            ServiceError::Execution("session not found immediately after creation".to_owned())
        })?;
        Ok(entry.summary(id.to_string()))
    }

    /// List all live runtime sessions as serializable summaries.
    #[service_command(
        name = "sysml.sessions.list",
        category = Execution,
        description = "List all live runtime sessions (state-machine, action, orchestrator) as typed summaries",
        returns = "Vec<SessionSummary>",
        stateful = true,
    )]
    pub fn sessions_list(&self) -> Result<Vec<execution::SessionSummary>, ServiceError> {
        let mut out: Vec<execution::SessionSummary> = self
            .sessions
            .iter()
            .map(|entry| {
                let id = entry.key().to_string();
                entry.value().summary(id)
            })
            .collect();
        // Deterministic order: oldest first by creation time.
        out.sort_by_key(|s| s.created_at_ms);
        Ok(out)
    }

    /// Fetch full detail for a single session, including the latest snapshot.
    #[service_command(
        name = "sysml.sessions.info",
        category = Execution,
        description = "Return full detail for a session, including subsystems and the latest snapshot",
        returns = "Option<SessionDetail>",
        stateful = true,
    )]
    pub fn sessions_info(
        &self,
        #[doc = "Session id (UUID) returned by any *.start command"] session_id: &str,
        #[doc = "When false, strip the 'variables' map from latest_snapshot. A large multi-subsystem workspace has ~14k variables (~920 KB JSON); skipping them cuts this payload by 99% and is the right default for polling callers that only need tick / current_state / subsystem summaries. Defaults to true for backward compatibility."]
        include_variables: Option<bool>,
    ) -> Result<Option<execution::SessionDetail>, ServiceError> {
        let keep_vars = include_variables.unwrap_or(true);
        let key = ElementId::from_string(session_id);
        // NB: polling `sessions.info` deliberately does NOT refresh the idle
        // clock. Expiry tracks SESSION inactivity (no step/inject/reset), so a
        // paused session a client merely watches is still "inactive" and will
        // be reaped — and `info` can report `is_expired: true` to drive the
        // stale-session banner. Only advancement (`step`) is activity.
        Ok(self.sessions.get(&key).map(|entry| {
            let mut d = entry.value().detail(session_id.to_owned());
            if !keep_vars {
                if let Some(ref mut snap) = d.latest_snapshot {
                    // Replace the Arc with an empty one rather than
                    // mutating through it — the session may have other
                    // readers still sharing the snapshot.
                    snap.variables = std::sync::Arc::new(std::collections::HashMap::new());
                }
            }
            d
        }))
    }

    /// Cheap internal peek used by the session-events WS handler. Returns
    /// `(current_tick, is_completed)` for a session without cloning the
    /// full snapshot — lets the stream loop skip the 920 KB serialization
    /// work on no-change polls. Not a `#[service_command]` because the
    /// JSON over-the-wire path already has `sessions.info`.
    pub fn session_pulse(&self, session_id: &str) -> Option<(u64, bool)> {
        let key = ElementId::from_string(session_id);
        self.sessions
            .get(&key)
            .map(|entry| (entry.value().current_tick(), entry.value().is_completed()))
    }

    /// Subscribe to push-notified snapshots for the given session.
    ///
    /// Returns `None` if the session does not exist. Each call yields an
    /// independent [`tokio::sync::broadcast::Receiver`]; slow subscribers
    /// receive `RecvError::Lagged(N)` and are expected to resync via
    /// `sessions.info` — the server does not queue per-receiver.
    ///
    /// Replaces the 33 ms poll-peek loop the WebSocket layer used in
    /// Stage 4 with a lock-step push pattern driven by
    /// [`Orchestrator::set_snapshot_observer`].
    pub fn subscribe_session_snapshots(
        &self,
        session_id: &str,
    ) -> Option<
        tokio::sync::broadcast::Receiver<
            std::sync::Arc<sysml_runtime::orchestrator::ExecutionSnapshot>,
        >,
    > {
        let key = ElementId::from_string(session_id);
        self.sessions
            .get(&key)
            .map(|entry| entry.value().subscribe_snapshots())
    }

    /// Drop all expired sessions across every bucket. Returns the count removed.
    #[service_command(
        name = "sysml.sessions.reap",
        category = Execution,
        description = "Drop all sessions past the inactivity timeout; returns the count removed",
        returns = "usize",
        stateful = true,
    )]
    pub fn sessions_reap(&self) -> Result<usize, ServiceError> {
        let before = self.sessions.len();
        self.retain_sessions(|_, s| !s.is_expired());
        Ok(before - self.sessions.len())
    }

    /// Return current session budgets and usage per kind.
    #[service_command(
        name = "sysml.sessions.quota",
        category = Execution,
        description = "Report per-kind session budgets and current usage",
        returns = "SessionQuota",
        stateful = true,
    )]
    pub fn sessions_quota(&self) -> Result<execution::SessionQuota, ServiceError> {
        let simulation = execution::BucketUsage {
            used: self.session_count(execution::SessionKind::Simulation),
            cap: execution::quota_for(execution::SessionKind::Simulation),
        };
        let action = execution::BucketUsage {
            used: self.session_count(execution::SessionKind::Action),
            cap: execution::quota_for(execution::SessionKind::Action),
        };
        let orchestrator = execution::BucketUsage {
            used: self.session_count(execution::SessionKind::Orchestrator),
            cap: execution::quota_for(execution::SessionKind::Orchestrator),
        };
        Ok(execution::SessionQuota {
            simulation,
            action,
            orchestrator,
        })
    }

    /// Stop and remove a session (kind-agnostic).
    ///
    /// On stop, a snapshot of the session is archived via the registered
    /// [`SessionArchive`] so it remains queryable through
    /// `sysml.sessions.archive.*` after its live state has been released.
    ///
    /// If the stopped session is a child of a [`batch::BatchSession`], its
    /// [`batch::ChildDescriptor::status`] transitions to
    /// [`batch::ChildStatus::Complete`] (or `Failed` on a runtime error),
    /// the child's verdicts are copied across, and the parent batch's
    /// rollup status is recomputed. The archived session's origin is set
    /// from the parent batch's [`batch::BatchKind`] instead of
    /// [`SessionOrigin::Run`] so the Sweep / MonteCarlo / TradeStudy
    /// workflow views can surface it correctly in archive queries.
    #[service_command(
        name = "sysml.sessions.stop",
        category = Execution,
        description = "Terminate a session of any kind and release its resources",
        returns = "()",
        stateful = true,
    )]
    pub fn sessions_stop(
        &self,
        #[doc = "Opaque session id returned by any *.start command"] session_id: &str,
    ) -> Result<(), ServiceError> {
        let key = ElementId::from_string(session_id);
        let (_, session) = self.remove_session(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        // Resolve the batch (if any) that owns this session, and the
        // per-batch origin override to thread into the archive record.
        let batch_origin = self.batch_origin_for_session(session_id);
        // Archive the completed session so the UI can list / replay it later.
        // Failure is logged but does not block the stop — archive is a
        // best-effort secondary store, not the authoritative session state.
        if let Err(e) = self.archive_session_entry(session_id, &session, batch_origin) {
            tracing::warn!(session_id, error = %e, "failed to archive stopped session");
        }
        // Flip the child descriptor to Complete and recompute the parent
        // rollup. Any archive verdicts recorded against this session id are
        // copied onto the descriptor so the frontend slice view has them
        // without a second archive round-trip.
        // Capture the batch's requested outcomes from THIS session's own
        // time series while we still hold it. `remove_session` has already
        // taken it out of the map, so nothing else can read it after this
        // function returns — the reading has to happen here or not at all.
        let outcomes = self.read_batch_outcomes(session_id, &session);
        self.mark_batch_child_complete(session_id, outcomes);
        tracing::debug!(session_id, "session stopped");
        Ok(())
    }

    /// Build an [`ArchivedSession`] from a live runtime session and record it.
    ///
    /// Used internally by `sessions_stop` so the generic stop path doubles
    /// as the archive hook. Kind-specific `*.stop` commands that delegate
    /// to `sessions_stop` inherit this behaviour for free.
    ///
    /// `batch_origin` overrides the origin inferred from `session.kind`
    /// when the session belongs to a batch — sweep / monte-carlo /
    /// trade-study children archive under the workflow origin, not `Run`.
    ///
    /// Verdicts and the golden marker are preserved across re-records so
    /// out-of-band writers (e.g. a verification runner that recorded the
    /// session before stop) do not get clobbered by the stop hook.
    fn archive_session_entry(
        &self,
        session_id: &str,
        session: &execution::RuntimeSession,
        batch_origin: Option<SessionOrigin>,
    ) -> Result<(), sysml_store::ArchiveError> {
        let origin = batch_origin.unwrap_or_else(|| session_kind_origin(session.kind));
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let ticks = session.latest_tick();
        // `ExecutionSnapshot::value_units` is a tick-INVARIANT table
        // (`build_value_units`: "m_ref is immutable post-mint"), which the
        // runtime deliberately holds as an `Arc` and clones by pointer into
        // every snapshot — one table, N cheap handles.
        //
        // `serde_json::to_value` does not know that. Serialising each snapshot
        // independently deep-copies the whole table once per tick, so an
        // archived session paid for `MAX_HISTORY` (1000) full copies of a map
        // that never changed. Measured on `examples/radiation-cooling` — a
        // six-attribute model whose table is 453 entries / 60.5 KB, i.e. 97%
        // of each snapshot — one `sessions.stop` grew the service by ~1.2 GB
        // and never gave it back, because the archive is in-memory and holds
        // the record for the process lifetime. A 25-child sweep therefore
        // asked for ~30 GB and the OOM killer took the process.
        //
        // Hoist the table out of the per-tick payload and keep exactly one
        // copy on the record. Nothing reads it off an archived snapshot (the
        // only consumers are `archivedSnapshotsToSeries`, which reads `tick` +
        // `variables`, and the compare shell's snapshot count), so this drops
        // duplication, not evidence.
        let mut snapshot_value_units: Option<serde_json::Value> = None;
        let snapshots: Vec<serde_json::Value> = session
            .history()
            .iter()
            .filter_map(|s| serde_json::to_value(s).ok())
            .map(|mut value| {
                if let Some(obj) = value.as_object_mut() {
                    // `value_units` is `skip_serializing_if` empty, so an
                    // absent key simply means "this run had no measured
                    // slots" — not an error.
                    if let Some(units) = obj.remove("value_units") {
                        if snapshot_value_units.is_none() {
                            snapshot_value_units = Some(units);
                        }
                    }
                }
                value
            })
            .collect();
        // Carry across verdicts / golden marker from any pre-existing
        // archive record — `InMemorySessionArchive::record` is an
        // idempotent replace, so without this we'd silently clobber
        // verdicts written by other code paths (verification runner,
        // pre-seeded fixtures, etc.) when the session stops.
        let (prior_verdicts, prior_golden, prior_overrides) = self
            .archive
            .get(session_id)
            .map(|a| (a.verdicts, a.golden, a.overrides))
            .unwrap_or_default();
        let entry = ArchivedSession {
            id: session_id.to_owned(),
            label: session.label.clone(),
            origin,
            workspace_uri: session.uri.clone(),
            created_at: session.created_at_ms as i64,
            ended_at: now_ms,
            ticks,
            // `ArchivedSession::overrides` is documented as "overrides applied
            // at session start", and until create-time overrides existed
            // nothing could populate it — it only ever carried forward
            // whatever a prior record held. Now the session knows its own
            // scenario, so write it. A prior record still wins when it has
            // something, so an externally-ingested record (verify.record_external)
            // is not clobbered by an empty live value.
            overrides: if prior_overrides.is_empty() {
                session.create_overrides.clone()
            } else {
                prior_overrides
            },
            verdicts: prior_verdicts,
            snapshots,
            snapshot_value_units,
            golden: prior_golden,
            provenance: session.provenance.clone(),
        };
        self.archive.record(entry)
    }

    /// Return the [`SessionOrigin`] a batch-child session should archive
    /// under, or `None` if the session is not a child of any batch.
    fn batch_origin_for_session(&self, session_id: &str) -> Option<SessionOrigin> {
        for entry in self.batches.iter() {
            let guard = match entry.value().read() {
                Ok(g) => g,
                Err(_) => continue,
            };
            if guard
                .children
                .iter()
                .any(|c| c.session_id == session_id)
            {
                return Some(guard.kind.to_origin());
            }
        }
        None
    }

    /// Read the outcomes the owning batch asked for off a stopping session.
    ///
    /// Returns an empty map when the session belongs to no batch, or when its
    /// batch requested no outcomes — the common case, and the one that must
    /// stay free.
    ///
    /// The reading comes from the session's own `TimeSeriesBuffer`, the same
    /// series `sysml.sessions.timeseries` serves, so a captured outcome and
    /// the curve a user can plot for that child are the same numbers by
    /// construction rather than by coincidence. Units come from the run's
    /// `value_units` table, which is the slot-derived source the normalized
    /// snapshot already prefers.
    ///
    /// Every requested name yields an entry. A name the model never recorded
    /// produces an `unavailable` reading naming the variable, never a zero and
    /// never a silent omission.
    fn read_batch_outcomes(
        &self,
        session_id: &str,
        session: &execution::RuntimeSession,
    ) -> BTreeMap<String, batch::OutcomeReading> {
        let requested = self.batch_outcomes_for_session(session_id);
        if requested.is_empty() {
            return BTreeMap::new();
        }
        let series = session.time_series();
        // The last snapshot carries the run's `value_units` table by `Arc`;
        // it is tick-invariant, so any snapshot would do.
        let units = session.history().back().map(|s| Arc::clone(&s.value_units));
        requested
            .into_iter()
            .map(|name| {
                let reading = match series.last(&name) {
                    Some((time_ms, value)) => batch::OutcomeReading::read(
                        value,
                        time_ms,
                        units
                            .as_ref()
                            .and_then(|u| u.get(&name))
                            .and_then(|m| m.unit.clone()),
                        // Keep the SHAPE, not just the endpoint. `lttb` is the
                        // same decimator `sessions.timeseries_decimated`
                        // serves charts from, so a retained trace and a
                        // plotted one agree.
                        sysml_runtime::timeseries::lttb(
                            &series.series(&name),
                            batch::OUTCOME_SERIES_POINTS,
                        ),
                    ),
                    None => batch::OutcomeReading::unavailable(format!(
                        "'{name}' was not recorded by this run"
                    )),
                };
                (name, reading)
            })
            .collect()
    }

    /// The outcome names requested by the batch owning `session_id`, or an
    /// empty vec when the session is not a batch child.
    fn batch_outcomes_for_session(&self, session_id: &str) -> Vec<String> {
        for entry in self.batches.iter() {
            let Ok(guard) = entry.value().read() else { continue };
            if guard.children.iter().any(|c| c.session_id == session_id) {
                return guard.outcomes.clone();
            }
        }
        Vec::new()
    }

    /// Flip a child descriptor to [`batch::ChildStatus::Complete`] on the
    /// batch (if any) that owns `session_id`. Copies any archive verdicts
    /// recorded for the session onto the descriptor and recomputes the
    /// batch's rollup status. A no-op when the session is not a batch
    /// child.
    fn mark_batch_child_complete(
        &self,
        session_id: &str,
        outcomes: BTreeMap<String, batch::OutcomeReading>,
    ) {
        // Copy verdicts from the archive (the session was just recorded).
        let verdicts: Vec<ArchivedVerdict> = self
            .archive
            .get(session_id)
            .map(|a| a.verdicts)
            .unwrap_or_default();
        for entry in self.batches.iter() {
            let mut guard = match entry.value().write() {
                Ok(g) => g,
                Err(_) => continue,
            };
            let matched = guard
                .child_mut_by_session_id(session_id)
                .map(|child| {
                    child.status = batch::ChildStatus::Complete;
                    child.verdicts = verdicts.clone();
                    child.outcomes = outcomes.clone();
                })
                .is_some();
            if matched {
                guard.recompute_status();
                return;
            }
        }
    }

    // -- Session archive commands (R4.1) --

    /// List archived sessions matching the supplied filter.
    #[service_command(
        name = "sysml.sessions.archive.list",
        category = Execution,
        description = "List archived sessions (completed runs) matching optional workspace / origin / since / only_golden filters",
        returns = "ArchiveListResult { entries: Vec<ArchivedSessionSummary> }",
    )]
    pub fn sessions_archive_list(
        &self,
        #[doc = "Restrict to sessions whose workspace_uri matches exactly"]
        workspace_uri: Option<&str>,
        #[doc = "Restrict to sessions produced by this workflow: run | verify | sweep | montecarlo | tradestudy"]
        origin: Option<&str>,
        #[doc = "Unix-millisecond lower bound on created_at (inclusive)"]
        since: Option<i64>,
        #[doc = "If true, only sessions tagged golden are returned"]
        only_golden: Option<bool>,
    ) -> Result<types::ArchiveListResult, ServiceError> {
        let origin_enum = origin
            .map(parse_session_origin)
            .transpose()?;
        let filter = ArchiveFilter {
            workspace_uri: workspace_uri.map(|s| s.to_owned()),
            origin: origin_enum,
            since,
            only_golden: only_golden.unwrap_or(false),
        };
        Ok(types::ArchiveListResult {
            entries: self.archive.list(filter),
        })
    }

    /// Fetch the full payload of one archived session.
    #[service_command(
        name = "sysml.sessions.archive.get",
        category = Execution,
        description = "Fetch the full archived session record (metadata + snapshots + verdicts) by id",
        returns = "ArchiveGetResult { entry: Option<ArchivedSession> }",
    )]
    pub fn sessions_archive_get(
        &self,
        #[doc = "Opaque archived session id"] id: &str,
    ) -> Result<types::ArchiveGetResult, ServiceError> {
        Ok(types::ArchiveGetResult {
            entry: self.archive.get(id),
        })
    }

    /// Mark an archived session as golden (pin it against LRU eviction).
    #[service_command(
        name = "sysml.sessions.archive.mark_golden",
        category = Execution,
        description = "Tag an archived session as a golden reference run — pins it against archive eviction",
        returns = "ArchiveMarkGoldenResult { ok: bool }",
    )]
    pub fn sessions_archive_mark_golden(
        &self,
        #[doc = "Opaque archived session id"] id: &str,
        #[doc = "Human-readable label for the golden marker (e.g. 'v1.3 baseline')"] label: &str,
    ) -> Result<types::ArchiveMarkGoldenResult, ServiceError> {
        self.archive
            .mark_golden(id, label.to_owned())
            .map_err(archive_err_to_service)?;
        Ok(types::ArchiveMarkGoldenResult { ok: true })
    }

    /// Remove the golden tag from an archived session.
    #[service_command(
        name = "sysml.sessions.archive.unmark_golden",
        category = Execution,
        description = "Remove the golden tag from an archived session — it becomes eligible for LRU eviction again",
        returns = "ArchiveUnmarkGoldenResult { ok: bool }",
    )]
    pub fn sessions_archive_unmark_golden(
        &self,
        #[doc = "Opaque archived session id"] id: &str,
    ) -> Result<types::ArchiveUnmarkGoldenResult, ServiceError> {
        self.archive
            .unmark_golden(id)
            .map_err(archive_err_to_service)?;
        Ok(types::ArchiveUnmarkGoldenResult { ok: true })
    }

    /// Step a session forward by one tick, optionally injecting an event first.
    ///
    /// Returns the new `SessionSummary` so polling UIs can refresh in a
    /// single round-trip.
    fn sessions_step_internal(
        &self,
        session_id: &str,
        event: Option<&str>,
        overrides: Option<&[(String, String)]>,
        ticks: Option<u64>,
    ) -> Result<execution::SessionSummary, ServiceError> {
        // Bulk-step count (default 1). Validate BEFORE mutating so a bad count
        // never half-applies overrides/event. Cap is a hard error, never a
        // silent clamp — the caller must ask for a legal amount.
        let ticks = ticks.unwrap_or(1);
        if ticks == 0 {
            return Err(ServiceError::InvalidInput(
                "ticks must be >= 1".to_owned(),
            ));
        }
        if ticks > execution::MAX_BULK_STEP_TICKS {
            return Err(ServiceError::InvalidInput(format!(
                "ticks={ticks} exceeds MAX_BULK_STEP_TICKS ({}); request a smaller amount \
                 or step again",
                execution::MAX_BULK_STEP_TICKS
            )));
        }
        let key = ElementId::from_string(session_id);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        if let Some(overrides) = overrides {
            // Alias-aware override path (RSC-2.5): each name resolves
            // through the slot alias table (canonical tree-path AND
            // runtime spellings both registered per slot), then against
            // existing context variables (runtime-dynamic names). A name
            // matching neither is a hard RS002 error — silent creation
            // of typo'd override targets was removed (contract amendment
            // 2026-06-11, `session-backend-contract.md` overrides
            // section). The raw key path
            // (`sysml_runtime::compiler::apply_overrides`) still sets
            // exact keys unconditionally for batch-spawn seeding.
            //
            // P5-followup: RS002/OverrideError is always caller-input-
            // shaped (a mistyped or stale override target/value) — never
            // an internal invariant violation — so it maps to
            // `InvalidInput` (HTTP 400), not `Execution` (HTTP 500). Was
            // 500 for every override failure until this fix.
            entry
                .orchestrator
                .apply_overrides_with_aliases(overrides)
                .map_err(|e| ServiceError::InvalidInput(e.to_string()))?;
        }
        if let Some(ev) = event {
            let target = entry.subsystem_name.clone().ok_or_else(|| {
                ServiceError::Execution(format!(
                    "session {session_id} has no primary subsystem — use sessions.inject with an explicit subsystem name"
                ))
            })?;
            entry.orchestrator.inject_event(&target, ev);
        }
        // Overrides + event are applied ONCE above; then advance `ticks` ticks
        // (default 1 = the classic single-step). `step_many` stops early on a
        // breakpoint pause or the orchestrator's configured tick/time limit.
        //
        // BP1: capture the REAL count `step_many` advanced (it was
        // previously discarded via `let _ = ...`) and stamp it onto the
        // returned summary's `ticks_advanced`. This is the one call site
        // that ever sets this field non-zero — callers compare it against
        // the requested `ticks` (combined with `paused`/`completed`) to
        // detect a halt, rather than a dedicated `HaltReason` enum.
        let advanced = entry.step_many(ticks);
        let mut summary = entry.summary(session_id.to_owned());
        summary.ticks_advanced = advanced;
        Ok(summary)
    }

    #[service_command(
        name = "sysml.sessions.step",
        category = Execution,
        description = "Advance any session, optionally injecting an event and applying context overrides ONCE before stepping. `ticks` (default 1) runs that many ticks server-side in one call — for fine-dt runs that need thousands of ticks to reach an event (avoids a per-tick round-trip); the run still stops early at a breakpoint pause or the session's configured tick/time limit. Every advanced tick is recorded to the time series, so chart data stays complete; poll sessions.timeseries_decimated for it.",
        returns = "SessionSummary",
        stateful = true,
    )]
    pub fn sessions_step(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
        #[doc = "Optional event to inject into the primary subsystem before stepping"]
        event: Option<&str>,
        #[doc = "Optional context overrides applied before stepping; numeric strings are parsed into typed values"]
        overrides: Option<&[(String, String)]>,
        #[doc = "Number of ticks to advance in this call (default 1). Runs server-side; capped at MAX_BULK_STEP_TICKS — a larger value is InvalidInput, never silently clamped."]
        ticks: Option<u64>,
    ) -> Result<execution::SessionSummary, ServiceError> {
        self.sessions_step_internal(session_id, event, overrides, ticks)
    }

    /// Inject an event into a named subsystem and advance the session.
    ///
    /// Works on any kind of session; pick the subsystem by name as
    /// registered on the underlying orchestrator.
    fn sessions_inject_internal(
        &self,
        session_id: &str,
        subsystem: &str,
        event: &str,
        overrides: Option<&[(String, String)]>,
    ) -> Result<execution::SessionSummary, ServiceError> {
        let key = ElementId::from_string(session_id);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        if let Some(overrides) = overrides {
            // Same alias-aware override resolution as sessions.step
            // (RSC-2.5: slot alias table → existing context variable →
            // RS002 hard error on unknown names). P5-followup: InvalidInput
            // (400), not Execution (500) — see sessions.step's comment.
            entry
                .orchestrator
                .apply_overrides_with_aliases(overrides)
                .map_err(|e| ServiceError::InvalidInput(e.to_string()))?;
        }
        let _ = entry.inject_event(subsystem, event);
        Ok(entry.summary(session_id.to_owned()))
    }

    #[service_command(
        name = "sysml.sessions.inject",
        category = Execution,
        description = "Inject an event into a named subsystem of any session and advance one tick, optionally applying context overrides first",
        returns = "SessionSummary",
        stateful = true,
    )]
    pub fn sessions_inject(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
        #[doc = "Subsystem name as registered on the orchestrator"] subsystem: &str,
        #[doc = "Event name to inject"] event: &str,
        #[doc = "Optional context overrides applied before stepping; numeric strings are parsed into typed values"]
        overrides: Option<&[(String, String)]>,
    ) -> Result<execution::SessionSummary, ServiceError> {
        self.sessions_inject_internal(session_id, subsystem, event, overrides)
    }

    /// Reset a session to its initial state, clearing history.
    #[service_command(
        name = "sysml.sessions.reset",
        category = Execution,
        description = "Reset any session to its initial state, clearing step history and restarting the expiry timer",
        returns = "SessionSummary",
        stateful = true,
    )]
    pub fn sessions_reset(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
    ) -> Result<execution::SessionSummary, ServiceError> {
        let key = ElementId::from_string(session_id);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        entry.reset();
        tracing::debug!(session_id, "session reset");
        Ok(entry.summary(session_id.to_owned()))
    }

    /// Resume a session that is paused at a breakpoint.
    ///
    /// BP2: `RuntimeSession::resume()` (`execution.rs`) already existed but
    /// had no service command reaching it. Idempotent — resuming a session
    /// that is not currently paused is a no-op success, never an error
    /// (mirrors the tolerant shape of `sessions.reset`/`sessions.rename`,
    /// not a strict precondition check). Does NOT auto-step: the client
    /// must follow up with `sessions.step` to actually advance past the
    /// breakpoint.
    #[service_command(
        name = "sysml.sessions.resume",
        category = Execution,
        description = "Clear the pause flag on a session halted at a breakpoint so subsequent sessions.step calls advance again. Idempotent: a no-op success on a session that isn't paused. Does not itself advance any ticks.",
        returns = "SessionSummary",
        stateful = true,
    )]
    pub fn sessions_resume(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
    ) -> Result<execution::SessionSummary, ServiceError> {
        let key = ElementId::from_string(session_id);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        entry.resume();
        tracing::debug!(session_id, "session resumed");
        Ok(entry.summary(session_id.to_owned()))
    }

    /// Set the user-facing display label on a session.
    ///
    /// The label is free-form and has no uniqueness constraint; it appears
    /// on `SessionSummary` and is surfaced by the sidebar UI. Pass an empty
    /// string to clear the label.
    #[service_command(
        name = "sysml.sessions.rename",
        category = Execution,
        description = "Set the display label on any session",
        returns = "()",
        stateful = true,
    )]
    pub fn sessions_rename(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
        #[doc = "New display label; empty string clears it"] label: &str,
    ) -> Result<(), ServiceError> {
        let key = ElementId::from_string(session_id);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        entry.label = if label.is_empty() {
            None
        } else {
            Some(label.to_owned())
        };
        Ok(())
    }

    /// List the variable names captured in this session's canonical
    /// time-series buffer (ADR-011 §6 / S3.T9 Phase B).
    ///
    /// The buffer is populated on every successful step from
    /// `snapshot_view::normalize(&snap).scalar_vars`. Names are sorted
    /// for deterministic ordering across calls. Companion to
    /// `sysml.sessions.timeseries` — callers populate variable
    /// pickers / dropdowns from this list and then request individual
    /// series.
    #[service_command(
        name = "sysml.sessions.timeseries_names",
        category = Execution,
        description = "List the variable names captured in a session's canonical time-series buffer",
        returns = "TimeSeriesNamesResult",
    )]
    pub fn sessions_timeseries_names(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
    ) -> Result<types::TimeSeriesNamesResult, ServiceError> {
        let key = ElementId::from_string(session_id);
        let entry = self.sessions.get(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        let buf = entry.value().time_series();
        let mut names: Vec<String> = buf.series_names().into_iter().map(String::from).collect();
        names.sort();
        Ok(types::TimeSeriesNamesResult {
            names,
            len: buf.len(),
            capacity: buf.capacity(),
        })
    }

    /// Fetch the raw `(time_ms, value)` series for one variable from
    /// the session's canonical time-series buffer (ADR-011 §6 /
    /// S3.T9 Phase B).
    ///
    /// `start_ms` and `end_ms` are inclusive bounds — pass `None`/
    /// `None` for the full buffered range. Returns empty `points`
    /// when `var` isn't captured (no error — the variable just
    /// doesn't have a series yet, e.g. an attribute that hasn't
    /// resolved to a scalar).
    ///
    /// Use [`Self::sessions_timeseries_decimated`] instead for
    /// chart rendering on long sessions — this command returns the
    /// raw points and the buffer holds up to ~12.5 K ticks at the
    /// default 100 MB memory budget.
    #[service_command(
        name = "sysml.sessions.timeseries",
        category = Execution,
        description = "Fetch (time_ms, value) points for a single variable from a session's canonical time-series buffer (optionally bounded)",
        returns = "TimeSeriesResult",
    )]
    pub fn sessions_timeseries(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
        #[doc = "Variable name (as it appears in NormalizedSnapshot::scalar_vars)"] var: &str,
        #[doc = "Inclusive lower bound on time_ms; None = unbounded"]
        start_ms: Option<f64>,
        #[doc = "Inclusive upper bound on time_ms; None = unbounded"]
        end_ms: Option<f64>,
    ) -> Result<types::TimeSeriesResult, ServiceError> {
        let key = ElementId::from_string(session_id);
        let entry = self.sessions.get(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        let buf = entry.value().time_series();
        let points: Vec<types::TimeSeriesPoint> = buf
            .series_windowed(var, start_ms, end_ms)
            .into_iter()
            .map(|(time_ms, value)| types::TimeSeriesPoint { time_ms, value })
            .collect();
        Ok(types::TimeSeriesResult {
            var: var.to_owned(),
            points,
        })
    }

    /// Fetch an LTTB-decimated series for one variable (ADR-011 §6 /
    /// S3.T9 Phase B).
    ///
    /// Wraps `sysml_runtime::timeseries::lttb` over the same windowed
    /// series [`Self::sessions_timeseries`] returns. Use this for
    /// chart rendering where ~`target_points` is the on-screen pixel
    /// count — LTTB preserves visual shape (peaks, troughs, slope
    /// changes) while shipping fewer bytes than the raw series.
    ///
    /// `target_points` below 3 (the LTTB minimum) or above the raw
    /// series length returns the raw series unchanged, matching the
    /// underlying `lttb()` semantics.
    #[service_command(
        name = "sysml.sessions.timeseries_decimated",
        category = Execution,
        description = "Fetch an LTTB-decimated time series for a single variable (preserves visual shape; ideal for chart rendering)",
        returns = "TimeSeriesResult",
    )]
    pub fn sessions_timeseries_decimated(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
        #[doc = "Variable name (as it appears in NormalizedSnapshot::scalar_vars)"] var: &str,
        #[doc = "Target decimated point count (typical: on-screen pixel width)"]
        target_points: usize,
        #[doc = "Inclusive lower bound on time_ms; None = unbounded"]
        start_ms: Option<f64>,
        #[doc = "Inclusive upper bound on time_ms; None = unbounded"]
        end_ms: Option<f64>,
    ) -> Result<types::TimeSeriesResult, ServiceError> {
        let key = ElementId::from_string(session_id);
        let entry = self.sessions.get(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        let buf = entry.value().time_series();
        let raw = buf.series_windowed(var, start_ms, end_ms);
        let decimated = sysml_runtime::timeseries::lttb(&raw, target_points);
        let points: Vec<types::TimeSeriesPoint> = decimated
            .into_iter()
            .map(|(time_ms, value)| types::TimeSeriesPoint { time_ms, value })
            .collect();
        Ok(types::TimeSeriesResult {
            var: var.to_owned(),
            points,
        })
    }

    /// Enumerate the subsystems of a session without fetching the full snapshot.
    ///
    /// Cheaper than `sessions.info` for UIs that only need the subsystem list
    /// (e.g. to populate an inject-event dropdown). Preserves insertion order.
    #[service_command(
        name = "sysml.sessions.subsystems",
        category = Execution,
        description = "List the subsystems of any session without fetching a full snapshot",
        returns = "Vec<SubsystemSummary>",
    )]
    pub fn sessions_subsystems(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
    ) -> Result<Vec<execution::SubsystemSummary>, ServiceError> {
        let key = ElementId::from_string(session_id);
        let entry = self.sessions.get(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        Ok(entry.detail(session_id.to_owned()).subsystems)
    }

    // ------------------------------------------------------------------
    // Breakpoint primitives (R1.2 — backend foundation for frontend
    // SessionControl debugger API, built by Agent A in parallel).
    // ------------------------------------------------------------------

    /// Register a breakpoint on a running session.
    ///
    /// Returns an opaque `BreakpointId` the caller stores and later passes to
    /// [`Self::breakpoint_clear`]. See [`sysml_runtime::breakpoint::Breakpoint`]
    /// for the supported hit conditions (state entry, transition fire, action
    /// invoke, constraint violation, threshold crossing).
    #[service_command(
        name = "sysml.breakpoint.set",
        category = Execution,
        description = "Register a breakpoint on a running session; returns an opaque id",
        returns = "BreakpointId (string)",
        stateful = true,
    )]
    pub fn breakpoint_set(
        &self,
        #[doc = "Opaque session id returned by any *.start command"] session_id: &str,
        #[doc = "Breakpoint specification; see sysml_runtime::breakpoint::Breakpoint"] breakpoint: sysml_runtime::breakpoint::Breakpoint,
    ) -> Result<sysml_runtime::breakpoint::BreakpointId, ServiceError> {
        let key = ElementId::from_string(session_id);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        Ok(entry.set_breakpoint(breakpoint))
    }

    /// Remove a previously-registered breakpoint from a session.
    ///
    /// Unknown ids are treated as success (idempotent clear). If the cleared
    /// breakpoint was the one currently holding the session paused, the
    /// session's pause state is released.
    #[service_command(
        name = "sysml.breakpoint.clear",
        category = Execution,
        description = "Remove a breakpoint from a session by id (idempotent)",
        returns = "()",
        stateful = true,
    )]
    pub fn breakpoint_clear(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
        #[doc = "Breakpoint id returned by sysml.breakpoint.set"] breakpoint_id: &str,
    ) -> Result<(), ServiceError> {
        let key = ElementId::from_string(session_id);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        entry.clear_breakpoint(breakpoint_id);
        Ok(())
    }

    /// List all currently-registered breakpoints on a session.
    ///
    /// Returned in deterministic id order so polling UIs get stable rows.
    #[service_command(
        name = "sysml.breakpoint.list",
        category = Execution,
        description = "List registered breakpoints for a session as (id, breakpoint) pairs",
        returns = "Vec<(BreakpointId, Breakpoint)>",
        stateful = true,
    )]
    pub fn breakpoint_list(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
    ) -> Result<
        Vec<(
            sysml_runtime::breakpoint::BreakpointId,
            sysml_runtime::breakpoint::Breakpoint,
        )>,
        ServiceError,
    > {
        let key = ElementId::from_string(session_id);
        let entry = self.sessions.get(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        Ok(entry.list_breakpoints())
    }

    /// Fork a running session into an independent child at the current tick.
    ///
    /// The child gets a fresh UUID, restarted expiry clock, and an independent
    /// orchestrator (deep clone via `Orchestrator::fork`). The parent is
    /// untouched — stepping either side does not affect the other.
    ///
    /// Honours the per-kind cap: if the bucket for the parent's kind is full,
    /// the fork fails with the same error message as a fresh start.
    #[service_command(
        name = "sysml.sessions.fork",
        category = Execution,
        description = "Fork a session at its current tick into an independent child",
        returns = "SessionSummary",
        stateful = true,
    )]
    pub fn sessions_fork(
        &self,
        #[doc = "Opaque session id of the parent session"] session_id: &str,
    ) -> Result<execution::SessionSummary, ServiceError> {
        let parent_key = ElementId::from_string(session_id);
        let parent_kind = {
            let entry = self.sessions.get(&parent_key).ok_or_else(|| {
                ServiceError::ElementNotFound(format!("no session: {session_id}"))
            })?;
            entry.kind
        };
        self.cap_check(parent_kind)?;
        let child = {
            let entry = self.sessions.get(&parent_key).ok_or_else(|| {
                ServiceError::ElementNotFound(format!("no session: {session_id}"))
            })?;
            entry.fork_child()
        };
        // Build the summary from the local value before handing the
        // session off to the catalog — avoids a post-insert re-lookup
        // that could race with a concurrent `sessions.stop`.
        let child_id = execution::new_session_id();
        let summary = child.summary(child_id.to_string());
        tracing::debug!(
            parent_id = session_id,
            child_id = %child_id,
            fork_point_tick = ?summary.fork_point_tick,
            kind = %parent_kind,
            "session forked"
        );
        self.insert_session(child_id, child);
        Ok(summary)
    }

    /// Fork a session and atomically apply parameter overrides to the child
    /// before returning. Useful for launching parameter sweeps from a common
    /// prefix without a race window in which the child is live but unadjusted.
    ///
    /// When the optional `at_tick` argument is supplied, the child is
    /// rewound to that tick first (R4: golden-baseline compare replay).
    /// The parent session retains a bounded archive of orchestrator
    /// snapshots
    /// ([`DEFAULT_SNAPSHOT_RETENTION_TICKS`](execution::DEFAULT_SNAPSHOT_RETENTION_TICKS)
    /// by default) from which the rewound child is reconstructed.
    ///
    /// # Errors
    ///
    /// - `at_tick` greater than the parent's current tick →
    ///   [`ServiceError::ForkAtTick`] with
    ///   [`FutureTick`](execution::ForkAtTickError::FutureTick).
    /// - `at_tick` older than the retained window →
    ///   [`ServiceError::ForkAtTick`] with
    ///   [`SnapshotMissing`](execution::ForkAtTickError::SnapshotMissing).
    #[service_command(
        name = "sysml.sessions.fork_with_overrides",
        category = Execution,
        description = "Fork a session (optionally rewound to a past tick) and atomically apply parameter overrides to the child",
        returns = "SessionSummary",
        stateful = true,
    )]
    pub fn sessions_fork_with_overrides(
        &self,
        #[doc = "Opaque session id of the parent session"] session_id: &str,
        #[doc = "Parameter overrides as (name, value) pairs; values parsed into numeric or string values"]
        overrides: &[(String, String)],
        #[doc = "Optional tick to rewind to before forking. None or null = fork from current tick (backward compatible). Past ticks require a retained orchestrator snapshot."]
        at_tick: Option<u64>,
    ) -> Result<execution::SessionSummary, ServiceError> {
        let parent_key = ElementId::from_string(session_id);
        let parent_kind = {
            let entry = self.sessions.get(&parent_key).ok_or_else(|| {
                ServiceError::ElementNotFound(format!("no session: {session_id}"))
            })?;
            entry.kind
        };
        self.cap_check(parent_kind)?;
        let mut child = {
            let entry = self.sessions.get(&parent_key).ok_or_else(|| {
                ServiceError::ElementNotFound(format!("no session: {session_id}"))
            })?;
            match at_tick {
                None => entry.fork_child(),
                Some(t) => entry.fork_child_at(t)?,
            }
        };
        // Apply overrides in-place BEFORE inserting — this is what makes
        // the fork+override operation atomic from a caller's perspective.
        // No visible state where the child exists without the overrides.
        // Use the alias-aware path so UI-typed canonical keys resolve via
        // the slot alias table (RSC-2.5). An unknown override name fails
        // with RS002 and the child is dropped without ever entering the
        // catalog — atomicity holds for the error case too. P5-followup:
        // InvalidInput (400), not Execution (500) — see sessions.step's
        // comment.
        child
            .orchestrator
            .apply_overrides_with_aliases(overrides)
            .map_err(|e| ServiceError::InvalidInput(e.to_string()))?;
        let child_id = execution::new_session_id();
        let summary = child.summary(child_id.to_string());
        tracing::debug!(
            parent_id = session_id,
            child_id = %child_id,
            override_count = overrides.len(),
            fork_point_tick = ?summary.fork_point_tick,
            at_tick = ?at_tick,
            kind = %parent_kind,
            "session forked with overrides"
        );
        self.insert_session(child_id, child);
        Ok(summary)
    }

    /// Compare two sessions' latest snapshots and return a structured diff.
    ///
    /// Semantic shape:
    /// - `subsystem_diffs` contains every subsystem whose `current_state`
    ///   differs between the two sides, plus any subsystem that exists on
    ///   only one side (the absent side gets `None`). Sorted alphabetically
    ///   by subsystem name for stable cross-call ordering.
    /// - `variable_diffs` contains every context variable whose value differs
    ///   (including variables present on only one side), excluding the
    ///   synthetic `t_ms`, `tick`, and `__*` bookkeeping vars which drift by
    ///   construction when two sessions run on separate clocks. Sorted
    ///   alphabetically.
    /// - `current_tick_a` / `current_tick_b` report the latest snapshot tick
    ///   (0 if the session has not been stepped yet).
    ///
    /// When one side has no recorded history, a synthetic snapshot is
    /// built from its current orchestrator context and subsystem list so
    /// the diff still reports subsystems present on the stepped side.
    ///
    /// For tick-aligned diffing across the shared history range
    /// ("where did these two timelines first diverge?"), use
    /// [`SysmlService::sessions_diff_timeline`] instead.
    #[service_command(
        name = "sysml.sessions.diff",
        category = Execution,
        description = "Diff two sessions' latest snapshots (subsystem states + context variables)",
        returns = "SessionDivergence",
    )]
    pub fn sessions_diff(
        &self,
        #[doc = "Opaque session id of side A"] a_id: &str,
        #[doc = "Opaque session id of side B"] b_id: &str,
    ) -> Result<execution::SessionDivergence, ServiceError> {
        let a_key = ElementId::from_string(a_id);
        let b_key = ElementId::from_string(b_id);
        let a_entry = self.sessions.get(&a_key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {a_id}"))
        })?;
        let b_entry = self.sessions.get(&b_key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {b_id}"))
        })?;

        let current_tick_a = a_entry.latest_tick();
        let current_tick_b = b_entry.latest_tick();

        // Build a snapshot for each side. When a session has no recorded
        // history yet, fall back to a synthetic placeholder built from the
        // current orchestrator context + the subsystem list. This keeps
        // `diff_snapshots` as the single source of diff logic and
        // correctly reports subsystems present on the stepped side as
        // diffs against the fresh side.
        fn synthetic_snapshot(
            entry: &execution::RuntimeSession,
        ) -> sysml_runtime::orchestrator::ExecutionSnapshot {
            use sysml_runtime::orchestrator::ExecutionSnapshot;
            use sysml_runtime::SubsystemState;
            let subsystem_states = entry
                .orchestrator
                .subsystems()
                .iter()
                .map(|sub| {
                    let name = sub.name.clone();
                    let state = SubsystemState {
                        name: name.clone(),
                        kind: sub.executor.kind_label(),
                        current_state: sub.executor.current_state_name().to_owned(),
                        completed: sub.executor.is_completed(),
                        available_transitions: Vec::new(),
                        outputs: Vec::new(),
                        sends: Vec::new(),
                        // Static introspection snapshot (not a per-tick step) — no
                        // triggering transfer is observable here.
                        incoming_transition_trigger: None,
                        deferred_event_count: sub.executor.deferred_event_count(),
                        // Subsystem.source_element_id is an ElementId; forward
                        // its id-keyed identity to the frontend directly.
                        source_element_id: sub.source_element_id.clone(),
                    };
                    (name, state)
                })
                .collect();
            ExecutionSnapshot {
                tick: 0,
                time_ms: 0.0,
                subsystem_states,
                variables: std::sync::Arc::clone(&entry.orchestrator.context.variables),
                messages: Vec::new(),
                constraint_results: Vec::new(),
                assertion_checkpoints: Vec::new(),
                guard_diagnoses: Vec::new(),
                causation_links: Vec::new(),
                completed: false,
                port_values: std::collections::HashMap::new(),
                derivatives: std::collections::HashMap::new(),
                resolved_refs: std::collections::HashMap::new(),
                flow_drop_warnings: Vec::new(),
                value_units: Default::default(),
                step_size_health: Vec::new(),
            }
        }

        let a_owned;
        let a_snap: &sysml_runtime::orchestrator::ExecutionSnapshot = match a_entry.history().back() {
            Some(s) => s,
            None => {
                a_owned = synthetic_snapshot(&a_entry);
                &a_owned
            }
        };
        let b_owned;
        let b_snap: &sysml_runtime::orchestrator::ExecutionSnapshot = match b_entry.history().back() {
            Some(s) => s,
            None => {
                b_owned = synthetic_snapshot(&b_entry);
                &b_owned
            }
        };

        let (subsystem_diffs, variable_diffs) = execution::diff_snapshots(a_snap, b_snap);

        Ok(execution::SessionDivergence {
            a_id: a_id.to_owned(),
            b_id: b_id.to_owned(),
            current_tick_a,
            current_tick_b,
            subsystem_diffs,
            variable_diffs,
        })
    }

    /// Compare two sessions' full recorded histories tick-by-tick and
    /// return the first divergence point plus per-tick deltas.
    ///
    /// Walks both sessions' bounded snapshot histories (`MAX_HISTORY = 1000`)
    /// in parallel, identifies the shared tick range, and emits a sparse
    /// sequence of `TickDiff`s — only ticks where at least one subsystem
    /// state or non-bookkeeping variable differs are included.
    ///
    /// `first_divergence_tick` is the tick of the earliest non-empty
    /// `TickDiff`, or `None` if the two sessions agree across the entire
    /// shared range. `history_truncated` flags that the real divergence
    /// point may lie before `shared_start_tick` because history eviction
    /// has dropped earlier snapshots.
    #[service_command(
        name = "sysml.sessions.diff_timeline",
        category = Execution,
        description = "Walk two sessions' histories tick-by-tick and report where they first diverged",
        returns = "SessionTimelineDivergence",
    )]
    pub fn sessions_diff_timeline(
        &self,
        #[doc = "Opaque session id of side A"] a_id: &str,
        #[doc = "Opaque session id of side B"] b_id: &str,
    ) -> Result<execution::SessionTimelineDivergence, ServiceError> {
        let a_key = ElementId::from_string(a_id);
        let b_key = ElementId::from_string(b_id);
        let a_entry = self.sessions.get(&a_key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {a_id}"))
        })?;
        let b_entry = self.sessions.get(&b_key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {b_id}"))
        })?;

        let a_snaps = a_entry.snapshots_slice();
        let b_snaps = b_entry.snapshots_slice();
        let a_fork = a_entry.fork_point_tick;
        let b_fork = b_entry.fork_point_tick;

        // Empty-history handling: if either side has no snapshots, there
        // is no shared range and no diffs to report.
        if a_snaps.is_empty() || b_snaps.is_empty() {
            return Ok(execution::SessionTimelineDivergence {
                a_id: a_id.to_owned(),
                b_id: b_id.to_owned(),
                shared_start_tick: None,
                shared_end_tick: None,
                first_divergence_tick: None,
                tick_diffs: Vec::new(),
                history_truncated: false,
            });
        }

        // Snapshots are recorded in tick order; front = oldest, back = newest.
        let a_first = a_snaps.first().copied().unwrap().tick;
        let a_last = a_snaps.last().copied().unwrap().tick;
        let b_first = b_snaps.first().copied().unwrap().tick;
        let b_last = b_snaps.last().copied().unwrap().tick;

        let shared_start = a_first.max(b_first);
        let shared_end = a_last.min(b_last);

        if shared_start > shared_end {
            // Histories don't overlap at all — definitely truncated past
            // the point where any comparison was possible.
            return Ok(execution::SessionTimelineDivergence {
                a_id: a_id.to_owned(),
                b_id: b_id.to_owned(),
                shared_start_tick: None,
                shared_end_tick: None,
                first_divergence_tick: None,
                tick_diffs: Vec::new(),
                history_truncated: true,
            });
        }

        // Truncation means eviction has dropped snapshots that both sides
        // *could* have shared if history were unbounded. A fresh fork
        // (child reseeded with only the parent's latest snapshot) is NOT
        // truncated — nothing was lost because the child never had the
        // earlier ticks anyway.
        //
        // The earliest "ideal" shared start is the latest fork point
        // among the two sides (or 1, if neither was forked). Truncation
        // fires when `shared_start` has been evicted past that boundary.
        let ideal_start = a_fork.into_iter().chain(b_fork).max().unwrap_or(1);
        let history_truncated = shared_start > ideal_start;

        // Merge-join walk: both histories are sorted by tick (the
        // orchestrator records one snapshot per step, oldest at the
        // front), so we can advance two cursors instead of hashing
        // every tick. This avoids HashMap allocation and runs in a
        // single pass over the shared range.
        let mut tick_diffs = Vec::new();
        let mut first_divergence_tick = None;
        let mut i = 0;
        let mut j = 0;
        while i < a_snaps.len() && j < b_snaps.len() {
            let a_tick = a_snaps[i].tick;
            let b_tick = b_snaps[j].tick;
            if a_tick < shared_start {
                i += 1;
                continue;
            }
            if b_tick < shared_start {
                j += 1;
                continue;
            }
            if a_tick > shared_end || b_tick > shared_end {
                break;
            }
            match a_tick.cmp(&b_tick) {
                std::cmp::Ordering::Less => i += 1,
                std::cmp::Ordering::Greater => j += 1,
                std::cmp::Ordering::Equal => {
                    let (subsystem_diffs, variable_diffs) =
                        execution::diff_snapshots(a_snaps[i], b_snaps[j]);
                    if !(subsystem_diffs.is_empty() && variable_diffs.is_empty()) {
                        if first_divergence_tick.is_none() {
                            first_divergence_tick = Some(a_tick);
                        }
                        tick_diffs.push(execution::TickDiff {
                            tick: a_tick,
                            subsystem_diffs,
                            variable_diffs,
                        });
                    }
                    i += 1;
                    j += 1;
                }
            }
        }

        Ok(execution::SessionTimelineDivergence {
            a_id: a_id.to_owned(),
            b_id: b_id.to_owned(),
            shared_start_tick: Some(shared_start),
            shared_end_tick: Some(shared_end),
            first_divergence_tick,
            tick_diffs,
            history_truncated,
        })
    }

    /// Return the structural topology of a running session, grouped by module
    /// (var_prefix) with physics domain classification on each subsystem.
    ///
    /// Used by the multi-physics simulation UI to build the System Browser,
    /// Domain Lanes, and Diagram Overlay.
    #[service_command(
        name = "sysml.sessions.topology",
        category = Execution,
        description = "Return the structural topology of a session (modules, subsystems, physics domains)",
        returns = "SystemTopology",
        stateful = true,
    )]
    pub fn sessions_topology(
        &self,
        #[doc = "Session id (UUID)"] session_id: &str,
    ) -> Result<execution::SystemTopology, ServiceError> {
        let key = ElementId::from_string(session_id);
        let entry = self.sessions.get(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        Ok(entry.value().topology())
    }
    /// Check constraints in a loaded model.
    #[service_command(
        name = "sysml.constraint.check",
        category = Execution,
        description = "Evaluate all constraints in a model with optional parameter overrides",
        returns = "Vec<ConstraintResult>",
    )]
    pub fn check_constraints(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Key=value parameter overrides (values parsed as int/float/bool/string)"] overrides: &[(String, String)],
    ) -> Result<Vec<ConstraintResult>, ServiceError> {
        // Use workspace graph for evaluation (cross-file references resolve),
        // but scope constraint extraction to the target file's elements.
        let graph = self.workspace_aware_graph()?;
        let file_ids = self.file_element_ids(uri)?;

        // Route through the salsa-cached `workspace_eval_context_with_library`
        // query via `eval_context_with_overrides`. The ~14k-binding seed walk
        // pays once per graph revision instead of once per call (ADR-011 §3).
        let base_ctx = self.eval_context_with_overrides(overrides)?;

        // Constraint extraction + per-instance evaluation route through the one
        // home: `ModelCompiler::evaluate_constraints_per_instance`. It evaluates
        // each constraint once PER OCCURRENCE of its owning definition (SysML v2
        // BooleanEvaluation per occurrence) and owns the owner-scoped context
        // overlay, so the service no longer re-implements constraint scoping.
        let compiler =
            sysml_runtime::compiler::ModelCompiler::from_arc(std::sync::Arc::clone(&graph));
        let cgraph = compiler.graph();
        // Filter: non-library, and if file-scoped, only from that file.
        let constraint_set = sysml_runtime::constraints::extract_constraints_filtered(
            cgraph,
            |e| {
                !cgraph.is_library_element(&e.id)
                    && (file_ids.is_empty() || file_ids.contains(&e.id))
            },
        );
        let precompiled =
            sysml_runtime::constraints::precompile_constraint_set(&constraint_set);
        let per_instance = compiler
            .evaluate_constraints_per_instance(&precompiled, &base_ctx)
            .map_err(|e| ServiceError::Execution(e.message))?;

        // Deduplicate by (owner, instance, name, expression): one verdict per
        // occurrence. The per-instance key preserves N distinct occurrences of
        // the same definition constraint instead of collapsing them to one (the
        // legacy (name, expression) key did the latter and also conflated two
        // distinct constraints that shared a name+expr across owners).
        let mut seen = std::collections::HashSet::new();
        Ok(per_instance
            .into_iter()
            .filter(|pi| {
                let key = (
                    pi.result.constraint.owner_id.clone(),
                    pi.instance_element_id.clone(),
                    pi.result.constraint.description.clone().unwrap_or_default(),
                    pi.result.constraint.expr.clone(),
                );
                seen.insert(key)
            })
            .map(|pi| {
                let r = &pi.result;
                // Inconclusive (value-less / unresolved) is distinct from Fail.
                // One home for that mapping: `EvaluationResult::verdict`.
                let verdict = r.verdict();
                let message = if !r.diagnostics.is_empty() {
                    Some(
                        r.diagnostics
                            .iter()
                            .map(|d| d.message.clone())
                            .collect::<Vec<_>>()
                            .join("; "),
                    )
                } else if r.inconclusive {
                    Some("inconclusive".to_owned())
                } else {
                    None
                };
                ConstraintResult {
                    name: r
                        .constraint
                        .description
                        .clone()
                        .unwrap_or_else(|| r.constraint.expr.clone()),
                    expression: Some(r.constraint.expr.clone()),
                    verdict,
                    actual: None,
                    expected: None,
                    message,
                    instance_element_id: pi.instance_element_id.clone(),
                    instance_path: pi.instance_path.clone(),
                }
            })
            .collect())
    }

    /// Evaluate a standalone expression string.
    #[service_command(
        name = "sysml.expression.eval",
        category = Execution,
        description = "Evaluate a standalone expression with optional variable bindings",
        returns = "Value",
    )]
    pub fn eval_expression(
        &self,
        #[doc = "Expression to evaluate (e.g. '2 + 3', 'speed * 1.5')"] expr: &str,
        #[doc = "Variable bindings as key=value pairs"] context: &[(String, String)],
    ) -> Result<Value, ServiceError> {
        let mut ctx = EvalContext::new();
        sysml_runtime::compiler::apply_overrides(&mut ctx, context);

        let ir = sysml_runtime::expressions::compile_simple_expression(expr).map_err(|diags| {
            ServiceError::Execution(
                diags
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        })?;

        let evaluator = sysml_runtime::expressions::ExpressionEvaluator::new();
        evaluator.eval(&ir, &ctx).map_err(|e| {
            ServiceError::Execution(e.to_string())
        })
    }

    // -- Verification, analysis, solving, tracing, flow --

    /// Run a named verification case and return structured results.
    #[service_command(
        name = "sysml.verify",
        category = Execution,
        description = "Run a named verification case against model requirements with optional parameter overrides",
        returns = "VerifyResult { verdict, summary, requirements, diagnostics }",
    )]
    pub fn verify(
        &self,
        #[doc = "Name of the verification case to run"] case_name: &str,
        #[doc = "Key=value parameter overrides for the evaluation context"] overrides: &[(String, String)],
    ) -> Result<VerifyResult, ServiceError> {
        let graph = self.workspace_aware_graph()?;

        let case_ir =
            sysml_runtime::cases::compile_verification_case(case_name, &graph).map_err(
                |diags| {
                    ServiceError::Execution(
                        diags
                            .iter()
                            .map(|d| d.to_string())
                            .collect::<Vec<_>>()
                            .join("; "),
                    )
                },
            )?;

        let ctx = self.eval_context_with_overrides(overrides)?;
        let runner = sysml_runtime::cases::VerificationRunner::new();
        let result = runner.verify(&case_ir, &ctx);
        // `sysml.verify` evaluates against static overrides/defaults —
        // label it so (§2.1a ruling (d)); the trajectory path is
        // `sessions.verify`, never silently substituted.
        Ok(self.build_verify_result(&result, &case_ir, &ctx, EvaluationMode::Static))
    }

    /// Project a completed verification run into the shared [`VerifyResult`]
    /// shape (rollup summary + per-requirement `serialize_requirement_result_for_case`).
    ///
    /// One home for every verdict surface (`sysml.verify`, `sysml.sessions.verify`)
    /// so they never drift into a second response shape (CLAUDE #4). Both feed a
    /// `VerificationResult` from the ONE `VerificationRunner`; the only thing
    /// that differs is where `ctx` came from (static overrides for `verify`, a
    /// live session's orchestrator context for `sessions.verify`).
    fn build_verify_result(
        &self,
        result: &sysml_runtime::cases::VerificationResult,
        case_ir: &sysml_runtime::cases::VerificationCaseIR,
        ctx: &sysml_runtime::expressions::EvalContext,
        evaluation_mode: EvaluationMode,
    ) -> VerifyResult {
        let requirement_verdicts: Vec<sysml_runtime::cases::VerdictKind> = result
            .requirement_results
            .iter()
            .map(|r| r.verdict)
            .collect();
        let rollup =
            sysml_runtime::aggregates::verdict_rollup_from_verdicts(&requirement_verdicts);
        let summary = VerifySummary::from_rollup(rollup);

        VerifyResult {
            verdict: format!("{}", result.verdict),
            evaluation_mode: evaluation_mode.as_str().to_owned(),
            summary,
            requirements: result
                .requirement_results
                .iter()
                .map(|r| {
                    let detail = evaluation::serialize_requirement_result_for_case(
                        r,
                        &case_ir.requirements,
                        ctx,
                    );
                    let obj = detail.as_object();
                    VerifyRequirementResult {
                        requirement_id: r.requirement_id.clone(),
                        requirement_text: obj
                            .and_then(|o| o.get("requirement_text"))
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        verdict: format!("{}", r.verdict),
                        actual: obj.and_then(|o| o.get("actual")).cloned(),
                        expected: obj.and_then(|o| o.get("expected")).cloned(),
                        margin: obj.and_then(|o| o.get("margin")).and_then(|v| v.as_f64()),
                        constraints: obj
                            .and_then(|o| o.get("constraints"))
                            .and_then(|v| v.as_array())
                            .cloned()
                            .unwrap_or_default(),
                        element_id: obj
                            .and_then(|o| o.get("element_id"))
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        requirement_element_id: obj
                            .and_then(|o| o.get("requirement_element_id"))
                            .and_then(|v| v.as_str())
                            .map(str::to_owned),
                        message: r.message.clone(),
                    }
                })
                .collect(),
            diagnostics: result.diagnostics.iter().map(|d| d.to_string()).collect(),
        }
    }

    /// Verify a RUNNING session's declared verification cases against its LIVE
    /// state — the session-based counterpart of [`verify`](Self::verify).
    ///
    /// The value supply (the load-bearing choice, steward ruling): the check-time
    /// context is a clone of the session's orchestrator `context`, whose slot
    /// store (`slots`/`slot_reader`) is wired — the ONLY path that resolves
    /// slot-store-only derived attributes like `tripped`/`trip_time` (a fresh
    /// `EvalContext` + flat `snapshot.variables` overlay silently drops them).
    /// So the verdicts reflect exactly the run the caller drove (any live-injected
    /// overrides + advanced ticks), with no re-run. This deliberately does NOT go
    /// through `verify_with_simulation` (a stateless single-SM re-run that can't
    /// drive a multi-subsystem workspace chain — task #12); it reuses the live
    /// workspace session. Verdicts come from the one `VerificationRunner`; the
    /// response is the same [`VerifyResult`] shape as `verify`, one per case.
    #[service_command(
        name = "sysml.sessions.verify",
        category = Execution,
        description = "Verify a running session's declared verification cases against its LIVE final-tick state (reads simulation-produced attributes like tripped/trip_time from the session's orchestrator context, including its slot store), routing each case through the one VerificationRunner. Reflects live-injected overrides and advanced ticks with no re-run. Optional case_names filters to specific cases; omit for all declared cases in the session's workspace.",
        returns = "Vec<VerifyResult>",
        stateful = true,
    )]
    pub fn sessions_verify(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
        #[doc = "Optional verification-case names to run; omit or empty = all declared cases"]
        case_names: Option<&[String]>,
    ) -> Result<Vec<VerifyResult>, ServiceError> {
        let key = ElementId::from_string(session_id);
        // VALUE SUPPLY: clone the LIVE session's orchestrator context (slot store
        // wired) — read-only snapshot use. A plain `.clone()` per today's API; the
        // eval-identity cull arc's site audit will migrate this to the read-only
        // constructor (this use never mutates the cloned context).
        let (
            ctx,
            uri,
            verified_at_tick,
            verified_at_time_ms,
            session_label,
            session_kind,
            session_created_at,
            session_provenance,
        ) = {
            let entry = self.sessions.get(&key).ok_or_else(|| {
                ServiceError::ElementNotFound(format!("no session: {session_id}"))
            })?;
            (
                entry.orchestrator.context.alias_live(),
                entry.uri.clone(),
                entry.orchestrator.tick(),
                // The model's own clock at the verified tick. Read here, with
                // the tick, so the pair is consistent: deriving it later from
                // tick x dt would be an inference about the very run the
                // reader is trying to inspect.
                entry.orchestrator.time_ms(),
                entry.label.clone(),
                entry.kind,
                entry.created_at_ms as i64,
                entry.provenance.clone(),
            )
        };

        let graph = self.workspace_aware_graph()?;

        let wanted: Option<std::collections::HashSet<&str>> =
            case_names.map(|names| names.iter().map(String::as_str).collect());

        let runner = sysml_runtime::cases::VerificationRunner::new();
        let mut out = Vec::new();
        // Per-element verdict rows for the canvas verdict sidecar
        // (`sysml.diagram.verdict_overlay`): the sidecar's ONLY producer is
        // this command (see `RuntimeSession::latest_verification`).
        let mut verdict_rows: Vec<(ElementId, sysml_runtime::VerdictKind, Option<f64>)> =
            Vec::new();
        // Per-case archive rows: `sessions.verify` is ALSO the archive-verdict
        // producer (the store had NO production writer — `verify.timeline` and
        // the batch-child descriptor copy read an always-empty field until
        // this). Appended to the session's archive record below; timestamps
        // per verdict, so repeat verifies preserve flip history.
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        let mut archived_verdicts: Vec<ArchivedVerdict> = Vec::new();
        for case_id in crate::workspace_verify::discover_verification_cases(&graph) {
            let Some(case_name) = graph.get_element(&case_id).and_then(|e| e.name.clone()) else {
                continue;
            };
            if let Some(wanted) = &wanted {
                if !wanted.contains(case_name.as_str()) {
                    continue;
                }
            }
            // P6: pin the case's OWN subtree digest at record time so a
            // later read (`verify.executions`) can flag "this case changed
            // since this execution". `None` only if the case element cannot
            // be digested — honest unknown, never a fabricated claim. Held
            // regardless of compile outcome: the case subtree exists whether
            // or not it compiles to runnable IR.
            let case_digest = graph.subtree_digest(&case_id);
            match sysml_runtime::cases::compile_verification_case(&case_name, &graph) {
                Ok(case_ir) => {
                    let result = runner.verify(&case_ir, &ctx);
                    archived_verdicts.push(ArchivedVerdict::trajectory(
                        case_name.clone(),
                        result.verdict.to_string(),
                        now_ms,
                        Some(sysml_store::ArchivedEvidence {
                            session_id: session_id.to_owned(),
                            tick: verified_at_tick,
                            time_ms: Some(verified_at_time_ms),
                            element_id: Some(case_ir.id.clone()),
                        }),
                        case_digest.clone(),
                    ));
                    // Case-level row first: `case_ir.id` is the verification
                    // case's real element id, which IS a scene node in views
                    // that expose the verification package — the collapsed-
                    // scene join target (in-case requirements render as
                    // compartment rows until the case node is expanded).
                    verdict_rows.push((
                        ElementId::from_string(&case_ir.id),
                        result.verdict,
                        None,
                    ));
                    crate::workspace_verify::collect_verdict_rows(
                        &result.requirement_results,
                        &case_ir.requirements,
                        &mut verdict_rows,
                    );
                    // `sessions.verify` evaluates against the LIVE session
                    // context — a trajectory verdict (§2.1a ruling (d)).
                    out.push(self.build_verify_result(
                        &result,
                        &case_ir,
                        &ctx,
                        EvaluationMode::Trajectory,
                    ));
                }
                Err(diags) => {
                    // Honest surface: a case that fails to compile is an
                    // Error-verdict VerifyResult, not a silent skip — same shape.
                    archived_verdicts.push(ArchivedVerdict::trajectory(
                        case_name.clone(),
                        sysml_runtime::cases::VerdictKind::Error.to_string(),
                        now_ms,
                        Some(sysml_store::ArchivedEvidence {
                            session_id: session_id.to_owned(),
                            tick: verified_at_tick,
                            time_ms: Some(verified_at_time_ms),
                            element_id: None,
                        }),
                        case_digest.clone(),
                    ));
                    out.push(VerifyResult {
                        verdict: format!("{}", sysml_runtime::cases::VerdictKind::Error),
                        evaluation_mode: EvaluationMode::Trajectory.as_str().to_owned(),
                        summary: VerifySummary::from_rollup(
                            sysml_runtime::aggregates::verdict_rollup_from_verdicts(&[
                                sysml_runtime::cases::VerdictKind::Error,
                            ]),
                        ),
                        requirements: Vec::new(),
                        diagnostics: diags.iter().map(|d| d.to_string()).collect(),
                    });
                }
            }
        }

        // Store the outcome on the session for the canvas verdict sidecar.
        // Whole-batch replace (this verify is the new latest), anchored at the
        // tick it was computed against. The session may since have been
        // dropped (reaped/stopped mid-verify) — then there is nothing to
        // decorate and the caller still gets its result.
        if let Some(mut entry) = self.sessions.get_mut(&key) {
            entry.latest_verification = Some(sysml_diagram::VerificationVerdicts {
                verified_at_tick,
                verdicts: verdict_rows,
            });
        }

        // APPEND the per-case verdicts onto the session's archive record
        // (upserting a minimal live-session record when none exists yet).
        // `archive_session_entry` at stop preserves these (prior_verdicts) and
        // recomputes everything else from the live session, so the interim
        // record is safe. Append — not replace — so `verify.timeline` keeps
        // per-run flip history. Failure is logged, never blocks the verify.
        if !archived_verdicts.is_empty() {
            let mut record = self.archive.get(session_id).unwrap_or_else(|| ArchivedSession {
                id: session_id.to_owned(),
                label: session_label,
                origin: self
                    .batch_origin_for_session(session_id)
                    .unwrap_or_else(|| session_kind_origin(session_kind)),
                workspace_uri: uri.clone(),
                created_at: session_created_at,
                ended_at: now_ms,
                ticks: verified_at_tick,
                overrides: Vec::new(),
                verdicts: Vec::new(),
                snapshots: Vec::new(),
                snapshot_value_units: None,
                golden: None,
                provenance: session_provenance,
            });
            record.verdicts.extend(archived_verdicts);
            if let Err(e) = self.archive.record(record) {
                tracing::warn!(session_id, error = %e, "failed to record verify verdicts to archive");
            }
        }
        Ok(out)
    }

    /// Return the verdict timeline for the current workspace — when did
    /// verdicts flip across historical verification runs.
    ///
    /// Walks the [`SessionArchive`] (populated by `sessions.stop`) and emits
    /// one `VerdictTimelineEntry` per recorded verdict, ordered
    /// oldest → newest. The UI renders one lane per `case_id` with a marker
    /// at each entry's `timestamp`; `evidence` lets the UI deep-link to the
    /// tick that produced the verdict.
    ///
    /// Workspace scoping is keyed SERVER-SIDE on session provenance (B6):
    /// every archived session records the absolute workspace root it
    /// executed against (`SessionProvenance::workspace_root`, captured at
    /// mint by `resolve_git_root`). This command resolves the current root
    /// by the SAME rule and includes only sessions whose recorded root
    /// matches — closing the cross-workspace bleed the W7b interim shape
    /// accepted. When no root resolves (nothing loaded / multiple project
    /// roots), the timeline honestly spans the whole archive; there is no
    /// identity to key on, and fabricating one would be worse.
    #[service_command(
        name = "sysml.verify.timeline",
        category = Execution,
        description = "Return verdict-flip history across past verification runs of the current workspace (scoped server-side via session provenance), with optional case and timestamp filters",
        returns = "VerdictTimelineResult { entries: [{ session_id, timestamp, case_id, verdict, evaluation_mode, evidence?, external? }] }",
    )]
    pub fn verify_timeline(
        &self,
        #[doc = "Optional list of case ids to restrict the timeline to. Omit to include every case in the workspace."]
        case_ids: Option<&[String]>,
        #[doc = "Optional Unix-millisecond lower bound: only entries with timestamp >= since_timestamp are returned. Omit for all time."]
        since_timestamp: Option<i64>,
    ) -> Result<verify_timeline::VerdictTimelineResult, ServiceError> {
        // Same conversion as `capture_session_provenance` so the query-time
        // key is byte-identical to the mint-time key.
        let root = self
            .resolve_git_root()
            .map(|p| p.to_string_lossy().into_owned());
        // Current model digest for the B10 external-verdict staleness label
        // (`matches_current_model`). Same identity as mint-time provenance.
        // `None` when nothing is loaded: a timeline over the whole archive
        // is a legitimate read with no current model to compare against.
        let current_digest = self
            .workspace_aware_graph()
            .ok()
            .map(|g| g.content_digest());
        verify_timeline::build_timeline(
            self.archive.as_ref(),
            root.as_deref(),
            case_ids,
            since_timestamp,
            current_digest.as_deref(),
        )
    }

    /// Ingest externally produced verification verdicts (B10 §3.2).
    ///
    /// The push-shaped counterpart of `sessions.verify`: a CI runner /
    /// pytest plugin / HIL rig posts one run's worth of case verdicts and
    /// they land as a synthetic [`ArchivedSession`] with
    /// `SessionOrigin::External` — a degenerate session (`ticks: 0`, no
    /// snapshots; the same shape as `sessions.verify`'s interim
    /// live-record upsert) but a full citizen of the archive: it lists,
    /// filters, golden-pins, and appears in `verify.timeline`.
    ///
    /// Fail-hard validation (fabricated traceability is worse than a
    /// rejected batch): empty verdict list, unknown verdict string,
    /// blank `tool`/`declared_digest`, or a `case_id` that does not
    /// resolve to a verification case in the current workspace all reject
    /// the WHOLE batch. A `declared_digest` that does not match the
    /// current model is recorded and labeled, never rejected — results
    /// produced against an older model are legitimate evidence and the
    /// mismatch is the signal (B6 "corroborating, never gating"
    /// precedent). Provenance is captured at ingestion by the same
    /// helper as every session mint path, so the record carries BOTH
    /// digests: the client's claim (layer 3) and the workspace state at
    /// ingestion (B6, corroborating).
    #[service_command(
        name = "sysml.verify.record_external",
        category = Execution,
        description = "Ingest externally produced verification verdicts (CI, pytest, HIL) as a synthetic archived session (origin 'external'). Requires the producing tool name and the model content digest the results were produced against (declared_digest); each verdict row is {case_id: <verification case NAME in the current workspace>, verdict: pass|fail|inconclusive|error, artifacts?: [uri]}. Unknown case names or verdict strings reject the whole batch; a stale declared_digest is recorded and labeled, never rejected. Verdicts appear in sysml.verify.timeline with the external evidence block.",
        returns = "RecordExternalResult { session_id, recorded, declared_digest, current_digest, matches_current_model }",
        stateful = true,
    )]
    pub fn verify_record_external(
        &self,
        #[doc = "Producing tool, e.g. 'pytest-7.4' (required, non-blank)"] tool: &str,
        #[doc = "ModelGraph content digest the results were produced against (required — same identity space as baselines and session provenance)"]
        declared_digest: &str,
        #[doc = "Verdict rows: array of {case_id, verdict, artifacts?}"]
        verdicts: &serde_json::Value,
        #[doc = "Opaque run reference in the tool's namespace (CI job URL, test-run id)"]
        run_ref: Option<String>,
        #[doc = "Run-level artifact URIs attached to every verdict"] artifacts: Option<
            &[String],
        >,
        #[doc = "Optional human label for the archive entry"] label: Option<String>,
    ) -> Result<RecordExternalResult, ServiceError> {
        let tool = tool.trim();
        if tool.is_empty() {
            return Err(ServiceError::InvalidInput(
                "`tool` is required and must be non-empty".to_owned(),
            ));
        }
        let declared_digest = declared_digest.trim();
        if declared_digest.is_empty() {
            return Err(ServiceError::InvalidInput(
                "`declared_digest` is required — an external verdict that cannot say what \
                 model it tested is not evidence"
                    .to_owned(),
            ));
        }
        let rows = verdicts.as_array().filter(|a| !a.is_empty()).ok_or_else(|| {
            ServiceError::InvalidInput(
                "`verdicts` must be a non-empty array of {case_id, verdict, artifacts?}"
                    .to_owned(),
            )
        })?;

        let graph = self.workspace_aware_graph()?;
        // Case identity = the case NAME (the same key `sessions.verify`
        // writes to `ArchivedVerdict.case_id`); the resolved ElementId is
        // captured onto the evidence rather than thrown away.
        let case_index: std::collections::HashMap<String, String> =
            crate::workspace_verify::discover_verification_cases(&graph)
                .into_iter()
                .filter_map(|id| {
                    graph
                        .get_element(&id)
                        .and_then(|e| e.name.clone())
                        .map(|name| (name, id.to_string()))
                })
                .collect();

        const VERDICT_STRINGS: [&str; 4] = ["pass", "fail", "inconclusive", "error"];
        let run_artifacts: &[String] = artifacts.unwrap_or(&[]);
        let mut archived: Vec<ArchivedVerdict> = Vec::with_capacity(rows.len());
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        for (i, row) in rows.iter().enumerate() {
            let case_id = row
                .get("case_id")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| {
                    ServiceError::InvalidInput(format!(
                        "verdicts[{i}]: `case_id` is required and must be a non-empty string"
                    ))
                })?;
            let verdict = row.get("verdict").and_then(|v| v.as_str()).ok_or_else(|| {
                ServiceError::InvalidInput(format!(
                    "verdicts[{i}]: `verdict` is required (pass | fail | inconclusive | error)"
                ))
            })?;
            if !VERDICT_STRINGS.contains(&verdict) {
                return Err(ServiceError::InvalidInput(format!(
                    "verdicts[{i}]: unknown verdict '{verdict}' — expected one of \
                     {VERDICT_STRINGS:?}"
                )));
            }
            let element_id = case_index.get(case_id).cloned().ok_or_else(|| {
                ServiceError::ElementNotFound(format!(
                    "verdicts[{i}]: no verification case named '{case_id}' in the current \
                     workspace — a typo'd name silently archived would be fabricated \
                     traceability; the whole batch is rejected"
                ))
            })?;
            let mut row_artifacts: Vec<String> = run_artifacts.to_vec();
            if let Some(extra) = row.get("artifacts").and_then(|v| v.as_array()) {
                row_artifacts.extend(extra.iter().filter_map(|a| a.as_str().map(str::to_owned)));
            }
            // P6: pin the resolved case's own subtree digest at ingestion,
            // the same capture as `sessions.verify` — so `verify.executions`
            // can flag whether the case changed since this external run.
            let case_digest = graph.subtree_digest(&ElementId::from_string(&element_id));
            archived.push(ArchivedVerdict::external(
                case_id,
                verdict,
                now_ms,
                sysml_store::ExternalEvidence {
                    tool: tool.to_owned(),
                    declared_digest: declared_digest.to_owned(),
                    run_ref: run_ref.clone(),
                    artifacts: row_artifacts,
                    element_id: Some(element_id),
                },
                case_digest,
            ));
        }

        // Provenance captured at ingestion by the ONE helper every session
        // mint path uses — the record carries the workspace state at
        // ingestion (corroborating) alongside the client's declared digest.
        let provenance = self.capture_session_provenance()?;
        let current_digest = provenance.model_digest.clone();
        // `workspace_uri` is run-scope metadata, not workspace identity
        // (W7b); for an ingestion there is no run scope, so it carries the
        // provenance root when one resolves — never a fabricated file URI.
        let workspace_uri = provenance.workspace_root.clone().unwrap_or_default();
        let session_id = execution::new_session_id().to_string();
        let recorded = archived.len();
        self.archive
            .record(ArchivedSession {
                id: session_id.clone(),
                label,
                origin: SessionOrigin::External,
                workspace_uri,
                created_at: now_ms,
                ended_at: now_ms,
                ticks: 0,
                overrides: Vec::new(),
                verdicts: archived,
                snapshots: Vec::new(),
                snapshot_value_units: None,
                golden: None,
                provenance: Some(provenance),
            })
            .map_err(|e| ServiceError::Store(format!("failed to record external verdicts: {e}")))?;

        Ok(RecordExternalResult {
            session_id,
            recorded,
            matches_current_model: declared_digest == current_digest,
            declared_digest: declared_digest.to_owned(),
            current_digest,
        })
    }

    /// Resolve the CURRENT workspace context the execution projections need:
    /// the whole-model digest (for external `matches_current_model`) and a
    /// per-case resolution (element id + current subtree digest, for
    /// `case_changed_since` / `case_element_id`), keyed by case NAME — the
    /// archive's `ArchivedVerdict.case_id` key. Returns empties when no
    /// workspace is loaded (a projection over the whole archive with no
    /// current model to compare against is still a legitimate read).
    fn current_execution_context(
        &self,
    ) -> (Option<String>, HashMap<String, executions::CaseResolution>) {
        let Ok(graph) = self.workspace_aware_graph() else {
            return (None, HashMap::new());
        };
        let current_digest = Some(graph.content_digest());
        let mut cases = HashMap::new();
        for id in crate::workspace_verify::discover_verification_cases(&graph) {
            if let Some(name) = graph.get_element(&id).and_then(|e| e.name.clone()) {
                cases.insert(
                    name,
                    executions::CaseResolution {
                        element_id: id.to_string(),
                        subtree_digest: graph.subtree_digest(&id),
                    },
                );
            }
        }
        (current_digest, cases)
    }

    /// Project the archive into verification EXECUTIONS (newest-first).
    ///
    /// An execution is a recorded performance of one-or-more verification
    /// cases in a context — an archived session (trajectory `sessions.verify`
    /// or an external `record_external` ingest) carrying at least one
    /// verdict. This is a PROJECTION over the existing `SessionArchive`, not
    /// a new store; a pure simulation run with no verdicts is not an
    /// execution and is filtered out. Workspace scoping is keyed server-side
    /// on B6 session provenance, identical to `verify.timeline`. Each
    /// per-case result carries the P6 stale flag (`case_changed_since`:
    /// stored case digest vs the current case subtree digest; `null` when
    /// unresolvable), and External executions carry whole-model
    /// `matches_current_model`.
    #[service_command(
        name = "sysml.verify.executions",
        category = Execution,
        description = "List verification executions (newest-first) as a projection over the session archive: each is an archived session (trajectory or external ingest) carrying >=1 verdict, with origin, evaluation_mode, B6 provenance, external run identity, per-case results (verdict + case_changed_since stale flag), and verdict counts. Verdict-less simulation runs are not executions. Optional case_name filter keeps only executions touching that case. Scoped server-side via session provenance, like sysml.verify.timeline.",
        returns = "ExecutionsResult { executions: [{ execution_id, origin, label?, timestamp, evaluation_mode, provenance?, external?, results: [{ case_id, verdict, evaluation_mode, timestamp, case_digest?, case_changed_since }], counts }] }",
    )]
    pub fn verify_executions(
        &self,
        #[doc = "Optional verification-case NAME: keep only executions that touched that case. Omit for all executions."]
        case_name: Option<String>,
    ) -> Result<executions::ExecutionsResult, ServiceError> {
        let root = self
            .resolve_git_root()
            .map(|p| p.to_string_lossy().into_owned());
        let (current_digest, current_cases) = self.current_execution_context();
        executions::build_executions(
            self.archive.as_ref(),
            root.as_deref(),
            current_digest.as_deref(),
            &current_cases,
            case_name.as_deref(),
        )
    }

    /// Per-case latest verification status across executions, qualified by
    /// evaluation_mode (P2).
    ///
    /// The EXECUTION-SIDE projection only: latest trajectory verdict and
    /// latest external verdict per case, each with its own staleness — never
    /// one flat consolidated field (the §2.1a(d) discipline). The FE
    /// composes this with the static desk-check read it already fetches; a
    /// trajectory pass never masquerades as the current static verdict, or
    /// vice versa. Composes the same core as `verify.executions` — one
    /// archive walk, reduced to the newest verdict per (case, mode).
    #[service_command(
        name = "sysml.verify.latest_status",
        category = Execution,
        description = "Per-case latest verification status across executions, context-qualified by evaluation_mode: {trajectory?, external?} with verdict, execution_id, timestamp, case_changed_since, and mode-specific provenance (trajectory model_digest; external tool + matches_current_model). Execution-side only — the caller composes it with the static read. Scoped server-side via session provenance.",
        returns = "LatestStatusResult { cases: [{ case_id, case_element_id?, latest: { trajectory?: { verdict, execution_id, timestamp, case_changed_since, model_digest? }, external?: { verdict, execution_id, timestamp, tool?, matches_current_model, case_changed_since } } }] }",
    )]
    pub fn verify_latest_status(
        &self,
    ) -> Result<executions::LatestStatusResult, ServiceError> {
        let root = self
            .resolve_git_root()
            .map(|p| p.to_string_lossy().into_owned());
        let (current_digest, current_cases) = self.current_execution_context();
        let rows = executions::build_executions(
            self.archive.as_ref(),
            root.as_deref(),
            current_digest.as_deref(),
            &current_cases,
            None,
        )?
        .executions;
        Ok(executions::build_latest_status(&rows, &current_cases))
    }

    /// Run a named analysis case and return structured results.
    #[service_command(
        name = "sysml.analysis.run",
        category = Execution,
        description = "Run a named analysis case using the solver registry with optional parameter overrides",
        returns = "AnalysisResult { case_name, tool_name, outputs, converged, iterations }",
    )]
    pub fn analysis_run(
        &self,
        #[doc = "Name of the analysis case to run"] case_name: &str,
        #[doc = "Key=value parameter overrides for the evaluation context"] overrides: &[(String, String)],
    ) -> Result<AnalysisResult, ServiceError> {
        // F3 (unification wave): workspace-aware discovery so an analysis case
        // defined in a sibling file resolves — matches the eval/verify family.
        let graph = self.workspace_aware_graph()?;

        let case_ir =
            sysml_runtime::cases::compile_analysis_case(case_name, &graph).map_err(|diags| {
                ServiceError::Execution(
                    diags
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            })?;

        let ctx = self.eval_context_with_overrides(overrides)?;
        let registry = sysml_runtime::SolverRegistry::default();
        let result = case_ir
            .execute(&registry, &ctx)
            .map_err(|e| ServiceError::Execution(e.to_string()))?;

        // Objective verdict (§7.23.2) — ONLY when the case declares a `verify`'d
        // objective; ABSENT (never null-faked) otherwise. No second solve: the
        // already-solved `result` is verified via `verify_solved_objective`
        // (the shared core `run_and_verify` also delegates to), through the ONE
        // VerificationRunner, and projected onto the shared `VerifyResult` shape.
        // Mode is `static` (study §3.2): a one-shot solve has no session/ticks.
        let objective_verdict = if case_ir.objective_requirements.is_empty() {
            None
        } else {
            let solved = case_ir.verify_solved_objective(&result, &ctx);
            Some(self.build_verify_result(
                &solved.result,
                &solved.case,
                &solved.context,
                EvaluationMode::Static,
            ))
        };

        let input_parameters: Vec<AnalysisParameter> = case_ir
            .parameters
            .iter()
            .map(|p| {
                let direction = match p.direction {
                    sysml_runtime::solver_plugin::ParamDirection::In => "in",
                    sysml_runtime::solver_plugin::ParamDirection::Out => "out",
                    sysml_runtime::solver_plugin::ParamDirection::InOut => "inout",
                };
                AnalysisParameter {
                    name: p.sysml_name.clone(),
                    param_type: p.tool_name.clone().unwrap_or_else(|| "unknown".to_owned()),
                    default_value: p.value.as_ref().map(|v| format!("{v:?}")),
                    direction: direction.to_owned(),
                }
            })
            .collect();

        Ok(AnalysisResult {
            case_name: case_name.to_owned(),
            tool_name: case_ir.tool_name,
            input_parameters,
            outputs: result
                .outputs
                .iter()
                .map(|(k, v)| (k.clone(), format!("{v:?}")))
                .collect(),
            converged: result.converged,
            iterations: result.iterations,
            objective_verdict,
        })
    }

    /// Run a trade study: evaluate design alternatives against an objective.
    #[service_command(
        name = "sysml.trade_study",
        category = Execution,
        description = "Run a trade study evaluating design alternatives against a minimize/maximize objective",
        returns = "json",
    )]
    pub fn trade_study(
        &self,
        #[doc = "Name of the trade study analysis case"] study_name: &str,
        #[doc = "Key=value parameter overrides"] overrides: &[(String, String)],
    ) -> Result<serde_json::Value, ServiceError> {
        // F3 (unification wave): workspace-aware discovery (siblings' convention).
        let graph = self.workspace_aware_graph()?;

        let ir = sysml_runtime::cases::compile_trade_study(study_name, &graph).map_err(
            |diags| {
                ServiceError::Execution(
                    diags
                        .iter()
                        .map(|d| d.to_string())
                        .collect::<Vec<_>>()
                        .join("; "),
                )
            },
        )?;

        let ctx = self.eval_context_with_overrides(overrides)?;
        let result = ir
            .execute(&ctx)
            .map_err(ServiceError::Execution)?;

        Ok(serde_json::json!({
            "study_name": study_name,
            "alternatives": result.scores.iter().map(|(n, s)| {
                serde_json::json!({"name": n, "score": s})
            }).collect::<Vec<_>>(),
            "best": result.best,
            "best_score": result.best_score,
        }))
    }

    /// Run Monte Carlo analysis with parameter distributions, statistics, and histograms.
    ///
    /// `config` is a JSON object with the following shape:
    /// ```json
    /// {
    ///   "iterations": 1000,           // optional, default 1000
    ///   "seed": 42,                   // optional u64
    ///   "parameters": [               // optional, parameter distributions
    ///     {"name": "x", "distribution": "uniform", "min": 0.0, "max": 100.0},
    ///     {"name": "y", "distribution": "normal", "mean": 50.0, "std_dev": 10.0},
    ///     {"name": "z", "distribution": "triangular", "min": 0.0, "mode": 50.0, "max": 100.0},
    ///     {"name": "w", "distribution": "fixed", "value": 42.0}
    ///   ]
    /// }
    /// ```
    ///
    /// Returns a JSON object with:
    /// - `iterations`, `seed`
    /// - `constraint_pass_rates` (per-constraint name/expression/pass_rate/counts)
    /// - `parameter_statistics` (mean/std_dev/min/max/p5/p50/p95 per parameter)
    /// - `parameter_histograms` (bin_edges/counts/max_count per parameter)
    /// - `discovered_parameters` (model attribute names + defaults, for UI config)
    /// - `discovered_constraints` (constraints + referenced variables)
    #[service_command(
        name = "sysml.montecarlo.run",
        category = Execution,
        description = "Run Monte Carlo constraint analysis with parameter distributions, statistics, and histograms",
        returns = "json",
    )]
    pub fn montecarlo(
        &self,
        #[doc = "Configuration: iterations, seed, parameters (distributions)"]
        config: &serde_json::Value,
    ) -> Result<serde_json::Value, ServiceError> {
        // Workspace-aware graph: resolves cross-file imports for elaboration.
        let graph_arc = self.workspace_aware_graph()?;

        // Parse top-level config (all optional with sensible defaults).
        let iterations = config
            .get("iterations")
            .and_then(|v| v.as_u64())
            .unwrap_or(1000) as usize;
        let seed = config.get("seed").and_then(|v| v.as_u64());

        // Extract & precompile constraints — salsa-cached (RSC-6.3, L9c).
        // `workspace_precompiled_constraints` routes through the memoized
        // `workspace_precompiled_constraints_best` query: the
        // `extract_and_precompile` walk pays once per graph revision (ADR-011
        // §3) and a warm cache returns the same `Arc` with zero re-walks.
        // Replaces the former direct
        // `sysml_runtime::constraints::extract_and_precompile(&graph_arc)`
        // bypass that recompiled the constraint set on every Monte Carlo run.
        let constraints = self.workspace_precompiled_constraints()?;

        // Build discovered_constraints array for UI config view.
        let discovered_constraints: Vec<serde_json::Value> = constraints
            .compiled
            .iter()
            .map(|tc| {
                let vars: Vec<String> = tc.expr_ir.free_variables().into_iter().collect();
                serde_json::json!({
                    "name": tc.constraint.description.clone()
                        .unwrap_or_else(|| tc.constraint.expr.clone()),
                    "expression": tc.constraint.expr,
                    "referenced_variables": vars,
                })
            })
            .collect();

        // Base eval context — salsa-cached canonical context (RSC-6.3, L9c).
        // `eval_context` routes through the memoized
        // `workspace_eval_context` / `file_eval_context` query, replacing the
        // hand-rolled per-element seed walk. Monte Carlo now evaluates against
        // the same context every other command sees (ISQ-tagged quantities,
        // calc registry, lazy `Value::Ref` resolution included) and the
        // ~N-element seed pays once per graph revision instead of per run.
        let base_context = self.eval_context()?;

        // discovered_parameters is UI-config metadata: the model's *attribute*
        // names + numeric defaults for the simulation app's parameter form.
        // This is an attribute-kind-filtered projection the cached EvalContext
        // can't reproduce — the seed binds every named feature (constraints,
        // parts, …), ISQ-tags numeric defaults to `Value::Quantity`, and
        // stores `Value::Ref` for value-less features. So it stays a direct
        // read over the already-memoized workspace graph (no salsa execution,
        // not a second eval-seed walk).
        let discovered_parameters: Vec<serde_json::Value> = graph_arc
            .elements
            .values()
            .filter(|elem| {
                matches!(
                    elem.kind,
                    sysml_core::ElementKind::AttributeUsage
                        | sysml_core::ElementKind::AttributeDefinition
                ) && elem.name.is_some()
            })
            .map(|elem| {
                let value = elem.get_prop("value").or_else(|| elem.get_prop("default"));
                let default_val = match value {
                    Some(sysml_core::Value::Int(i)) => Some(serde_json::json!(i)),
                    Some(sysml_core::Value::Float(f)) => Some(serde_json::json!(f)),
                    _ => None,
                };
                serde_json::json!({
                    "name": elem.name.as_deref().unwrap_or_default(),
                    "default": default_val,
                })
            })
            .collect();

        // Parse parameter distributions from config.parameters[].
        let mut parameters: Vec<(String, sysml_runtime::montecarlo::Distribution)> = Vec::new();
        if let Some(params_json) = config.get("parameters").and_then(|v| v.as_array()) {
            for p in params_json {
                let name = match p.get("name").and_then(|v| v.as_str()) {
                    Some(n) => n.to_owned(),
                    None => continue,
                };
                let dist_type = p
                    .get("distribution")
                    .and_then(|v| v.as_str())
                    .unwrap_or("uniform");
                let distribution = match dist_type {
                    "uniform" => {
                        let min = p.get("min").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let max = p.get("max").and_then(|v| v.as_f64()).unwrap_or(100.0);
                        sysml_runtime::montecarlo::Distribution::Uniform { min, max }
                    }
                    "normal" => {
                        let mean = p.get("mean").and_then(|v| v.as_f64()).unwrap_or(50.0);
                        let std_dev = p.get("std_dev").and_then(|v| v.as_f64()).unwrap_or(10.0);
                        sysml_runtime::montecarlo::Distribution::Normal { mean, std_dev }
                    }
                    "triangular" => {
                        let min = p.get("min").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        let mode = p.get("mode").and_then(|v| v.as_f64()).unwrap_or(50.0);
                        let max = p.get("max").and_then(|v| v.as_f64()).unwrap_or(100.0);
                        sysml_runtime::montecarlo::Distribution::Triangular { min, mode, max }
                    }
                    "fixed" => {
                        let value = p.get("value").and_then(|v| v.as_f64()).unwrap_or(0.0);
                        sysml_runtime::montecarlo::Distribution::Fixed(value)
                    }
                    _ => continue,
                };
                parameters.push((name, distribution));
            }
        }

        let mc_config = sysml_runtime::montecarlo::MonteCarloConfig {
            iterations,
            seed,
            parameters,
            sampling_strategy: sysml_runtime::montecarlo::SamplingStrategy::default(),
            correlations: None,
        };

        let runner = sysml_runtime::montecarlo::MonteCarloRunner::new(
            mc_config,
            // `MonteCarloRunner` owns its constraint set; clone the inner set
            // out of the salsa-cached `Arc`. The expensive extract+precompile
            // walk is already memoized — this is a cheap structural clone, not
            // a recompile.
            (*constraints).clone(),
            base_context,
        );
        let result = runner.run();

        // Serialize results.
        let constraint_pass_rates: Vec<serde_json::Value> = result
            .constraint_pass_rates
            .iter()
            .map(|cpr| {
                serde_json::json!({
                    "name": cpr.name,
                    "expression": cpr.expression,
                    "pass_rate": cpr.pass_rate,
                    "pass_count": cpr.pass_count,
                    "fail_count": cpr.fail_count,
                    "inconclusive_count": cpr.inconclusive_count,
                })
            })
            .collect();

        let parameter_statistics: serde_json::Map<String, serde_json::Value> = result
            .parameter_statistics
            .iter()
            .map(|(name, stats)| {
                (
                    name.clone(),
                    serde_json::json!({
                        "mean": stats.mean,
                        "std_dev": stats.std_dev,
                        "min": stats.min,
                        "max": stats.max,
                        "p5": stats.p5,
                        "p50": stats.p50,
                        "p95": stats.p95,
                    }),
                )
            })
            .collect();

        let parameter_histograms: serde_json::Map<String, serde_json::Value> = result
            .parameter_histograms
            .iter()
            .map(|(name, hist)| {
                (
                    name.clone(),
                    serde_json::json!({
                        "bin_edges": hist.bin_edges,
                        "counts": hist.counts,
                        "max_count": hist.max_count,
                    }),
                )
            })
            .collect();

        Ok(serde_json::json!({
            "iterations": result.iterations,
            "seed": result.seed,
            "constraint_pass_rates": constraint_pass_rates,
            "parameter_statistics": parameter_statistics,
            "parameter_histograms": parameter_histograms,
            "discovered_parameters": discovered_parameters,
            "discovered_constraints": discovered_constraints,
        }))
    }

    /// Run a named verification case end-to-end against the elaborated
    /// workspace graph.
    ///
    /// Composes the runtime primitives that previously lived in the LSP
    /// `handle_scenario_run`: state-machine compile (every `state def`),
    /// `extract_event_script` from the verification case, auto-step loop
    /// with per-tick assertion eval, and final `VerificationRunner`.
    ///
    /// `max_ticks` overrides `OrchestratorConfig::max_ticks` (the hard
    /// fail-stop on the auto-step loop). `None` keeps the default.
    ///
    /// Returns the JSON shape the LSP handler emits today:
    ///   { verdict, requirement_results, assertion_checkpoints, trace, final_snapshot }
    ///
    /// Per-tick assertion eval cadence: every tick, gated on every
    /// referenced variable being present in the tick context. The spec
    /// (SysML v2 §VerificationCaseUsage execution semantics) defines
    /// *what* a satisfied assertion is, not *when* during the run;
    /// per-tick + variable-presence gate is the chosen implementation
    /// decision and matches the LSP behaviour we are replacing.
    #[service_command(
        name = "sysml.scenario.run",
        category = Execution,
        description = "Run a verification case end-to-end (SM compile + event-script + auto-step + per-tick assertion eval). Returns verdict + trace + assertion_checkpoints.",
        returns = "JSON { verdict, requirement_results, assertion_checkpoints, trace, final_snapshot }",
    )]
    pub fn scenario_run(
        &self,
        #[doc = "Verification case name (e.g. \"MyVerificationCase\")"] case_name: &str,
        #[doc = "Optional max-ticks override (defaults to OrchestratorConfig::max_ticks)"]
        max_ticks: Option<u32>,
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        crate::scenario::run_scenario(&graph, case_name, max_ticks)
    }

    /// Return the full execution trace for a session as JSON.
    ///
    /// `{ trace: [TickSnapshot, ...] }` — every snapshot in
    /// `RuntimeSession::history()` serialized via the canonical
    /// `scenario::snapshot_to_json` encoder (same shape every other
    /// transport's snapshot consumer uses).
    #[service_command(
        name = "sysml.timeline.getTrace",
        category = Execution,
        description = "Return the full execution trace for a runtime session — every TickSnapshot from history() serialized to JSON.",
        returns = "JSON { trace: [TickSnapshot, ...] }",
    )]
    pub fn timeline_get_trace(
        &self,
        #[doc = "Runtime session id (ElementId-as-string)"] session_id: &str,
    ) -> Result<serde_json::Value, ServiceError> {
        match self
            .sessions()
            .get(&ElementId::from_string(session_id))
        {
            Some(session) => {
                let trace: Vec<serde_json::Value> = session
                    .history()
                    .iter()
                    .map(crate::scenario::snapshot_to_json)
                    .collect();
                Ok(serde_json::json!({ "trace": trace }))
            }
            None => Err(ServiceError::Execution(format!(
                "session not found: {session_id}"
            ))),
        }
    }

    /// Return a single TickSnapshot from a session's history.
    ///
    /// Out-of-range tick returns `ServiceError::Execution`.
    #[service_command(
        name = "sysml.timeline.getSnapshot",
        category = Execution,
        description = "Return a single TickSnapshot from a runtime session's history at the given tick index.",
        returns = "JSON TickSnapshot",
    )]
    pub fn timeline_get_snapshot(
        &self,
        #[doc = "Runtime session id (ElementId-as-string)"] session_id: &str,
        #[doc = "Tick index into session.history() (0-based)"] tick: usize,
    ) -> Result<serde_json::Value, ServiceError> {
        match self
            .sessions()
            .get(&ElementId::from_string(session_id))
        {
            Some(session) => match session.history().get(tick) {
                Some(snapshot) => Ok(crate::scenario::snapshot_to_json(snapshot)),
                None => Err(ServiceError::Execution(format!(
                    "tick {tick} not found in trace"
                ))),
            },
            None => Err(ServiceError::Execution(format!(
                "session not found: {session_id}"
            ))),
        }
    }

    /// Run constraint network solving with optional rollup and sweep.
    ///
    /// `mode` controls behavior:
    /// - `"check"` — propagation + DOF analysis only
    /// - `"rollup"` — also computes hierarchical rollup for a given property
    /// - `"sweep"` — also runs sensitivity sweep (param:lo:hi format)
    ///
    /// `rollup_property` is the property name for rollup (e.g. "mass").
    /// `sweep_spec` is the sweep specification (e.g. "speed:0:200").
    #[service_command(
        name = "sysml.solve",
        category = Execution,
        description = "Run constraint network solving with binding propagation, DOF analysis, optional rollup and sensitivity sweep",
        returns = "SolveResult { solved, iterations, unsolved, dof, rollups, sensitivity }",
    )]
    #[allow(clippy::indexing_slicing)]
    pub fn solve(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Key=value parameter overrides for known variable values"] overrides: &[(String, String)],
        #[doc = "Property name to compute hierarchical rollup (e.g. 'mass')"] rollup_property: Option<&str>,
        #[doc = "Sensitivity sweep spec as 'param:lo:hi' (e.g. 'speed:0:200')"] sweep_spec: Option<&str>,
    ) -> Result<SolveResult, ServiceError> {
        let graph = self.require_graph(uri)?;

        let mut network = sysml_runtime::solver::build_constraint_network(&graph);
        let precompiled = self.workspace_precompiled_constraints()?;

        // Apply overrides
        for (key, val) in overrides {
            let value = sysml_runtime::compiler::parse_value_string(val);
            network.set(key.clone(), value);
        }

        let result = network.propagate();
        let dof = sysml_runtime::solver::analyze_dof(&network, &precompiled);

        // Rollup
        let rollups: std::collections::HashMap<String, String> =
            if let Some(prop) = rollup_property {
                graph
                    .roots()
                    .filter(|e| e.kind.is_usage() || e.kind.is_definition())
                    .filter_map(|e| {
                        let name = e.name.as_deref()?;
                        let value = sysml_runtime::solver::compute_rollup(
                            &graph,
                            &e.id,
                            prop,
                            sysml_runtime::solver::AggregationKind::Sum,
                        )?;
                        Some((format!("{name}.{prop}"), format!("{value:?}")))
                    })
                    .collect()
            } else {
                std::collections::HashMap::new()
            };

        // Sweep
        let sensitivity = if let Some(spec) = sweep_spec {
            let parts: Vec<&str> = spec.split(':').collect();
            if parts.len() == 3 {
                let param = parts[0];
                let lo: f64 = parts[1].parse().unwrap_or(0.0);
                let hi: f64 = parts[2].parse().unwrap_or(100.0);
                let ctx = self.eval_context()?;
                let s = sysml_runtime::solver::sweep_parameter(
                    param,
                    (lo, hi),
                    101,
                    &precompiled,
                    &ctx,
                );
                Some(SensitivityInfo {
                    parameter: s.parameter.clone(),
                    steps: s.samples.len(),
                    effects: s
                        .constraint_effects
                        .iter()
                        .map(|e| SensitivityEffect {
                            constraint_name: e.constraint_name.clone(),
                            flip_value: e.flip_value,
                            flip_direction: e
                                .flip_direction
                                .as_ref()
                                .map(|d| format!("{d:?}")),
                        })
                        .collect(),
                })
            } else {
                None
            }
        } else {
            None
        };

        Ok(SolveResult {
            solved: result
                .solved
                .iter()
                .map(|(k, v)| (k.clone(), format!("{v:?}")))
                .collect(),
            iterations: result.iterations,
            unsolved: result.unsolved,
            dof: DofInfo {
                equations: dof.equations,
                variables: dof.variables,
                known_count: dof.known_count,
                free_count: dof.free_count,
                dof: dof.dof,
                status: format!("{:?}", dof.status),
            },
            rollups,
            sensitivity,
        })
    }

    /// Generate a sequence trace from flow simulation with message injection.
    #[allow(clippy::indexing_slicing)]
    #[service_command(
        name = "sysml.trace",
        category = Execution,
        description = "Generate a sequence trace by simulating message flow through compiled flow topology",
        returns = "TraceResult { lifelines, messages }",
    )]
    pub fn trace_sequence(
        &self,
        #[doc = "URI of the loaded model with flow connections"] uri: &str,
        #[doc = "Source port and payload pairs to inject"] inject_specs: &[(String, String)],
    ) -> Result<TraceResult, ServiceError> {
        let graph = self.require_graph(uri)?;

        let registry = sysml_runtime::flows::compile_ports(&graph);

        // RSC-3.5e.5 W3: classify_links folds in the flow producer (it walks the
        // flow elements itself). Route on the classified LinkGraph (real link
        // classes — PowerBond links produce no message deliveries). flow_ids are
        // dense-parallel to the link graph: FlowUsage links use their display
        // label (== the former flow id; flows intern first, in compile_flows
        // order), connector-only links get the §8 Q5 `message:src->tgt` label —
        // so ingest_classified's dense-parallel debug_assert is satisfied.
        let (link_graph, _link_diags) =
            sysml_runtime::links::classify_links(&graph, &registry);
        let flow_ids: Vec<String> = link_graph
            .iter()
            .map(|l| {
                if l.kind == sysml_runtime::links::LinkSourceKind::FlowUsage {
                    l.display_label(&graph)
                } else {
                    format!("message:{}->{}", l.source.key(), l.target.key())
                }
            })
            .collect();

        let mut plane = sysml_runtime::exchange::ExchangePlane::new();
        plane.ingest_classified(link_graph, flow_ids);

        let mut builder = sysml_runtime::sequence::SequenceTraceBuilder::new();
        let mut time_ms = 0.0;

        for (source_key, payload_str) in inject_specs {
            let payload = sysml_runtime::compiler::parse_value_string(payload_str);
            plane.send(source_key, payload.clone());
            let mut reg = registry.clone();
            let delivered = plane.route_pending_with_ports(Some(&mut reg));

            for msg in &delivered {
                builder.record_flow_delivery(
                    &msg.source,
                    &msg.target,
                    &msg.flow_id,
                    Some(msg.payload.clone()),
                    time_ms,
                );
            }
            time_ms += 10.0;
        }

        let trace = builder.build();

        Ok(TraceResult {
            lifelines: trace
                .lifelines
                .iter()
                .map(|ll| TraceLifeline {
                    index: ll.index,
                    name: ll.name.clone(),
                    kind: ll.kind.clone(),
                })
                .collect(),
            messages: trace
                .messages
                .iter()
                .filter_map(|msg| {
                    let from = trace.lifelines.get(msg.from_lifeline)?;
                    let to = trace.lifelines.get(msg.to_lifeline)?;
                    Some(TraceMessage {
                        sequence: msg.sequence,
                        from: from.name.clone(),
                        to: to.name.clone(),
                        label: msg.label.clone(),
                        timestamp_ms: msg.timestamp_ms,
                        payload: msg.payload.as_ref().map(|p| format!("{p:?}")),
                    })
                })
                .collect(),
        })
    }

    /// Inspect port connections and flow paths in a model.
    #[service_command(
        name = "sysml.flow.inspect",
        category = Execution,
        description = "Inspect port connections, flow paths, and optionally inject a payload to test delivery",
        returns = "FlowResult { ports, flows, delivery }",
    )]
    pub fn flow_inspect(
        &self,
        #[doc = "URI of the loaded model with port/flow definitions"] uri: &str,
        #[doc = "Source port key to inject payload into"] inject_source: Option<&str>,
        #[doc = "Payload value to inject (parsed as int/float/bool/string)"] inject_payload: Option<&str>,
    ) -> Result<FlowResult, ServiceError> {
        let graph = self.require_graph(uri)?;

        let registry = sysml_runtime::flows::compile_ports(&graph);

        let ports: Vec<FlowPortInfo> = registry
            .iter()
            .map(|(key, port)| FlowPortInfo {
                key: key.to_owned(),
                owner: port.owner.clone(),
                name: port.name.clone(),
                definition: port.definition.clone(),
                direction: format!("{}", port.effective_direction()),
                conjugated: port.is_conjugated,
            })
            .collect();

        // RSC-3.1 (D-3.0.1): classify the links so flow_inspect can surface the
        // link class + via_interface. RSC-3.5e.5 W3: classify_links folds in the
        // flow producer — it walks the flow elements itself, so flow_items are
        // built directly from the FlowUsage subset of the classified graph
        // (every field is native on the LinkIR; the id is the element's display
        // label, byte-identical to the former flow id).
        let (link_graph, _link_diags) =
            sysml_runtime::links::classify_links(&graph, &registry);

        let flow_items: Vec<FlowConnectionInfo> = link_graph
            .iter()
            .filter(|l| l.kind == sysml_runtime::links::LinkSourceKind::FlowUsage)
            .map(|l| FlowConnectionInfo {
                id: l.display_label(&graph),
                source: l.source.key(),
                target: l.target.key(),
                succession: l.is_succession,
                payload_type: l.payload_type.clone(),
                link_class: Some(l.class.as_str().to_owned()),
                via_interface: l.via_interface.as_ref().map(ToString::to_string),
            })
            .collect();

        // flow_ids dense-parallel to the link graph for ingest_classified
        // (FlowUsage links first, in compile_flows order, labelled by display
        // label; connector-only links get the §8 Q5 `message:src->tgt` label).
        let flow_ids: Vec<String> = link_graph
            .iter()
            .map(|l| {
                if l.kind == sysml_runtime::links::LinkSourceKind::FlowUsage {
                    l.display_label(&graph)
                } else {
                    format!("message:{}->{}", l.source.key(), l.target.key())
                }
            })
            .collect();

        // Port-level health (FL010-FL016) reads the classified LinkGraph.
        // Compute it now, before the optional delivery branch moves link_graph
        // into the exchange plane (W3 dropped the former second classify_links).
        let port_diags =
            sysml_runtime::flows::port_health_diagnostics(&link_graph, &registry, &graph);

        let delivery = if let (Some(source), Some(payload_str)) = (inject_source, inject_payload) {
            let payload = sysml_runtime::compiler::parse_value_string(payload_str);
            // RSC-3.5c.2a — reuse the pre-classified link_graph (no second
            // classify_links call). The plane routes on real classes so PowerBond
            // links produce no message deliveries. On corpus models that have no
            // PowerBond-typed flows this is parity-preserving.
            let mut plane = sysml_runtime::exchange::ExchangePlane::new();
            plane.ingest_classified(link_graph, flow_ids);
            plane.send(source, payload);
            let mut reg = registry.clone();
            let delivered = plane.route_pending_with_ports(Some(&mut reg));
            delivered
                .iter()
                .map(|msg| FlowDeliveryInfo {
                    flow_id: msg.flow_id.clone(),
                    source: msg.source.clone(),
                    target: msg.target.clone(),
                    sequence: msg.sequence,
                })
                .collect()
        } else {
            Vec::new()
        };

        // Run both graph-level (FL001-FL009) and port-level (FL010-FL016) health
        // diagnostics. The port diagnostics were computed above from the same
        // classified LinkGraph (RSC-3.5e.5 W3 — no second classify_links pass).
        let mut raw_diags = sysml_runtime::flows::flow_health_diagnostics(&graph);
        raw_diags.extend(port_diags);

        let diagnostics: Vec<FlowDiagnostic> = raw_diags
            .iter()
            .map(|d| {
                let severity = match d.severity {
                    sysml_span::Severity::Error => "error",
                    sysml_span::Severity::Warning => "warning",
                    sysml_span::Severity::Info => "info",
                };
                FlowDiagnostic {
                    code: d.code.clone().unwrap_or_default(),
                    severity: severity.to_owned(),
                    message: d.message.clone(),
                    port: None,
                }
            })
            .collect();

        Ok(FlowResult {
            ports,
            flows: flow_items,
            delivery,
            diagnostics,
        })
    }

    /// Export a loaded model as PlantUML notation.
    ///
    /// `view` selects the diagram flavour: `"general"` (default,
    /// element-and-relationship overview), `"state"` (state-transition
    /// diagram), `"action"` (sequence diagram over action participants;
    /// errors if no action elements are present), or `"sequence"`
    /// (sequence diagram over action participants with no events).
    #[service_command(
        name = "sysml.export.plantuml",
        category = Visualization,
        description = "Export a loaded model as PlantUML notation. `view` selects general | state | action | sequence (default general).",
        returns = "string (PlantUML)",
    )]
    pub fn export_plantuml(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "View selector: \"general\" (default), \"state\", \"action\", or \"sequence\""]
        view: Option<&str>,
    ) -> Result<String, ServiceError> {
        let graph = self.require_graph(uri)?;
        let view = view.unwrap_or("general");

        match view {
            "general" => Ok(sysml_diagram::to_plantuml(&graph)),
            "state" => Ok(sysml_diagram::to_plantuml_state_view(&graph)),
            "action" => {
                let participants: Vec<String> = graph
                    .elements
                    .values()
                    .filter(|e| {
                        matches!(
                            e.kind,
                            sysml_core::ElementKind::ActionUsage
                                | sysml_core::ElementKind::ActionDefinition
                        )
                    })
                    .filter_map(|e| e.name.clone())
                    .collect();
                if participants.is_empty() {
                    return Err(ServiceError::Visualization(
                        "no action elements found for action view".to_string(),
                    ));
                }
                Ok(sysml_diagram::to_plantuml_sequence(&participants, &[]))
            }
            "sequence" => {
                let participants: Vec<String> = graph
                    .elements
                    .values()
                    .filter(|e| {
                        matches!(
                            e.kind,
                            sysml_core::ElementKind::ActionUsage
                                | sysml_core::ElementKind::ActionDefinition
                        )
                    })
                    .filter_map(|e| e.name.clone())
                    .collect();
                Ok(sysml_diagram::to_plantuml_sequence(&participants, &[]))
            }
            other => Err(ServiceError::InvalidInput(format!(
                "unknown PlantUML view '{other}'; expected one of: general, state, action, sequence"
            ))),
        }
    }

    // -- Visualization delegates (Phase 3) --

    /// Export a loaded model as canonical JSON.
    #[service_command(
        name = "sysml.export.json",
        category = Visualization,
        description = "Export a loaded model as canonical SysML v2 JSON",
        returns = "string (JSON)",
    )]
    pub fn export_json(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
    ) -> Result<String, ServiceError> {
        let graph = self.require_graph(uri)?;

        Ok(visualization::export_json(&graph))
    }

    /// Render a user-authored ViewUsage / ViewDefinition as a Sprotty
    /// SModel diagram.
    ///
    /// Resolves `view_usage_id` to a `ViewSummary`, composes a
    /// `ViewRequest` from its Expose / filter / rendering memberships,
    /// and dispatches into the same rendering pipeline as
    /// [`Self::diagram`]. Honours the per-file URI scoping rules
    /// (`__workspace__` reads the merged graph; per-file URIs scope
    /// down to the matching file).
    #[service_command(
        name = "sysml.views.render",
        category = Visualization,
        description = "Render a user-authored ViewUsage as a diagram (composes ViewRequest from the view's Expose / filter / rendering memberships)",
        returns = "JSON (SModel)",
    )]
    pub fn views_render(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "ElementId of the ViewUsage / ViewDefinition to render"]
        view_usage_id: &sysml_core::ElementId,
        #[doc = "IDs of nodes to show expanded (merged with the view's auto-expansion)"]
        expanded_ids: &HashSet<String>,
    ) -> Result<serde_json::Value, ServiceError> {
        // Views render against the WORKSPACE graph only. A ViewUsage's
        // Expose / filter memberships are cross-file by spec (SysML v2
        // `ownedExposure` ranges across the namespace), so a per-file render
        // would be a structurally incomplete half-answer — it silently omits
        // exposed elements declared in other files. Fail hard when no
        // workspace is loaded; there is NO per-file fallback (RSC-6.5 / Q3,
        // fail-hard-refactor-handoff.md §P4; steward-ruled Jun 18 2026 — every
        // transport that calls views_render is workspace-aware, no CLI caller).
        let _ = uri; // all URIs render against the single workspace graph
        let ws_graph = self.workspace_aware_graph()?;
        let summaries = self.workspace_view_index()?;
        let request = visualization::build_view_request_for_view_usage_with_summaries(
            &ws_graph,
            &summaries,
            view_usage_id,
            expanded_ids,
        )
        .ok_or_else(|| ServiceError::NotFound(format!(
            "view usage {} not found in workspace",
            view_usage_id
        )))?;
        self.diagram_with_cached(WORKSPACE_URI, &request)
    }

    /// Views-family scope resolution (scope-collapse W3): query against
    /// the (more-resolved) workspace graph whenever one is loaded, and
    /// remember which file to narrow the results back to for file scope.
    ///
    /// Returns `(query_uri, narrow_to_file)`. This is NOT a soft
    /// fallback: cross-file `expose` targets only resolve on the
    /// workspace graph, so "views declared in this file" is answered
    /// most correctly by querying the workspace and then filtering to
    /// declarations whose source span lives in the requested file.
    fn views_query_scope<'a>(&self, uri: &'a str) -> (&'a str, Option<&'a str>) {
        match GraphScope::parse(uri) {
            GraphScope::Workspace => (WORKSPACE_URI, None),
            GraphScope::File(_) if self.workspace_aware_graph().is_ok() => {
                (WORKSPACE_URI, Some(uri))
            }
            GraphScope::File(_) => (uri, None),
        }
    }

    /// List user-authored ViewUsage / ViewDefinition elements declared
    /// in a loaded model.
    ///
    /// Each entry surfaces its Expose memberships (qualified names),
    /// ViewRenderingMembership children, and ElementFilterMembership
    /// children. Resolution of Expose targets and evaluation of filter
    /// expressions are performed at render time (Phase 5b/5c) — this
    /// command is the catalog the UI uses to populate the Views panel.
    ///
    /// `uri = "__workspace__"` lists views from every loaded file.
    /// Any other URI lists only views whose source span lives in the
    /// matching file.
    #[service_command(
        name = "sysml.views.list",
        category = Query,
        description = "Legacy wrapper over sysml.query: list user-authored ViewUsage / ViewDefinition elements (id, name, exposed namespaces, render and filter members)",
        returns = "Vec<ViewSummary>",
    )]
    pub fn views_list(
        &self,
        #[doc = "URI of the loaded model. Use '__workspace__' for the merged workspace graph."]
        uri: &str,
    ) -> Result<Vec<ViewSummary>, ServiceError> {
        let (query_uri, narrow_to_file) = self.views_query_scope(uri);
        let summaries = self.query_all_summaries(
            query_uri,
            sysml_query::QuerySpec {
                filter: sysml_query::Filter::View { viewpoint_id: None },
                projection: sysml_query::Projection::SummaryExpand,
                ..sysml_query::QuerySpec::default()
            },
        )?;
        let mut views = view_summaries_from_query_rows(summaries);
        if let Some(file_uri) = narrow_to_file {
            views.retain(|summary| {
                source_span_matches_uri(summary.source_span.as_ref(), file_uri)
            });
        }
        Ok(views)
    }

    /// Find every ViewUsage / ViewDefinition that satisfies a given
    /// viewpoint. Walks the model for views whose nested
    /// `ViewpointUsage` children specialise / type / subset the
    /// referenced `ViewpointDefinition`.
    #[service_command(
        name = "sysml.views.by_viewpoint",
        category = Query,
        description = "Legacy wrapper over sysml.query: list user-authored views that satisfy the given ViewpointDefinition / ViewpointUsage",
        returns = "Vec<ViewSummary>",
    )]
    pub fn views_by_viewpoint(
        &self,
        #[doc = "URI of the loaded model. Use '__workspace__' for the merged workspace graph."]
        uri: &str,
        #[doc = "ElementId of the ViewpointDefinition / ViewpointUsage."]
        viewpoint_id: &sysml_core::ElementId,
    ) -> Result<Vec<ViewSummary>, ServiceError> {
        let (query_uri, narrow_to_file) = self.views_query_scope(uri);
        let summaries = self.query_all_summaries(
            query_uri,
            sysml_query::QuerySpec {
                filter: sysml_query::Filter::View {
                    viewpoint_id: Some(viewpoint_id.clone()),
                },
                projection: sysml_query::Projection::SummaryExpand,
                ..sysml_query::QuerySpec::default()
            },
        )?;
        let mut views = view_summaries_from_query_rows(summaries);
        if let Some(file_uri) = narrow_to_file {
            views.retain(|summary| {
                source_span_matches_uri(summary.source_span.as_ref(), file_uri)
            });
        }
        Ok(views)
    }

    /// Find every ViewpointDefinition / ViewpointUsage whose
    /// StakeholderMembership children reference the given stakeholder
    /// PartUsage. Returns ElementIds (UI dereferences with
    /// `sysml.element`).
    #[service_command(
        name = "sysml.viewpoints.by_stakeholder",
        category = Query,
        description = "Legacy wrapper over sysml.query: list ViewpointDefinitions / ViewpointUsages whose StakeholderMembership references the given stakeholder PartUsage",
        returns = "Vec<ElementId>",
    )]
    pub fn viewpoints_by_stakeholder(
        &self,
        #[doc = "URI of the loaded model. Use '__workspace__' for the merged workspace graph."]
        uri: &str,
        #[doc = "ElementId of the stakeholder PartUsage."]
        stakeholder_id: &sysml_core::ElementId,
    ) -> Result<Vec<sysml_core::ElementId>, ServiceError> {
        // Ids carry no source span to narrow by, so file scope here only
        // picks the query graph — the historical behavior, preserved.
        let (query_uri, _narrow_to_file) = self.views_query_scope(uri);
        self.query_all_ids(
            query_uri,
            sysml_query::QuerySpec {
                filter: sysml_query::Filter::Viewpoint {
                    stakeholder_id: Some(stakeholder_id.clone()),
                },
                projection: sysml_query::Projection::Ids,
                ..sysml_query::QuerySpec::default()
            },
        )
    }

    /// Build a `view scratch :> Interconnection { expose ...; }`
    /// snippet from a list of qualified names or element references,
    /// for the editor's "create view def from selection" affordance.
    /// Pure string formatting — no graph access.
    #[service_command(
        name = "sysml.views.create_scratch",
        category = Visualization,
        description = "Build a 'view scratch :> Interconnection { expose ...; }' source snippet from a list of qualified names",
        returns = "string (SysML source)",
    )]
    pub fn views_create_scratch(
        &self,
        #[doc = "Qualified names or element references to expose. One `expose <name>;` line is emitted per entry."]
        expose: &Vec<String>,
    ) -> Result<String, ServiceError> {
        Ok(sysml_core::views_create_scratch_snippet(expose))
    }

    /// Open a diagram for `uri` and return the projected SModel JSON.
    ///
    /// Updates `diagram_manager().open_diagrams` so subsequent diagnostic
    /// cycles can auto-refresh the same view. Diagnostics are pulled from
    /// `service.diagnostics(uri)` and overlaid onto the SGraph in place.
    #[service_command(
        name = "sysml.diagram.open",
        category = Visualization,
        description = "Open a diagram for the given URI and view type, returning the SModel JSON. Updates open_diagrams for auto-refresh.",
        returns = "JSON (SGraph)",
    )]
    pub fn diagram_open(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "View type (e.g. \"general\", \"state\", \"action\"); defaults to \"general\""]
        view_type: Option<&str>,
    ) -> Result<serde_json::Value, ServiceError> {
        let view_type = diagram::parse_view_type(view_type.unwrap_or("general"));
        let expanded = self
            .diagram_manager
            .expanded_nodes
            .get(uri)
            .map(|e| e.value().clone())
            .unwrap_or_default();
        let value = self.build_diagram_smodel(uri, view_type, &expanded)?;
        self.diagram_manager
            .open_diagrams
            .insert(uri.to_owned(), view_type);
        Ok(value)
    }

    /// Switch the diagram view-type for `uri` and return the projected SModel.
    ///
    /// Behaves like `diagram_open` but with an explicit view_type (no default)
    /// and the same `open_diagrams` write semantics.
    #[service_command(
        name = "sysml.diagram.view",
        category = Visualization,
        description = "Switch a diagram's view-type for the given URI, returning the SModel JSON. Updates open_diagrams.",
        returns = "JSON (SGraph)",
    )]
    pub fn diagram_view(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "View type (e.g. \"general\", \"state\", \"action\")"] view_type: &str,
    ) -> Result<serde_json::Value, ServiceError> {
        let vt = diagram::parse_view_type(view_type);
        let expanded = self
            .diagram_manager
            .expanded_nodes
            .get(uri)
            .map(|e| e.value().clone())
            .unwrap_or_default();
        let value = self.build_diagram_smodel(uri, vt, &expanded)?;
        self.diagram_manager
            .open_diagrams
            .insert(uri.to_owned(), vt);
        Ok(value)
    }

    /// Return the renderer-agnostic **ViewModel** for `uri` + `view` as JSON.
    ///
    /// The ViewModel (Bucket 1) is the canonical wire artifact a frontend
    /// consumes: the promoted `DiagramIR` scene (geometry/structure) plus its
    /// renderer-agnostic addenda — design tokens (the color palette), the
    /// `ElementId↔Span` text-map (the bidirectional text↔diagram link), and
    /// interaction descriptors (semantic affordances joined by `ElementId`, e.g.
    /// the go-to-definition target). Unlike `sysml.diagram.view` (which returns a
    /// Sprotty SModel for the live renderer), this exposes the full ViewModel for
    /// the new React-SVG renderer and any transport.
    ///
    /// Cache-backed (routes through `cached_view_model` →
    /// `workspace_view_model_best`); reads the per-URI expanded-node state like
    /// `diagram_view`. No state mutation.
    #[service_command(
        name = "sysml.diagram.viewmodel",
        category = Visualization,
        description = "Return the renderer-agnostic ViewModel (scene + design tokens + text-map + interaction descriptors + frame) for a DECLARED view (ViewUsage / ViewDefinition), scoped by its Expose / filter memberships, as JSON. The canonical wire artifact for the new renderer. Mirrors sysml.views.render but returns the ViewModel instead of an SModel.",
        returns = "JSON (ViewModel)",
    )]
    pub fn diagram_view_model(
        &self,
        #[doc = "URI of the loaded model (unused — views render against the workspace graph)"]
        uri: &str,
        #[doc = "ElementId of the ViewUsage / ViewDefinition to render"]
        view_usage_id: &sysml_core::ElementId,
        #[doc = "IDs of nodes to show expanded (merged with the view's auto-expansion)"]
        expanded_ids: &HashSet<String>,
    ) -> Result<serde_json::Value, ServiceError> {
        // Like `views_render`: a view's Expose / filter memberships are cross-file
        // by spec, so the scene is composed against the WORKSPACE graph. Scoping by
        // the view's exposes is what keeps the projection from dumping the whole
        // elaborated graph (incl. the standard library). The expose id in the
        // resulting request key also drives `frame` population (Bucket 1.10).
        let _ = uri;
        let ws_graph = self.workspace_aware_graph()?;
        let summaries = self.workspace_view_index()?;
        let request = visualization::build_view_request_for_view_usage_with_summaries(
            &ws_graph,
            &summaries,
            view_usage_id,
            expanded_ids,
        )
        .ok_or_else(|| {
            ServiceError::NotFound(format!("view usage {view_usage_id} not found in workspace"))
        })?;
        self.view_model_with_cached(WORKSPACE_URI, &request)
    }

    /// Return the per-tick **simulation overlay** for a live session, joined to
    /// the diagram scene of `view_type` by `ElementId`.
    ///
    /// This is the session-scoped companion to `sysml.diagram.viewmodel`: the
    /// ViewModel (scene + tokens + text-map + interactions) is a pure function of
    /// the graph and salsa-cached; the simulation overlay is **session state**
    /// (active SM substates, live scalar readings, time-series channels) built
    /// from the session's latest [`ExecutionSnapshot`] + time-series buffer, so
    /// it is delivered as a separate artifact rather than embedded in the
    /// graph-keyed ViewModel (Bucket 1.8). The frontend fetches the scene once
    /// and polls this per tick, joining `SimOverlay::elements` /
    /// `OverlayChannel::element_id` to scene nodes by id.
    ///
    /// Fails hard on an unknown session or one that has not produced a snapshot
    /// yet — there is no "empty overlay" success shape.
    #[service_command(
        name = "sysml.diagram.sim_overlay",
        category = Visualization,
        description = "Return the per-tick simulation overlay (active-element highlights, live scalar badges, time-series channel directory) for a live session, joined to the given DECLARED view's scene by ElementId, as JSON. Companion to sysml.diagram.viewmodel — pass the SAME view_usage_id so the overlay joins the same scene.",
        returns = "JSON (SimOverlay)",
    )]
    pub fn diagram_sim_overlay(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
        #[doc = "ElementId of the ViewUsage / ViewDefinition the overlay joins to (must match the sysml.diagram.viewmodel call)"]
        view_usage_id: &sysml_core::ElementId,
        #[doc = "IDs of nodes shown expanded (must match the viewmodel call so scene ids align)"]
        expanded_ids: &HashSet<String>,
    ) -> Result<serde_json::Value, ServiceError> {
        let key = ElementId::from_string(session_id);
        let entry = self.sessions.get(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        let session = entry.value();
        let snapshot = session.history().back().ok_or_else(|| {
            ServiceError::ElementNotFound(format!(
                "session {session_id} has not produced a snapshot yet"
            ))
        })?;

        // Build the SAME scoped scene the viewmodel command builds (declared view,
        // workspace graph) so overlay element ids join the rendered scene.
        let ws_graph = self.workspace_aware_graph()?;
        let summaries = self.workspace_view_index()?;
        let request = visualization::build_view_request_for_view_usage_with_summaries(
            &ws_graph,
            &summaries,
            view_usage_id,
            expanded_ids,
        )
        .ok_or_else(|| {
            ServiceError::NotFound(format!("view usage {view_usage_id} not found in workspace"))
        })?;
        let view_model = self.cached_view_model(&request)?.ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no view-model for view: {view_usage_id}"))
        })?;

        let overlay = sysml_diagram::build_sim_overlay(
            &view_model.scene,
            snapshot,
            session.time_series(),
            &ws_graph,
        );
        Ok(serde_json::to_value(&overlay).unwrap_or_default())
    }

    /// Return the per-run **verdict overlay** for a live session, joined to the
    /// diagram scene of `view_usage_id` by `ElementId`.
    ///
    /// The companion verdict sidecar to `sysml.diagram.viewmodel`: where the
    /// ViewModel is a pure function of the graph (salsa-cached) and
    /// `sysml.diagram.sim_overlay` carries live activity/value state, this
    /// carries the **constraint solver verdicts** (pass/fail badges + the solved
    /// scalar value behind each) built from the session's latest
    /// [`ExecutionSnapshot::constraint_results`]. It is **session state**,
    /// keyed by the constraint usage's `ElementId`, delivered as a separate
    /// artifact rather than embedded in the graph-keyed ViewModel — this is the
    /// correct home for the solver state the retired `Parametric` generator used
    /// to (wrongly) bake into the salsa-cached scene. The frontend fetches the
    /// scene once and polls this per run, joining `VerdictOverlay::elements` to
    /// scene nodes by id.
    ///
    /// Fails hard on an unknown session or one that has not produced a snapshot
    /// yet — there is no "empty overlay" success shape.
    #[service_command(
        name = "sysml.diagram.verdict_overlay",
        category = Visualization,
        description = "Return the per-run verdict overlay (constraint solver pass/fail badges + solved scalar values) for a live session, joined to the given DECLARED view's scene by ElementId, as JSON. Companion verdict sidecar to sysml.diagram.viewmodel — pass the SAME view_usage_id so the overlay joins the same scene.",
        returns = "JSON (VerdictOverlay)",
    )]
    pub fn diagram_verdict_overlay(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
        #[doc = "ElementId of the ViewUsage / ViewDefinition the overlay joins to (must match the sysml.diagram.viewmodel call)"]
        view_usage_id: &sysml_core::ElementId,
        #[doc = "IDs of nodes shown expanded (must match the viewmodel call so scene ids align)"]
        expanded_ids: &HashSet<String>,
    ) -> Result<serde_json::Value, ServiceError> {
        let key = ElementId::from_string(session_id);
        let entry = self.sessions.get(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        let session = entry.value();
        let snapshot = session.history().back().ok_or_else(|| {
            ServiceError::ElementNotFound(format!(
                "session {session_id} has not produced a snapshot yet"
            ))
        })?;

        // Build the SAME scoped scene the viewmodel command builds (declared view,
        // workspace graph) so overlay element ids join the rendered scene.
        let ws_graph = self.workspace_aware_graph()?;
        let summaries = self.workspace_view_index()?;
        let request = visualization::build_view_request_for_view_usage_with_summaries(
            &ws_graph,
            &summaries,
            view_usage_id,
            expanded_ids,
        )
        .ok_or_else(|| {
            ServiceError::NotFound(format!("view usage {view_usage_id} not found in workspace"))
        })?;
        let view_model = self.cached_view_model(&request)?.ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no view-model for view: {view_usage_id}"))
        })?;

        // Merge the session's stored verification outcome (populated by
        // `sessions_verify` ONLY) — verification wins on collision, stale
        // verdicts stay labeled via `verified_at_tick`.
        let overlay = sysml_diagram::build_verdict_overlay(
            &view_model.scene,
            snapshot,
            session.latest_verification.as_ref(),
        );
        Ok(serde_json::to_value(&overlay).unwrap_or_default())
    }

    /// Return the **diagnostics overlay** for a declared view's scene: which
    /// scene elements carry validation diagnostics (badge severity + tooltip
    /// detail), joined by `ElementId`.
    ///
    /// The diagnostics companion to `sysml.diagram.viewmodel` — pass the SAME
    /// `view_usage_id` so the overlay joins the same scene. Unlike the verdict
    /// / sim overlays (session-derived), this reads the workspace's
    /// **readiness-gated** diagnostics (`self.diagnostics`), so it needs no
    /// session and reflects the current model, not a run.
    ///
    /// A [`Diagnostic`] carries a span, not an `ElementId`, so each span is
    /// resolved to an element via the ide-db position-map reverse index
    /// (`element_id_at`) before the pure `sysml-diagram` builder joins it to
    /// the scene by id. Diagnostics with no span, an unresolvable span, or an
    /// element not present in this scene are skipped — the overlay stays sparse
    /// and scene-scoped (no fabricated placement).
    #[service_command(
        name = "sysml.diagram.diagnostic_overlay",
        category = Visualization,
        description = "Return the diagnostics overlay (validation-diagnostic badges: worst-case severity + per-message tooltip detail) for the given DECLARED view's scene, joined by ElementId, as JSON. Companion to sysml.diagram.viewmodel — pass the SAME view_usage_id so the overlay joins the same scene. Reads workspace diagnostics (readiness-gated); needs no session.",
        returns = "JSON (DiagnosticOverlay)",
    )]
    pub fn diagram_diagnostic_overlay(
        &self,
        #[doc = "ElementId of the ViewUsage / ViewDefinition the overlay joins to (must match the sysml.diagram.viewmodel call)"]
        view_usage_id: &sysml_core::ElementId,
        #[doc = "IDs of nodes shown expanded (must match the viewmodel call so scene ids align)"]
        expanded_ids: &HashSet<String>,
    ) -> Result<serde_json::Value, ServiceError> {
        // Workspace diagnostics — aggregate over every loaded user file (the
        // `__workspace__` sentinel is a graph-accessor key, not a diagnostics
        // file id). Each per-file call is already readiness-gated at its one
        // home (`compute_full_diagnostics`); do not re-filter here.
        let diags: Vec<sysml_span::Diagnostic> = self
            .loaded_uris()
            .into_iter()
            .filter(|u| u != WORKSPACE_URI)
            .filter_map(|u| self.diagnostics(&u).ok())
            .flatten()
            .collect();

        // Build the SAME scoped scene the viewmodel command builds (declared
        // view, workspace graph) so overlay element ids join the rendered scene.
        let ws_graph = self.workspace_aware_graph()?;
        let summaries = self.workspace_view_index()?;
        let request = visualization::build_view_request_for_view_usage_with_summaries(
            &ws_graph,
            &summaries,
            view_usage_id,
            expanded_ids,
        )
        .ok_or_else(|| {
            ServiceError::NotFound(format!("view usage {view_usage_id} not found in workspace"))
        })?;
        let view_model = self.cached_view_model(&request)?.ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no view-model for view: {view_usage_id}"))
        })?;

        // Resolve each diagnostic's span → ElementId via the position-map
        // reverse index, then let the pure builder join by hard id. Identity-
        // first: no name is ever consulted. Unresolvable spans drop out here.
        let resolved: Vec<(ElementId, &sysml_span::Diagnostic)> = {
            let guard = self.host.lock().unwrap();
            let analysis = guard.analysis();
            diags
                .iter()
                .filter_map(|d| {
                    let span = d.span.as_ref()?;
                    let file_id = guard.file_id(&span.file)?;
                    let sf = guard.source_file(file_id)?;
                    let id = analysis.position_map(sf).element_id_at(span.start)?;
                    Some((id, d))
                })
                .collect()
        };

        let overlay = sysml_diagram::build_diagnostic_overlay(&view_model.scene, &resolved);
        Ok(serde_json::to_value(&overlay).unwrap_or_default())
    }

    /// Re-project a diagram with a caller-supplied expanded-node set.
    ///
    /// Replaces the per-URI `expanded_nodes` set with `expanded_node_ids`,
    /// prunes stale ids, then projects + overlays. Caller-driven set is the
    /// authoritative state: transports that maintain toggle semantics resolve
    /// to the final set before invoking.
    #[service_command(
        name = "sysml.diagram.expand",
        category = Visualization,
        description = "Re-project a diagram with the given expanded-node set, returning the SModel JSON. Replaces expanded_nodes state for the URI.",
        returns = "JSON (SGraph)",
    )]
    pub fn diagram_expand(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "View type (e.g. \"general\", \"state\", \"action\")"] view_type: &str,
        #[doc = "Full set of expanded node ids (replaces existing state)"]
        expanded_node_ids: &Vec<String>,
    ) -> Result<serde_json::Value, ServiceError> {
        let vt = diagram::parse_view_type(view_type);
        let mut expanded: HashSet<String> = expanded_node_ids.iter().cloned().collect();
        // Prune stale ids against the elaborated graph before storing.
        if let Ok(graph) = self.workspace_aware_graph() {
            diagram::prune_expanded_ids(&mut expanded, &graph);
        }
        self.diagram_manager
            .expanded_nodes
            .insert(uri.to_owned(), expanded.clone());
        self.build_diagram_smodel(uri, vt, &expanded)
    }

    /// Headless SModel export (no overlay, no state mutation).
    ///
    /// CLI-facing peer of `diagram_open`. `expand_all` forces every element
    /// id into the expanded set; otherwise an empty set is used.
    #[service_command(
        name = "sysml.export.smodel",
        category = Visualization,
        description = "Export a Sprotty SModel JSON for the given URI and view. `expand_all` expands every element; otherwise the expansion set is empty. No diagnostic overlay, no state mutation.",
        returns = "JSON (SGraph)",
    )]
    pub fn export_smodel(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "View name (general / interconnection / state / action / requirements / browser / sequence / grid / geometry / parametric)"]
        view: &str,
        #[doc = "When true, the expanded-node set contains every element id in the graph"]
        expand_all: bool,
    ) -> Result<serde_json::Value, ServiceError> {
        let vt = diagram::parse_view_type(view);
        // RSC-6.5 / L13: resolved+elaborated via salsa (was: ad-hoc
        // `elaborate()` of an unresolved parse-only clone).
        let graph = self.elaborated_graph(uri)?;
        let expanded_ids: HashSet<String> = if expand_all {
            graph.elements.keys().map(|id| id.to_string()).collect()
        } else {
            HashSet::new()
        };
        let request = sysml_diagram::ViewRequest::new(vt).with_expanded(expanded_ids);
        let sgraph = sysml_diagram::smodel::to_smodel_with(&graph, &request);
        Ok(serde_json::to_value(&sgraph).unwrap_or_default())
    }

    /// Render the flow connections for `uri` as both PlantUML text AND a
    /// Sprotty SModel sequence diagram.
    ///
    /// Returns the design-target `{ plantuml: String, smodel: object }`
    /// dual-shape: a single graph walk feeds both renderers so transports
    /// can pick the format they need without paying for two queries.
    ///
    /// `flow_id` is reserved for narrowing to a single named flow; today
    /// it is accepted but ignored (the renderer always compiles every flow
    /// in the graph). Mirrors the prior LSP `handle_flow_visualize` shape
    /// minus the `format`/`content` keys.
    #[service_command(
        name = "sysml.flow.visualize",
        category = Visualization,
        description = "Render the flow connections for the given URI. Returns {plantuml, smodel} — PlantUML sequence text + the Sprotty SModel JSON for the same flows.",
        returns = "JSON {plantuml, smodel}",
    )]
    pub fn flow_visualize(
        &self,
        #[doc = "Optional flow id to narrow to (reserved; today renders all flows)"]
        _flow_id: Option<&str>,
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        // One classified LinkGraph is the single source of truth — both the
        // PlantUML text and the SModel are derived from it, so they agree.
        // Connectors surface and PowerBonds (continuous physics) drop
        // (RSC-3.5c.2b).
        let (lg, _diags) = sysml_runtime::classify_links_from_graph(&graph);

        let mut participants: Vec<String> = Vec::new();
        let mut events: Vec<sysml_diagram::SequenceEvent> = Vec::new();
        for link in lg.iter().filter(|l| l.class.routes_as_message()) {
            for p in [&link.source.owner, &link.target.owner] {
                if !participants.contains(p) {
                    participants.push(p.clone());
                }
            }
            events.push(sysml_diagram::SequenceEvent {
                source: link.source.owner.clone(),
                target: link.target.owner.clone(),
                label: format!("{} -> {}", link.source.port, link.target.port),
            });
        }

        let plantuml = sysml_diagram::to_plantuml_sequence(&participants, &events);
        let sgraph = sysml_diagram::smodel::generate_sequence_from_flows(&lg, Some(&graph));
        let smodel = serde_json::to_value(&sgraph).unwrap_or_default();

        Ok(serde_json::json!({
            "plantuml": plantuml,
            "smodel": smodel,
        }))
    }

    /// Render a single named action's control-flow graph as both PlantUML
    /// activity text AND an SModel ActionFlowView.
    ///
    /// Returns the design-target `{ plantuml: String, smodel: object }`
    /// dual-shape. Compiles the action via `sysml_runtime::compile_action`
    /// and renders both formats from the resulting `ActionGraphIR`.
    #[service_command(
        name = "sysml.action.visualize",
        category = Visualization,
        description = "Render a named action's control flow. Returns {plantuml, smodel} — PlantUML activity text + the Sprotty SModel ActionFlowView for the same action.",
        returns = "JSON {plantuml, smodel}",
    )]
    pub fn action_visualize(
        &self,
        #[doc = "Action definition or usage name (qualified or short)"] action_id: &str,
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let ir = sysml_runtime::actions::compile_action(action_id, &graph).map_err(|diags| {
            let msgs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
            ServiceError::Visualization(msgs.join("; "))
        })?;

        let plantuml = sysml_diagram::to_plantuml_activity(&ir);
        let sgraph = sysml_diagram::smodel::generate_action_named(&graph, action_id);
        let smodel = serde_json::to_value(&sgraph).unwrap_or_default();

        Ok(serde_json::json!({
            "plantuml": plantuml,
            "smodel": smodel,
        }))
    }

    // -- Store delegates (Phase 3) --

    /// Access the store backend (if configured).
    pub fn store(&self) -> Option<&Arc<RwLock<dyn sysml_store::Store + Send + Sync>>> {
        self.store.as_ref()
    }

    /// Store a model snapshot.
    #[service_command(
        name = "sysml.store.save",
        category = Storage,
        description = "Store a model snapshot with version metadata",
        returns = "()",
    )]
    pub fn store_model(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "Snapshot metadata (commit ID, message, timestamp)"] meta: SnapshotMeta,
        #[doc = "The model graph to store"] graph: &ModelGraph,
    ) -> Result<(), ServiceError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;

        let mut guard = store
            .write()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;

        storage::store_model(&mut *guard, project, meta, graph)
    }

    /// Load a model snapshot from the store.
    #[service_command(
        name = "sysml.store.load",
        category = Storage,
        description = "Load a stored model snapshot by project and commit ID",
        returns = "ModelGraph?",
    )]
    pub fn load_model(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "Commit identifier"] commit: &CommitId,
    ) -> Result<Option<ModelGraph>, ServiceError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;

        let guard = store
            .read()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;

        storage::load_model(&*guard, project, commit)
    }

    /// Get the latest commit for a project from the store.
    #[service_command(
        name = "sysml.store.latest",
        category = Storage,
        description = "Get the latest commit ID for a project from the store",
        returns = "CommitId?",
    )]
    pub fn latest_commit(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
    ) -> Result<Option<CommitId>, ServiceError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;

        let guard = store
            .read()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;

        storage::latest_commit(&*guard, project)
    }

    /// List all projects in the store.
    #[service_command(
        name = "sysml.store.projects",
        category = Storage,
        description = "List all projects in the store",
        returns = "Vec<ProjectId>",
    )]
    pub fn list_projects(&self) -> Result<Vec<ProjectId>, ServiceError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;

        let guard = store
            .read()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;

        storage::list_projects(&*guard)
    }

    /// List commits for a project in the store.
    #[service_command(
        name = "sysml.store.history",
        category = Storage,
        description = "List all commits for a project (most recent first)",
        returns = "Vec<SnapshotMeta>",
    )]
    pub fn list_commits(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
    ) -> Result<Vec<SnapshotMeta>, ServiceError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;

        let guard = store
            .read()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;

        storage::list_commits(&*guard, project)
    }

    /// Diff two stored snapshots (B3 — baselines/diff spine).
    #[service_command(
        name = "sysml.store.diff",
        category = Storage,
        description = "Element-level diff between two stored snapshots. `from`/`to` each accept a baseline name (resolved first) or a commit id; omitted `to` means the latest commit. Optional element_ids narrows the result (the changed-since composition). Scope renames / anonymous-sibling shifts surface as removed+added per the ADR-009 identity contract.",
        returns = "GraphDiff",
    )]
    pub fn store_diff(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "Baseline name or commit id to diff from"] from: &str,
        #[doc = "Baseline name or commit id to diff to (default: latest commit)"] to: Option<
            String,
        >,
        #[doc = "Element ids to narrow the diff to (empty = no filter)"] element_ids: &HashSet<
            String,
        >,
    ) -> Result<sysml_core::diff::GraphDiff, ServiceError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;
        let guard = store
            .read()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;
        let ids: Option<Vec<sysml_id::ElementId>> = if element_ids.is_empty() {
            None
        } else {
            Some(
                element_ids
                    .iter()
                    .map(|s| sysml_id::ElementId::from_string(s.clone()))
                    .collect(),
            )
        };
        storage::diff_snapshots(&*guard, project, from, to.as_deref(), ids.as_deref())
    }

    /// Resolve the directory git provenance is captured under.
    ///
    /// ONE home for provenance root resolution (B6 + session provenance):
    /// the construction-time workspace root (LSP path), else the UNIQUE
    /// user directory-rooted project registered by
    /// load_workspace/open_context (the HTTP server constructs rootless
    /// and loads later; stdlib-bundle projects are excluded). Several
    /// roots = ambiguous → `None`, never a guess.
    fn resolve_git_root(&self) -> Option<std::path::PathBuf> {
        self.workspace_root.clone().or_else(|| {
            let host = self.host.lock().unwrap();
            let mut roots: Vec<_> = host
                .project_directory_roots()
                .into_iter()
                .filter(|(handle, _)| handle.0 != sysml_ide_db::host::STDLIB_BUNDLE_PROJECT_ID)
                .map(|(_, dir)| dir)
                .collect();
            match roots.len() {
                1 => roots.pop(),
                _ => None,
            }
        })
    }

    /// Capture session provenance at mint time (B6 remainder).
    ///
    /// ONE home called by every path that mints a [`execution::RuntimeSession`]
    /// (`simulate_start` / `action_start` / `orchestrate_workspace_start` /
    /// `continuous_auto` — `sessions.create` composes those, so it inherits
    /// capture for free). The digest is `ModelGraph::content_digest()` over
    /// the same workspace-aware graph the session executes — identical to
    /// the identity the store's commits/baselines use, so digest equality
    /// against a baseline commit id is real equivalence. Git state is
    /// corroborating only (dirty recorded, never refused), same rules as
    /// baseline provenance.
    ///
    /// Fails only if the workspace graph is unreadable — which every mint
    /// site has already required to build the session, so in practice this
    /// never adds a failure path.
    fn capture_session_provenance(
        &self,
    ) -> Result<sysml_store::SessionProvenance, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let root = self.resolve_git_root();
        let git = root.as_deref().and_then(git_provenance::capture);
        let file_manifest = self.workspace_file_manifest(root.as_deref());
        Ok(sysml_store::SessionProvenance {
            model_digest: graph.content_digest(),
            git,
            workspace_root: root.map(|p| p.to_string_lossy().into_owned()),
            file_manifest,
        })
    }

    /// Per-file provenance manifest for the current workspace (§6.2).
    ///
    /// Every non-stdlib source file's SHA-256 content hash, keyed by a
    /// workspace-relative path. Enumerated through
    /// [`FileSet::user_file_ids`](sysml_ide_db::FileSet::user_file_ids) —
    /// the one-home "workspace files minus stdlib" primitive
    /// `workspace_verify` uses — and hashed from the salsa `SourceFile`
    /// text, so an unsaved editor overlay is captured AS LOADED, not from
    /// disk (the bytes the session actually executes). Paths are made
    /// root-relative so the manifest is reproducible across machines and
    /// leaks no absolute path into downloaded reports or blessed baselines
    /// (steward ruling 2026-07-23); a file outside `root` (or a capture
    /// with no known root) keeps its canonical URI verbatim — honest and
    /// rare. Sorted by path for a byte-stable block.
    ///
    /// Called ONLY from session-mint sites via
    /// [`capture_session_provenance`](Self::capture_session_provenance) —
    /// never a salsa query recomputed per keystroke, same discipline as
    /// `model_digest`.
    fn workspace_file_manifest(
        &self,
        root: Option<&std::path::Path>,
    ) -> Vec<sysml_store::FileProvenance> {
        use sha2::{Digest, Sha256};
        let host = self.host.lock().unwrap();
        let mut manifest: Vec<sysml_store::FileProvenance> = host
            .files()
            .user_file_ids()
            .filter_map(|fid| {
                let uri = host.files().uri(fid)?;
                let sf = host.files().source_file(fid)?;
                let content_hash = format!("{:x}", Sha256::digest(sf.text(host.db()).as_bytes()));
                Some(sysml_store::FileProvenance {
                    path: provenance_relative_path(uri, root),
                    content_hash,
                })
            })
            .collect();
        manifest.sort_by(|a, b| a.path.cmp(&b.path));
        manifest
    }

    /// Create a named, immutable baseline.
    #[service_command(
        name = "sysml.store.baseline.create",
        category = Storage,
        description = "Create a named, immutable baseline pointing at a commit (default: the latest). Baselines can never be renamed or retargeted; the referenced commit becomes eviction-exempt. When the workspace root is a git work tree, git provenance (HEAD sha, dirty flag, branch) is recorded as corroborating metadata — a dirty tree is recorded honestly, never refused (the content-addressed commit digest is the identity; B6 steward ruling 2026-07-16).",
        returns = "BaselineMeta",
    )]
    pub fn store_baseline_create(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "Baseline name (unique per project)"] name: &str,
        #[doc = "Commit id to baseline (default: latest)"] commit: Option<String>,
    ) -> Result<sysml_store::BaselineMeta, ServiceError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;
        // Captured before taking the store lock — it shells out to git.
        let root = self.resolve_git_root();
        let provenance = root.as_deref().and_then(git_provenance::capture);
        let mut guard = store
            .write()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;
        let commit = commit.map(CommitId::new);
        storage::create_baseline(&mut *guard, project, name, commit.as_ref(), provenance)
    }

    /// List a project's baselines.
    #[service_command(
        name = "sysml.store.baseline.list",
        category = Storage,
        description = "List a project's baselines (most recently created first)",
        returns = "Vec<BaselineMeta>",
    )]
    pub fn store_baseline_list(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
    ) -> Result<Vec<sysml_store::BaselineMeta>, ServiceError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;
        let guard = store
            .read()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;
        storage::list_baselines(&*guard, project)
    }

    /// Snapshot the current elaborated workspace into the store.
    #[service_command(
        name = "sysml.store.save_workspace",
        category = Storage,
        description = "Snapshot the current elaborated workspace graph into the store under a content-addressed commit id (SHA-256 of the diff-compared content). Idempotent: an unchanged workspace returns the existing commit's metadata instead of minting a new one. The graph is read through the same workspace accessor `sysml.workspace.requirement_rows` uses, so element ids in stored snapshots correlate 1:1 with row ids.",
        returns = "SnapshotMeta",
    )]
    pub fn store_save_workspace(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "Commit message (default: 'workspace snapshot')"] message: Option<String>,
    ) -> Result<SnapshotMeta, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;
        let mut guard = store
            .write()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;
        storage::save_workspace_snapshot(
            &mut *guard,
            project,
            &graph,
            message.as_deref().unwrap_or("workspace snapshot"),
        )
    }

    /// Suspect requirement attribution between two stored snapshots (R9).
    #[service_command(
        name = "sysml.workspace.requirement_suspects",
        category = Query,
        description = "Requirement rows suspect against a baseline: diffs two stored snapshots (`from`/`to` accept a baseline name or commit id; omitted `to` = latest commit), attributes every change to its nearest owning requirement, and propagates downstream along Derive edges. Causes distinguish text edits (with before/after bodies), other content changes, added/removed children, identity-not-in-baseline (ADR-009: never name-matched), and upstream suspicion. Each record carries `cleared_by` (workflow event seq) when a non-superseded suspect-clearing attestation covers it — cleared rows are not suspect for display but stay listed.",
        returns = "Vec<SuspectRecordView>",
    )]
    pub fn requirement_suspects(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "Baseline name or commit id to diff from"] from: &str,
        #[doc = "Baseline name or commit id to diff to (default: latest commit)"] to: Option<
            String,
        >,
    ) -> Result<Vec<workflow::SuspectRecordView>, ServiceError> {
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;
        let guard = store
            .read()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;
        workflow::suspects_with_clearings(
            &*guard,
            self.workflow_store.as_ref(),
            project,
            from,
            to.as_deref(),
        )
    }

    /// Attest that a suspect requirement's intent is unchanged.
    #[service_command(
        name = "sysml.workflow.attest_suspect_clearing",
        category = Storage,
        description = "Record a suspect-clearing attestation: `actor` reviewed `element_id`'s changes since `baseline` (name or commit id) and vouches the intent still holds. Mints/resolves the current content commit first (idempotent) and pins it as `attested_commit` — any later content change supersedes the attestation and suspicion re-fires. Fails if the element is not actually suspect against the baseline, or if `actor` is blank (no silent default identity).",
        returns = "WorkflowEvent",
    )]
    pub fn workflow_attest_suspect_clearing(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "The suspect requirement's element id"] element_id: &ElementId,
        #[doc = "Baseline name or commit id the clearing is against"] baseline: &str,
        #[doc = "Why the change does not invalidate intent"] rationale: &str,
        #[doc = "Who attests (explicit identity, required)"] actor: &str,
    ) -> Result<sysml_store::WorkflowEvent, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;
        let mut guard = store
            .write()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;
        workflow::attest_suspect_clearing(
            &mut *guard,
            self.workflow_store.as_ref(),
            project,
            &graph,
            element_id,
            baseline,
            rationale,
            actor,
        )
    }

    /// Record a manual-verification attestation on an element.
    #[service_command(
        name = "sysml.workflow.attest_verification",
        category = Storage,
        description = "Record a MANUAL verification act on an element (B10 layer 3, human leg): `actor` verified `element_id` by `method` (one of the spec's VerificationMethodKind: inspect | analyze | demo | test — validated, closed set). An ATTESTATION in the append-only workflow sidecar — never a computed verdict; it must never render as a verdict chip or enter verdict rollups. Mints/resolves the current content commit and pins it as `attested_commit`; any later content change supersedes the attestation at display time. Requires a non-blank `actor` and `statement`.",
        returns = "WorkflowEvent",
    )]
    pub fn workflow_attest_verification(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "The verified element's id (verification case or requirement)"]
        element_id: &ElementId,
        #[doc = "Verification method: inspect | analyze | demo | test"] method: &str,
        #[doc = "What the engineer actually did/observed"] statement: &str,
        #[doc = "Who attests (explicit identity, required)"] actor: &str,
    ) -> Result<sysml_store::WorkflowEvent, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;
        let mut guard = store
            .write()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;
        workflow::attest_verification(
            &mut *guard,
            self.workflow_store.as_ref(),
            project,
            &graph,
            element_id,
            method,
            statement,
            actor,
        )
    }

    /// Audited re-link of workflow history to a successor identity.
    #[service_command(
        name = "sysml.workflow.relink",
        category = Storage,
        description = "Record a deliberate re-link of workflow history from a dead element id to its successor (ADR-009: identity changes are never auto-matched; a re-link is itself an audited event). The target must exist in the current workspace.",
        returns = "WorkflowEvent",
    )]
    pub fn workflow_relink(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "The dead/prior element id"] from_element: &ElementId,
        #[doc = "The successor element id (must exist)"] to_element: &ElementId,
        #[doc = "Why these identities are the same artifact"] rationale: &str,
        #[doc = "Who re-links (explicit identity, required)"] actor: &str,
    ) -> Result<sysml_store::WorkflowEvent, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        workflow::relink(
            self.workflow_store.as_ref(),
            &graph,
            project,
            from_element,
            to_element,
            rationale,
            actor,
        )
    }

    /// Record a review comment on an element.
    #[service_command(
        name = "sysml.workflow.comment",
        category = Storage,
        description = "Record a review comment on an element in the append-only workflow sidecar. The element must exist in the current workspace (new writes against dead ids are rejected — history on later-churned ids is handled at read time). Requires an explicit non-blank `actor` and `body`.",
        returns = "WorkflowEvent",
    )]
    pub fn workflow_comment(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "The element the comment is about (must exist)"] element_id: &ElementId,
        #[doc = "Comment text"] body: &str,
        #[doc = "Who comments (explicit identity, required)"] actor: &str,
    ) -> Result<sysml_store::WorkflowEvent, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        workflow::comment(
            self.workflow_store.as_ref(),
            &graph,
            project,
            element_id,
            body,
            actor,
        )
    }

    /// Assign an engineer to an element.
    #[service_command(
        name = "sysml.workflow.assign",
        category = Storage,
        description = "Assign an engineer to an element (workflow sidecar). Folded state keeps the latest assignee; the log keeps every assignment. The element must exist in the current workspace; requires an explicit non-blank `actor` and `assignee`.",
        returns = "WorkflowEvent",
    )]
    pub fn workflow_assign(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "The element being assigned (must exist)"] element_id: &ElementId,
        #[doc = "Who is assigned"] assignee: &str,
        #[doc = "Who assigns (explicit identity, required)"] actor: &str,
    ) -> Result<sysml_store::WorkflowEvent, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        workflow::assign(
            self.workflow_store.as_ref(),
            &graph,
            project,
            element_id,
            assignee,
            actor,
        )
    }

    /// Transition an element's approval state.
    #[service_command(
        name = "sysml.workflow.set_approval",
        category = Storage,
        description = "Transition an element's approval state (workflow sidecar; closed vocabulary: draft, in_review, approved, rejected — 'draft' is every element's initial state). The transition's `from` is derived server-side from the folded event log, never client-claimed; a no-op transition (target == current state) is rejected. The element must exist in the current workspace; requires an explicit non-blank `actor`.",
        returns = "WorkflowEvent",
    )]
    pub fn workflow_set_approval(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "The element whose approval state changes (must exist)"] element_id: &ElementId,
        #[doc = "Target state: draft | in_review | approved | rejected"] to: &str,
        #[doc = "Who transitions (explicit identity, required)"] actor: &str,
    ) -> Result<sysml_store::WorkflowEvent, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        workflow::set_approval(
            self.workflow_store.as_ref(),
            &graph,
            project,
            element_id,
            to,
            actor,
        )
    }

    /// Record a sign-off attestation statement.
    #[service_command(
        name = "sysml.workflow.sign_off",
        category = Storage,
        description = "Record a sign-off attestation statement against an element (workflow sidecar; all sign-offs are kept oldest-first in folded state — a sign-off is a statement of record, never overwritten). The element must exist in the current workspace; requires an explicit non-blank `actor` and `statement`.",
        returns = "WorkflowEvent",
    )]
    pub fn workflow_sign_off(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "The element being signed off (must exist)"] element_id: &ElementId,
        #[doc = "The attestation statement"] statement: &str,
        #[doc = "Who signs (explicit identity, required)"] actor: &str,
    ) -> Result<sysml_store::WorkflowEvent, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        workflow::sign_off(
            self.workflow_store.as_ref(),
            &graph,
            project,
            element_id,
            statement,
            actor,
        )
    }

    /// Raw workflow event log (oldest-first).
    #[service_command(
        name = "sysml.workflow.log",
        category = Query,
        description = "The append-only workflow event log for a project, oldest-first; optionally filtered to one element. Events keyed on ids that no longer exist are still returned — history is never deleted or silently re-attached (ADR-009).",
        returns = "Vec<WorkflowEvent>",
    )]
    pub fn workflow_log(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "Filter to one element's history"] element_id: Option<String>,
    ) -> Result<Vec<sysml_store::WorkflowEvent>, ServiceError> {
        let element_id = element_id.map(sysml_id::ElementId::from_string);
        self.workflow_store
            .events(project, element_id.as_ref())
            .map_err(workflow::workflow_err)
    }

    /// Folded workflow state of one element.
    #[service_command(
        name = "sysml.workflow.state",
        category = Query,
        description = "Current workflow state of one element, derived by folding its event log (never authored): latest approval + assignee, sign-offs, suspect-clearing attestations (each flagged `superseded` when the requirement changed again after it), comment count, and `orphaned` (the id no longer exists in the current graph — history belongs to a prior identity).",
        returns = "ElementWorkflowState",
    )]
    pub fn workflow_state(
        &self,
        #[doc = "Project identifier"] project: &ProjectId,
        #[doc = "The element whose state to fold"] element_id: &ElementId,
    ) -> Result<workflow::ElementWorkflowState, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let store = self
            .store
            .as_ref()
            .ok_or_else(|| ServiceError::Store("no store configured".to_owned()))?;
        let guard = store
            .read()
            .map_err(|e| ServiceError::Store(format!("lock poisoned: {e}")))?;
        workflow::element_state(
            &*guard,
            self.workflow_store.as_ref(),
            &graph,
            project,
            element_id,
        )
    }

    /// The workspace root (if constructed via `from_workspace`).
    pub fn workspace_root(&self) -> Option<&Path> {
        self.workspace_root.as_deref()
    }

    // -- Domain state accessors (used by LSP server) --

    /// Access the project registry.
    pub fn project_registry(&self) -> &std::sync::RwLock<project_registry::ProjectRegistry> {
        &self.project_registry
    }

    /// Access the diagram manager (open diagrams, expanded nodes, graph cache).
    pub fn diagram_manager(&self) -> &diagram_manager::DiagramManager {
        &self.diagram_manager
    }

    /// Look up cached external source text by key.
    ///
    /// Service-owned half of LSP-43 (S2.T6). The LSP keeps URI
    /// normalization + the live-editor salsa_doc fast path + the async
    /// disk read; the cache lives here. Pair with
    /// [`Self::cache_external_source`] to populate after a fresh read.
    pub fn cached_external_source(&self, key: &str) -> Option<Arc<String>> {
        self.hover_source_cache
            .read()
            .unwrap()
            .get(key)
            .cloned()
    }

    /// Insert (or replace) a cached external source under `key`.
    pub fn cache_external_source(&self, key: String, source: Arc<String>) {
        self.hover_source_cache.write().unwrap().insert(key, source);
    }

    /// Drop the cached external source for `key`, if any.
    pub fn invalidate_external_source(&self, key: &str) {
        self.hover_source_cache.write().unwrap().remove(key);
    }

    /// Access the active runtime sessions (unified map).
    ///
    /// This accessor exposes the underlying `DashMap` for reads
    /// (`.get`, `.iter`, `.get_mut` of existing entries). **All mutations
    /// that change the population of the map** — inserts, removes, retains —
    /// must go through [`insert_session`], [`remove_session`], or
    /// [`retain_sessions`] so the per-kind counter stays in sync.
    pub fn sessions(&self) -> &DashMap<ElementId, execution::RuntimeSession> {
        &self.sessions
    }

    /// Build a fresh `[AtomicUsize; 3]` with every counter at zero.
    fn new_session_counts() -> [AtomicUsize; execution::SessionKind::VARIANT_COUNT] {
        [AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0)]
    }

    /// Insert or replace a session under `key`, keeping per-kind counters
    /// in sync. Returns the previous entry, if any.
    pub fn insert_session(
        &self,
        key: ElementId,
        session: execution::RuntimeSession,
    ) -> Option<execution::RuntimeSession> {
        let new_idx = session.kind.index();
        let prev = self.sessions.insert(key, session);
        match &prev {
            Some(old) => {
                let old_idx = old.kind.index();
                if old_idx != new_idx {
                    self.session_counts[old_idx].fetch_sub(1, Ordering::Relaxed);
                    self.session_counts[new_idx].fetch_add(1, Ordering::Relaxed);
                }
            }
            None => {
                self.session_counts[new_idx].fetch_add(1, Ordering::Relaxed);
            }
        }
        prev
    }

    /// Remove a session by key and return `(key, session)` if present,
    /// decrementing the matching per-kind counter.
    pub fn remove_session(
        &self,
        key: &ElementId,
    ) -> Option<(ElementId, execution::RuntimeSession)> {
        let removed = self.sessions.remove(key);
        if let Some((_, ref session)) = removed {
            self.session_counts[session.kind.index()].fetch_sub(1, Ordering::Relaxed);
        }
        removed
    }

    /// Retain only entries for which `predicate` returns true, updating
    /// per-kind counters for every entry dropped.
    pub fn retain_sessions<F>(&self, mut predicate: F)
    where
        F: FnMut(&ElementId, &mut execution::RuntimeSession) -> bool,
    {
        self.sessions.retain(|key, session| {
            let keep = predicate(key, session);
            if !keep {
                self.session_counts[session.kind.index()].fetch_sub(1, Ordering::Relaxed);
            }
            keep
        });
    }

    /// Count live sessions of a given kind (O(1); reads the atomic counter).
    pub fn session_count(&self, kind: execution::SessionKind) -> usize {
        self.session_counts[kind.index()].load(Ordering::Relaxed)
    }

    /// Enforce the per-kind cap before creating a new session of `kind`.
    ///
    /// If the bucket is full, lazy-reaps expired sessions of that kind and
    /// re-checks. Returns a bucket-specific error if still at cap.
    ///
    /// # Race
    ///
    /// Two concurrent callers can each see `count < cap` in their
    /// respective `cap_check` calls, then both insert, briefly pushing
    /// the bucket to `cap + N` where N is the number of racing callers.
    /// This is acceptable for the current single-threaded UX — the
    /// sidebar and compare-mode flows are sequential — and fixing it
    /// would add a global lock on session creation for no practical
    /// benefit. If the backend ever gains a multi-threaded caller that
    /// saturates a bucket concurrently, wrap the `cap_check → insert`
    /// pair in a `Mutex<()>` at the session-creation call sites.
    pub fn cap_check(&self, kind: execution::SessionKind) -> Result<(), ServiceError> {
        let cap = execution::quota_for(kind);
        let count = self.session_count(kind);
        if count < cap {
            return Ok(());
        }
        // Try to reclaim space by dropping expired sessions of this kind.
        self.retain_sessions(|_, s| !(s.kind == kind && s.is_expired()));
        let count = self.session_count(kind);
        if count < cap {
            return Ok(());
        }
        tracing::warn!(
            bucket = %kind,
            used = count,
            cap = cap,
            "session bucket full"
        );
        Err(ServiceError::Execution(format!(
            "{} bucket full ({}/{}) — stop a run or call sessions.reap for expired ones",
            kind.as_str(),
            count,
            cap
        )))
    }

    // -- Private helpers --

    fn execute_query_spec(
        &self,
        uri: &str,
        spec: &sysml_query::QuerySpec,
        profile: sysml_query::QueryProfile,
    ) -> Result<sysml_query::QueryResult, ServiceError> {
        let graph = self.query_graph(uri)?;
        let revision = sysml_query::graph_revision(&graph);
        let spec_key = serde_json::to_string(spec)
            .map_err(|e| ServiceError::Internal(format!("query spec serialization failed: {e}")))?;
        let cache_key = format!("{uri}\u{1f}{revision}\u{1f}{profile:?}\u{1f}{spec_key}");
        if let Some(cached) = self.query_cache.get(&cache_key) {
            return Ok(cached.clone().with_cache_status(sysml_query::QueryCacheStatus::Hit));
        }
        let result = sysml_query::execute_query_with_profile(&graph, spec, revision, profile)
            .map_err(|e| ServiceError::InvalidInput(e.to_string()))?
            .with_cache_status(sysml_query::QueryCacheStatus::Miss);
        self.query_cache.insert(cache_key, result.clone());
        Ok(result)
    }

    fn query_all_elements(
        &self,
        uri: &str,
        mut spec: sysml_query::QuerySpec,
    ) -> Result<Vec<Element>, ServiceError> {
        spec.projection = sysml_query::Projection::Elements;
        spec.limit = Some(1000);
        spec.cursor = None;
        let mut out = Vec::new();
        loop {
            let result = self.execute_query_spec(uri, &spec, sysml_query::QueryProfile::Service)?;
            match result.rows {
                sysml_query::QueryRows::Elements(mut elements) => out.append(&mut elements),
                _ => return Err(ServiceError::Internal("query_all_elements expected element rows".to_owned())),
            }
            match result.cursor {
                Some(cursor) => spec.cursor = Some(cursor),
                None => break,
            }
        }
        Ok(out)
    }

    fn query_all_ids(
        &self,
        uri: &str,
        mut spec: sysml_query::QuerySpec,
    ) -> Result<Vec<ElementId>, ServiceError> {
        spec.projection = sysml_query::Projection::Ids;
        spec.limit = Some(1000);
        spec.cursor = None;
        let mut out = Vec::new();
        loop {
            let result = self.execute_query_spec(uri, &spec, sysml_query::QueryProfile::Service)?;
            match result.rows {
                sysml_query::QueryRows::Ids(mut ids) => out.append(&mut ids),
                _ => return Err(ServiceError::Internal("query_all_ids expected id rows".to_owned())),
            }
            match result.cursor {
                Some(cursor) => spec.cursor = Some(cursor),
                None => break,
            }
        }
        Ok(out)
    }

    fn query_all_summaries(
        &self,
        uri: &str,
        mut spec: sysml_query::QuerySpec,
    ) -> Result<Vec<sysml_query::ElementSummary>, ServiceError> {
        spec.projection = sysml_query::Projection::SummaryExpand;
        spec.limit = Some(1000);
        spec.cursor = None;
        let mut out = Vec::new();
        loop {
            let result = self.execute_query_spec(uri, &spec, sysml_query::QueryProfile::Service)?;
            match result.rows {
                sysml_query::QueryRows::Summary(mut summaries) => out.append(&mut summaries),
                _ => return Err(ServiceError::Internal("query_all_summaries expected summary rows".to_owned())),
            }
            match result.cursor {
                Some(cursor) => spec.cursor = Some(cursor),
                None => break,
            }
        }
        Ok(out)
    }

    /// Single graph-selection helper for the unified query primitive.
    fn query_graph(&self, uri: &str) -> Result<Arc<ModelGraph>, ServiceError> {
        self.require_graph(uri)
    }

    /// Require a graph for a given URI, returning a useful error if not
    /// found. For `__workspace__` returns the elaborated whole-workspace
    /// graph; for any other URI returns the per-file parsed graph
    /// (parse-only, no resolve, no elaborate — matches the historical
    /// per-URI DashMap entry shape).
    ///
    /// This is the canonical fail-hard accessor — every transport that
    /// needs a `ModelGraph` for a URI goes through here. The earlier
    /// `graph()` / `snapshot()` `Option` wrappers were deleted (audit
    /// cluster `C-soft-fallback-graph-snapshot`); callers must surface a
    /// typed `ServiceError` rather than collapsing to `None`.
    pub fn require_graph(&self, uri: &str) -> Result<Arc<ModelGraph>, ServiceError> {
        match GraphScope::parse(uri) {
            GraphScope::Workspace => self.workspace_aware_graph(),
            GraphScope::File(file_uri) => {
                // ONE lock acquisition for snapshot + SourceFile (lock-order
                // invariant on `host_analysis` — the 2026-07-17 wedge).
                let (analysis, sf) = self.locked_analysis_with(|host| {
                    host.file_id(&file_uri).and_then(|id| host.source_file(id))
                });
                let sf = sf.ok_or_else(|| {
                    ServiceError::ElementNotFound(format!("no graph for URI: {file_uri}"))
                })?;
                let parsed = analysis.parse_file(sf);
                Ok(Arc::new(parsed.graph().clone()))
            }
        }
    }

    /// File-level **resolved + elaborated** graph for `uri`, salsa-memoized.
    ///
    /// For `__workspace__` this is the cross-file workspace graph. For a
    /// single file it routes through `elaborate_file_best`, which runs
    /// `resolve_file_best` *then* `elaborate` (analysis.rs) — so the graph
    /// is name-resolved, unlike the parse-only [`require_graph`]. Fails
    /// hard when the URI has no loaded source file.
    ///
    /// RSC-6.5 / ledger L13: this is the one home for "I need a properly
    /// elaborated graph for this URI". It replaces the soft fallback in
    /// `export_smodel`, which ad-hoc `elaborate()`'d an UNRESOLVED parse-only
    /// clone — resolution is salsa-owned; no command re-elaborates by hand.
    /// The single-file (no-workspace) CLI export path (`sysml export smodel
    /// file.sysml`) keeps working — it just gets a resolved graph now.
    /// (`views_render` does NOT use this — views are workspace-only and fail
    /// hard without one; see its body.)
    fn elaborated_graph(&self, uri: &str) -> Result<Arc<ModelGraph>, ServiceError> {
        if matches!(GraphScope::parse(uri), GraphScope::Workspace) {
            return self.workspace_aware_graph();
        }
        let (sf, project_id) = {
            let guard = self.host.lock().unwrap();
            let file_id = guard.file_id(uri).ok_or_else(|| {
                ServiceError::ElementNotFound(format!("no graph for URI: {uri}"))
            })?;
            let sf = guard.source_file(file_id).ok_or_else(|| {
                ServiceError::ElementNotFound(format!("no graph for URI: {uri}"))
            })?;
            let project_id = guard.files().project_id(file_id);
            (sf, project_id)
        };
        let analysis = self.host_analysis();
        Ok(Arc::new(
            analysis.elaborate_file_best(sf, project_id).graph().clone(),
        ))
    }

    /// Get the workspace-elaborated graph. Fails hard when no workspace
    /// project file set is loaded — there is no soft per-file fallback.
    ///
    /// Routes through Salsa's `elaborate_workspace[_with_library]` query
    /// so the result is memoized — only re-runs when an input file (or
    /// the library / project file set) changes.
    ///
    /// Q2 of the resolution-tier collapse made this fail-hard
    /// fallback to `require_graph(uri)` masked the missing-workspace
    /// case as a parse-only result; callers couldn't distinguish a
    /// fully-elaborated workspace graph from an unresolved single file.
    ///
    /// Public so LSP runtime entry points (scenario, Monte Carlo) can
    /// consume the same elaborated graph the service-side commands use.
    pub fn workspace_aware_graph(&self) -> Result<Arc<ModelGraph>, ServiceError> {
        // ONE lock acquisition for a coherent (snapshot, PFS) pair; the
        // elaboration query runs lock-free (see `workspace_analysis`).
        let (analysis, pfs) = self.workspace_analysis()?;
        let elaborated = sysml_ide_db::elaborate_workspace_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        Ok(Arc::clone(elaborated.graph()))
    }

    /// The single configured `Snapshot` every execution command must use.
    ///
    /// One home (principle #4) for execution-snapshot wiring: it resolves the
    /// workspace-aware graph, threads the `@DataSource` **source directory** so
    /// the runtime compiler can load `SampledFunction` CSV lookup tables (and
    /// seed their `__sf_*` slots — without it any such model hard-errors RS003),
    /// and threads the salsa-cached physics executor.
    ///
    /// Source-dir resolution is `project_root_dir(uri) ?? workspace_root()`:
    /// a real file URI resolves to its owning project root; the synthetic
    /// `__workspace__` URI has no file-keyed project (`project_root_dir` returns
    /// `None` by construction), so the workspace root is the correct lookup for
    /// that URI class — not a soft fallback.
    ///
    /// Every execution command (`*.start`, `verify_with_simulation{,_trace}`,
    /// `ode_sweep`, `continuous_*`, `batch.create`) routes through here instead
    /// of open-coding `Snapshot::new`, so the source-dir/physics wiring can no
    /// longer be silently dropped at a single site.
    fn execution_snapshot(&self, uri: &str) -> Result<sysml_ide_db::Snapshot, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let mut snap = sysml_ide_db::Snapshot::new(graph);
        let source_dir = {
            let host = self.host.lock().unwrap();
            // A real file URI resolves to its owning project root; the synthetic
            // `__workspace__` URI has no file-keyed project, so fall back to the
            // workspace project's directory root by handle.
            host.project_root_dir(uri).or_else(|| {
                host.project_root_dir_for_handle(sysml_project::ProjectHandle(
                    Self::SERVICE_WORKSPACE_PROJECT_ID,
                ))
            })
        }
        .or_else(|| self.workspace_root().map(|p| p.to_path_buf()));
        if let Some(dir) = source_dir {
            snap = snap.with_source_dir(dir);
        }
        if let Some(exec) = self.workspace_physics_executor() {
            snap = snap.with_cached_physics_executor(exec);
        }
        Ok(snap)
    }

    /// RSC-6.4: the salsa-cached physics executor for the loaded workspace.
    ///
    /// Returns `None` if no workspace is loaded or the model has no PowerBond
    /// physics topology (where `build_workspace_orchestrator` builds inline and
    /// fast-fails anyway). Keyed on the same `(pfs, library)` as
    /// `workspace_aware_graph`, so the executor derives from the identical
    /// elaborated graph the orchestrator is built over — threading it via
    /// `Snapshot::with_cached_physics_executor` is byte-identical to the inline
    /// step-7 build, but built once per graph version instead of per session.
    fn workspace_physics_executor(
        &self,
    ) -> Option<Arc<sysml_runtime::physics::PhysicsExecutor>> {
        let pfs = self.host.lock().unwrap().project_file_set(
            sysml_project::ProjectHandle(Self::SERVICE_WORKSPACE_PROJECT_ID),
        )?;
        let analysis = self.host_analysis();
        sysml_ide_db::workspace_physics_executor_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        )
        .arc()
    }


    /// Get the set of element IDs belonging to a specific file.
    /// Used to scope operations (constraints, simulations) to one file
    /// while evaluating against the full workspace graph.
    ///
    /// Returns `Ok(HashSet::new())` for the synthetic `__workspace__`
    /// URI — that's an intentional empty set meaning "no per-file
    /// filtering, evaluate over the whole workspace graph" (callers
    /// short-circuit on empty filter). Real graph-load failures
    /// propagate as `ServiceError` rather than silently degrading the
    /// file-scoped call into a workspace-wide sweep.
    fn file_element_ids(
        &self,
        uri: &str,
    ) -> Result<std::collections::HashSet<sysml_id::ElementId>, ServiceError> {
        if uri == WORKSPACE_URI {
            // Intentional: empty filter = no scoping. This is the
            // documented sentinel, not a soft fallback.
            return Ok(std::collections::HashSet::new());
        }
        let file_basename = std::path::Path::new(uri)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(uri);
        let ws = self.workspace_aware_graph()?;
        Ok(ws
            .elements
            .values()
            .filter(|e| {
                e.spans
                    .iter()
                    .any(|s| s.file == file_basename || s.file.ends_with(file_basename))
            })
            .map(|e| e.id.clone())
            .collect())
    }

    // -- Evaluation / what-if / aggregation / workspace-verify commands --

    /// Evaluate a single element's value by its ID.
    #[service_command(
        name = "sysml.evaluate",
        category = Execution,
        description = "Evaluate a single element's expression value by its ID",
        returns = "string?",
    )]
    pub fn evaluate_element(
        &self,
        #[doc = "Element ID to evaluate"] element_id: &ElementId,
    ) -> Result<Option<String>, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let element = graph.get_element(element_id).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("element not found: {element_id}"))
        })?;
        Ok(evaluation::evaluate_element(element, &graph))
    }

    /// Evaluate one expression-bearing element and return structured runtime data.
    #[service_command(
        name = "sysml.evaluate.expression",
        category = Execution,
        description = "Evaluate one expression-bearing element with optional overrides, returning value, verdict, context, and diagnostics",
        returns = "EvaluateExpressionResult",
    )]
    pub fn evaluate_expression(
        &self,
        #[doc = "Element ID to evaluate"] element_id: &ElementId,
        #[doc = "Key=value overrides applied to the evaluation context"] overrides: &[(String, String)],
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let element = graph.get_element(element_id).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("element not found: {element_id}"))
        })?;
        let ctx = self.eval_context_with_overrides(overrides)?;
        Ok(evaluation::evaluate_expression_with_context(element, &graph, &ctx))
    }

    /// Evaluate all constraints in a graph and return per-constraint results.
    #[service_command(
        name = "sysml.evaluate.constraints",
        category = Execution,
        description = "Evaluate all constraint elements in a model, returning pass/fail results with details",
        returns = "Vec<EvalConstraintResult>",
    )]
    pub fn evaluate_constraints(
        &self,
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let results = evaluation::evaluate_constraints(&graph);
        let serialized: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let ast = expression_ast::project_one(&graph, &r.element_id)
                    .and_then(|p| p.ast);
                // R1.3: emit the universal Verdict shape alongside the legacy
                // `satisfied` boolean. Downstream workflows (Run, Verify,
                // Monte Carlo, Sweep, Trade Study) can migrate to reading
                // `verdict` directly; older consumers still work via the
                // `satisfied` field.
                let verdict_json = serde_json::to_value(&r.verdict)
                    .unwrap_or(serde_json::Value::Null);
                // B8: enrich the row with name / kind / expr so transports
                // don't have to re-walk the graph to label rows. Mirrors what
                // the LSP's `handle_evaluate_all` used to do post-hoc; that
                // re-walk has been deleted.
                let element = graph.get_element(&r.element_id);
                let name = element
                    .and_then(|e| e.name.clone())
                    .unwrap_or_else(|| "<unnamed>".to_owned());
                let kind = element
                    .map(|e| e.kind.as_str().to_owned())
                    .unwrap_or_else(|| "ConstraintUsage".to_owned());
                let expr = element
                    .and_then(|e| evaluation::get_expression_string_public(e, &graph));
                serde_json::json!({
                    "element_id": r.element_id.to_string(),
                    "satisfied": r.satisfied,
                    "display": r.display,
                    "detail": r.detail,
                    "ast": ast,
                    "verdict": verdict_json,
                    // Eval-result-loss collapse (P2):
                    // `error` is populated on the failure path with the EvalError
                    // display string; on success it stays `null`. Orthogonal to
                    // `verdict.actual`. Audit cluster:
                    // C-soft-fallback-eval-result-loss.
                    "error": r.error,
                    // B8 enrichment: per-row labels so transports don't re-walk.
                    "name": name,
                    "kind": kind,
                    "expr": expr,
                })
            })
            .collect();
        Ok(serde_json::Value::Array(serialized))
    }

    /// Evaluate all verification case elements in a graph.
    #[service_command(
        name = "sysml.evaluate.verification_cases",
        category = Execution,
        description = "Evaluate all verification case elements in a model, returning verdicts per case",
        returns = "Vec<VerificationCaseResult>",
    )]
    pub fn evaluate_verification_cases(
        &self,
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let results = evaluation::evaluate_verification_cases(&graph);
        let serialized: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "element_id": r.element_id.to_string(),
                    "case_id": r.case_id,
                    "case_name": r.case_name,
                    // Model-structure grouping key (History latest-status
                    // bands) — the ownership chain, `Pkg::Sub::Case`.
                    "qualified_name": r.qualified_name,
                    "subject": r.subject,
                    // DECLARED @VerificationMethod kinds ([1..*] — plural by
                    // spec). Replaces the never-populated singular `method`
                    // (the IR field was dead; deleted with it).
                    "methods": r.methods,
                    // B10 layer 2: this read evaluates against the CURRENT
                    // graph (recomputed per revision, never archived) —
                    // constant today, honest labeling per §2.1a(d); the
                    // case-view mode chip reads it off this wire.
                    "evaluation_mode": EvaluationMode::Static.as_str(),
                    "verdict": format!("{:?}", r.verdict),
                    "total_requirements": r.total_requirements,
                    "passed_requirements": r.passed_requirements,
                    "display": r.display,
                    "requirements": r.requirements,
                    "diagnostics": r.diagnostics,
                })
            })
            .collect();
        Ok(serde_json::Value::Array(serialized))
    }

    /// Evaluate all analysis case elements in a graph.
    #[service_command(
        name = "sysml.evaluate.analysis_cases",
        category = Execution,
        description = "Evaluate all analysis case elements in a model, returning output summaries per case",
        returns = "Vec<AnalysisCaseResult>",
    )]
    pub fn evaluate_analysis_cases(
        &self,
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let results = evaluation::evaluate_analysis_cases(&graph);
        let serialized: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "element_id": r.element_id.to_string(),
                    "case_name": r.case_name,
                    "display": r.display,
                    "subject": r.subject,
                    "objective": r.objective,
                    "tool_name": r.tool_name,
                    "tool_uri": r.tool_uri,
                    "parameters": r.parameters,
                    "constraints": r.constraints,
                    "result_expression": r.result_expression,
                    "diagnostics": r.diagnostics,
                })
            })
            .collect();
        Ok(serde_json::Value::Array(serialized))
    }

    /// Evaluate all calculation elements in a graph.
    #[service_command(
        name = "sysml.evaluate.calculations",
        category = Execution,
        description = "Evaluate all calculation elements in a model, returning computed values",
        returns = "Vec<(ElementId, display)>",
    )]
    pub fn evaluate_calculations(
        &self,
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        let results = evaluation::evaluate_calculations(&graph);
        let serialized: Vec<serde_json::Value> = results
            .iter()
            .map(|(id, _span, display)| {
                let ast = expression_ast::project_one(&graph, id).and_then(|p| p.ast);
                // B8: enrich the row with name / kind / expr so transports
                // don't have to re-walk the graph. Mirrors the LSP's deleted
                // `handle_evaluate_all` enrichment loop.
                let element = graph.get_element(id);
                let name = element
                    .and_then(|e| e.name.clone())
                    .unwrap_or_else(|| "<unnamed>".to_owned());
                let kind = element
                    .map(|e| e.kind.as_str().to_owned())
                    .unwrap_or_else(|| "CalculationUsage".to_owned());
                let expr = element
                    .and_then(|e| evaluation::get_expression_string_public(e, &graph));
                serde_json::json!({
                    "element_id": id.to_string(),
                    "display": display,
                    "ast": ast,
                    // B8 enrichment.
                    "name": name,
                    "kind": kind,
                    "expr": expr,
                })
            })
            .collect();
        Ok(serde_json::Value::Array(serialized))
    }

    /// Run an action by name to completion and return a trace.
    #[service_command(
        name = "sysml.action.run",
        category = Execution,
        description = "Compile and run a named action to completion, returning the execution trace",
        returns = "ActionTraceResult { steps, completed, total_steps }",
    )]
    pub fn action_run(
        &self,
        #[doc = "URI of the loaded model containing the action"] uri: &str,
        #[doc = "Name of the action definition to run"] action_name: &str,
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.require_graph(uri)?;
        let trace = evaluation::run_action(action_name, &graph)
            .map_err(ServiceError::Execution)?;
        Ok(serde_json::json!({
            "completed": trace.completed,
            "total_steps": trace.total_steps,
            "steps": trace.steps.iter().map(|s| {
                serde_json::json!({
                    "completed": s.completed,
                    "outputs": s.outputs,
                })
            }).collect::<Vec<_>>(),
        }))
    }

    /// What-if analysis: override a variable and see which constraints flip.
    #[service_command(
        name = "sysml.whatif",
        category = Execution,
        description = "Override a variable value and evaluate ALL constraints on the graph (baseline vs override), returning per-constraint flips plus an overlay payload (values, constraintResults, guardDiagnoses) suitable for the diagram overlay UI. Optional session_key selects an orchestrator session as the base context.",
        returns = "WhatIfResult { variable_name, override_value, baseline, overridden, flipped, values, constraintResults, guardDiagnoses, overriddenVariable, overriddenValue }",
    )]
    pub fn whatif(
        &self,
        #[doc = "Name of the variable to override"] variable_name: &str,
        #[doc = "Override value (as string, parsed via parse_value_string)"] override_value: &str,
        #[doc = "Optional orchestrator session_key (ElementId string). When provided, the active session's context is used as the baseline and the latest snapshot's guardDiagnoses are surfaced."]
        session_key: Option<&str>,
    ) -> Result<serde_json::Value, ServiceError> {
        // F3 (unification wave): workspace-aware discovery so a cross-file
        // constraint owner resolves for the owner-scoped context below.
        let graph = self.workspace_aware_graph()?;
        let override_val = sysml_runtime::compiler::parse_value_string(override_value);
        let precompiled = self.workspace_precompiled_constraints()?;

        // ---- Constraint evaluation (baseline vs single-variable override) ---
        // Session path: one instance-keyed orchestrator context (correct as-is
        // — per-occurrence values are already distinct in the live context).
        // No-session path: owner-scoped per-constraint contexts (fixes the
        // pre-existing whole-graph flat-walk that collided same-named
        // attributes across parts — see the `else` branch).
        let (baseline_results, override_results, values_ctx, guard_diagnoses) =
            if let Some(key) = session_key {
            let elem_key = sysml_id::ElementId::from_string(key);
            let Some(session) = self.sessions().get(&elem_key) else {
                return Err(ServiceError::InvalidInput(format!(
                    "whatif: session_key {key} not found"
                )));
            };
            let base_ctx = session.orchestrator.context.alias_live();
            let guards: Vec<serde_json::Value> = session
                .history()
                .back()
                .map(|snap| {
                    snap.guard_diagnoses
                        .iter()
                        .map(|gd| {
                            serde_json::json!({
                                "guard_expr": gd.guard_expr,
                                "transition_from": gd.transition.0,
                                "transition_to": gd.transition.1,
                                "event": gd.event,
                                "dependencies": gd.dependencies.iter().collect::<Vec<_>>(),
                                "dependency_values": gd
                                    .dependency_values
                                    .iter()
                                    .map(|(k, v)| {
                                        let json_val = match v {
                                            sysml_core::Value::Int(i) => serde_json::json!(i),
                                            sysml_core::Value::Float(f) => serde_json::json!(f),
                                            sysml_core::Value::Bool(b) => serde_json::json!(b),
                                            sysml_core::Value::String(s) => serde_json::json!(s),
                                            _ => serde_json::json!(format!("{:?}", v)),
                                        };
                                        (k.clone(), json_val)
                                    })
                                    .collect::<serde_json::Map<String, serde_json::Value>>(),
                                "satisfied": gd.satisfied,
                                "explanation": gd.explanation,
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let override_ctx = base_ctx.child_with(variable_name, override_val.clone());
            let baseline = precompiled.evaluate_all(&base_ctx);
            let overridden = precompiled.evaluate_all(&override_ctx);
            (baseline, overridden, override_ctx, guards)
        } else {
            // No active session: evaluate each constraint against an
            // OWNER-SCOPED context (the attribute values declared in its
            // owning part), not a flattened whole-graph map. The previous
            // implementation walked every named element's `value` into one
            // flat `EvalContext`; two parts that each declared an attribute of
            // the same name collided (HashMap last-writer-wins), so every
            // constraint saw one arbitrary value and could report a wrong
            // verdict. Owner-scoping resolves each name against its own part.
            // The override is layered only onto scopes that actually declare
            // the variable — `child_with` inserts unconditionally, so a bare
            // clone is used where the name is absent to avoid injecting a
            // phantom binding into an unrelated scope.
            let mut owner_ctx_cache: std::collections::HashMap<
                Option<sysml_core::ElementId>,
                sysml_runtime::constraints::EvalContext,
            > = std::collections::HashMap::new();
            let mut baseline = Vec::with_capacity(precompiled.compiled.len());
            let mut overridden = Vec::with_capacity(precompiled.compiled.len());
            let mut values_ctx = sysml_runtime::constraints::EvalContext::new();
            for tc in &precompiled.compiled {
                let owner = tc.constraint.owner_id.clone();
                let base = owner_ctx_cache
                    .entry(owner.clone())
                    .or_insert_with(|| match &owner {
                        Some(oid) => evaluation::owner_scoped_context(&graph, oid),
                        None => sysml_runtime::constraints::EvalContext::new(),
                    });
                let override_ctx = if base.get(variable_name).is_some() {
                    base.child_with(variable_name, override_val.clone())
                } else {
                    base.alias_live()
                };
                baseline.push(tc.evaluate(base));
                overridden.push(tc.evaluate(&override_ctx));
                // Merge this owner's scalar bindings into the flat overlay map
                // so the diagram can paint per-node values. Same-named
                // attributes across parts still collapse here (the overlay map
                // is keyed by bare name) — an inherent limit of the flat
                // name->value overlay contract, orthogonal to the now
                // per-owner verdict correctness.
                for (k, v) in override_ctx.variables.iter() {
                    if matches!(
                        v,
                        sysml_core::Value::Int(_)
                            | sysml_core::Value::Float(_)
                            | sysml_core::Value::Bool(_)
                            | sysml_core::Value::String(_)
                    ) {
                        values_ctx.set(k.clone(), v.clone());
                    }
                }
            }
            (baseline, overridden, values_ctx, Vec::new())
        };

        // Map runtime EvaluationResult (bool satisfied + inconclusive flag)
        // to the wire Option<bool>: inconclusive => None.
        let to_sat_json = |r: &sysml_runtime::constraints::EvaluationResult| -> serde_json::Value {
            if r.inconclusive {
                serde_json::json!({ "satisfied": serde_json::Value::Null })
            } else {
                serde_json::json!({ "satisfied": r.satisfied })
            }
        };
        let baseline_json: Vec<serde_json::Value> =
            baseline_results.iter().map(to_sat_json).collect();
        let overridden_json: Vec<serde_json::Value> =
            override_results.iter().map(to_sat_json).collect();

        // Zip baseline/override to compute flips. Same constraints in the
        // same order — pair by index.
        let mut flipped = Vec::new();
        for (b, o) in baseline_results.iter().zip(override_results.iter()) {
            let b_sat = (!b.inconclusive).then_some(b.satisfied);
            let o_sat = (!o.inconclusive).then_some(o.satisfied);
            if b_sat != o_sat {
                if let (Some(was), Some(is_now)) = (b_sat, o_sat) {
                    let name = o
                        .constraint
                        .description
                        .clone()
                        .unwrap_or_else(|| o.constraint.expr.clone());
                    flipped.push(serde_json::json!({
                        "name": name,
                        "now_passing": !was && is_now,
                    }));
                }
            }
        }

        // ---- Overlay payload ------------------------------------------------
        let values: serde_json::Map<String, serde_json::Value> = values_ctx
            .variables
            .iter()
            .map(|(k, v)| {
                let json_val = match v {
                    sysml_core::Value::Int(i) => serde_json::json!(i.to_string()),
                    sysml_core::Value::Float(f) => serde_json::json!(f.to_string()),
                    sysml_core::Value::Bool(b) => serde_json::json!(b.to_string()),
                    sysml_core::Value::String(s) => serde_json::json!(s.clone()),
                    _ => serde_json::json!(format!("{:?}", v)),
                };
                (k.clone(), json_val)
            })
            .collect();

        let constraint_results_json: Vec<serde_json::Value> = override_results
            .iter()
            .map(|cr| {
                serde_json::json!({
                    "name": cr.constraint.description.clone().unwrap_or_else(|| cr.constraint.expr.clone()),
                    "satisfied": if cr.inconclusive { serde_json::Value::Null } else { serde_json::json!(cr.satisfied) },
                    "expression": cr.constraint.expr,
                })
            })
            .collect();

        Ok(serde_json::json!({
            "variable_name": variable_name,
            "override_value": format!("{:?}", override_val),
            "baseline": baseline_json,
            "overridden": overridden_json,
            "flipped": flipped,
            "values": values,
            "constraintResults": constraint_results_json,
            "guardDiagnoses": guard_diagnoses,
            "overriddenVariable": variable_name,
            "overriddenValue": override_value,
        }))
    }

    /// Sweep a parameter across a range and evaluate constraints at each step.
    #[service_command(
        name = "sysml.whatif.sweep",
        category = Execution,
        description = "Sweep a parameter across a range and evaluate constraints at each step to find thresholds",
        returns = "SweepResult { variable_name, steps, threshold }",
    )]
    pub fn whatif_sweep(
        &self,
        #[doc = "Element ID of the variable to sweep"] element_id: &ElementId,
        #[doc = "Name of the variable to sweep"] variable_name: &str,
        #[doc = "Start value of sweep range"] start: f64,
        #[doc = "End value of sweep range"] end: f64,
        #[doc = "Number of steps in the sweep"] steps: usize,
    ) -> Result<serde_json::Value, ServiceError> {
        // F3 (unification wave): workspace-aware discovery so a swept element
        // defined in a sibling file resolves by id.
        let graph = self.workspace_aware_graph()?;
        let element = graph.get_element(element_id).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("element not found: {element_id}"))
        })?;
        let result = whatif::sweep_parameter(element, &graph, variable_name, start, end, steps);
        let value_to_json = |v: &sysml_core::Value| -> serde_json::Value {
            match v {
                sysml_core::Value::Float(f) => serde_json::json!(f),
                sysml_core::Value::Int(i) => serde_json::json!(i),
                _ => serde_json::Value::Null,
            }
        };
        Ok(serde_json::json!({
            "variable_name": result.variable_name,
            "threshold": result.threshold.as_ref().map(value_to_json).unwrap_or(serde_json::Value::Null),
            "steps": result.steps.iter().map(|s| serde_json::json!({
                "value": value_to_json(&s.value),
                "constraint_results": s.constraint_results.iter().map(|(name, sat)| serde_json::json!({"name": name, "satisfied": sat})).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        }))
    }

    /// Run a parameter sweep over an ODE+SM simulation, returning per-variant results.
    ///
    /// For each sweep value, creates a fresh Orchestrator (reusing `detect_ode_from_metadata`
    /// infrastructure), overrides the swept parameter, runs to completion, and records
    /// the time-to-completion, final state machine states, and final variable values.
    #[service_command(
        name = "sysml.trade_study.ode_sweep",
        category = Execution,
        description = "Sweep an ODE parameter across a range, running full simulations per value",
        returns = "json",
    )]
    pub fn ode_sweep(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Name of the state machine definition to simulate"] sm_name: &str,
        #[doc = "Name of the ODE parameter to sweep (e.g. 'loadCurrent')"] parameter_name: &str,
        #[doc = "Minimum value of sweep range"] min_value: f64,
        #[doc = "Maximum value of sweep range"] max_value: f64,
        #[doc = "Number of sweep steps (e.g. 20)"] steps: usize,
        #[doc = "Time step in milliseconds (default 10.0)"] dt_ms: Option<f64>,
        #[doc = "Maximum simulation time in milliseconds (default 30000.0)"] max_time_ms: Option<f64>,
        #[doc = "Baseline parameter overrides applied to ALL variants (e.g. [['leakageCurrent','0']] to isolate the Breaker path)"] baseline_overrides: Option<&[(String, String)]>,
    ) -> Result<serde_json::Value, ServiceError> {
        if steps == 0 {
            return Err(ServiceError::InvalidInput("steps must be > 0".to_owned()));
        }

        let snap = self.execution_snapshot(uri)?;

        let dt = dt_ms.unwrap_or(10.0);
        let max_t = max_time_ms.unwrap_or(30_000.0);

        let step_size = if steps > 1 { (max_value - min_value) / (steps as f64 - 1.0) } else { 0.0 };
        let sweep_values: Vec<f64> = (0..steps).map(|i| min_value + i as f64 * step_size).collect();

        // RSC-6.2: a sweep varies only the parameter value per variant; the
        // SM compile + full-graph SSR ODE detection + derivative parse are
        // graph-invariant. Prepare them ONCE here and assemble each variant
        // from the result, instead of re-walking the graph per step.
        let prepared = snap
            .prepare_single_ode(sm_name)
            .map_err(|e| ServiceError::Execution(e.message))?;

        let mut variants = Vec::with_capacity(steps);
        for &param_value in &sweep_values {
            let mut variant_overrides: Vec<(String, String)> = baseline_overrides
                .unwrap_or(&[])
                .to_vec();
            variant_overrides.push((parameter_name.to_owned(), param_value.to_string()));

            let (_orch, sim) = snap.run_simulation_prepared(
                &prepared, &variant_overrides, Some(dt), Some(max_t),
            ).map_err(|e| ServiceError::Execution(e.message))?;

            let (time_ms, completed, final_values) = match &sim {
                Some(s) => (s.time_ms, s.completed, snapshot_values_json(s)),
                None => (0.0, false, serde_json::Map::new()),
            };

            variants.push(serde_json::json!({
                "value": param_value,
                "time_to_completion_ms": time_ms,
                "completed": completed,
                "final_values": serde_json::Value::Object(final_values),
            }));
        }

        Ok(serde_json::json!({
            "parameter_name": parameter_name,
            "min_value": min_value,
            "max_value": max_value,
            "steps": steps,
            "dt_ms": dt,
            "max_time_ms": max_t,
            "variants": variants,
        }))
    }

    /// Run an ODE simulation with given overrides, then verify the model's
    /// verification cases against the simulated result — verdict via the ONE
    /// engine ([`VerificationRunner`](sysml_runtime::cases::VerificationRunner)).
    ///
    /// Generic engine: knows nothing about IEC or any specific standard. The
    /// service runs the sim, overlays its SETTLED (final-tick) values as the
    /// run's produced feature values, and lets each declared verification case
    /// read them through the model's own bindings. The verdict is a
    /// [`VerdictKind`](sysml_runtime::cases::VerdictKind) — NOT a flat
    /// constraint→bool (B4: there is one verdict engine, fed by many value
    /// suppliers; this is the ODE supplier, `scenario.run` is the event-script
    /// supplier). No `sim_*` magic keys are injected into the verification
    /// context (B1): a case binds to model features, not tool-private strings.
    /// A model with no verification case yields `Inconclusive` (§7.24.1 — no
    /// basis for a determination), never a vacuous pass.
    #[service_command(
        name = "sysml.verify_with_simulation",
        category = Execution,
        description = "Run ODE simulation with parameter overrides, then verify the model's verification cases against the result (verdict via VerificationRunner)",
        returns = "json",
    )]
    pub fn verify_with_simulation(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Name of the state machine definition to simulate"] sm_name: &str,
        #[doc = "Parameter overrides as (key, value) pairs"] overrides: &[(String, String)],
        #[doc = "Time step in milliseconds (default 1.0)"] dt_ms: Option<f64>,
        #[doc = "Maximum simulation time in milliseconds (default 60000.0)"] max_time_ms: Option<f64>,
    ) -> Result<serde_json::Value, ServiceError> {
        use sysml_runtime::cases::{compile_verification_case, VerdictKind, VerificationRunner};

        let graph = self.workspace_aware_graph()?;
        let snap = self.execution_snapshot(uri)?;
        let (_orch, final_snapshot) = snap.run_simulation(sm_name, overrides, dt_ms, max_time_ms)
            .map_err(|e| ServiceError::Execution(e.message))?;

        // The run's produced feature values = its settled (final-tick) state.
        // These are overlaid into the verification context; a case reads them
        // through its own model bindings. NO `sim_*` magic keys (B1).
        let mut eval_ctx = sysml_runtime::expressions::EvalContext::new();
        if let Some(s) = final_snapshot.as_ref() {
            for (k, v) in s.variables.iter() {
                eval_ctx.set(k.clone(), v.clone());
            }
        }

        // Route every declared verification case through the ONE verdict engine
        // (B4). Aggregate per the spec worst-wins ordering.
        let case_ids = crate::workspace_verify::discover_verification_cases(&graph);
        let mut requirement_results: Vec<serde_json::Value> = Vec::new();
        let mut cases_json: Vec<serde_json::Value> = Vec::new();
        let mut overall: Option<VerdictKind> = None;
        for case_id in &case_ids {
            let Some(case_name) = graph.elements.get(case_id).and_then(|e| e.name.clone()) else {
                continue;
            };
            match compile_verification_case(&case_name, &graph) {
                Ok(case_ir) => {
                    let result = VerificationRunner::new().verify(&case_ir, &eval_ctx);
                    overall = Some(overall.map_or(result.verdict, |o| o.aggregate(result.verdict)));
                    for (idx, r) in result.requirement_results.iter().enumerate() {
                        let requirement_element_id = case_ir
                            .requirements
                            .get(idx)
                            .and_then(|req| req.source_element_id.clone());
                        let mut entry = serde_json::json!({
                            "case": case_name,
                            "requirement_id": r.requirement_id,
                            "verdict": format!("{}", r.verdict),
                            "message": r.message,
                        });
                        if let Some(id) = requirement_element_id {
                            entry["requirement_element_id"] = serde_json::Value::String(id);
                        }
                        requirement_results.push(entry);
                    }
                    cases_json.push(serde_json::json!({
                        "case": case_name,
                        "case_id": case_id.to_string(),
                        "verdict": format!("{}", result.verdict),
                    }));
                }
                Err(diags) => {
                    let msgs: Vec<String> = diags.iter().map(|d| d.message.clone()).collect();
                    tracing::warn!(case = %case_name, errors = ?msgs, "verification case compilation failed");
                    overall = Some(overall.map_or(VerdictKind::Error, |o| o.aggregate(VerdictKind::Error)));
                    cases_json.push(serde_json::json!({
                        "case": case_name,
                        "verdict": format!("{}", VerdictKind::Error),
                        "message": msgs.join("; "),
                    }));
                }
            }
        }

        // No declared verification case → no basis for a determination
        // (§7.24.1): honest Inconclusive, never a vacuous pass.
        let verdict = overall.unwrap_or(VerdictKind::Inconclusive);

        let sim_json = final_snapshot.as_ref()
            .map(|s| snapshot_sim_json(s, sm_name))
            .unwrap_or_else(|| serde_json::json!({"time_ms": 0, "completed": false}));

        Ok(serde_json::json!({
            // Trajectory-mode verdict: produced from a live ODE run's settled
            // state, not static defaults (§2.1a(d) "rendered ALWAYS"; study §3.4).
            "evaluation_mode": EvaluationMode::Trajectory.as_str(),
            "simulation": sim_json,
            "verdict": format!("{}", verdict),
            "cases": cases_json,
            "requirement_results": requirement_results,
        }))
    }

    /// Run a single simulation scenario and return the full time-series trace.
    ///
    /// Like verify_with_simulation but returns sampled snapshots for charting.
    /// Used when the user clicks into a scenario to see the detailed trace.
    #[service_command(
        name = "sysml.verify_with_simulation_trace",
        category = Execution,
        description = "Run ODE simulation and return full time-series trace for charting (sampled to max_points)",
        returns = "json",
    )]
    pub fn verify_with_simulation_trace(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
        #[doc = "Name of the state machine definition to simulate"] sm_name: &str,
        #[doc = "Parameter overrides as (key, value) pairs"] overrides: &[(String, String)],
        #[doc = "Time step in milliseconds (default 1.0)"] dt_ms: Option<f64>,
        #[doc = "Maximum simulation time in milliseconds (default 60000.0)"] max_time_ms: Option<f64>,
        #[doc = "Maximum trace points to return (default 500)"] max_points: Option<usize>,
    ) -> Result<serde_json::Value, ServiceError> {
        let max_pts = max_points.unwrap_or(500);

        let snap = self.execution_snapshot(uri)?;
        let (orch, _snap) = snap.run_simulation(sm_name, overrides, dt_ms, max_time_ms)
            .map_err(|e| ServiceError::Execution(e.message))?;
        let trace = orch.trace();

        // Sample trace down to max_points.
        let step = if trace.len() > max_pts { trace.len() / max_pts } else { 1 };
        let mut time_series: std::collections::HashMap<String, Vec<serde_json::Value>> = std::collections::HashMap::new();
        let mut state_transitions: Vec<serde_json::Value> = Vec::new();
        let mut prev_state = String::new();

        for (i, snap) in trace.iter().enumerate() {
            if i % step == 0 || i == trace.len() - 1 {
                // Collect numeric variables at this time point.
                for (k, v) in snap.variables.iter() {
                    if sysml_runtime::expressions::is_internal_var(k) { continue; }
                    if let sysml_core::Value::Float(f) = v {
                        time_series.entry(k.clone()).or_default().push(serde_json::json!({
                            "t": snap.time_ms,
                            "v": f,
                        }));
                    }
                }
            }

            // Track state transitions (always, not sampled).
            if let Some(ss) = snap.subsystem_states.get(sm_name) {
                if ss.current_state != prev_state {
                    state_transitions.push(serde_json::json!({
                        "time_ms": snap.time_ms,
                        "from": if prev_state.is_empty() { "initial" } else { &prev_state },
                        "to": ss.current_state,
                        "tick": snap.tick,
                    }));
                    prev_state = ss.current_state.clone();
                }
            }
        }

        let series_json: serde_json::Map<String, serde_json::Value> = time_series
            .into_iter()
            .map(|(k, v)| (k, serde_json::Value::Array(v)))
            .collect();

        Ok(serde_json::json!({
            // Same trajectory evaluation as `verify_with_simulation`, surfaced as
            // a time-series trace (§2.1a(d) "rendered ALWAYS"; study §3.4).
            "evaluation_mode": EvaluationMode::Trajectory.as_str(),
            "total_ticks": trace.len(),
            "sampled_points": max_pts.min(trace.len()),
            "time_series": serde_json::Value::Object(series_json),
            "state_transitions": state_transitions,
            "final_time_ms": trace.last().map(|s| s.time_ms).unwrap_or(0.0),
            "final_state": trace.last().and_then(|s| s.subsystem_states.get(sm_name)).map(|s| s.current_state.as_str()).unwrap_or("unknown"),
        }))
    }

    /// Compute aggregate statuses for all relevant owners in a graph.
    #[service_command(
        name = "sysml.aggregate",
        category = Execution,
        description = "Compute aggregate constraint/verification/requirement status for all owners in a model",
        returns = "Vec<AggregateStatus>",
    )]
    pub fn aggregate(
        &self,
        #[doc = "URI of the loaded model"] uri: &str,
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.require_graph(uri)?;
        let statuses = aggregation::aggregate_all_statuses(&graph);
        let serialized: Vec<serde_json::Value> = statuses
            .iter()
            .map(|s| {
                serde_json::json!({
                    "owner_name": s.owner_name,
                    "constraints_passed": s.constraints_passed,
                    "constraints_failed": s.constraints_failed,
                    "verifications_passed": s.verifications_passed,
                    "verifications_failed": s.verifications_failed,
                    "requirements_satisfied": s.requirements_satisfied,
                    "requirements_unsatisfied": s.requirements_unsatisfied,
                    "display": aggregation::format_aggregate_lens(s),
                })
            })
            .collect();
        Ok(serde_json::Value::Array(serialized))
    }

    /// Return tree + stats for every loaded user URI in one round-trip.
    ///
    /// Used by the simulation app to hydrate its workspace store after
    /// `sysml.load_workspace`, replacing N×(tree GET + stats POST) round
    /// trips with a single batched call. Excludes synthetic graphs
    /// (`__workspace__`, `__stdlib__`).
    #[service_command(
        name = "sysml.workspace.info",
        category = Query,
        description = "Return tree + stats for every loaded user URI in one call (excludes __workspace__/__stdlib__).",
        returns = "Vec<WorkspaceUriInfo { uri, tree, stats }>",
    )]
    pub fn workspace_info(
        &self,
        #[doc = "Optional explicit list of URIs to query. Omit to return info for every loaded user URI."]
        uris: Option<&[String]>,
    ) -> Result<Vec<types::WorkspaceUriInfo>, ServiceError> {
        let target_uris: Vec<String> = match uris {
            Some(list) => list
                .iter()
                .filter(|u| u.as_str() != WORKSPACE_URI && u.as_str() != "__stdlib__")
                .cloned()
                .collect(),
            None => {
                let host = self.host.lock().unwrap();
                host.files()
                    .user_file_ids()
                    .filter_map(|fid| host.files().uri(fid).map(ToString::to_string))
                    .filter(|u| u != WORKSPACE_URI && u != "__stdlib__")
                    .collect()
            }
        };

        let mut results = Vec::with_capacity(target_uris.len());
        for uri in target_uris {
            let Ok(graph) = self.require_graph(&uri) else {
                continue;
            };
            let tree = query::model_tree(&graph, None);
            let stats = query::stats(&graph);
            results.push(types::WorkspaceUriInfo { uri, tree, stats });
        }
        Ok(results)
    }

    /// Workspace-level lifecycle summary (Bucket A transport-bypass closeout).
    ///
    /// Aggregates the per-root project-discovery report, the host's
    /// loaded counts (user projects / projects-incl-stdlib / tracked
    /// files), and the transport's telemetry counters into a single
    /// record. The LSP `sysml.workspace.info` / `sysml.project.info`
    /// handlers delegate here so every transport sees the same shape.
    ///
    /// `workspace_roots` and `telemetry_counters` are transport-side
    /// state: the LSP forwards its currently-known roots + counter
    /// snapshot; CLI / MCP callers pass `&[]` / `&[]` and get a summary
    /// that omits discovery and counters but still reports loaded host
    /// counts.
    #[service_command(
        name = "sysml.workspace.info_summary",
        category = Query,
        description = "Workspace-level summary: per-root discovery + loaded host counts + transport-supplied telemetry counters.",
        returns = "WorkspaceInfoSummary",
    )]
    pub fn workspace_info_summary(
        &self,
        #[doc = "Absolute workspace-root paths to report on."] workspace_roots: &[String],
        #[doc = "Telemetry counters from the calling transport (key, value)."]
        telemetry_counters: &[(String, u64)],
    ) -> Result<types::WorkspaceInfoSummary, ServiceError> {
        use std::path::Path;

        let mut discovery_entries = Vec::with_capacity(workspace_roots.len());
        for root in workspace_roots {
            let path = Path::new(root);
            let entry = match project_discovery::discover_lsp_workspace_silent(path, true) {
                Ok(discovery) => {
                    let project_names: Vec<String> = discovery
                        .projects
                        .iter()
                        .map(|p| p.info.name.clone())
                        .collect();
                    let project_roots: Vec<String> = discovery
                        .projects
                        .iter()
                        .filter_map(|p| match &p.root {
                            sysml_project::ProjectRoot::Directory(dir) => {
                                Some(dir.display().to_string())
                            }
                            _ => None,
                        })
                        .collect();
                    types::WorkspaceDiscoveryEntry::Discovered {
                        root: root.clone(),
                        mode: discovery.discovery_mode.to_string(),
                        description: discovery.discovery_description,
                        include_stdlib: discovery.include_stdlib,
                        project_count: discovery.projects.len(),
                        project_names,
                        project_roots,
                    }
                }
                Err(err) => types::WorkspaceDiscoveryEntry::Failed {
                    root: root.clone(),
                    error: err.to_string(),
                },
            };
            discovery_entries.push(entry);
        }

        let (user_projects, total_projects_including_stdlib, tracked_files) = {
            let host = self.host.lock().unwrap();
            (host.project_count(), host.salsa_project_count(), host.file_count())
        };

        let counters: std::collections::BTreeMap<String, u64> = telemetry_counters
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .collect();

        Ok(types::WorkspaceInfoSummary {
            workspace_roots: workspace_roots.to_vec(),
            discovery: discovery_entries,
            loaded: types::WorkspaceInfoLoaded {
                user_projects,
                total_projects_including_stdlib,
                tracked_files,
            },
            telemetry_counters: counters,
        })
    }

    /// Report workspace-level model-content capability flags (S4.T3).
    ///
    /// Backend-owned successor to the simulation-app's hand-written tree
    /// walk in `editors/simulation-app/src/store/workspace.ts:178-321`.
    /// Walks the salsa-cached elaborated workspace graph and returns a
    /// single struct of feature flags + name lists the FE uses to gate
    /// UI panels: state-machine selector, action panel, requirements
    /// view, trade-study view, ODE editor, port-flow panel.
    ///
    /// Workspace-scoped: takes no parameters. Operates on the loaded
    /// user-project file set (excludes stdlib). Returns a non-capable
    /// report (all flags `false`, empty name lists) when no workspace
    /// is loaded — matches the FE's "empty workspace ⇒ empty
    /// capabilities" default.
    #[service_command(
        name = "sysml.workspace.capabilities",
        category = Query,
        description = "Workspace-level model-content feature flags + name lists for the simulation app's UI gating.",
        returns = "WorkspaceCapabilitiesResult",
    )]
    pub fn workspace_capabilities(
        &self,
    ) -> Result<types::WorkspaceCapabilitiesResult, ServiceError> {
        let pfs_opt = self
            .host
            .lock()
            .unwrap()
            .project_file_set(sysml_project::ProjectHandle(Self::SERVICE_WORKSPACE_PROJECT_ID));
        let Some(pfs) = pfs_opt else {
            // No user workspace loaded → empty (non-capable) report.
            return Ok(types::WorkspaceCapabilitiesResult::default());
        };
        let analysis = self.host_analysis();
        let cached = ws_caps::workspace_capabilities_best(
            analysis.db(),
            pfs,
            analysis.library_graph(),
        );
        let d = cached.data();
        Ok(types::WorkspaceCapabilitiesResult {
            has_state_machines: d.has_state_machines,
            has_action_flows: d.has_action_flows,
            has_ode_dynamics: d.has_ode_dynamics,
            has_port_flows: d.has_port_flows,
            has_multiple_subsystems: d.has_multiple_subsystems,
            has_constraints: d.has_constraints,
            has_requirements: d.has_requirements,
            has_trade_studies: d.has_trade_studies,
            state_machine_names: d.state_machine_names.clone(),
            action_flow_names: d.action_flow_names.clone(),
            trade_study_names: d.trade_study_names.clone(),
        })
    }

    /// List `.sysml` / `.kerml` files under a workspace directory (recursive).
    ///
    /// Returns a tree pruned to directories that contain SysML/KerML files.
    /// Skips dotfiles, `node_modules/`, `target/`, `dist/`. Replaces the
    /// REST-only `/workspace/files` handler so MCP / CLI consumers can use
    /// the same listing.
    #[service_command(
        name = "sysml.workspace.files",
        category = Query,
        description = "Recursively list .sysml/.kerml files under a workspace directory; tree pruned to directories that contain such files.",
        returns = "WorkspaceFilesResult { root, entries }",
    )]
    pub fn workspace_files(
        &self,
        #[doc = "Absolute path to the workspace root to scan"] root: &str,
        #[doc = "Maximum recursion depth (default: 5)"] max_depth: Option<u32>,
    ) -> Result<fs::WorkspaceFilesResult, ServiceError> {
        fs::list_workspace_files(std::path::Path::new(root), max_depth)
    }

    /// Run workspace-wide verification across all loaded documents.
    #[service_command(
        name = "sysml.workspace.verify",
        category = Execution,
        description = "Run cross-file workspace verification, merging all loaded graphs and evaluating all verification cases",
        returns = "WorkspaceVerifyResult { total_cases, passed, failed, elapsed, model_digest }",
    )]
    pub fn workspace_verify(
        &self,
        #[doc = "Timeout in seconds for the verification run (default: 30)"] timeout_secs: Option<u64>,
    ) -> Result<serde_json::Value, ServiceError> {
        let (uris, library_data) = {
            let host = self.host.lock().unwrap();
            let library_data: Option<sysml_ide_db::LibraryData> = host
                .library_graph()
                .map(|lg| lg.data(host.db()).clone());
            let uris: Vec<String> = host
                .files()
                .user_file_ids()
                .filter_map(|fid| host.files().uri(fid).map(ToString::to_string))
                .collect();
            (uris, library_data)
        };
        let mut doc_graphs: Vec<(String, ModelGraph)> = Vec::with_capacity(uris.len());
        for uri in uris {
            if let Ok(graph) = self.require_graph(&uri) {
                doc_graphs.push((uri, (*graph).clone()));
            }
        }
        let timeout = Duration::from_secs(timeout_secs.unwrap_or(30));
        let library_ref = library_data.as_ref().map(|d| d.graph());
        let result =
            workspace_verify::run_workspace_verification(&doc_graphs, library_ref, timeout);
        // Run-level provenance (steward-ruled home for the frame chip; the
        // evaluate.verification_cases array stays bare). The digest MUST be
        // the salsa workspace-graph identity — the SAME space B6 session
        // provenance, baselines, and record_external's staleness compare use
        // ("digest equality against a baseline commit id is real
        // equivalence") — NOT the F7 hand-merged graph this command verifies
        // over. Live-caught: the merged-graph digest never matched
        // record_external's current_digest, so a fresh external ingest read
        // as stale. The merged-vs-workspace graph split itself is the open
        // B5 billet.
        let model_digest = self.workspace_aware_graph()?.content_digest();
        Ok(serde_json::json!({
            "total_cases": result.total_cases,
            "passed": result.passed,
            "failed": result.failed,
            "elapsed_ms": result.elapsed.as_millis() as u64,
            "per_file": result.per_file.iter().collect::<Vec<_>>(),
            "model_digest": model_digest,
        }))
    }

    // -- System capabilities (R4) --

    /// Report feature flags the service supports.
    ///
    /// Read-only and side-effect-free. Frontends query this before
    /// enabling optional UI paths (e.g. fork-at-tick) so a stale
    /// backend is detected at runtime instead of at error time.
    #[service_command(
        name = "sysml.system.capabilities",
        category = Query,
        description = "Report service feature-flag capabilities (fork-at-tick, snapshot retention, etc.)",
        returns = "ServiceCapabilities",
    )]
    pub fn system_capabilities(&self) -> Result<ServiceCapabilities, ServiceError> {
        Ok(ServiceCapabilities {
            has_fork_at_tick: true,
            snapshot_retention_ticks: execution::DEFAULT_SNAPSHOT_RETENTION_TICKS as u64,
        })
    }

    // -- Salsa query telemetry (S2.T11 / Bucket F) --

    /// Return salsa query execution statistics from the shared host.
    ///
    /// `executions` counts how many tracked queries actually ran;
    /// `validations` counts cache validations (re-checks of unchanged
    /// inputs). `hit_ratio` is `validations / (executions + validations)`
    /// and approximates the salsa cache hit rate.
    #[service_command(
        name = "sysml.salsa.stats",
        category = Query,
        description = "Salsa query execution statistics (executions, validations, hit ratio)",
        returns = "SalsaStats { executions, validations, hit_ratio }",
    )]
    pub fn salsa_stats(&self) -> Result<types::SalsaStats, ServiceError> {
        let host = self.host.lock().unwrap();
        let stats = host.query_stats();
        Ok(types::SalsaStats {
            executions: stats.executions,
            validations: stats.validations,
            hit_ratio: stats.hit_ratio(),
        })
    }

    /// Reset salsa query execution statistics on the shared host to zero.
    #[service_command(
        name = "sysml.salsa.stats.reset",
        category = Query,
        description = "Reset salsa query execution statistics to zero",
        returns = "SalsaStatsResetResult { status }",
    )]
    pub fn salsa_stats_reset(&self) -> Result<types::SalsaStatsResetResult, ServiceError> {
        let host = self.host.lock().unwrap();
        host.reset_query_stats();
        Ok(types::SalsaStatsResetResult {
            status: "reset".to_owned(),
        })
    }

    // -- Workspace lifecycle (S2.T11 / Bucket F — LSP-04, LSP-07, LSP-70) --

    /// Rediscover workspace state across `roots` and rebuild the shared
    /// host: per-root project discovery → deterministic ID assignment
    /// (`10 + idx`) → host reset → load each project → enable stdlib →
    /// rebuild service's project registry snapshot → refresh workspace
    /// `ProjectFileSet`.
    ///
    /// Does NOT preserve in-memory unsaved-edit buffers — the LSP shell
    /// owns the open-document concept and is responsible for capturing
    /// buffers before this call and restoring them after via
    /// `host.set_file_content_in_project`.
    ///
    /// Does NOT walk the filesystem to index loose `.sysml`/`.kerml`
    /// files; that's the LSP indexer's job (UX progress notifications).
    /// Cross-transport callers (CLI/MCP/REST) get the structural reset
    /// + project registry sync, which is sufficient for read-side
    /// queries against discovered projects.
    #[service_command(
        name = "sysml.workspace.refresh",
        category = FileManagement,
        description = "Rediscover projects across workspace roots, reset the shared host, re-register projects, and re-enable stdlib. Returns the discovered project list + stdlib status. Does NOT preserve open-document buffers (LSP shell handles that).",
        returns = "WorkspaceRefreshResult { projects, stdlib_loaded, roots_count }",
    )]
    pub fn workspace_refresh(
        &self,
        #[doc = "Absolute workspace-root paths to walk. Each root contributes its discovered projects to the shared host."]
        roots: &[String],
        #[doc = "Force-enable stdlib (Some(true)) / force-disable (Some(false)) / honour per-root discovery (None — default true if any root requested it)."]
        enable_stdlib: Option<bool>,
    ) -> Result<types::WorkspaceRefreshResult, ServiceError> {
        use std::path::Path;

        let mut discovered_projects: Vec<sysml_project::Project> = Vec::new();
        let mut include_stdlib = false;
        for root in roots {
            let path = Path::new(root);
            match project_discovery::discover_lsp_workspace(path, true) {
                Ok(mut discovery) => {
                    include_stdlib = include_stdlib || discovery.include_stdlib;
                    discovered_projects.append(&mut discovery.projects);
                }
                Err(error) => {
                    tracing::warn!(
                        root = %path.display(),
                        error = %error,
                        "workspace refresh failed for root"
                    );
                }
            }
        }

        // Deterministic project IDs across refresh cycles — same shape
        // as the prior LSP-side `rediscover_workspace_state`.
        discovered_projects.sort_by_key(|p| {
            let root = match &p.root {
                sysml_project::ProjectRoot::Directory(dir) => dir
                    .canonicalize()
                    .unwrap_or_else(|_| dir.clone())
                    .display()
                    .to_string(),
                _ => format!("in-memory:{}", p.info.name),
            };
            (root, p.info.name.clone())
        });
        for (idx, project) in discovered_projects.iter_mut().enumerate() {
            project.id = sysml_project::ProjectHandle(10 + idx as u32);
        }

        let final_include_stdlib = enable_stdlib.unwrap_or(include_stdlib);

        // Reset the analysis host and load the refreshed project set.
        let stdlib_loaded = {
            let mut host = self.host.lock().unwrap();
            *host = sysml_ide_db::AnalysisHost::new();
            for project in &discovered_projects {
                host.load_project(project.clone());
            }
            if final_include_stdlib {
                match host.enable_stdlib() {
                    Ok(loaded) => loaded,
                    Err(error) => {
                        tracing::warn!(error = %error, "failed to enable stdlib during refresh");
                        false
                    }
                }
            } else {
                false
            }
        };

        // Replace project registry snapshot.
        {
            let mut registry = self.project_registry.write().unwrap();
            *registry = project_registry::ProjectRegistry::new();
            for project in &discovered_projects {
                registry.register(project.clone());
            }
        }

        // Refresh the workspace ProjectFileSet so the salsa-tracked
        // workspace queries see the new file set.
        self.ensure_workspace_pfs();

        let projects: Vec<types::ProjectRefreshSummary> = discovered_projects
            .iter()
            .map(|p| types::ProjectRefreshSummary {
                id: p.id.0,
                name: p.info.name.clone(),
                root: match &p.root {
                    sysml_project::ProjectRoot::Directory(dir) => {
                        Some(dir.display().to_string())
                    }
                    _ => None,
                },
            })
            .collect();

        Ok(types::WorkspaceRefreshResult {
            projects,
            stdlib_loaded,
            roots_count: roots.len(),
        })
    }

    // -- Dependency status (S2.T11 / Bucket F — LSP-63) --

    /// Walk every workspace root, find its manifest, hydrate its
    /// dependency tree, and emit a JSON report describing each
    /// dependency's resolution outcome plus per-root hydration
    /// summaries. The shape is identical to the LSP-only handler
    /// it supersedes (`handle_dependency_status`).
    ///
    /// `roots` is the list of absolute workspace-root paths to walk.
    /// In the LSP transport, this comes from `workspace_index.workspace_roots`;
    /// CLI / MCP / REST callers pass the list directly.
    #[service_command(
        name = "sysml.dependency.status",
        category = Query,
        description = "Walk workspace roots, hydrate manifest dependencies, and report per-root resolution outcomes + summary counts.",
        returns = "Value (dependency status JSON)",
    )]
    pub fn dependency_status(
        &self,
        #[doc = "Absolute workspace-root paths to walk. Each root's `sysml.toml` is hydrated independently; missing manifests surface as `{status: no_manifest}` entries."]
        roots: &[String],
    ) -> Result<serde_json::Value, ServiceError> {
        use std::path::Path;
        use sysml_manifest::Dependency;

        let mut root_entries = Vec::new();
        let mut total_dependencies = 0usize;
        let mut hydrated_dependencies = 0usize;
        let mut hydrated_path_dependencies = 0usize;
        let mut hydrated_packages = 0usize;
        let mut failed_dependencies = 0usize;
        let mut unsupported_dependencies = 0usize;
        let mut invalid_dependencies = 0usize;

        for root in roots {
            let root_path = Path::new(root);
            let entry = match sysml_manifest::find_manifest(root_path) {
                Ok(Some((manifest_path, manifest))) => {
                    let manifest_dir = manifest_path
                        .parent()
                        .map(Path::to_path_buf)
                        .unwrap_or_else(|| root_path.to_path_buf());
                    let report = project_discovery::hydrate_manifest_dependencies(
                        &manifest,
                        &manifest_dir,
                        false,
                    );
                    let outcomes_by_name: std::collections::BTreeMap<
                        &str,
                        &project_discovery::DependencyResolutionOutcome,
                    > = report
                        .outcomes
                        .iter()
                        .map(|outcome| (outcome.dependency_name.as_str(), outcome))
                        .collect();

                    let package_json = |pkg: &sysml_resolve::ResolvedPackage| {
                        let (source_kind, source_detail) = match &pkg.source {
                            sysml_resolve::PackageSource::Path(path) => {
                                ("path", serde_json::json!({ "path": path }))
                            }
                            sysml_resolve::PackageSource::Git { url, commit } => {
                                ("git", serde_json::json!({ "url": url, "commit": commit }))
                            }
                            sysml_resolve::PackageSource::Kpar { url } => {
                                ("kpar", serde_json::json!({ "url": url }))
                            }
                            sysml_resolve::PackageSource::Registry {
                                backend,
                                package,
                                requested,
                                version,
                            } => (
                                "registry",
                                serde_json::json!({
                                    "backend": backend,
                                    "package": package,
                                    "requested_requirement": requested,
                                    "version": version,
                                    "resolved_version": version
                                }),
                            ),
                            sysml_resolve::PackageSource::Stdlib => {
                                ("stdlib", serde_json::json!({}))
                            }
                        };
                        serde_json::json!({
                            "name": pkg.name.clone(),
                            "version": pkg.version.clone(),
                            "source": source_kind,
                            "source_detail": source_detail,
                            "lock_source": pkg.source.to_lock_source(),
                            "source_dir": pkg.source_dir.display().to_string(),
                        })
                    };

                    let hydrated_all_json: Vec<serde_json::Value> =
                        report.hydrated_packages.iter().map(package_json).collect();
                    let hydrated_path_json: Vec<serde_json::Value> = report
                        .hydrated_packages
                        .iter()
                        .filter_map(|pkg| match &pkg.source {
                            sysml_resolve::PackageSource::Path(_) => Some(serde_json::json!({
                                "name": pkg.name.clone(),
                                "version": pkg.version.clone(),
                                "source_dir": pkg.source_dir.display().to_string(),
                            })),
                            _ => None,
                        })
                        .collect();

                    hydrated_packages += report.hydrated_packages.len();
                    hydrated_path_dependencies += hydrated_path_json.len();

                    let mut deps_json = Vec::new();
                    let mut root_failed = Vec::new();
                    let mut root_hydrated = 0usize;
                    let mut root_failed_count = 0usize;
                    for (name, dep) in &manifest.dependencies {
                        total_dependencies += 1;
                        let source = project_discovery::dependency_source_kind(dep);
                        let declared_detail = match dep {
                            Dependency::Registry(version) => {
                                serde_json::json!({
                                    "version": version,
                                    "requested_requirement": version,
                                })
                            }
                            Dependency::Detailed(d) => serde_json::json!({
                                "path": d.path.clone(),
                                "git": d.git.clone(),
                                "registry": d.registry.clone(),
                                "tag": d.tag.clone(),
                                "branch": d.branch.clone(),
                                "rev": d.rev.clone(),
                                "kpar": d.kpar.clone(),
                                "version": d.version.clone(),
                                "requested_requirement": d.version.clone(),
                            }),
                        };

                        let outcome = outcomes_by_name.get(name.as_str()).copied();
                        let mut status = "ready";
                        let mut resolution = serde_json::json!({
                            "status": "hydrated",
                            "hydrated_package_count": 0,
                            "hydrated_packages": [],
                        });

                        if let Some(outcome) = outcome {
                            if let Some(failure) = &outcome.failure {
                                root_failed_count += 1;
                                failed_dependencies += 1;
                                if failure.reason == "unsupported_source" {
                                    unsupported_dependencies += 1;
                                }
                                if failure.source_kind == "invalid" {
                                    invalid_dependencies += 1;
                                }
                                status = match (failure.source_kind, failure.reason) {
                                    ("invalid", _) => "invalid",
                                    (_, "unsupported_source") => "unsupported",
                                    ("path", "missing_dependency") => "missing",
                                    _ => "error",
                                };
                                resolution = serde_json::json!({
                                    "status": "failed",
                                    "reason": failure.reason,
                                    "message": failure.message,
                                    "action": failure.action,
                                });
                                root_failed.push(serde_json::json!({
                                    "name": name,
                                    "source": source,
                                    "reason": failure.reason,
                                    "message": failure.message,
                                    "action": failure.action,
                                }));
                            } else {
                                root_hydrated += 1;
                                hydrated_dependencies += 1;
                                let resolved_packages: Vec<serde_json::Value> =
                                    outcome.hydrated_packages.iter().map(package_json).collect();
                                resolution = serde_json::json!({
                                    "status": "hydrated",
                                    "hydrated_package_count": outcome.hydrated_packages.len(),
                                    "hydrated_packages": resolved_packages,
                                    "requested_requirement": dep.as_registry_requirement(),
                                    "resolved_version": outcome.hydrated_packages.iter().find_map(|pkg| match &pkg.source {
                                        sysml_resolve::PackageSource::Registry { version, .. } => Some(version.clone()),
                                        _ => None,
                                    }),
                                });
                            }
                        } else {
                            root_failed_count += 1;
                            failed_dependencies += 1;
                            status = "error";
                            resolution = serde_json::json!({
                                "status": "failed",
                                "reason": "internal_error",
                                "message": "dependency outcome missing from resolver report",
                                "action": "Re-run dependency status and open an issue if this persists.",
                            });
                        }

                        deps_json.push(serde_json::json!({
                            "name": name,
                            "source": source,
                            "status": status,
                            "detail": {
                                "declared": declared_detail,
                                "resolution": resolution,
                            },
                        }));
                    }

                    serde_json::json!({
                        "root": root,
                        "manifest": manifest_path.display().to_string(),
                        "project": manifest.project.name,
                        "dependency_count": manifest.dependencies.len(),
                        "dependencies": deps_json,
                        "hydrated_path_dependencies": hydrated_path_json,
                        "hydrated_dependencies": hydrated_all_json,
                        "failed_dependencies": root_failed,
                        "hydration_summary": {
                            "declared_dependencies": manifest.dependencies.len(),
                            "hydrated_dependencies": root_hydrated,
                            "failed_dependencies": root_failed_count,
                            "hydrated_packages": report.hydrated_packages.len(),
                            "failed_dependency_entries": report.failures.len(),
                        },
                    })
                }
                Ok(None) => serde_json::json!({
                    "root": root,
                    "status": "no_manifest",
                }),
                Err(error) => serde_json::json!({
                    "root": root,
                    "status": "error",
                    "error": error.to_string(),
                }),
            };
            root_entries.push(entry);
        }

        Ok(serde_json::json!({
            "roots": root_entries,
            "summary": {
                "total_dependencies": total_dependencies,
                "hydrated_path_dependencies": hydrated_path_dependencies,
                "hydrated_dependencies": hydrated_dependencies,
                "failed_dependencies": failed_dependencies,
                "unsupported_dependencies": unsupported_dependencies,
                "invalid_dependencies": invalid_dependencies,
                "hydrated_packages": hydrated_packages,
                "roots": roots.len(),
            }
        }))
    }

    // -- Library cache (S2.T11 / Bucket F — LSP-46) --

    /// Snapshot of the library cache file (size, element count, version).
    ///
    /// Reads the on-disk cache produced by the LSP startup pipeline. Returns
    /// `{status: "no_library"}` when no library is configured, or a stats
    /// object when the cache is present (or `exists: false` when missing).
    #[service_command(
        name = "sysml.cache.status",
        category = Query,
        description = "Library cache file snapshot (size_bytes, element_count, crate_version, exists).",
        returns = "Value (cache stats JSON)",
    )]
    pub fn cache_status(&self) -> Result<serde_json::Value, ServiceError> {
        let Some(config) = library_cache::find_library_config() else {
            return Ok(serde_json::json!({"status": "no_library"}));
        };
        let cache = library_cache::LibraryCache::new(config);
        Ok(library_cache::library_cache_stats_json(&cache))
    }

    /// Delete the library cache file from disk.
    ///
    /// Returns `{status: "cleared"}` on success, `{status: "no_library"}`
    /// when no library is configured, or `{error}` on filesystem failure.
    #[service_command(
        name = "sysml.cache.clear",
        category = Query,
        description = "Delete the library cache file from disk.",
        returns = "Value ({status: cleared|no_library} | {error})",
    )]
    pub fn cache_clear(&self) -> Result<serde_json::Value, ServiceError> {
        let Some(config) = library_cache::find_library_config() else {
            return Ok(serde_json::json!({"status": "no_library"}));
        };
        let cache = library_cache::LibraryCache::new(config);
        match cache.clear() {
            Ok(()) => Ok(serde_json::json!({"status": "cleared"})),
            Err(e) => Ok(serde_json::json!({"error": e.to_string()})),
        }
    }

    /// Compute the rebuild-payload for a library cache rebuild.
    ///
    /// Captures the cache snapshot before/after a clear, plus the host's
    /// current library state (whether `host.library_graph()` is set), and
    /// runs the actual cache clear. Does NOT spawn the library reload —
    /// transports that need the reload (the LSP) are responsible for
    /// kicking it off after this call (the LSP needs `tower-lsp` work-done
    /// progress notifications).
    ///
    /// Returns the JSON payload the LSP previously assembled inline, with
    /// equivalent fields. The `library_after_request` always reads
    /// `{"state": "Unloaded"}` because callers reset library state right
    /// after invoking this command.
    #[service_command(
        name = "sysml.cache.rebuild",
        category = Query,
        description = "Compute a library cache rebuild payload: clears the cache, returns before/after snapshots and library state. Reload spawn stays on the transport.",
        returns = "Value (rebuild status JSON)",
    )]
    pub fn cache_rebuild(&self) -> Result<serde_json::Value, ServiceError> {
        let configured_library_path = library_cache::find_library_config()
            .map(|config| config.library_path.display().to_string());

        let library_before_state = {
            let host = self.host.lock().unwrap();
            if host.library_graph().is_some() {
                "Loaded"
            } else {
                "Unloaded"
            }
        };

        let (cache_before, cache_after_clear, clear_status, clear_error) =
            if let Some(config) = library_cache::find_library_config() {
                let cache = library_cache::LibraryCache::new(config);
                let before = library_cache::library_cache_stats_json(&cache);
                let (status, err) = match cache.clear() {
                    Ok(()) => ("cleared", None),
                    Err(e) => ("clear_failed", Some(e.to_string())),
                };
                let after = library_cache::library_cache_stats_json(&cache);
                (before, after, status.to_owned(), err)
            } else {
                (
                    serde_json::json!({"status": "no_library"}),
                    serde_json::json!({"status": "no_library"}),
                    "no_library".to_owned(),
                    None,
                )
            };

        Ok(serde_json::json!({
            "status": "rebuilding",
            "clear_status": clear_status,
            "clear_error": clear_error,
            "configured_library_path": configured_library_path,
            "cache_before": cache_before,
            "cache_after_clear": cache_after_clear,
            "library_before": {
                "state": library_before_state,
            },
            "library_after_request": {
                "state": "Unloaded",
            },
        }))
    }

    // ------------------------------------------------------------------
    // Batch sessions (R5.0 — parent-of-many coordination layer)
    // ------------------------------------------------------------------

    /// Create a [`batch::BatchSession`] parenting N independent runtime
    /// sessions.
    ///
    /// Each entry in `children_params` is applied as an override map to a
    /// fresh child runtime session. The set of children is fixed at
    /// creation time — the parent is a passive metadata container once
    /// spawned, progression happens by driving the child sessions via
    /// the ordinary `sysml.sessions.*` commands.
    ///
    /// `children_params` is a JSON-encoded `Vec<Object>` where each
    /// object is the override map for one child (numeric values are
    /// accepted as JSON numbers or numeric strings; other values are
    /// passed through for downstream interpretation). The wire shape is
    /// a string rather than a JSON array so the service-command macro's
    /// current type set can cover it without growing a
    /// `Vec<serde_json::Value>` wire mapping.
    ///
    /// When `subsystem_name` is provided, each child is spawned as a
    /// state-machine simulation of that subsystem; otherwise a
    /// workspace orchestrator child is spawned. All children share the
    /// same origin URI so archive queries can later group them.
    ///
    /// Wire JSON shape:
    ///
    /// ```json
    /// {
    ///   "kind": "sweep" | "monte_carlo" | "trade_study",
    ///   "uri": "file:///...",
    ///   "subsystem_name": "OptionalSMName",
    ///   "children_params": "[{\"mass\": 1.0}, {\"mass\": 2.0}]",
    ///   "label": "optional free-form"
    /// }
    /// ```
    #[service_command(
        name = "sysml.batch.create",
        category = Execution,
        description = "Create a BatchSession with N child runtime sessions (sweep / monte_carlo / trade_study)",
        returns = "{ batch_id: string, child_session_ids: Vec<string> }",
        stateful = true,
    )]
    pub fn batch_create(
        &self,
        #[doc = "Batch workflow kind: `sweep`, `monte_carlo`, or `trade_study`"] kind: &str,
        #[doc = "URI of the loaded model the children simulate"] uri: &str,
        #[doc = "Optional state machine name; when set, children are simulate.start sessions of that SM"]
        subsystem_name: Option<&str>,
        #[doc = "JSON-encoded array of per-child override maps (e.g. '[{\"mass\": 1.0}, {\"mass\": 2.0}]')"]
        children_params: &str,
        #[doc = "Optional free-form display label surfaced on the workflow tab"]
        label: Option<&str>,
        #[doc = "JSON-encoded array of variable names to measure on each child (e.g. '[\"temperature\"]')"]
        outcomes: Option<&str>,
        #[doc = "Simulation time step in milliseconds for every child (default 1.0)"]
        dt_ms: Option<f64>,
        #[doc = "Model-time budget in milliseconds for every child (default 60000). Size this to the study's horizon or children stop early."]
        max_time_ms: Option<f64>,
    ) -> Result<types::BatchCreateResult, ServiceError> {
        let kind_enum = batch::BatchKind::from_str(kind).ok_or_else(|| {
            ServiceError::InvalidInput(format!(
                "unknown batch kind '{kind}' (expected one of: sweep, monte_carlo, trade_study)"
            ))
        })?;

        let parsed: Vec<std::collections::BTreeMap<String, serde_json::Value>> =
            serde_json::from_str(children_params).map_err(|e| {
                ServiceError::InvalidInput(format!(
                    "children_params must be a JSON array of objects: {e}"
                ))
            })?;

        // Outcomes are the variables the study wants measured on every child.
        // Encoded as a JSON array string for the same reason `children_params`
        // is: the service-command macro's wire types do not cover `Vec<String>`.
        let requested_outcomes: Vec<String> = match outcomes {
            None => Vec::new(),
            Some(raw) if raw.trim().is_empty() => Vec::new(),
            Some(raw) => serde_json::from_str(raw).map_err(|e| {
                ServiceError::InvalidInput(format!(
                    "outcomes must be a JSON array of variable names: {e}"
                ))
            })?,
        };

        if parsed.len() > batch::MAX_CHILDREN_PER_BATCH {
            return Err(ServiceError::InvalidInput(format!(
                "batch has {} children but MAX_CHILDREN_PER_BATCH = {}; paginate the sweep",
                parsed.len(),
                batch::MAX_CHILDREN_PER_BATCH
            )));
        }

        // Honour the batch cap first so we never partially spawn children
        // we cannot register. `MAX_BATCHES` is a soft cap — this mirrors
        // the session-bucket policy and keeps rejection deterministic.
        if self.batches.len() >= batch::MAX_BATCHES {
            return Err(ServiceError::Execution(format!(
                "batch registry full ({}/{}); stop a batch before starting another",
                self.batches.len(),
                batch::MAX_BATCHES
            )));
        }

        // Every child compiles against the same workspace-aware graph (resolved
        // inside `execution_snapshot` per child) so overrides applied per-child
        // are the only visible difference between them.

        // Compile + spawn children in parallel. Child compilation
        // (ModelCompiler → StateMachineIR / workspace orchestrator) is
        // CPU-heavy and independent per-child; rayon maps N compiles
        // across the available cores. A failure in any child triggers
        // a sequential rollback of every child that was inserted, so
        // the service never holds orphaned sessions unreachable from any
        // batch. The per-child `spawn_batch_child` call remains the
        // authoritative creator — it owns cap-check, override
        // application, and session insertion.
        use rayon::prelude::*;
        let enumerated: Vec<(usize, std::collections::BTreeMap<String, serde_json::Value>)> =
            parsed.into_iter().enumerate().collect();
        let spawn_results: Vec<Result<batch::ChildDescriptor, ServiceError>> = enumerated
            .into_par_iter()
            .map(|(index, params)| {
                let overrides: Vec<(String, String)> = params
                    .iter()
                    .map(|(k, v)| (k.clone(), batch::param_value_to_override_string(v)))
                    .collect();
                let session_id = self.spawn_batch_child(
                    uri,
                    subsystem_name,
                    &overrides,
                    dt_ms,
                    max_time_ms,
                )?;
                Ok(batch::ChildDescriptor {
                    session_id,
                    index,
                    params,
                    status: batch::ChildStatus::Pending,
                    verdicts: Vec::new(),
                    outcomes: BTreeMap::new(),
                })
            })
            .collect();

        // Partition results: a single failure triggers rollback of every
        // child that successfully inserted. Descriptors stay in the
        // original generation order because rayon's `collect::<Vec<_>>()`
        // preserves input ordering.
        let mut descriptors: Vec<batch::ChildDescriptor> =
            Vec::with_capacity(spawn_results.len());
        let mut first_err: Option<ServiceError> = None;
        for result in spawn_results {
            match result {
                Ok(descriptor) => descriptors.push(descriptor),
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
            }
        }
        if let Some(e) = first_err {
            // Rollback: stop every child already in the session map.
            // `sessions_stop` is best-effort — a cleanup failure is
            // logged, not propagated, because the primary failure is
            // already the outgoing error.
            for descriptor in &descriptors {
                if let Err(cleanup) = self.sessions_stop(&descriptor.session_id) {
                    tracing::warn!(
                        session_id = %descriptor.session_id,
                        error = %cleanup,
                        "failed to clean up child after batch.create rollback",
                    );
                }
            }
            return Err(e);
        }
        let child_session_ids: Vec<String> = descriptors
            .iter()
            .map(|d| d.session_id.clone())
            .collect();

        let batch_session = batch::BatchSession::new(
            kind_enum,
            label.map(|s| s.to_owned()),
            descriptors,
            requested_outcomes,
        );
        let batch_id = batch_session.id.clone();
        self.batches
            .insert(batch_id.clone(), Arc::new(RwLock::new(batch_session)));

        tracing::info!(
            batch_id = %batch_id,
            kind = %kind,
            child_count = child_session_ids.len(),
            "batch created",
        );

        Ok(types::BatchCreateResult {
            batch_id,
            child_session_ids,
        })
    }

    /// Spawn one child runtime session for a batch.
    ///
    /// When `subsystem_name` is `Some`, compiles that state machine and
    /// builds a simulation session (the same path as
    /// `sysml.simulate.start`). Otherwise builds a workspace orchestrator
    /// (the same path as `sysml.orchestrate.workspace.start`). Overrides
    /// are applied in-place to the fresh orchestrator context *before*
    /// insertion so the caller never observes a window where the child
    /// is live without its per-run parameters.
    fn spawn_batch_child(
        &self,
        uri: &str,
        subsystem_name: Option<&str>,
        overrides: &[(String, String)],
        dt_ms: Option<f64>,
        max_time_ms: Option<f64>,
    ) -> Result<String, ServiceError> {
        let snap = self.execution_snapshot(uri)?;
        let (session, kind) = match subsystem_name {
            Some(sm_name) => {
                self.cap_check(execution::SessionKind::Simulation)?;
                // F1 (unification wave, ledger L44): build the single-SM child
                // via `build_sm_orchestrator` so it mints/binds slots (RS003/4/5
                // gate) — the bare `Orchestrator::new` + `add_state_machine`
                // shape here dropped transition-effect assignment writebacks,
                // the exact defect L44 fixed on `simulate_start`. Same builder,
                // one home.
                let orchestrator = snap
                    .build_sm_orchestrator(sm_name, dt_ms, max_time_ms)
                    .map_err(|e| ServiceError::Execution(e.message))?;
                let mut s = execution::RuntimeSession::new(
                    orchestrator,
                    uri.to_owned(),
                    execution::SessionKind::Simulation,
                    Some(sm_name.to_owned()),
                );
                sysml_runtime::compiler::apply_overrides(&mut s.orchestrator.context, overrides);
                // See the note on the workspace arm below: an archived child
                // must say which point of the study it was.
                s.create_overrides = overrides.to_vec();
                (s, execution::SessionKind::Simulation)
            }
            None => {
                self.cap_check(execution::SessionKind::Orchestrator)?;
                // Salsa-cached seed + constraint precompile: the batch
                // passes the workspace graph from `workspace_aware_graph(uri)`
                // (see `create_batch`, ~line 5803), so both cached query
                // paths are correct here.
                let base_ctx = self.eval_context_with_overrides(&[])?;
                let precompiled = self.workspace_precompiled_constraints()?;
                let port_flow = self.workspace_port_flow_resources()?;
                let gated = self.workspace_gated_expressions()?;
                let ref_cache = self.workspace_ref_resolve_cache()?;
                let orchestrator = snap
                    .build_workspace_orchestrator(
                        base_ctx,
                        Some(precompiled),
                        Some(port_flow),
                        Some(gated),
                        Some(ref_cache),
                        &[],
                        dt_ms,
                        max_time_ms,
                    )
                    .map_err(|e| ServiceError::Execution(e.message))?;
                let mut s = execution::RuntimeSession::new(
                    orchestrator,
                    uri.to_owned(),
                    execution::SessionKind::Orchestrator,
                    None,
                );
                sysml_runtime::compiler::apply_overrides(&mut s.orchestrator.context, overrides);
                // Record WHICH point of the study this child is. Without it
                // an archived sweep child is anonymous: the batch registry
                // knows its params, but the registry cannot be enumerated and
                // does not survive a restart, so the archive is the only
                // durable record — and it was storing a run with no statement
                // of what was varied to produce it.
                s.create_overrides = overrides.to_vec();
                (s, execution::SessionKind::Orchestrator)
            }
        };
        let session_id = execution::new_session_id();
        tracing::debug!(
            session_id = %session_id,
            kind = %kind,
            override_count = overrides.len(),
            "batch child spawned",
        );
        let session_id_str = session_id.to_string();
        self.insert_session(session_id, session);
        Ok(session_id_str)
    }

    /// Return the full [`batch::BatchSession`] snapshot.
    ///
    /// Wire JSON shape: `{ batch: BatchSession }` where `BatchSession` is
    /// the structure defined in [`crate::batch`].
    #[service_command(
        name = "sysml.batch.status",
        category = Execution,
        description = "Fetch the current BatchSession snapshot (kind, children, rollup status)",
        returns = "{ batch: BatchSession }",
        stateful = true,
    )]
    pub fn batch_status(
        &self,
        #[doc = "Opaque batch id returned by sysml.batch.create"] batch_id: &str,
    ) -> Result<types::BatchStatusResult, ServiceError> {
        let entry = self.batches.get(batch_id).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no batch: {batch_id}"))
        })?;
        let guard = entry
            .value()
            .read()
            .map_err(|_| ServiceError::Internal("batch lock poisoned".into()))?;
        Ok(types::BatchStatusResult {
            batch: guard.clone(),
        })
    }

    /// Return every child descriptor for a batch.
    ///
    /// When `include_verdicts` is false (default), each descriptor's
    /// `verdicts` vector is cleared to keep payloads small — the
    /// frontend rehydrates via `sysml.sessions.archive.get` if needed.
    #[service_command(
        name = "sysml.batch.results",
        category = Execution,
        description = "Return per-child descriptors for a batch, optionally including each child's verdicts",
        returns = "{ children: Vec<ChildDescriptor> }",
        stateful = true,
    )]
    pub fn batch_results(
        &self,
        #[doc = "Opaque batch id returned by sysml.batch.create"] batch_id: &str,
        #[doc = "Include each child's archived verdicts in the response"] include_verdicts: bool,
    ) -> Result<types::BatchResultsResult, ServiceError> {
        let entry = self.batches.get(batch_id).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no batch: {batch_id}"))
        })?;
        let guard = entry
            .value()
            .read()
            .map_err(|_| ServiceError::Internal("batch lock poisoned".into()))?;
        let mut children = guard.children.clone();
        if !include_verdicts {
            for c in &mut children {
                c.verdicts.clear();
            }
        }
        Ok(types::BatchResultsResult { children })
    }

    /// Apply a [`batch::BatchFilter`] and return the matching subset.
    ///
    /// Filters are additive — every clause must pass. Verdicts are
    /// always included in the slice response since slicing is typically
    /// used for drill-down, not bulk transport.
    ///
    /// Wire JSON shape for `filter`:
    ///
    /// ```json
    /// {
    ///   "only_status": "pending" | "running" | "complete" | "failed",
    ///   "only_verdict": "pass" | "fail" | "inconclusive" | "error",
    ///   "param_predicate": {
    ///       "param": "mass",
    ///       "op": "lt" | "le" | "gt" | "ge" | "eq" | "ne",
    ///       "value": 2.0
    ///   }
    /// }
    /// ```
    #[service_command(
        name = "sysml.batch.slice",
        category = Execution,
        description = "Filter a batch's children by status, verdict, or parameter predicate",
        returns = "{ children: Vec<ChildDescriptor> }",
        stateful = true,
    )]
    pub fn batch_slice(
        &self,
        #[doc = "Opaque batch id returned by sysml.batch.create"] batch_id: &str,
        #[doc = "Additive filter clauses; see the BatchFilter JSON shape"] filter: batch::BatchFilter,
    ) -> Result<types::BatchSliceResult, ServiceError> {
        let entry = self.batches.get(batch_id).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no batch: {batch_id}"))
        })?;
        let guard = entry
            .value()
            .read()
            .map_err(|_| ServiceError::Internal("batch lock poisoned".into()))?;
        Ok(types::BatchSliceResult {
            children: guard.filter_children(&filter),
        })
    }

    /// Access the live batch-session map.
    ///
    /// Not exposed as a service command; intended for test code and
    /// internal tooling that needs to peek at the parent state directly.
    pub fn batches(&self) -> &DashMap<String, Arc<RwLock<batch::BatchSession>>> {
        &self.batches
    }

    // -- Sensitivity analysis (R7.4) --

    /// Post-process a completed `kind: "sensitivity"` batch into
    /// per-parameter Morris μ/σ or Sobol S_i/S_Ti indices.
    ///
    /// Input payload shape (`params` is a JSON string so the command
    /// macro can pass it through without a dedicated request struct):
    ///
    /// ```json
    /// {
    ///   "batch_id": "<uuid>",
    ///   "method":  "morris" | "sobol",
    ///   "parameters_of_interest": "[{\"name\":\"m\",\"min\":0.5,\"max\":2.0}, ...]",
    ///   "output_metric":          "trip_time"     // verdict or child-param key
    /// }
    /// ```
    ///
    /// `parameters_of_interest` MUST list the ranges in the same order
    /// the batch's children_params were generated with — the ordering
    /// is what decodes Morris "which parameter moved" and Sobol "column
    /// index" semantics.
    ///
    /// `output_metric` picks the scalar output per child. Resolution
    /// order:
    ///
    /// 1. If any verdict on the child carries `evidence.<metric>` as a
    ///    number, use that.
    /// 2. Else if the child has `params.<metric>` as a number, use that
    ///    (common for calc-def-derived KPIs threaded back through
    ///    overrides).
    /// 3. Else `NaN`, which is treated as "no sample" and contributes
    ///    no elementary effect.
    ///
    /// For Morris the batch must have `r * (d + 1)` children in
    /// trajectory-major order; for Sobol it must have `N * (d + 2)`
    /// children in `[A; B; C_0; ...; C_{d-1}]` block order. The
    /// sensitivity module enforces divisibility but does not validate
    /// the deeper Morris "consecutive-points-differ-in-one-parameter"
    /// invariant — callers who built their batch through
    /// [`sensitivity::morris_trajectories`] / [`sensitivity::sobol_concat`]
    /// are safe by construction.
    #[service_command(
        name = "sysml.sensitivity.analyze",
        category = Execution,
        description = "Compute per-parameter Morris (μ, σ) or Sobol (S_i, S_Ti) indices from a completed sensitivity batch",
        returns = "{ method, parameters: Vec<SensitivityResult> }",
        stateful = true,
    )]
    pub fn sensitivity_analyze(
        &self,
        #[doc = "Opaque batch id returned by sysml.batch.create with kind=sensitivity"]
        batch_id: &str,
        #[doc = "Sensitivity method: \"morris\" or \"sobol\""] method: &str,
        #[doc = "JSON-encoded array of ParamRange objects (one per parameter, same order as batch generation)"]
        parameters_of_interest: &str,
        #[doc = "Metric name used to score each child (verdict evidence key or params key)"]
        output_metric: &str,
        #[doc = "Morris level count p (default 4). Ignored for Sobol."]
        morris_levels: Option<usize>,
    ) -> Result<types::SensitivityAnalyzeResult, ServiceError> {
        let method_enum =
            sensitivity::SensitivityMethod::from_str(method).ok_or_else(|| {
                ServiceError::InvalidInput(format!(
                    "unknown sensitivity method '{method}' (expected morris | sobol)"
                ))
            })?;

        let params: Vec<sensitivity::ParamRange> =
            serde_json::from_str(parameters_of_interest).map_err(|e| {
                ServiceError::InvalidInput(format!(
                    "parameters_of_interest must be a JSON array of ParamRange: {e}"
                ))
            })?;
        if params.is_empty() {
            return Err(ServiceError::InvalidInput(
                "parameters_of_interest must contain at least one parameter".into(),
            ));
        }
        let d = params.len();

        let entry = self.batches.get(batch_id).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no batch: {batch_id}"))
        })?;
        let guard = entry
            .value()
            .read()
            .map_err(|_| ServiceError::Internal("batch lock poisoned".into()))?;

        // Build rows in the same order the batch's children were
        // generated. We trust the batch ordering is trajectory-major
        // (Morris) or A/B/C block-major (Sobol); see docstring.
        let children = &guard.children;
        let rows: Vec<Vec<f64>> = children
            .iter()
            .map(|c| {
                params
                    .iter()
                    .map(|pr| {
                        c.params
                            .get(&pr.name)
                            .and_then(batch::param_value_as_f64)
                            .unwrap_or(f64::NAN)
                    })
                    .collect()
            })
            .collect();

        let y: Vec<f64> = children
            .iter()
            .map(|c| extract_child_metric(c, output_metric))
            .collect();

        let results = match method_enum {
            sensitivity::SensitivityMethod::Morris => {
                let p = morris_levels.unwrap_or(4);
                let per_traj = d + 1;
                if rows.len() % per_traj != 0 {
                    return Err(ServiceError::InvalidInput(format!(
                        "morris requires child count % (d+1) == 0; got {} rows for d={}",
                        rows.len(),
                        d,
                    )));
                }
                sensitivity::compute_morris_indices(&params, &rows, &y, p)
            }
            sensitivity::SensitivityMethod::Sobol => {
                // Expect rows = [A(N); B(N); C_0(N); ...; C_{d-1}(N)]
                // → total = N · (d + 2).
                if rows.len() % (d + 2) != 0 {
                    return Err(ServiceError::InvalidInput(format!(
                        "sobol requires child count % (d+2) == 0; got {} rows for d={}",
                        rows.len(),
                        d,
                    )));
                }
                let n = rows.len() / (d + 2);
                if n == 0 {
                    return Err(ServiceError::InvalidInput(
                        "sobol batch is empty".into(),
                    ));
                }
                let y_a = y[..n].to_vec();
                let y_b = y[n..2 * n].to_vec();
                let y_c: Vec<Vec<f64>> = (0..d)
                    .map(|i| {
                        let start = (2 + i) * n;
                        y[start..start + n].to_vec()
                    })
                    .collect();
                sensitivity::compute_sobol_indices(&params, &y_a, &y_b, &y_c)
            }
        };

        Ok(types::SensitivityAnalyzeResult {
            method: method_enum,
            parameters: results,
        })
    }

    // ------------------------------------------------------------------
    // Causation trace (R7.1)
    // ------------------------------------------------------------------

    /// Walk the causation graph backward from a "failure event" and return the
    /// chain of upstream causes.
    ///
    /// The root event is resolved in priority order:
    ///   1. `root_event_id` — direct lookup by recorder-assigned id.
    ///   2. `(root_tick, root_target)` — most recent event whose `tick` and
    ///      `target` match. Useful when the caller has a variable name and
    ///      a tick number (e.g. from a failing constraint verdict) but no
    ///      opaque event id.
    ///
    /// `max_depth` caps the BFS walk. When absent or zero, defaults to
    /// [`sysml_runtime::DEFAULT_TRACE_DEPTH`] (5).
    #[service_command(
        name = "sysml.causation.trace",
        category = Execution,
        description = "Walk backward through the causation graph from a root event; returns the root plus upstream chain",
        returns = "CausationTraceResult",
        stateful = true,
    )]
    pub fn causation_trace(
        &self,
        #[doc = "Opaque session id"] session_id: &str,
        #[doc = "Explicit causation event id (preferred when known)"]
        root_event_id: Option<&str>,
        #[doc = "Tick of the root event (used when root_event_id is absent)"]
        root_tick: Option<u64>,
        #[doc = "Target element / variable of the root event (used when root_event_id is absent)"]
        root_target: Option<&str>,
        #[doc = "Maximum BFS depth; 0 or absent = default (5). Clamped to u8 range."]
        max_depth: Option<u32>,
    ) -> Result<types::CausationTraceResult, ServiceError> {
        let key = ElementId::from_string(session_id);
        let entry = self.sessions.get(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        let recorder = entry.orchestrator.causation();
        let resolved_depth: u8 = match max_depth {
            Some(0) | None => sysml_runtime::DEFAULT_TRACE_DEPTH,
            Some(d) => u8::try_from(d.min(u8::MAX as u32)).unwrap_or(u8::MAX),
        };
        let root = if let Some(id) = root_event_id {
            recorder.find(id).cloned()
        } else if let (Some(tick), Some(target)) = (root_tick, root_target) {
            recorder.find_by_tick_target(tick, target).cloned()
        } else {
            None
        };
        let chain = match &root {
            Some(ev) => recorder.trace(&ev.id, resolved_depth),
            None => Vec::new(),
        };
        Ok(types::CausationTraceResult {
            root,
            chain,
            max_depth_used: resolved_depth as u32,
        })
    }

    // -- Command catalog --

    /// Returns a machine-readable JSON catalog of all service commands.
    ///
    /// Uses the inventory-based registry populated by `#[service_command]`
    /// annotations. This is the single source of truth for MCP tool definitions,
    /// REST API specs, and agent reference documentation.
    #[allow(clippy::expect_used)]
    pub fn command_catalog() -> serde_json::Value {
        serde_json::to_value(registered_command_metas())
            .expect("command registry serialization cannot fail")
    }

    // -- Infra / test-only knobs (not registered as service commands) --

    /// Override the orchestrator snapshot retention window on a specific
    /// session. Intended for tests and infrastructure that need tighter
    /// eviction bounds than the default
    /// [`execution::DEFAULT_SNAPSHOT_RETENTION_TICKS`].
    ///
    /// Returns `Ok(())` on success, or `Err(ElementNotFound)` when the
    /// session id is unknown.
    pub fn set_session_snapshot_retention(
        &self,
        session_id: &str,
        ticks: usize,
    ) -> Result<(), ServiceError> {
        let key = ElementId::from_string(session_id);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        entry.set_snapshot_retention(ticks);
        Ok(())
    }

    /// Override the archive cadence on a specific session (UX closeout
    /// arc #7). Intended for tests that need to force every-tick archiving
    /// (`ticks = 1`) to exercise the retention/eviction contract in
    /// isolation from the cadence feature, without stepping
    /// [`execution::DEFAULT_ARCHIVE_CADENCE_TICKS`] times.
    ///
    /// Returns `Ok(())` on success, or `Err(ElementNotFound)` when the
    /// session id is unknown.
    pub fn set_session_archive_cadence(
        &self,
        session_id: &str,
        ticks: u64,
    ) -> Result<(), ServiceError> {
        let key = ElementId::from_string(session_id);
        let mut entry = self.sessions.get_mut(&key).ok_or_else(|| {
            ServiceError::ElementNotFound(format!("no session: {session_id}"))
        })?;
        entry.set_archive_cadence(ticks);
        Ok(())
    }
}

impl std::fmt::Debug for SysmlService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SysmlService")
            .field("loaded_files", &self.host.lock().unwrap().file_count())
            .field("has_store", &self.store.is_some())
            .field("workspace_root", &self.workspace_root)
            .finish()
    }
}

impl SysmlService {
    /// Build the SModel JSON for a diagram render, applying the
    /// service-native diagnostic overlay.
    ///
    /// Routes through the salsa-tracked `cached_smodel` when the request
    /// is cacheable; falls back to a direct `to_smodel_with` otherwise.
    /// Diagnostics come from `self.diagnostics(uri)` so overlay severity
    /// follows the same full-pipeline output the LSP would publish.
    ///
    /// Separated from the `#[service_impl]` block so the proc macro
    /// doesn't try to register it as a service command.
    fn build_diagram_smodel(
        &self,
        uri: &str,
        view_type: sysml_diagram::smodel::ViewType,
        expanded_ids: &HashSet<String>,
    ) -> Result<serde_json::Value, ServiceError> {
        let graph = self.workspace_aware_graph()?;
        // Prune the URI's expanded set against the current graph so stale
        // ids (from prior edits) drop out before the next projection.
        if let Some(mut entry) = self.diagram_manager.expanded_nodes.get_mut(uri) {
            diagram::prune_expanded_ids(entry.value_mut(), &graph);
        }
        let request = sysml_diagram::ViewRequest::new(view_type).with_expanded(expanded_ids.clone());
        let mut sgraph = match self.cached_smodel(&request)? {
            Some(arc) => (*arc).clone(),
            None => sysml_diagram::smodel::to_smodel_with(&graph, &request),
        };
        let diags = self.diagnostics(uri).unwrap_or_default();
        if !diags.is_empty() {
            diagram::overlay_diagnostics(&mut sgraph, &diags);
        }
        Ok(serde_json::to_value(&sgraph).unwrap_or_default())
    }
}

/// Convert a `TreeNode` projection into the workspace-model-tree wire shape:
/// resolve the per-node span to a line/character range (LSP `Position`
/// semantics), recurse into children, and stamp the per-node `uri`.
///
/// Used by `SysmlService::workspace_model_tree`. The span lookup prefers
/// a span whose `file` matches the requested URI; falls back to the
/// first span otherwise. When no real (non-synthetic) span exists, emits
/// a zero range — matches the prior LSP handler's behaviour.
fn build_tree_node_with_range(
    node: &TreeNode,
    graph: &sysml_core::ModelGraph,
    uri: &str,
    content: &str,
) -> TreeNodeWithRange {
    const SYNTHETIC_FILE: &str = "<synthetic>";
    let element_id = node.element_id.clone().unwrap_or_else(|| node.id.clone());
    let element = graph.elements.get(&element_id);

    let range = element
        .and_then(|e| {
            e.spans
                .iter()
                .find(|s| s.file == uri && s.file != SYNTHETIC_FILE)
                .or_else(|| e.spans.first())
        })
        .map(|s| {
            let (start_line, start_char) = position::offset_to_line_col(s.start, content);
            let (end_line, end_char) = position::offset_to_line_col(s.end, content);
            TreeNodeRange {
                start: TreeNodePosition {
                    line: start_line,
                    character: start_char,
                },
                end: TreeNodePosition {
                    line: end_line,
                    character: end_char,
                },
            }
        })
        .unwrap_or(TreeNodeRange {
            start: TreeNodePosition { line: 0, character: 0 },
            end: TreeNodePosition { line: 0, character: 0 },
        });

    let children: Vec<TreeNodeWithRange> = node
        .children
        .iter()
        .map(|c| build_tree_node_with_range(c, graph, uri, content))
        .collect();

    TreeNodeWithRange {
        id: node.id.to_string(),
        name: node.name.clone().unwrap_or_default(),
        kind: format!("{:?}", node.kind),
        uri: uri.to_owned(),
        range,
        children,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing)]
mod tests {
    use super::*;

    #[test]
    fn test_empty_service() {
        let service = SysmlService::empty();
        assert_eq!(service.loaded_uris().len(), 0);
        assert!(service.require_graph("nonexistent").is_err());
    }

    #[test]
    fn provenance_path_is_relative_to_root() {
        let root = std::path::Path::new("/home/u/proj");
        // A file under the root is stripped to a relative path — no
        // absolute prefix leaks into a report/baseline.
        assert_eq!(
            provenance_relative_path("file:///home/u/proj/parts/Engine.sysml", Some(root)),
            "parts/Engine.sysml"
        );
        // Raw-path spelling (no file:// scheme) is handled identically.
        assert_eq!(
            provenance_relative_path("/home/u/proj/Top.sysml", Some(root)),
            "Top.sysml"
        );
    }

    #[test]
    fn provenance_path_falls_back_to_canonical_uri() {
        let root = std::path::Path::new("/home/u/proj");
        // Outside the root → canonical URI verbatim (honest, not a bogus
        // relative path built from `..`).
        let outside = provenance_relative_path("file:///etc/other/Lib.sysml", Some(root));
        assert!(outside.contains("Lib.sysml"));
        assert!(!outside.starts_with(".."));
        // No known root → canonical URI, never an absolute-path leak masked
        // as relative.
        assert_eq!(
            provenance_relative_path("file:///x/Y.sysml", None),
            sysml_ide_db::canonical_uri("file:///x/Y.sysml")
        );
    }

    #[test]
    fn workspace_file_manifest_hashes_loaded_text_sorted() {
        use sha2::{Digest, Sha256};
        let service = SysmlService::empty();
        service.load_source("file:///w/b.sysml", "package B;").unwrap();
        service.load_source("file:///w/a.sysml", "package A;").unwrap();
        let root = std::path::Path::new("/w");
        let manifest = service.workspace_file_manifest(Some(root));
        assert_eq!(manifest.len(), 2);
        // Sorted by path for a byte-stable block.
        assert_eq!(manifest[0].path, "a.sysml");
        assert_eq!(manifest[1].path, "b.sysml");
        // content_hash is SHA-256 (hex) of the loaded text.
        let expect_a = format!("{:x}", Sha256::digest("package A;".as_bytes()));
        assert_eq!(manifest[0].content_hash, expect_a);
    }

    #[test]
    fn test_load_source() {
        let service = SysmlService::empty();
        service
            .load_source("test.sysml", "package Vehicle { part engine; }")
            .unwrap();

        assert_eq!(service.loaded_uris().len(), 1);
        let graph = service.require_graph("test.sysml").unwrap();
        assert!(crate::query::stats(&graph).total_elements > 0);
    }

    #[test]
    fn views_list_surfaces_view_definitions() {
        let service = SysmlService::empty();
        service
            .load_source(
                "test.sysml",
                "package P { view def OverviewView; view ConfigView; }",
            )
            .unwrap();

        let views = service.views_list("test.sysml").unwrap();
        let names: Vec<_> = views.iter().filter_map(|v| v.name.as_deref()).collect();
        assert!(
            names.contains(&"OverviewView"),
            "expected OverviewView, got {names:?}"
        );
        assert!(
            names.contains(&"ConfigView"),
            "expected ConfigView, got {names:?}"
        );
    }

    #[test]
    fn views_list_returns_empty_for_model_with_no_views() {
        let service = SysmlService::empty();
        service
            .load_source("test.sysml", "package P { part engine; }")
            .unwrap();
        let views = service.views_list("test.sysml").unwrap();
        assert!(views.is_empty());
    }

    #[test]
    fn test_find() {
        let service = SysmlService::empty();
        service
            .load_source("test.sysml", "package Vehicle { part engine; }")
            .unwrap();

        let results = service.find("test.sysml", "Vehicle", None).unwrap();
        assert!(!results.is_empty());
        assert_eq!(results[0].name.as_deref(), Some("Vehicle"));
    }

    #[test]
    fn test_query_primitive_summary_and_cache() {
        let service = SysmlService::empty();
        service
            .load_source("test.sysml", "package Vehicle { part engine; }")
            .unwrap();

        let spec = serde_json::json!({
            "filter": { "type": "kind", "kinds": ["PartUsage"] },
            "projection": "summary"
        });
        let first = service.query("test.sysml", &spec).unwrap();
        assert_eq!(first.cache_status, sysml_query::QueryCacheStatus::Miss);
        let second = service.query("test.sysml", &spec).unwrap();
        assert_eq!(second.cache_status, sysml_query::QueryCacheStatus::Hit);
        let sysml_query::QueryRows::Summary(rows) = second.rows else {
            panic!("expected summary rows");
        };
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].name.as_deref(), Some("engine"));
    }

    #[test]
    fn test_query_inventory_dispatch() {
        let service = SysmlService::empty();
        service
            .load_source("test.sysml", "package Vehicle { part engine; }")
            .unwrap();
        let value = execute_command(
            &service,
            "sysml.query",
            serde_json::json!({
                "uri": "test.sysml",
                "spec": {
                    "filter": { "type": "name_match", "name_match": { "contains": "engine" } },
                    "projection": "ids"
                }
            }),
        )
        .unwrap();
        assert_eq!(value["total_estimate"], serde_json::json!(1));
    }

    #[test]
    fn test_query_bad_filter_validation() {
        let service = SysmlService::empty();
        service
            .load_source("test.sysml", "package Vehicle { part engine; }")
            .unwrap();
        let spec = serde_json::json!({
            "filter": { "type": "name_match", "name_match": { "contains": "engine", "exact": "engine" } }
        });
        assert!(service.query("test.sysml", &spec).is_err());
    }

    #[test]
    fn test_model_tree() {
        let service = SysmlService::empty();
        service
            .load_source("test.sysml", "package Vehicle { part engine; }")
            .unwrap();

        let tree = service.model_tree("test.sysml", None, None).unwrap();
        assert!(!tree.is_empty());

        // With max_depth=1, roots show with direct children but grandchildren are truncated
        let tree_limited = service.model_tree("test.sysml", Some(1), None).unwrap();
        assert!(!tree_limited.is_empty());
        // Same number of roots, but deeper children are truncated
        assert_eq!(tree.len(), tree_limited.len());
    }

    #[test]
    fn test_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("test.sysml");
        std::fs::write(&file, "package P { part x; }").unwrap();

        let service = SysmlService::from_file(&file).unwrap();
        assert_eq!(service.loaded_uris().len(), 1);
    }

    #[test]
    fn test_from_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.sysml"), "package A {}").unwrap();
        std::fs::write(dir.path().join("b.sysml"), "package B {}").unwrap();
        std::fs::write(dir.path().join("c.txt"), "not sysml").unwrap();

        let service = SysmlService::from_workspace(dir.path()).unwrap();
        assert_eq!(service.loaded_uris().len(), 2);
    }

    // Task #159 regression: per-file diagnostics on a file inside a loaded
    // workspace must see cross-file definitions from sibling files.
    // Before the fix, `load_workspace` registered files via
    // `set_file_content` (no project_id), so `compute_full_diagnostics`
    // fell back to the file-only resolution path and emitted spurious
    // E200 "no definition `X` found" diagnostics for cross-file imports.
    #[test]
    fn diagnostics_see_cross_file_imports_after_load_workspace() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("definitions.sysml"),
            "package Definitions {\n    part def Widget;\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("usage.sysml"),
            "package Usage {\n    import Definitions::*;\n    part w : Widget;\n}\n",
        )
        .unwrap();

        let service = SysmlService::empty();
        let _ = service.load_workspace(dir.path()).unwrap();

        let usage_uri = dir
            .path()
            .canonicalize()
            .unwrap()
            .join("usage.sysml")
            .to_string_lossy()
            .to_string();
        let diags = service.diagnostics(&usage_uri).unwrap();

        // The cross-file type Widget must resolve — no E200 mentioning it,
        // and no IM001 saying Definitions is unresolved in the current
        // workspace context.
        let widget_unresolved: Vec<_> = diags
            .iter()
            .filter(|d| d.message.contains("Widget"))
            .collect();
        assert!(
            widget_unresolved.is_empty(),
            "expected no diagnostics mentioning Widget after workspace load, got: {:?}",
            widget_unresolved
                .iter()
                .map(|d| &d.message)
                .collect::<Vec<_>>()
        );
        let im001: Vec<_> = diags
            .iter()
            .filter(|d| {
                d.code.as_deref() == Some("IM001")
                    && d.message.contains("Definitions")
            })
            .collect();
        assert!(
            im001.is_empty(),
            "expected no IM001 for Definitions after workspace load, got: {:?}",
            im001.iter().map(|d| &d.message).collect::<Vec<_>>()
        );
    }

    // B10 regression: two successive load_workspace calls with different
    // roots must not bleed each other's docs into the merged __workspace__.
    #[test]
    fn load_workspace_scopes_to_new_root() {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        std::fs::write(
            dir_a.path().join("a.sysml"),
            "package A { part def UniqueA; }",
        )
        .unwrap();
        std::fs::write(
            dir_b.path().join("b.sysml"),
            "package B { part def UniqueB; }",
        )
        .unwrap();

        let service = SysmlService::empty();
        let _ = service.load_workspace(dir_a.path()).unwrap();

        let ws_after_a = service.workspace_aware_graph().unwrap();
        assert!(
            ws_after_a
                .elements
                .values()
                .any(|e| e.name.as_deref() == Some("UniqueA")),
            "UniqueA must appear after loading workspace A",
        );
        drop(ws_after_a);

        let _ = service.load_workspace(dir_b.path()).unwrap();

        let ws_after_b = service.workspace_aware_graph().unwrap();
        let graph = ws_after_b.as_ref();
        assert!(
            graph
                .elements
                .values()
                .any(|e| e.name.as_deref() == Some("UniqueB")),
            "UniqueB must appear after loading workspace B",
        );
        assert!(
            !graph
                .elements
                .values()
                .any(|e| e.name.as_deref() == Some("UniqueA")),
            "UniqueA must NOT persist after switching to workspace B (B10)",
        );

        // And the doc graph for A must be gone from the host's file set.
        let a_uri = dir_a
            .path()
            .canonicalize()
            .unwrap()
            .join("a.sysml")
            .to_string_lossy()
            .to_string();
        assert!(
            service.host.lock().unwrap().file_id(&a_uri).is_none(),
            "doc graph for workspace A must be dropped on load of workspace B",
        );
    }

    // Diagnostic for B9: run against a real on-disk workspace to see what
    // happens in production. The workspace is caller-supplied: point
    // SYSML_DIAG_WS at one (e.g. `examples/espresso-production-cell`).
    #[test]
    #[ignore]
    fn diag_b9_examples_workspace() {
        let root = std::env::var("SYSML_DIAG_WS").expect(
            "SYSML_DIAG_WS must name a workspace directory to diagnose \
             (e.g. SYSML_DIAG_WS=examples/espresso-production-cell)",
        );
        let service = SysmlService::empty();
        let _ = service
            .load_workspace(std::path::Path::new(&root))
            .unwrap();
        let ws = service.workspace_aware_graph().unwrap();
        eprintln!("\n=== B9 diagnostic (examples workspace) ===");
        eprintln!("__workspace__ elements = {}", ws.elements.len());
        eprintln!("library_packages count = {}", ws.library_packages().len());
        let ctx = sysml_ide_db::eval_context_seed::context_from_graph(&ws);
        eprintln!("ctx vars (filter ON) = {}", ctx.variables.len());
        // Sample ALL operator-looking keys
        let op_keys: Vec<&String> = ctx
            .variables
            .keys()
            .filter(|k| k.chars().all(|c| !c.is_alphanumeric() && c != '_' && c != '.'))
            .take(20)
            .collect();
        eprintln!("ctx operator-like keys = {:?}", op_keys);

        // Also build a workspace orchestrator and measure its context directly
        // (this is the actual runtime path the live backend uses).
        let base_ctx = sysml_ide_db::eval_context_seed::context_from_graph(&ws);
        let precompiled = Arc::new(sysml_runtime::constraints::extract_and_precompile(&ws));
        let port_flow = Arc::new(sysml_runtime::flows::build_port_flow_resources(&ws));
        let gated = Arc::new(sysml_runtime::compiler::build_gated_expressions(&ws));
        let snap = sysml_ide_db::Snapshot::new(Arc::clone(&ws));
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
            .unwrap();
        eprintln!("orchestrator ctx vars (tick 0) = {}", orchestrator.context.variables.len());
        let mut orchestrator = orchestrator;
        for _ in 0..5 { orchestrator.step(); }
        eprintln!("orchestrator ctx vars (tick 5) = {}", orchestrator.context.variables.len());
        for _ in 0..45 { orchestrator.step(); }
        eprintln!("orchestrator ctx vars (tick 50) = {}", orchestrator.context.variables.len());
        for _ in 0..50 { orchestrator.step(); }
        eprintln!("orchestrator ctx vars (tick 100) = {}", orchestrator.context.variables.len());
        let orch_op_keys: Vec<&String> = orchestrator
            .context
            .variables
            .keys()
            .filter(|k| k.chars().all(|c| !c.is_alphanumeric() && c != '_' && c != '.'))
            .take(20)
            .collect();
        eprintln!("orchestrator operator-like keys after step = {:?}", orch_op_keys);
        // What are the first 30 keys overall?
        let mut all_keys: Vec<&String> = orchestrator.context.variables.keys().collect();
        all_keys.sort();
        let sample: Vec<&&String> = all_keys.iter().take(20).collect();
        eprintln!("first 20 keys sorted = {:?}", sample);
    }

    // Diagnostic for B9: after load_workspace, inspect library_packages count
    // and the first few variables that would flow into the EvalContext.
    #[test]
    #[ignore]
    fn diag_b9_stdlib_filter_state() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("tiny.sysml"),
            "package Tiny { part def Boiler { attribute temp = 20.0; } }",
        )
        .unwrap();

        let service = SysmlService::empty();
        let _ = service.load_workspace(dir.path()).unwrap();

        let ws = service.workspace_aware_graph().unwrap();
        eprintln!("\n=== B9 diagnostic ===");
        eprintln!("__workspace__ elements = {}", ws.elements.len());
        eprintln!("library_packages count = {}", ws.library_packages().len());
        let lib_pkg_names: Vec<String> = ws
            .library_package_elements()
            .filter_map(|e| e.name.clone())
            .take(10)
            .collect();
        eprintln!("library_packages sample = {:?}", lib_pkg_names);

        let ctx = sysml_ide_db::eval_context_seed::context_from_graph(&ws);
        eprintln!("ctx vars (filter ON) = {}", ctx.variables.len());
        let mut sample: Vec<&String> = ctx.variables.keys().take(15).collect();
        sample.sort();
        eprintln!("ctx sample keys = {:?}", sample);

        let ctx_all =
            sysml_ide_db::eval_context_seed::context_from_graph_with_options(&ws, true);
        eprintln!("ctx vars (filter OFF) = {}", ctx_all.variables.len());

        // Count how many elements have !is_library_element:
        let user_count = ws
            .elements
            .values()
            .filter(|e| !ws.is_library_element(&e.id))
            .count();
        eprintln!("non-library element count = {}", user_count);
    }

    #[test]
    fn test_eval_context() {
        let service = SysmlService::empty();
        service
            .load_source("test.sysml", "package P { attribute x = 42; }")
            .unwrap();

        let ctx = service
            .eval_context_with_overrides(&[("y".into(), "10".into())])
            .unwrap();
        assert_eq!(ctx.get("y"), Some(&Value::Int(10)));
    }

    #[test]
    fn test_snapshot_queries() {
        let service = SysmlService::empty();
        service
            .load_source(
                "test.sysml",
                "package Pkg { part engine; requirement safetyReq; }",
            )
            .unwrap();

        let graph = service.require_graph("test.sysml").unwrap();
        let found = sysml_core::query::find_by_name(&graph, None, "Pkg").count();
        assert_eq!(found, 1);

        let stats = crate::query::stats(&graph);
        assert!(stats.total_elements >= 3);

        let tree = crate::query::model_tree(&graph, None);
        assert!(!tree.is_empty());
    }

    // -----------------------------------------------------------------------
    // B1 — sessions.list / info / reap / quota + per-kind cap isolation
    // -----------------------------------------------------------------------

    /// Insert a fake empty-orchestrator session of the given kind. Used to
    /// exercise catalog and cap-check paths without a real model.
    fn insert_fake_session(
        service: &SysmlService,
        kind: execution::SessionKind,
    ) -> String {
        let orchestrator = sysml_runtime::orchestrator::Orchestrator::new(Default::default());
        let session = execution::RuntimeSession::new(
            orchestrator,
            "test://uri".to_owned(),
            kind,
            None,
        );
        let id = execution::new_session_id();
        let id_str = id.to_string();
        service.insert_session(id, session);
        id_str
    }

    // ------------------------------------------------------------------
    // Breakpoint service commands (R1.2)
    // ------------------------------------------------------------------

    #[test]
    fn test_breakpoint_set_clear_list_round_trip_via_service() {
        use sysml_runtime::breakpoint::{Breakpoint, CompareOp};

        let service = SysmlService::empty();
        let sid = insert_fake_session(&service, execution::SessionKind::Simulation);

        // list is empty
        assert!(service.breakpoint_list(&sid).unwrap().is_empty());

        // set two breakpoints
        let id1 = service
            .breakpoint_set(&sid, Breakpoint::state_entry("running"))
            .unwrap();
        let id2 = service
            .breakpoint_set(
                &sid,
                Breakpoint::threshold_crossing("v_bus", CompareOp::Ge, 12.5),
            )
            .unwrap();

        let listed = service.breakpoint_list(&sid).unwrap();
        assert_eq!(listed.len(), 2);

        // Sorted order
        let ids: Vec<&str> = listed.iter().map(|(id, _)| id.as_str()).collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids, sorted);

        // Content preserved
        let kinds: std::collections::HashSet<&str> = listed
            .iter()
            .map(|(_, b)| match b {
                Breakpoint::StateEntry { .. } => "state-entry",
                Breakpoint::ThresholdCrossing(..) => "threshold-crossing",
                _ => "other",
            })
            .collect();
        assert!(kinds.contains("state-entry"));
        assert!(kinds.contains("threshold-crossing"));

        // clear one
        service.breakpoint_clear(&sid, &id1).unwrap();
        let listed2 = service.breakpoint_list(&sid).unwrap();
        assert_eq!(listed2.len(), 1);
        assert_eq!(listed2[0].0, id2);

        // clear unknown id is idempotent
        service
            .breakpoint_clear(&sid, "does-not-exist")
            .unwrap();
        assert_eq!(service.breakpoint_list(&sid).unwrap().len(), 1);
    }

    #[test]
    fn test_breakpoint_set_on_missing_session_errors() {
        use sysml_runtime::breakpoint::Breakpoint;

        let service = SysmlService::empty();
        let err = service
            .breakpoint_set("no-such-session", Breakpoint::state_entry("s1"))
            .unwrap_err();
        assert!(err.to_string().contains("no session"), "{err}");
    }

    #[test]
    fn test_breakpoint_list_and_clear_on_missing_session_errors() {
        let service = SysmlService::empty();
        assert!(service.breakpoint_list("no-such-session").is_err());
        assert!(service
            .breakpoint_clear("no-such-session", "any")
            .is_err());
    }

    #[test]
    fn test_breakpoint_dispatch_via_execute_command() {
        use sysml_runtime::breakpoint::Breakpoint;

        // Verify the commands are reachable through the inventory-based JSON dispatch
        // — this is the same path used by MCP and REST transports.
        let service = SysmlService::empty();
        let sid = insert_fake_session(&service, execution::SessionKind::Simulation);

        let set_result = execute_command(
            &service,
            "sysml.breakpoint.set",
            serde_json::json!({
                "session_id": sid,
                "breakpoint": Breakpoint::state_entry("running"),
            }),
        )
        .unwrap();
        let new_id = set_result.as_str().unwrap().to_owned();
        assert!(!new_id.is_empty());

        let list_result = execute_command(
            &service,
            "sysml.breakpoint.list",
            serde_json::json!({ "session_id": sid }),
        )
        .unwrap();
        let arr = list_result.as_array().unwrap();
        assert_eq!(arr.len(), 1);

        let _clear_result = execute_command(
            &service,
            "sysml.breakpoint.clear",
            serde_json::json!({ "session_id": sid, "breakpoint_id": new_id }),
        )
        .unwrap();
        let after = execute_command(
            &service,
            "sysml.breakpoint.list",
            serde_json::json!({ "session_id": sid }),
        )
        .unwrap();
        assert_eq!(after.as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_sessions_list_empty_and_populated() {
        let service = SysmlService::empty();
        assert!(service.sessions_list().unwrap().is_empty());

        let _sim = insert_fake_session(&service, execution::SessionKind::Simulation);
        let _act = insert_fake_session(&service, execution::SessionKind::Action);
        let _orc = insert_fake_session(&service, execution::SessionKind::Orchestrator);

        let list = service.sessions_list().unwrap();
        assert_eq!(list.len(), 3);
        let kinds: HashSet<_> = list.iter().map(|s| s.kind).collect();
        assert!(kinds.contains(&execution::SessionKind::Simulation));
        assert!(kinds.contains(&execution::SessionKind::Action));
        assert!(kinds.contains(&execution::SessionKind::Orchestrator));
    }

    #[test]
    fn test_sessions_info_returns_detail_for_live_session() {
        let service = SysmlService::empty();
        let id = insert_fake_session(&service, execution::SessionKind::Orchestrator);
        let detail = service.sessions_info(&id, None).unwrap().unwrap();
        assert_eq!(detail.summary.id, id);
        assert_eq!(detail.summary.kind, execution::SessionKind::Orchestrator);
        assert!(detail.subsystems.is_empty()); // empty orchestrator
    }

    #[test]
    fn test_sessions_info_returns_none_for_missing() {
        let service = SysmlService::empty();
        let detail = service.sessions_info("no-such-id", None).unwrap();
        assert!(detail.is_none());
    }

    #[test]
    fn test_sessions_reap_drops_expired_only() {
        let service = SysmlService::empty();
        let live_id = insert_fake_session(&service, execution::SessionKind::Simulation);
        let dead_id = insert_fake_session(&service, execution::SessionKind::Simulation);

        // Force one session into the expired state via the test helper.
        service
            .sessions
            .get_mut(&ElementId::from_string(&dead_id))
            .unwrap()
            .test_mark_expired();

        let dropped = service.sessions_reap().unwrap();
        assert_eq!(dropped, 1);
        assert!(service.sessions.contains_key(&ElementId::from_string(&live_id)));
        assert!(!service.sessions.contains_key(&ElementId::from_string(&dead_id)));
    }

    #[test]
    fn test_sessions_quota_reports_per_kind_counts() {
        let service = SysmlService::empty();
        let _a = insert_fake_session(&service, execution::SessionKind::Simulation);
        let _b = insert_fake_session(&service, execution::SessionKind::Simulation);
        let _c = insert_fake_session(&service, execution::SessionKind::Action);

        let q = service.sessions_quota().unwrap();
        assert_eq!(q.simulation.used, 2);
        assert_eq!(q.simulation.cap, 30);
        assert_eq!(q.action.used, 1);
        assert_eq!(q.action.cap, 30);
        assert_eq!(q.orchestrator.used, 0);
        assert_eq!(q.orchestrator.cap, 20);
    }

    #[test]
    fn test_cap_check_isolates_kinds() {
        let service = SysmlService::empty();
        // Fill the Simulation bucket to its cap.
        for _ in 0..execution::quota_for(execution::SessionKind::Simulation) {
            insert_fake_session(&service, execution::SessionKind::Simulation);
        }
        // Simulation should now error.
        let err = service
            .cap_check(execution::SessionKind::Simulation)
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("simulation bucket full"),
            "expected bucket-specific error, got {msg}"
        );
        // Action and Orchestrator are still free.
        assert!(service.cap_check(execution::SessionKind::Action).is_ok());
        assert!(
            service
                .cap_check(execution::SessionKind::Orchestrator)
                .is_ok()
        );
    }

    #[test]
    fn test_cap_check_lazy_reaps_expired() {
        let service = SysmlService::empty();
        // Fill Simulation bucket, then mark every session expired.
        let mut ids = Vec::new();
        for _ in 0..execution::quota_for(execution::SessionKind::Simulation) {
            ids.push(insert_fake_session(
                &service,
                execution::SessionKind::Simulation,
            ));
        }
        for id in &ids {
            service.sessions.get_mut(&ElementId::from_string(id)).unwrap().test_mark_expired();
        }
        // cap_check should reap all of them and succeed.
        assert!(service.cap_check(execution::SessionKind::Simulation).is_ok());
        assert_eq!(
            service.session_count(execution::SessionKind::Simulation),
            0
        );
    }

    // -----------------------------------------------------------------------
    // B2 — generic session operations (stop/step/inject/reset)
    // -----------------------------------------------------------------------

    /// Build a trivial state-machine model with two states and a "go" event.
    const TWO_STATE_SOURCE: &str = r#"
        state def TwoStep {
            entry; then idle;
            state idle;
            state running;
            transition first first idle then running accept go;
        }
    "#;

    /// Build a sim session using the real start command so all the plumbing
    /// is exercised. Returns the session id.
    fn start_sim_session(service: &SysmlService) -> String {
        // Insert a fake session directly — the full compile path pulls in a
        // graph, which unit tests avoid. The fake has an empty orchestrator
        // so step() is a no-op tick.
        let _ = TWO_STATE_SOURCE;
        insert_fake_session(service, execution::SessionKind::Simulation)
    }

    #[test]
    fn test_sessions_stop_removes_session_regardless_of_kind() {
        let service = SysmlService::empty();
        let sim_id = insert_fake_session(&service, execution::SessionKind::Simulation);
        let act_id = insert_fake_session(&service, execution::SessionKind::Action);

        service.sessions_stop(&sim_id).unwrap();
        assert!(!service.sessions.contains_key(&ElementId::from_string(&sim_id)));
        // Action still present — stop is per-id, not per-kind.
        assert!(service.sessions.contains_key(&ElementId::from_string(&act_id)));

        service.sessions_stop(&act_id).unwrap();
        assert!(!service.sessions.contains_key(&ElementId::from_string(&act_id)));
    }

    #[test]
    fn test_sessions_stop_missing_id_errors() {
        let service = SysmlService::empty();
        let err = service.sessions_stop("no-such-id").unwrap_err();
        assert!(err.to_string().contains("no session"));
    }

    #[test]
    fn test_sessions_step_returns_summary_and_advances_tick() {
        let service = SysmlService::empty();
        let id = start_sim_session(&service);

        let initial_tick = service.sessions_info(&id, None).unwrap().unwrap().summary.tick;

        // Empty orchestrator: step() bumps tick without doing real work.
        let s1 = service.sessions_step(&id, None, None, None).unwrap();
        assert!(s1.tick > initial_tick);
        assert_eq!(s1.id, id);
        assert_eq!(s1.history_len, 1);

        let s2 = service.sessions_step(&id, None, None, None).unwrap();
        assert!(s2.tick > s1.tick);
        assert_eq!(s2.history_len, 2);

        // BP1: a step that neither breaks nor is a bulk step (ticks=None
        // defaults to 1) fully advances, so ticks_advanced == 1 (== the
        // effective request) and there's no pause.
        assert!(!s2.paused);
        assert_eq!(s2.paused_at_breakpoint, None);
        assert_eq!(s2.ticks_advanced, 1);
    }

    /// BP1: a bulk `sessions.step` that halts partway on a breakpoint must
    /// surface the halt on the wire — `paused`, `paused_at_breakpoint`, and
    /// `ticks_advanced` (strictly less than the requested count, since the
    /// breakpoint fires on the very first tick here and step_many stops
    /// immediately).
    #[test]
    fn test_sessions_step_bulk_halts_on_breakpoint_reports_ticks_advanced() {
        use sysml_runtime::breakpoint::{Breakpoint, CompareOp};

        let service = SysmlService::empty();
        let id = start_sim_session(&service);

        // Seed a context variable that is already past the threshold, so
        // the breakpoint fires on tick 1 of the bulk step.
        service
            .sessions
            .get_mut(&ElementId::from_string(&id))
            .unwrap()
            .orchestrator
            .context
            .set("i_total".to_owned(), sysml_core::Value::Float(50.0));

        let bp_id = service
            .breakpoint_set(
                &id,
                Breakpoint::threshold_crossing("i_total", CompareOp::Gt, 32.0),
            )
            .unwrap();

        let requested: u64 = 10;
        let summary = service
            .sessions_step(&id, None, None, Some(requested))
            .unwrap();

        assert!(summary.paused, "bulk step must halt at the breakpoint");
        assert_eq!(summary.paused_at_breakpoint, Some(bp_id));
        assert!(
            summary.ticks_advanced < requested,
            "expected an early halt (ticks_advanced < {requested}), got {}",
            summary.ticks_advanced
        );
        assert_eq!(
            summary.ticks_advanced, 1,
            "condition holds from tick 1, so step_many should stop immediately"
        );
    }

    /// BP2: `sysml.sessions.resume` clears the pause so a subsequent
    /// `sessions.step` advances again. Idempotent — calling it on an
    /// already-running (non-paused) session is a no-op success.
    #[test]
    fn test_sessions_resume_clears_pause_and_allows_further_stepping() {
        use sysml_runtime::breakpoint::{Breakpoint, CompareOp};

        let service = SysmlService::empty();
        let id = start_sim_session(&service);

        service
            .sessions
            .get_mut(&ElementId::from_string(&id))
            .unwrap()
            .orchestrator
            .context
            .set("i_total".to_owned(), sysml_core::Value::Float(50.0));

        let bp_id = service
            .breakpoint_set(
                &id,
                Breakpoint::threshold_crossing("i_total", CompareOp::Gt, 32.0),
            )
            .unwrap();

        // Idempotent no-op on a not-yet-paused session.
        let pre = service.sessions_resume(&id).unwrap();
        assert!(!pre.paused);

        // Step until the breakpoint pauses us.
        let paused_summary = service.sessions_step(&id, None, None, None).unwrap();
        assert!(paused_summary.paused);
        assert_eq!(paused_summary.paused_at_breakpoint, Some(bp_id.clone()));

        // A further step while paused is a documented no-op (RuntimeSession::step).
        let still_paused = service.sessions_step(&id, None, None, None).unwrap();
        assert!(still_paused.paused);
        assert_eq!(still_paused.tick, paused_summary.tick);

        // Resume clears the pause.
        let resumed = service.sessions_resume(&id).unwrap();
        assert!(!resumed.paused);
        assert_eq!(resumed.paused_at_breakpoint, None);

        // Stepping now advances past the old pause point again (the
        // condition still holds, so it immediately re-fires and re-pauses
        // — this asserts the tick truly advanced, not that it stays clear
        // forever, matching debounce-free ThresholdCrossing semantics).
        let after = service.sessions_step(&id, None, None, None).unwrap();
        assert!(after.tick > paused_summary.tick);

        // Resume again (the session is once more paused, since the
        // condition never stopped holding and there's no debounce) —
        // still succeeds, whether the session was paused or running.
        let resumed_again = service.sessions_resume(&id);
        assert!(resumed_again.is_ok());
    }

    #[test]
    fn test_sessions_step_with_event_errors_when_no_primary_subsystem() {
        let service = SysmlService::empty();
        // Orchestrator session with no primary subsystem_name.
        let id = insert_fake_session(&service, execution::SessionKind::Orchestrator);
        let err = service.sessions_step(&id, Some("go"), None, None).unwrap_err();
        assert!(err.to_string().contains("no primary subsystem"));
    }

    #[test]
    fn test_sessions_inject_advances_with_event_on_orchestrator() {
        let service = SysmlService::empty();
        // Orchestrator without a primary subsystem — inject requires an
        // explicit subsystem name so it works on any session kind.
        let id = insert_fake_session(&service, execution::SessionKind::Orchestrator);
        let summary = service.sessions_inject(&id, "nonexistent", "go", None).unwrap();
        // Empty orchestrator: the inject queues the event but no subsystem
        // consumes it. We only assert the call succeeds and the tick advances.
        assert_eq!(summary.id, id);
        assert!(summary.tick >= 1);
    }

    #[test]
    fn test_sessions_reset_restarts_expiry_and_clears_history() {
        let service = SysmlService::empty();
        let id = start_sim_session(&service);

        // Step twice to build history.
        service.sessions_step(&id, None, None, None).unwrap();
        service.sessions_step(&id, None, None, None).unwrap();
        assert_eq!(
            service.sessions_info(&id, None).unwrap().unwrap().summary.history_len,
            2
        );

        // Mark expired, then reset — reset restarts the expiry clock.
        service.sessions.get_mut(&ElementId::from_string(&id)).unwrap().test_mark_expired();
        assert!(service.sessions_info(&id, None).unwrap().unwrap().summary.is_expired);

        let after = service.sessions_reset(&id).unwrap();
        assert_eq!(after.history_len, 0);
        assert!(!after.is_expired);
    }

    #[test]
    fn test_old_simulate_stop_is_a_forwarder() {
        let service = SysmlService::empty();
        let id = insert_fake_session(&service, execution::SessionKind::Simulation);
        // Old command still works on the unified catalog.
        service.simulate_stop(&id).unwrap();
        assert!(!service.sessions.contains_key(&ElementId::from_string(&id)));
    }

    #[test]
    fn test_old_orchestrate_stop_is_a_forwarder() {
        let service = SysmlService::empty();
        let id = insert_fake_session(&service, execution::SessionKind::Orchestrator);
        service.orchestrate_stop(&id).unwrap();
        assert!(!service.sessions.contains_key(&ElementId::from_string(&id)));
    }

    // -----------------------------------------------------------------------
    // B3 — sessions.fork / sessions.fork_with_overrides
    // -----------------------------------------------------------------------

    /// Insert a fake Simulation session carrying a real state machine so the
    /// orchestrator actually advances and captures subsystem state. Returns
    /// the session id.
    ///
    /// Built from a real compiled `StateMachineIR` via `load_source` +
    /// `execution_snapshot`/`build_sm_orchestrator` — the same production
    /// mint+bind path `simulate_start` now calls (ledger L44). This fixture
    /// used to hand-roll `Orchestrator::add_state_machine` +
    /// `mint_state_slots_for_test` (ledger L43) because no production
    /// single-SM-no-ODE mint entry point existed yet; now that
    /// `build_sm_orchestrator` does, this collapses onto it rather than
    /// keeping a second slot-minting path alive for a scenario production
    /// already covers — one home. The subsystem name is now
    /// "SM" (the declared state-def name `build_sm_orchestrator` registers
    /// under), not the old arbitrary "sm" label.
    fn insert_running_sim_session(service: &SysmlService) -> String {
        let uri = "fork-test.sysml";
        service
            .load_source(
                uri,
                r#"
                package ForkTest {
                    state def SM {
                        attribute speed : Real;
                        state A;
                        state B;
                        transition first A accept go do action { speed = 100; } then B;
                    }
                }
                "#,
            )
            .unwrap();

        let snap = service.execution_snapshot(uri).unwrap();
        let orchestrator = snap.build_sm_orchestrator("SM", None, None).unwrap();

        let session = execution::RuntimeSession::new(
            orchestrator,
            uri.to_owned(),
            execution::SessionKind::Simulation,
            Some("SM".to_owned()),
        );
        let id = execution::new_session_id();
        let id_str = id.to_string();
        service.insert_session(id, session);
        id_str
    }

    #[test]
    fn test_sessions_fork_is_independent() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);

        // Step the parent a couple of ticks (no event — stays in A).
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();

        // Fork.
        let child_summary = service.sessions_fork(&parent_id).unwrap();
        let child_id = child_summary.id.clone();
        assert_ne!(child_id, parent_id, "child must have a fresh UUID");
        assert_eq!(
            child_summary.uri, "fork-test.sysml",
            "child inherits uri from parent"
        );
        assert_eq!(
            child_summary.subsystem_name.as_deref(),
            Some("SM"),
            "child inherits primary subsystem name"
        );
        assert_eq!(
            child_summary.kind,
            execution::SessionKind::Simulation,
            "child inherits kind"
        );

        // Drive the child through the transition.
        let after_child = service
            .sessions_step(&child_id, Some("go"), None, None)
            .unwrap();
        assert_eq!(after_child.current_state.as_deref(), Some("B"));

        // Parent unchanged — still in A and no `speed` in its context.
        let parent_info = service.sessions_info(&parent_id, None).unwrap().unwrap();
        assert_eq!(parent_info.summary.current_state.as_deref(), Some("A"));
        let parent_entry = service.sessions.get(&ElementId::from_string(&parent_id)).unwrap();
        assert_eq!(
            parent_entry.orchestrator.context.get("speed"),
            None,
            "parent must NOT see child's assignment",
        );
    }

    #[test]
    fn test_sessions_fork_with_overrides_only_child_is_perturbed() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);

        // Seed the parent context so we can verify overrides replace, not
        // inherit-and-forget.
        service
            .sessions
            .get_mut(&ElementId::from_string(&parent_id))
            .unwrap()
            .orchestrator
            .context
            .set("gain".to_owned(), sysml_core::Value::Float(1.0));

        let overrides = vec![("gain".to_owned(), "2.5".to_owned())];
        let child_summary = service
            .sessions_fork_with_overrides(&parent_id, &overrides, None)
            .unwrap();
        let child_id = child_summary.id.clone();

        // Parent: gain still 1.0.
        let parent_entry = service.sessions.get(&ElementId::from_string(&parent_id)).unwrap();
        assert_eq!(
            parent_entry.orchestrator.context.get("gain"),
            Some(&sysml_core::Value::Float(1.0)),
            "parent context must be untouched",
        );

        // Child: gain is now 2.5.
        let child_entry = service.sessions.get(&ElementId::from_string(&child_id)).unwrap();
        assert_eq!(
            child_entry.orchestrator.context.get("gain"),
            Some(&sysml_core::Value::Float(2.5)),
            "child context must reflect overrides",
        );
    }

    #[test]
    fn test_sessions_fork_respects_cap_check() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);

        // Fill the Simulation bucket up to the cap (including the parent).
        // After this, sessions_fork must fail with a bucket-specific error.
        let cap = execution::quota_for(execution::SessionKind::Simulation);
        while service.session_count(execution::SessionKind::Simulation) < cap {
            insert_fake_session(&service, execution::SessionKind::Simulation);
        }

        let err = service.sessions_fork(&parent_id).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("simulation bucket full"),
            "expected per-kind cap error, got: {msg}",
        );
    }

    #[test]
    fn test_sessions_fork_missing_id_errors() {
        let service = SysmlService::empty();
        let err = service.sessions_fork("no-such-id").unwrap_err();
        assert!(err.to_string().contains("no session"));
    }

    // -----------------------------------------------------------------------
    // B4 — sessions.rename / sessions.subsystems
    // -----------------------------------------------------------------------

    #[test]
    fn test_sessions_rename_sets_and_clears_label() {
        let service = SysmlService::empty();
        let id = insert_fake_session(&service, execution::SessionKind::Simulation);

        // Initially null.
        assert!(service.sessions_info(&id, None).unwrap().unwrap().summary.label.is_none());

        // Set a label — reflected on the summary.
        service.sessions_rename(&id, "Rush hour scenario").unwrap();
        assert_eq!(
            service.sessions_info(&id, None).unwrap().unwrap().summary.label.as_deref(),
            Some("Rush hour scenario"),
        );

        // Empty string clears it.
        service.sessions_rename(&id, "").unwrap();
        assert!(service.sessions_info(&id, None).unwrap().unwrap().summary.label.is_none());
    }

    #[test]
    fn test_sessions_rename_missing_id_errors() {
        let service = SysmlService::empty();
        let err = service.sessions_rename("no-such-id", "x").unwrap_err();
        assert!(err.to_string().contains("no session"));
    }

    #[test]
    fn test_sessions_rename_survives_fork() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);
        service.sessions_rename(&parent_id, "parent run").unwrap();

        let child_summary = service.sessions_fork(&parent_id).unwrap();
        assert_eq!(
            child_summary.label.as_deref(),
            Some("parent run"),
            "fork should inherit the parent's label",
        );
    }

    #[test]
    fn test_sessions_subsystems_lists_named_subsystems() {
        let service = SysmlService::empty();
        // Uses the B3 helper — a Simulation session with one state-machine
        // subsystem named "SM".
        let id = insert_running_sim_session(&service);

        // Step once so snapshot state is populated (subsystems return
        // `current_state` from the latest snapshot if present).
        service.sessions_step(&id, None, None, None).unwrap();

        let subs = service.sessions_subsystems(&id).unwrap();
        assert_eq!(subs.len(), 1);
        assert_eq!(subs[0].name, "SM");
        assert_eq!(subs[0].kind_label, "stateMachine");
        assert_eq!(subs[0].current_state, "A");
    }

    #[test]
    fn test_sessions_subsystems_empty_for_bare_orchestrator() {
        let service = SysmlService::empty();
        let id = insert_fake_session(&service, execution::SessionKind::Orchestrator);
        let subs = service.sessions_subsystems(&id).unwrap();
        assert!(subs.is_empty());
    }

    #[test]
    fn test_sessions_subsystems_missing_id_errors() {
        let service = SysmlService::empty();
        let err = service.sessions_subsystems("no-such-id").unwrap_err();
        assert!(err.to_string().contains("no session"));
    }

    // -----------------------------------------------------------------------
    // B6 — transport/reconnect audit regression
    // -----------------------------------------------------------------------

    // -----------------------------------------------------------------------
    // B7 — sessions.diff
    // -----------------------------------------------------------------------

    #[test]
    fn test_sessions_diff_reports_empty_for_identical_sessions() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);
        service.sessions_step(&parent_id, None, None, None).unwrap();

        // Fork so both sides sit at the exact same tick/state.
        let child = service.sessions_fork(&parent_id).unwrap();

        let diff = service.sessions_diff(&parent_id, &child.id).unwrap();
        assert_eq!(diff.a_id, parent_id);
        assert_eq!(diff.b_id, child.id);
        assert_eq!(diff.current_tick_a, diff.current_tick_b);
        assert!(
            diff.subsystem_diffs.is_empty(),
            "fork + no steps should have zero subsystem diffs: {:?}",
            diff.subsystem_diffs,
        );
        assert!(
            diff.variable_diffs.is_empty(),
            "fork + no steps should have zero variable diffs: {:?}",
            diff.variable_diffs,
        );
    }

    #[test]
    fn test_sessions_diff_detects_divergent_subsystem_state() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);
        service.sessions_step(&parent_id, None, None, None).unwrap();

        let child_summary = service.sessions_fork(&parent_id).unwrap();
        let child_id = child_summary.id.clone();

        // Drive the child through the transition so it's in state B while
        // the parent stays in A.
        service.sessions_step(&child_id, Some("go"), None, None).unwrap();

        let diff = service.sessions_diff(&parent_id, &child_id).unwrap();
        assert_eq!(diff.subsystem_diffs.len(), 1);
        let sm_diff = &diff.subsystem_diffs[0];
        assert_eq!(sm_diff.name, "SM");
        assert_eq!(sm_diff.a_state.as_deref(), Some("A"));
        assert_eq!(sm_diff.b_state.as_deref(), Some("B"));

        // Transition assigned speed=100 on the child, which shows up as a
        // variable diff.
        let speed_diff = diff.variable_diffs.iter().find(|v| v.name == "speed");
        assert!(
            speed_diff.is_some(),
            "expected `speed` in variable_diffs; got: {:?}",
            diff.variable_diffs,
        );
        let speed = speed_diff.unwrap();
        assert_eq!(speed.a_value, None);
        assert_eq!(speed.b_value, Some(sysml_core::Value::Float(100.0)));
    }

    #[test]
    fn test_sessions_diff_ignores_bookkeeping_variables() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();

        let child = service.sessions_fork(&parent_id).unwrap();
        // After fork, step parent again so tick/t_ms diverge.
        service.sessions_step(&parent_id, None, None, None).unwrap();

        let diff = service.sessions_diff(&parent_id, &child.id).unwrap();
        // Tick-bookkeeping vars must be excluded from variable diffs.
        for var in &diff.variable_diffs {
            assert!(
                !sysml_runtime::expressions::is_internal_var(&var.name),
                "bookkeeping var {} leaked into diff",
                var.name,
            );
        }
        // The tick counts themselves ARE reported.
        assert!(
            diff.current_tick_a > diff.current_tick_b,
            "parent should have advanced past child: {} vs {}",
            diff.current_tick_a,
            diff.current_tick_b,
        );
    }

    #[test]
    fn test_sessions_diff_missing_id_errors() {
        let service = SysmlService::empty();
        let id = insert_running_sim_session(&service);
        let err = service.sessions_diff(&id, "no-such-id").unwrap_err();
        assert!(err.to_string().contains("no session"));
        let err = service.sessions_diff("no-such-id", &id).unwrap_err();
        assert!(err.to_string().contains("no session"));
    }

    // -----------------------------------------------------------------------
    // B8 — sessions.diff_timeline
    // -----------------------------------------------------------------------

    #[test]
    fn test_diff_timeline_returns_empty_for_no_history() {
        let service = SysmlService::empty();
        let a_id = insert_running_sim_session(&service);
        let b_id = insert_running_sim_session(&service);
        // Neither has been stepped — no history on either side.
        let diff = service.sessions_diff_timeline(&a_id, &b_id).unwrap();
        assert!(diff.shared_start_tick.is_none());
        assert!(diff.shared_end_tick.is_none());
        assert!(diff.first_divergence_tick.is_none());
        assert!(diff.tick_diffs.is_empty());
        assert!(!diff.history_truncated);
    }

    #[test]
    fn test_diff_timeline_fork_without_steps_reports_no_divergence() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);
        // Step parent a few times so the fork seed snapshot has a tick > 0.
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();

        let child = service.sessions_fork(&parent_id).unwrap();

        // Both sides sit at the same tick with the same seed snapshot;
        // the shared range is exactly that one tick and they match.
        let diff = service.sessions_diff_timeline(&parent_id, &child.id).unwrap();
        assert_eq!(diff.shared_start_tick, Some(diff.shared_end_tick.unwrap()));
        assert_eq!(diff.first_divergence_tick, None);
        assert!(diff.tick_diffs.is_empty());
    }

    #[test]
    fn test_diff_timeline_finds_first_divergence_after_fork() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);
        // Step parent twice so parent history = [tick 1, tick 2], both in A.
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();

        let child_summary = service.sessions_fork(&parent_id).unwrap();
        let child_id = child_summary.id.clone();

        // Both sides continue: parent steps 3 more times without firing
        // the transition (stays in A); child steps twice in A, then fires
        // "go" on step 3 (transitions to B at that tick, setting speed=100).
        service.sessions_step(&parent_id, None, None, None).unwrap(); // tick 3
        service.sessions_step(&parent_id, None, None, None).unwrap(); // tick 4
        service.sessions_step(&parent_id, None, None, None).unwrap(); // tick 5

        service.sessions_step(&child_id, None, None, None).unwrap();         // tick 3, A
        service.sessions_step(&child_id, None, None, None).unwrap();         // tick 4, A
        service.sessions_step(&child_id, Some("go"), None, None).unwrap();   // tick 5, B

        let diff = service.sessions_diff_timeline(&parent_id, &child_id).unwrap();
        // Shared range: parent has [1..=5], child was seeded with tick 2
        // and then stepped to [2, 3, 4, 5]. Intersection is [2..=5].
        assert_eq!(diff.shared_start_tick, Some(2));
        assert_eq!(diff.shared_end_tick, Some(5));
        // Ticks 2, 3, 4 all match (both in A with no `speed` set).
        // Tick 5 is the first divergence (parent A, child B + speed=100).
        assert_eq!(diff.first_divergence_tick, Some(5));
        assert_eq!(diff.tick_diffs.len(), 1);
        let td = &diff.tick_diffs[0];
        assert_eq!(td.tick, 5);
        assert_eq!(td.subsystem_diffs.len(), 1);
        assert_eq!(td.subsystem_diffs[0].name, "SM");
        assert_eq!(td.subsystem_diffs[0].a_state.as_deref(), Some("A"));
        assert_eq!(td.subsystem_diffs[0].b_state.as_deref(), Some("B"));
        // `speed` assignment on the child shows up as a variable diff.
        let speed = td.variable_diffs.iter().find(|v| v.name == "speed");
        assert!(speed.is_some());
        let speed = speed.unwrap();
        assert_eq!(speed.a_value, None);
        assert_eq!(speed.b_value, Some(sysml_core::Value::Float(100.0)));
    }

    #[test]
    fn test_diff_timeline_does_not_flag_fresh_fork_as_truncated() {
        // Regression guard: a fresh fork (child reseeded with only the
        // parent's latest snapshot) must NOT report history_truncated —
        // nothing was actually evicted, the child simply never had the
        // earlier ticks in the first place.
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();

        let child = service.sessions_fork(&parent_id).unwrap();

        // Parent keeps advancing; child sits at the fork point. Parent's
        // earliest retained tick is 1, child's is 3 (the fork seed), so
        // the shared range starts at 3. With fork_point_tick = 3, this
        // equals the ideal start → no truncation.
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();

        let diff = service
            .sessions_diff_timeline(&parent_id, &child.id)
            .unwrap();
        assert!(
            !diff.history_truncated,
            "fresh fork should NOT flag history as truncated; \
             nothing was actually evicted (shared_start = {:?})",
            diff.shared_start_tick,
        );
        assert_eq!(diff.shared_start_tick, Some(3));
    }

    #[test]
    fn test_diff_timeline_excludes_bookkeeping_vars() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);
        service.sessions_step(&parent_id, None, None, None).unwrap();
        let child = service.sessions_fork(&parent_id).unwrap();

        // Step both sides a different number of ticks — their t_ms and
        // tick will differ at their latest snapshots, but diff_timeline
        // walks the shared range (seed tick only) and must not surface
        // bookkeeping vars even within TickDiff entries. We step parent
        // extra just to drive the asymmetry.
        service.sessions_step(&parent_id, None, None, None).unwrap();

        let diff = service.sessions_diff_timeline(&parent_id, &child.id).unwrap();
        for td in &diff.tick_diffs {
            for var in &td.variable_diffs {
                assert!(
                    !sysml_runtime::expressions::is_internal_var(&var.name),
                    "bookkeeping var {} leaked into tick_diffs",
                    var.name,
                );
            }
        }
    }

    #[test]
    fn test_diff_timeline_missing_id_errors() {
        let service = SysmlService::empty();
        let id = insert_running_sim_session(&service);
        let err = service
            .sessions_diff_timeline(&id, "no-such-id")
            .unwrap_err();
        assert!(err.to_string().contains("no session"));
        let err = service
            .sessions_diff_timeline("no-such-id", &id)
            .unwrap_err();
        assert!(err.to_string().contains("no session"));
    }

    /// End-to-end session workflow: create → step → rename → fork →
    /// rename child → step both → diff → diff_timeline → reset → stop.
    ///
    /// This chains every session command the UX compare mode depends on
    /// and pins down the invariants at each step. Serves as executable
    /// documentation alongside the session backend contract doc.
    #[test]
    fn test_session_lifecycle_end_to_end() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);

        // 1. Initial summary reflects a fresh session.
        let initial = service.sessions_info(&parent_id, None).unwrap().unwrap();
        assert_eq!(initial.summary.tick, 0);
        assert!(initial.summary.label.is_none());
        assert!(initial.summary.fork_point_tick.is_none());

        // 2. Step a few times and verify tick + history grow.
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();
        let parent_after_steps = service.sessions_info(&parent_id, None).unwrap().unwrap();
        assert_eq!(parent_after_steps.summary.tick, 2);
        assert_eq!(parent_after_steps.summary.history_len, 2);

        // 3. Rename the parent; the label shows up on the summary.
        service.sessions_rename(&parent_id, "baseline run").unwrap();
        let named = service.sessions_info(&parent_id, None).unwrap().unwrap();
        assert_eq!(named.summary.label.as_deref(), Some("baseline run"));

        // 4. Fork — child inherits label + carries a fork_point_tick.
        let child = service.sessions_fork(&parent_id).unwrap();
        assert_eq!(child.label.as_deref(), Some("baseline run"));
        assert_eq!(child.fork_point_tick, Some(2));
        assert_ne!(child.id, parent_id);
        let child_id = child.id.clone();

        // 5. Rename the child; the parent's label stays put.
        service.sessions_rename(&child_id, "override run").unwrap();
        let parent_after_child_rename =
            service.sessions_info(&parent_id, None).unwrap().unwrap();
        assert_eq!(
            parent_after_child_rename.summary.label.as_deref(),
            Some("baseline run"),
            "parent label must not be affected by child rename"
        );

        // 6. Step both divergently: parent stays in A, child fires "go".
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&child_id, Some("go"), None, None).unwrap();
        let parent = service.sessions_info(&parent_id, None).unwrap().unwrap();
        let child_info = service.sessions_info(&child_id, None).unwrap().unwrap();
        assert_eq!(parent.summary.current_state.as_deref(), Some("A"));
        assert_eq!(child_info.summary.current_state.as_deref(), Some("B"));

        // 7. Latest-snapshot diff shows the split.
        let diff = service.sessions_diff(&parent_id, &child_id).unwrap();
        assert_eq!(diff.subsystem_diffs.len(), 1);
        assert_eq!(diff.subsystem_diffs[0].a_state.as_deref(), Some("A"));
        assert_eq!(diff.subsystem_diffs[0].b_state.as_deref(), Some("B"));

        // 8. Timeline diff reports a defined shared range anchored to
        //    the fork point, with no truncation.
        let timeline = service
            .sessions_diff_timeline(&parent_id, &child_id)
            .unwrap();
        assert!(!timeline.history_truncated);
        assert_eq!(timeline.shared_start_tick, Some(2));
        assert!(timeline.first_divergence_tick.is_some());

        // 9. Reset the child — clears fork_point, tick, and label stays.
        //    (Label is user data, reset only clears execution state.)
        let child_after_reset = service.sessions_reset(&child_id).unwrap();
        assert_eq!(child_after_reset.tick, 0);
        assert_eq!(child_after_reset.fork_point_tick, None);
        assert_eq!(child_after_reset.label.as_deref(), Some("override run"));

        // 10. Stop both via the kind-agnostic command.
        service.sessions_stop(&parent_id).unwrap();
        service.sessions_stop(&child_id).unwrap();
        assert!(service.sessions_list().unwrap().is_empty());
    }

    /// `sessions.diff` must report subsystem diffs even when one side has
    /// no recorded history. Regression guard for the fallback-path
    /// silently dropping subsystem_diffs.
    #[test]
    fn test_sessions_diff_fallback_reports_subsystem_diffs_when_one_side_fresh() {
        let service = SysmlService::empty();
        let a_id = insert_running_sim_session(&service);
        let b_id = insert_running_sim_session(&service);

        // Only side A gets stepped. Side B has no history.
        service.sessions_step(&a_id, None, None, None).unwrap();

        let diff = service.sessions_diff(&a_id, &b_id).unwrap();
        // Fresh session B has subsystem "SM" at its initial state "A",
        // same as side A after one no-op step, so subsystem_diffs should
        // be empty for matching state but the subsystem list should have
        // been considered. Force a real state divergence: step A through
        // the transition.
        service.sessions_step(&a_id, Some("go"), None, None).unwrap();
        let diff = service.sessions_diff(&a_id, &b_id).unwrap();
        assert_eq!(
            diff.subsystem_diffs.len(),
            1,
            "stepped A should differ from fresh B on subsystem state"
        );
        assert_eq!(diff.subsystem_diffs[0].a_state.as_deref(), Some("B"));
        assert_eq!(diff.subsystem_diffs[0].b_state.as_deref(), Some("A"));
    }

    /// `sessions.rename` on the parent after a fork must not bleed into
    /// the child's label — the fork copied the label by value.
    #[test]
    fn test_sessions_rename_parent_after_fork_does_not_leak_into_child() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_rename(&parent_id, "before fork").unwrap();

        let child = service.sessions_fork(&parent_id).unwrap();
        assert_eq!(child.label.as_deref(), Some("before fork"));

        // Now rename the parent post-fork.
        service.sessions_rename(&parent_id, "after fork").unwrap();
        let child_info = service.sessions_info(&child.id, None).unwrap().unwrap();
        assert_eq!(
            child_info.summary.label.as_deref(),
            Some("before fork"),
            "child label must not follow parent's post-fork rename"
        );
    }

    /// Three consecutive divergent ticks: `diff_timeline` should emit
    /// three `TickDiff` entries in tick order, with `first_divergence_tick`
    /// set to the earliest.
    #[test]
    fn test_diff_timeline_multiple_divergent_ticks_are_ordered() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);
        service.sessions_step(&parent_id, None, None, None).unwrap();

        let child = service.sessions_fork(&parent_id).unwrap();
        let child_id = child.id.clone();

        // Parent steps 3x in A. Child fires "go" immediately (tick 2, B)
        // and then steps twice more while in B. State B differs for 3
        // consecutive ticks.
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();
        service.sessions_step(&parent_id, None, None, None).unwrap();

        service.sessions_step(&child_id, Some("go"), None, None).unwrap();
        service.sessions_step(&child_id, None, None, None).unwrap();
        service.sessions_step(&child_id, None, None, None).unwrap();

        let diff = service
            .sessions_diff_timeline(&parent_id, &child_id)
            .unwrap();
        assert_eq!(diff.tick_diffs.len(), 3);
        // Monotonic tick order.
        assert!(diff.tick_diffs[0].tick < diff.tick_diffs[1].tick);
        assert!(diff.tick_diffs[1].tick < diff.tick_diffs[2].tick);
        assert_eq!(
            diff.first_divergence_tick,
            Some(diff.tick_diffs[0].tick),
            "first_divergence_tick must be the earliest tick_diff entry"
        );
    }

    /// `sessions.fork_with_overrides` parses numeric strings into typed
    /// values: whole-number strings land as `Int`, decimal strings as
    /// `Float`. This test locks down the parsing behavior the UX can
    /// assume when launching parameter sweeps.
    #[test]
    fn test_sessions_fork_with_overrides_parses_typed_numeric_values() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);

        // RSC-2.5 (RS002): override names must resolve to a slot alias or an
        // existing context variable — silent creation is gone. Seed the
        // targets on the parent; this test pins value-PARSING semantics,
        // not name creation.
        if let Some(mut entry) = service.sessions.get_mut(&ElementId::from_string(&parent_id)) {
            entry.orchestrator.context.set("count", sysml_core::Value::Int(0));
            entry.orchestrator.context.set("pi", sysml_core::Value::Float(0.0));
        }

        let overrides = vec![
            ("count".to_owned(), "42".to_owned()),
            ("pi".to_owned(), "3.14".to_owned()),
        ];
        let child_summary = service
            .sessions_fork_with_overrides(&parent_id, &overrides, None)
            .unwrap();
        let child_entry = service.sessions.get(&ElementId::from_string(&child_summary.id)).unwrap();
        assert_eq!(
            child_entry.orchestrator.context.get("count"),
            Some(&sysml_core::Value::Int(42)),
            "whole-number override should parse as Int"
        );
        assert_eq!(
            child_entry.orchestrator.context.get("pi"),
            Some(&sysml_core::Value::Float(3.14)),
            "decimal override should parse as Float"
        );
    }

    /// Verifies the rich Monte Carlo response shape produced by
    /// `sysml.montecarlo`: constraint pass rates, per-parameter
    /// statistics + histograms, and the discovery arrays used by the
    /// simulation app's configuration UI.
    #[test]
    fn test_montecarlo_rich_shape() {
        let service = SysmlService::empty();
        let src = "\
package P {\n\
    attribute x : Real = 50.0;\n\
    constraint c { x > 25.0 }\n\
}\n";
        service.load_source("mc-test.sysml", src).unwrap();

        let config = serde_json::json!({
            "iterations": 200,
            "seed": 7,
            "parameters": [
                {"name": "x", "distribution": "uniform", "min": 0.0, "max": 100.0}
            ]
        });

        let result = service.montecarlo(&config).unwrap();

        // Top-level fields must all be present.
        assert_eq!(result.get("iterations").and_then(|v| v.as_u64()), Some(200));
        assert_eq!(result.get("seed").and_then(|v| v.as_u64()), Some(7));
        assert!(result.get("constraint_pass_rates").unwrap().is_array());
        assert!(result.get("parameter_statistics").unwrap().is_object());
        assert!(result.get("parameter_histograms").unwrap().is_object());
        assert!(result.get("discovered_parameters").unwrap().is_array());
        assert!(result.get("discovered_constraints").unwrap().is_array());

        // The constraint should be discovered and evaluated.
        let cprs = result["constraint_pass_rates"].as_array().unwrap();
        assert!(!cprs.is_empty(), "expected at least one constraint pass rate");
        let cpr = &cprs[0];
        for field in ["name", "expression", "pass_rate", "pass_count", "fail_count", "inconclusive_count"] {
            assert!(cpr.get(field).is_some(), "missing {field} on constraint_pass_rates entry");
        }

        // x was sampled: stats + histogram present with required keys.
        let x_stats = result["parameter_statistics"].get("x")
            .expect("x statistics missing");
        for field in ["mean", "std_dev", "min", "max", "p5", "p50", "p95"] {
            assert!(x_stats.get(field).is_some(), "missing stats field {field}");
        }
        let mean = x_stats["mean"].as_f64().unwrap();
        // Uniform[0,100] expected mean ≈ 50; allow generous slack for 200 iters.
        assert!(mean > 30.0 && mean < 70.0, "uniform mean out of range: {mean}");

        let x_hist = result["parameter_histograms"].get("x")
            .expect("x histogram missing");
        assert!(x_hist["bin_edges"].as_array().unwrap().len() > 1);
        assert!(x_hist["counts"].as_array().unwrap().len() >= 1);
        assert!(x_hist["max_count"].as_u64().unwrap() > 0);
    }

    /// RSC-6.3 (L9c): Monte Carlo must not bypass salsa. Once the model's
    /// caches are warm, a second identical `sysml.montecarlo` run evaluates
    /// against memoized queries only — the constraint set (the
    /// `extract_and_precompile` walk, now behind
    /// `workspace_precompiled_constraints`) and the eval-context seed are
    /// cache hits, so the run triggers **zero** tracked-query executions.
    /// This pins the bypass elimination: the old direct
    /// `extract_and_precompile` + hand-rolled context walk re-ran every call.
    #[test]
    fn test_montecarlo_warm_cache_no_salsa_executions() {
        let service = SysmlService::empty();
        let src = "\
package P {\n\
    attribute x : Real = 50.0;\n\
    constraint c { x > 25.0 }\n\
}\n";
        service.load_source("mc-warm.sysml", src).unwrap();

        let config = serde_json::json!({
            "iterations": 50,
            "seed": 7,
            "parameters": [
                {"name": "x", "distribution": "uniform", "min": 0.0, "max": 100.0}
            ]
        });

        // First run warms every salsa query Monte Carlo touches: the workspace
        // graph, the precompiled constraint set, and the eval-context seed.
        let _ = service.montecarlo(&config).unwrap();

        // Reset stats, then run again against the unchanged model.
        service.salsa_stats_reset().unwrap();
        let _ = service.montecarlo(&config).unwrap();

        let stats = service.salsa_stats().unwrap();
        assert_eq!(
            stats.executions, 0,
            "warm-cache montecarlo must trigger zero tracked-query executions \
             (extract_and_precompile + eval-context seed are salsa-cached); \
             got {} executions, {} validations",
            stats.executions, stats.validations
        );
    }

    /// `sessions.fork_with_overrides` falls back to storing the string
    /// literally when the value is not parseable as a number. Locks down
    /// the string-fallback path in `apply_overrides`.
    #[test]
    fn test_sessions_fork_with_overrides_string_fallback() {
        let service = SysmlService::empty();
        let parent_id = insert_running_sim_session(&service);

        // RSC-2.5 (RS002): seed the target — overrides no longer create
        // unknown names; this test pins the string-fallback PARSING path.
        if let Some(mut entry) = service.sessions.get_mut(&ElementId::from_string(&parent_id)) {
            entry
                .orchestrator
                .context
                .set("mode", sysml_core::Value::String("auto".to_owned()));
        }

        let overrides = vec![("mode".to_owned(), "manual".to_owned())];
        let child_summary = service
            .sessions_fork_with_overrides(&parent_id, &overrides, None)
            .unwrap();
        let child_entry = service.sessions.get(&ElementId::from_string(&child_summary.id)).unwrap();
        match child_entry.orchestrator.context.get("mode") {
            Some(sysml_core::Value::String(s)) => assert_eq!(s, "manual"),
            Some(sysml_core::Value::Bool(_)) => {
                // Some parsers recognize certain tokens; accept this too.
            }
            other => panic!(
                "non-numeric override should parse as String (or recognized \
                 literal); got {:?}",
                other
            ),
        }
    }

    // -----------------------------------------------------------------------
    // R5.0 — Batch sessions
    // -----------------------------------------------------------------------

    /// Build a service with a trivial state machine loaded, ready for
    /// batch children to target. Returns `(service, uri, sm_name)`.
    fn batch_fixture_v2() -> (SysmlService, String, &'static str) {
        let service = SysmlService::empty();
        let src = r#"
            package Bench {
                attribute mass = 1.0;
                attribute tolerance = 0.1;
                state def BenchSM {
                    entry; then S1;
                    state S1;
                }
            }
        "#;
        service.load_source("bench.sysml", src).unwrap();
        (service, "bench.sysml".to_owned(), "BenchSM")
    }

    #[test]
    fn test_batch_create_spawns_children_and_registers_batch() {
        let (service, uri, sm_name) = batch_fixture_v2();
        let result = service
            .batch_create(
                "sweep",
                &uri,
                Some(sm_name),
                r#"[{"mass": 1.0}, {"mass": 2.0}, {"mass": 3.0}]"#,
                Some("mass sweep"),
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.child_session_ids.len(), 3);
        assert!(!result.batch_id.is_empty());

        // Each child session is individually addressable through
        // `sessions.info`.
        for sid in &result.child_session_ids {
            assert!(service.sessions_info(sid, None).unwrap().is_some());
        }

        // The parent batch is in the registry.
        assert!(service.batches.contains_key(&result.batch_id));
        let status = service.batch_status(&result.batch_id).unwrap();
        assert_eq!(status.batch.children.len(), 3);
        assert_eq!(status.batch.kind, batch::BatchKind::Sweep);
        assert_eq!(status.batch.label.as_deref(), Some("mass sweep"));
        // No child has stopped yet → all Pending → batch Pending.
        assert_eq!(status.batch.status, batch::BatchStatus::Pending);
    }

    #[test]
    fn test_batch_create_rejects_unknown_kind() {
        let (service, uri, sm_name) = batch_fixture_v2();
        let err = service
            .batch_create("bogus", &uri, Some(sm_name), "[]", None, None, None, None)
            .unwrap_err();
        assert!(err.to_string().contains("unknown batch kind"), "{err}");
    }

    #[test]
    fn test_batch_create_enforces_max_children_cap() {
        let (service, uri, sm_name) = batch_fixture_v2();
        // Build a JSON array with MAX_CHILDREN_PER_BATCH + 1 empty objects.
        let over_cap = batch::MAX_CHILDREN_PER_BATCH + 1;
        let params: Vec<serde_json::Value> = (0..over_cap)
            .map(|_| serde_json::json!({}))
            .collect();
        let params = serde_json::to_string(&params).unwrap();
        let err = service
            .batch_create("sweep", &uri, Some(sm_name), &params, None, None, None, None)
            .unwrap_err();
        assert!(
            err.to_string().contains("MAX_CHILDREN_PER_BATCH"),
            "{err}"
        );
    }

    #[test]
    fn test_batch_status_progresses_and_marks_children_on_stop() {
        let (service, uri, sm_name) = batch_fixture_v2();
        let created = service
            .batch_create(
                "monte_carlo",
                &uri,
                Some(sm_name),
                r#"[{}, {}, {}]"#,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let batch_id = created.batch_id.clone();
        assert_eq!(
            service.batch_status(&batch_id).unwrap().batch.status,
            batch::BatchStatus::Pending
        );

        // Stop child 0 — the batch should transition to Running with
        // counts { running: 0, completed: 1 }.
        service
            .sessions_stop(&created.child_session_ids[0])
            .unwrap();
        match service.batch_status(&batch_id).unwrap().batch.status {
            batch::BatchStatus::Running {
                running,
                completed,
            } => {
                assert_eq!(running, 0);
                assert_eq!(completed, 1);
            }
            other => panic!("expected Running, got {other:?}"),
        }

        // Stop children 1 and 2 — batch becomes Complete.
        service
            .sessions_stop(&created.child_session_ids[1])
            .unwrap();
        service
            .sessions_stop(&created.child_session_ids[2])
            .unwrap();
        assert_eq!(
            service.batch_status(&batch_id).unwrap().batch.status,
            batch::BatchStatus::Complete
        );
        // Every child descriptor should be Complete.
        for child in service.batch_status(&batch_id).unwrap().batch.children {
            assert_eq!(child.status, batch::ChildStatus::Complete);
        }
    }

    #[test]
    fn test_batch_results_respects_include_verdicts_flag() {
        let (service, uri, sm_name) = batch_fixture_v2();
        let created = service
            .batch_create(
                "sweep",
                &uri,
                Some(sm_name),
                r#"[{"mass": 1.0}]"#,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        // Seed a verdict against the child so completion copies it.
        let child_id = &created.child_session_ids[0];
        let verdict = ArchivedVerdict::trajectory("CaseX", "pass", 0, None, None);
        // Pre-populate the archive directly with a record containing the
        // verdict so `mark_batch_child_complete` picks it up.
        let archived = ArchivedSession {
            id: child_id.clone(),
            label: None,
            origin: SessionOrigin::Sweep,
            workspace_uri: uri.clone(),
            created_at: 0,
            ended_at: 0,
            ticks: 0,
            overrides: Vec::new(),
            verdicts: vec![verdict.clone()],
            snapshots: Vec::new(),
            snapshot_value_units: None,
            golden: None,
            provenance: None,
        };
        service.archive.record(archived).unwrap();
        // Stopping the child triggers mark_batch_child_complete which
        // reads the archive and copies verdicts onto the descriptor.
        service.sessions_stop(child_id).unwrap();

        // include_verdicts = false → verdicts cleared
        let bare = service.batch_results(&created.batch_id, false).unwrap();
        assert!(bare.children[0].verdicts.is_empty());

        // include_verdicts = true → verdicts preserved
        let full = service.batch_results(&created.batch_id, true).unwrap();
        assert_eq!(full.children[0].verdicts.len(), 1);
        assert_eq!(full.children[0].verdicts[0].verdict, "pass");
    }

    #[test]
    fn test_batch_slice_filters_and_combines_clauses() {
        let (service, uri, sm_name) = batch_fixture_v2();
        let created = service
            .batch_create(
                "trade_study",
                &uri,
                Some(sm_name),
                r#"[{"mass": 1.0}, {"mass": 2.0}, {"mass": 3.0}]"#,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        // All pending → slice by only_status=pending matches all three.
        let pending = service
            .batch_slice(
                &created.batch_id,
                batch::BatchFilter {
                    only_status: Some(batch::ChildStatusKind::Pending),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(pending.children.len(), 3);

        // Stop child 1 (mass=2.0) so it becomes Complete.
        service
            .sessions_stop(&created.child_session_ids[1])
            .unwrap();

        // only_status=complete AND param_predicate mass >= 2.0 → child 1.
        let subset = service
            .batch_slice(
                &created.batch_id,
                batch::BatchFilter {
                    only_status: Some(batch::ChildStatusKind::Complete),
                    only_verdict: None,
                    param_predicate: Some(batch::ParamPredicate {
                        param: "mass".into(),
                        op: batch::CompareOp::Ge,
                        value: 2.0,
                    }),
                },
            )
            .unwrap();
        assert_eq!(subset.children.len(), 1);
        assert_eq!(subset.children[0].index, 1);
    }

    #[test]
    fn test_batch_child_archive_origin_matches_batch_kind() {
        let (service, uri, sm_name) = batch_fixture_v2();
        let created = service
            .batch_create(
                "sweep",
                &uri,
                Some(sm_name),
                r#"[{"mass": 1.0}]"#,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let child_id = created.child_session_ids[0].clone();
        service.sessions_stop(&child_id).unwrap();
        let archived = service.archive.get(&child_id).unwrap();
        assert_eq!(archived.origin, SessionOrigin::Sweep);
    }

    #[test]
    fn test_batch_missing_id_errors_cleanly() {
        let service = SysmlService::empty();
        let err = service.batch_status("no-such").unwrap_err();
        assert!(err.to_string().contains("no batch"), "{err}");
        let err = service.batch_results("no-such", false).unwrap_err();
        assert!(err.to_string().contains("no batch"), "{err}");
        let err = service
            .batch_slice("no-such", batch::BatchFilter::default())
            .unwrap_err();
        assert!(err.to_string().contains("no batch"), "{err}");
    }

    // -- Sensitivity workflow (R7.4) --

    #[test]
    fn test_batch_create_accepts_sensitivity_kind() {
        let (service, uri, sm_name) = batch_fixture_v2();
        let result = service
            .batch_create(
                "sensitivity",
                &uri,
                Some(sm_name),
                r#"[{"mass": 1.0}, {"mass": 2.0}]"#,
                Some("morris"),
                None,
                None,
                None,
            )
            .unwrap();
        assert_eq!(result.child_session_ids.len(), 2);
        let status = service.batch_status(&result.batch_id).unwrap();
        assert_eq!(status.batch.kind, batch::BatchKind::Sensitivity);
    }

    #[test]
    fn test_sensitivity_analyze_morris_on_linear_model() {
        // We build a synthetic batch whose children carry BOTH the swept
        // params AND a `y` key computed from them, so
        // `extract_child_metric("y", ...)` can recover an output metric
        // without running a model. This mirrors how an Ishigami-style
        // client-side harness works.
        let (service, uri, sm_name) = batch_fixture_v2();

        let params = vec![
            sensitivity::ParamRange { name: "a".into(), min: 0.0, max: 1.0 },
            sensitivity::ParamRange { name: "b".into(), min: 0.0, max: 1.0 },
        ];
        let p = 4;
        let rows = sensitivity::morris_trajectories(&params, 5, p, 42);

        // Inject y as a param on each child so the extractor picks it
        // up: y = 3·a + 1·b.
        let mut children_params: Vec<serde_json::Value> = Vec::with_capacity(rows.len());
        for row in &rows {
            let y = 3.0 * row[0] + 1.0 * row[1];
            children_params.push(serde_json::json!({
                "a": row[0],
                "b": row[1],
                "y": y,
            }));
        }
        let children_params_str = serde_json::to_string(&children_params).unwrap();

        let batch = service
            .batch_create(
                "sensitivity",
                &uri,
                Some(sm_name),
                &children_params_str,
                None,
                None,
                None,
                None,
            )
            .unwrap();

        let params_json = serde_json::to_string(&params).unwrap();
        let result = service
            .sensitivity_analyze(
                &batch.batch_id,
                "morris",
                &params_json,
                "y",
                Some(p),
            )
            .unwrap();

        assert_eq!(result.method, sensitivity::SensitivityMethod::Morris);
        assert_eq!(result.parameters.len(), 2);
        assert_eq!(result.parameters[0].name, "a");
        assert_eq!(result.parameters[1].name, "b");

        // μ* should be |slope|: 3.0 for a, 1.0 for b (linear model).
        let mu_a = result.parameters[0].mu.unwrap();
        let mu_b = result.parameters[1].mu.unwrap();
        assert!((mu_a - 3.0).abs() < 1e-6, "μ*(a) = {mu_a}");
        assert!((mu_b - 1.0).abs() < 1e-6, "μ*(b) = {mu_b}");
    }

    #[test]
    fn test_sensitivity_analyze_rejects_unknown_method() {
        let (service, uri, sm_name) = batch_fixture_v2();
        let batch = service
            .batch_create(
                "sensitivity",
                &uri,
                Some(sm_name),
                r#"[{"a":0.1}]"#,
                None,
                None,
                None,
                None,
            )
            .unwrap();
        let err = service
            .sensitivity_analyze(
                &batch.batch_id,
                "garbage",
                r#"[{"name":"a","min":0,"max":1}]"#,
                "y",
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("unknown sensitivity method"), "{err}");
    }

    #[test]
    fn test_sensitivity_analyze_missing_batch_errors_cleanly() {
        let service = SysmlService::empty();
        let err = service
            .sensitivity_analyze(
                "no-such-batch",
                "morris",
                r#"[{"name":"a","min":0,"max":1}]"#,
                "y",
                None,
            )
            .unwrap_err();
        assert!(err.to_string().contains("no batch"), "{err}");
    }

    // ── RSC-3.5c.2a behavioral baseline ────────────────────────────────────
    // A minimal two-flow model (both MessageChannel-classified) with an inject.
    // Asserts that trace_sequence produces the expected lifelines + messages so
    // the migration from FlowRouter → ExchangePlane is parity-verified.
    //
    // Port defs carry `out item` / `in item` payload features so classify_links
    // infers MessageChannel (item-typed endpoint → D-3.0.1 "MessageChannel").
    // No PowerBond links exist, so the PowerBond delivery-gate is never reached
    // and results are identical to the FlowRouter path.
    const TRACE_BASELINE_MODEL: &str = r#"
        package TraceTest {
            item def Cmd { attribute value : Integer; }
            port def SrcPort { out item cmd : Cmd; }
            port def SinkPort { in item cmd : Cmd; }
            part def Source { port out1 : SrcPort; }
            part def Sink   { port in1  : SinkPort; }
            part def System {
                part src  : Source;
                part sinkA : Sink;
                part sinkB : Sink;
                flow from src.out1 to sinkA.in1;
                flow from src.out1 to sinkB.in1;
                connect src.out1 to sinkA.in1;
                connect src.out1 to sinkB.in1;
            }
        }
    "#;

    #[test]
    fn test_trace_sequence_delivery_baseline_exchangeplane() {
        // RSC-3.5c.2a — ExchangePlane path (the migrated implementation).
        // Two MessageChannel flows from src.out1 → sinkA.in1 and sinkB.in1.
        // Inject once; expect both sinks to receive the payload.
        //
        // SequenceTraceBuilder splits "owner.port" on '.' and uses the owner
        // part as the lifeline name, so lifelines are "src", "sinkA", "sinkB".
        let service = SysmlService::empty();
        service
            .load_source("trace-test.sysml", TRACE_BASELINE_MODEL)
            .unwrap();

        let result = service
            .trace_sequence(
                "trace-test.sysml",
                &[("src.out1".to_owned(), "42".to_owned())],
            )
            .unwrap();

        // Three distinct participant names must appear as lifelines.
        let lifeline_names: Vec<&str> = result.lifelines.iter().map(|ll| ll.name.as_str()).collect();
        assert!(
            lifeline_names.contains(&"src"),
            "expected 'src' lifeline, got {lifeline_names:?}"
        );
        assert!(
            lifeline_names.contains(&"sinkA"),
            "expected 'sinkA' lifeline, got {lifeline_names:?}"
        );
        assert!(
            lifeline_names.contains(&"sinkB"),
            "expected 'sinkB' lifeline, got {lifeline_names:?}"
        );

        // Two messages delivered (one per flow).
        assert_eq!(
            result.messages.len(),
            2,
            "expected 2 delivered messages, got {:?}",
            result.messages
        );

        // Both messages originate from src.
        for msg in &result.messages {
            assert_eq!(
                msg.from, "src",
                "message source must be 'src', got {:?}",
                msg
            );
        }
    }

    #[test]
    fn test_flow_inspect_delivery_baseline_exchangeplane() {
        // RSC-3.5c.2a — ExchangePlane path for flow_inspect injection.
        // Inject on src.out1; expect two deliveries (to sinkA.in1 + sinkB.in1).
        let service = SysmlService::empty();
        service
            .load_source("trace-test.sysml", TRACE_BASELINE_MODEL)
            .unwrap();

        let result = service
            .flow_inspect(
                "trace-test.sysml",
                Some("src.out1"),
                Some("99"),
            )
            .unwrap();

        assert_eq!(
            result.delivery.len(),
            2,
            "expected 2 deliveries from flow_inspect inject, got {:?}",
            result.delivery
        );
        for d in &result.delivery {
            assert_eq!(
                d.source, "src.out1",
                "delivery source must be src.out1, got {:?}",
                d
            );
        }
    }
}
