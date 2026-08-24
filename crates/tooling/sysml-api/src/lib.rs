//! # sysml-api
//!
//! REST API server for SysML v2 models.
//!
//! This crate provides an HTTP API backed by [`SysmlService`] for model
//! storage, retrieval, querying, and visualization.
//!
//! ## Endpoints
//!
//! ### Store endpoints (backward-compatible)
//! - `GET  /health` - Health check
//! - `GET  /projects` - List all projects
//! - `GET  /projects/:id/commits` - List commits for a project
//! - `GET  /projects/:id/commits/:commit/model` - Get model snapshot
//! - `POST /projects/:id/commits/:commit/model` - Store model snapshot (auth-gated)
//!
//! ### Source / query endpoints
//! - `POST /sources` - Load SysML source text
//! - `POST /api/query` - Run structured element-list query
//! - `GET  /models/:uri/find?pattern=...&kind=...` - Find elements
//! - `GET  /models/:uri/elements/:id` - Get element by ID
//! - `GET  /models/:uri/elements/:id/children` - Get element children
//! - `GET  /models/:uri/stats` - Model statistics
//! - `GET  /models/:uri/tree` - Model tree
//! - `GET  /models/:uri/unverified` - Unverified requirements
//!
//! ### Navigation endpoints
//! - `GET  /models/:uri/elements/:id/ancestors` - Element ancestors
//! - `GET  /models/:uri/elements/:id/descendants` - Element descendants
//! - `GET  /models/:uri/trace?source_kind=...&target_kind=...&relation_kind=...` - Trace matrix
//!
//! ### Analysis endpoints
//! - `GET  /models/:uri/diagnostics` - Model diagnostics
//! - `POST /eval` - Evaluate expression (auth-gated)
//! - `GET  /models` - List loaded model URIs
//!
//! ### Constraint checking
//! - `POST /models/:uri/check` - Run constraint checks (auth-gated)
//!
//! ### File loading
//! - `POST /files` - Load file from disk path (auth-gated)
//!
//! ### Simulation sessions (auth-gated)
//! - `POST   /sessions/simulate/start` - Start simulation session
//! - `POST   /sessions/:key/step` - Step simulation
//! - `DELETE /sessions/:key` - Stop simulation
//!
//! ### Action sessions (auth-gated)
//! - `POST /sessions/action/start` - Start action session
//! - `POST /sessions/action/:key/step` - Step action
//!
//! ### Continuous simulation / orchestrator sessions (auth-gated)
//! - (retired) `POST /sessions/continuous/start` — model-bypassing; use
//!   `sysml.simulate.continuous.auto` or `sysml.sessions.create`
//! - `DELETE /sessions/orchestrator/:key` - Stop orchestrator session
//!
//! Orchestrator sessions step through the unified `POST /sessions/:key/step`
//! route — the request body's `overrides` field carries context overrides
//! for any session kind.
//!
//! ### Visualization endpoints
//! - `GET  /models/:uri/export/json` - Canonical JSON export
//! - `GET  /models/:uri/views` - List declared views (ViewUsage / ViewDefinition)
//! - `GET  /models/:uri/views/:view_id/render` - Render a declared view as a ViewModel
//! - `GET  /models/:uri/views/by_viewpoint/:viewpoint_id` - Views satisfying a viewpoint
//! - `GET  /models/:uri/viewpoints/by_stakeholder/:stakeholder_id` - Viewpoints with given stakeholder
//! - `POST /views/scratch` - Build a scratch view-def snippet exposing the given elements
//!
//! ### Generic command dispatch (auth-gated)
//! - `POST /api/command` - Execute any registered service command by name
//!
//! ### Meta endpoints
//! - `GET  /commands` - Command catalog

mod lsp_ws;
mod progress_sse;
mod session_ws;

use std::collections::HashSet;
use std::sync::Arc;

use axum::{
    extract::{DefaultBodyLimit, Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::IntoResponse,
    routing::{delete, get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};

use sysml_core::json::{from_json_str, to_json_string};
use sysml_id::{CommitId, ElementId, ProjectId};
use sysml_service::readiness::Readiness;
use sysml_service::SysmlService;
use sysml_store::{InMemoryStore, SnapshotMeta};

/// Application state backed by [`SysmlService`].
pub struct AppState {
    pub service: Arc<SysmlService>,
    /// Background task that periodically reaps expired sessions.
    ///
    /// Spawned at construction (when called inside a tokio runtime), aborted
    /// when the `AppState` is dropped. Held as an opaque guard so callers
    /// don't need to manage it. No-op when constructed outside a runtime
    /// (e.g., in sync tests).
    _session_reaper: SessionReaperGuard,
}

impl AppState {
    /// Create new application state with an empty service (with an in-memory store).
    pub fn new() -> Self {
        let store = Arc::new(std::sync::RwLock::new(InMemoryStore::new()));
        let service = Arc::new(SysmlService::with_store(store));
        let _session_reaper = spawn_session_reaper(&service);
        AppState {
            service,
            _session_reaper,
        }
    }

    /// Create application state wrapping an existing service.
    pub fn with_service(service: Arc<SysmlService>) -> Self {
        let _session_reaper = spawn_session_reaper(&service);
        AppState {
            service,
            _session_reaper,
        }
    }
}

/// Abort guard for the session-reaper background task.
///
/// Dropping the guard aborts the task. `None` when no tokio runtime is
/// active at construction — in that case no task was spawned. The
/// reaper itself lives in `sysml_service::session_reaper` (see S2.T15);
/// this guard exists only to tie its lifetime to `AppState`.
struct SessionReaperGuard(Option<tokio::task::AbortHandle>);

impl Drop for SessionReaperGuard {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

fn spawn_session_reaper(service: &Arc<SysmlService>) -> SessionReaperGuard {
    SessionReaperGuard(sysml_service::session_reaper::spawn_session_reaper(service))
}

impl Default for AppState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Response / request types ────────────────────────────────────────────

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

/// Project list response.
#[derive(Debug, Serialize)]
pub struct ProjectsResponse {
    pub projects: Vec<String>,
}

/// Commits list response.
#[derive(Debug, Serialize)]
pub struct CommitsResponse {
    pub commits: Vec<CommitInfo>,
}

/// Commit information.
#[derive(Debug, Serialize)]
pub struct CommitInfo {
    pub id: String,
    pub parent: Option<String>,
    pub message: String,
    pub timestamp: u64,
}

/// Request body for storing a model.
#[derive(Debug, Deserialize)]
pub struct StoreModelRequest {
    pub message: String,
    #[serde(default)]
    pub parent: Option<String>,
    pub model: serde_json::Value,
}

/// Response for storing a model.
#[derive(Debug, Serialize)]
pub struct StoreModelResponse {
    pub commit: String,
    pub project: String,
}

/// Request body for loading source text.
#[derive(Debug, Deserialize)]
pub struct LoadSourceRequest {
    pub uri: String,
    pub source: String,
}

/// Request body for the unified query endpoint.
#[derive(Debug, Deserialize)]
pub struct QueryEngineRequest {
    pub uri: String,
    pub spec: serde_json::Value,
}

/// Query parameters for the find endpoint.
#[derive(Debug, Deserialize)]
pub struct FindQuery {
    pub pattern: String,
    pub kind: Option<String>,
}

/// Request body for the constraint check endpoint.
#[derive(Debug, Deserialize)]
pub struct CheckConstraintsRequest {
    #[serde(default)]
    pub overrides: Vec<(String, String)>,
}

/// Query parameters for the trace matrix endpoint.
#[derive(Debug, Deserialize)]
pub struct TraceQuery {
    pub source_kind: String,
    pub target_kind: String,
    pub relation_kind: String,
}

/// Request body for the eval expression endpoint.
#[derive(Debug, Deserialize)]
pub struct EvalExpressionRequest {
    pub expression: String,
    #[serde(default)]
    pub context: Vec<(String, String)>,
}

/// Request body for the load file endpoint.
#[derive(Debug, Deserialize)]
pub struct LoadFileRequest {
    pub path: String,
}

/// Request body for starting a simulation session.
#[derive(Debug, Deserialize)]
pub struct SimulateStartRequest {
    pub uri: String,
    pub sm_name: String,
}

/// Request body for stepping a simulation session.
#[derive(Debug, Deserialize)]
pub struct SimulateStepRequest {
    #[serde(default)]
    pub event: Option<String>,
    /// Optional parameter overrides as (name, value) pairs; values parsed as f64.
    #[serde(default)]
    pub overrides: Option<Vec<(String, String)>>,
}

/// Request body for starting an action session.
#[derive(Debug, Deserialize)]
pub struct ActionStartRequest {
    pub uri: String,
    pub action_name: String,
}

// `ContinuousStartRequest` / `POST /sessions/continuous/start` were removed in
// the execution-entry unification arc (execution-entry-unification-plan.md P5):
// the `continuous_start` command took ODE derivative strings from the caller,
// bypassing the model (principle #3). Model-driven continuous simulation runs
// via `sysml.simulate.continuous.auto` or the workspace orchestrator
// (`sysml.sessions.create`), reachable through the generic `/api/command`
// dispatcher and the auto-generated `/api/commands/{name}` routes.

// ── Helper to convert ServiceError → HTTP response ──────────────────────

#[allow(clippy::needless_pass_by_value)] // Consumes the error at every call site
fn service_err_response(e: sysml_service::ServiceError) -> axum::response::Response {
    use sysml_service::ServiceError;
    let (status, msg) = match &e {
        ServiceError::ElementNotFound(_) | ServiceError::NotFound(_) => {
            (StatusCode::NOT_FOUND, e.to_string())
        }
        ServiceError::InvalidInput(_) => (StatusCode::BAD_REQUEST, e.to_string()),
        ServiceError::Store(s) if s.contains("no store configured") => {
            (StatusCode::SERVICE_UNAVAILABLE, e.to_string())
        }
        _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    (status, Json(ErrorResponse { error: msg })).into_response()
}

/// Convert a `Serialize` value to `serde_json::Value`.
///
/// All our response types derive `Serialize` with no custom logic, so
/// serialization is infallible in practice. This helper keeps the
/// clippy `unwrap_used` lint happy.
#[allow(clippy::unwrap_used)] // Infallible: all response types are plain derive(Serialize)
fn to_json_value<T: serde::Serialize>(v: &T) -> serde_json::Value {
    serde_json::to_value(v).unwrap()
}

/// Optional `?with_readiness=1` query flag for URI-keyed endpoints.
///
/// When present and non-empty (any value other than "0" / "false"), the
/// handler wraps its JSON response in
/// `{ "data": <original>, "readiness": <Readiness> }` so the client can
/// observe the same readiness state the diagnostic gate (P-RA3) is
/// using. Default (no flag) preserves the existing raw response shape
/// — back-compat for current REST clients.
#[derive(Debug, Default, Deserialize)]
pub struct WithReadinessQuery {
    #[serde(default)]
    pub with_readiness: Option<String>,
}

impl WithReadinessQuery {
    /// True when the caller asked for a readiness envelope.
    pub fn is_set(&self) -> bool {
        matches!(self.with_readiness.as_deref(), Some(s) if !s.is_empty() && s != "0" && s != "false")
    }
}

/// Wrap `data` in `{ data, readiness }` if `flag.is_set()`, otherwise
/// return `data` unchanged. Mirrors the MCP-slice envelope shape (see
fn envelope_with_readiness(
    service: &sysml_service::SysmlService,
    uri: &str,
    flag: &WithReadinessQuery,
    data: serde_json::Value,
) -> serde_json::Value {
    if flag.is_set() {
        let readiness = service.readiness_for(uri);
        serde_json::json!({
            "data": data,
            "readiness": readiness,
        })
    } else {
        data
    }
}

// ── Existing endpoints (backward-compatible) ────────────────────────────

/// Health check endpoint.
async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_owned(),
        version: "0.1.0".to_owned(),
    })
}

/// List all projects.
async fn list_projects(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match run_service_blocking(state, |svc| svc.list_projects()).await.and_then(|r| r) {
        Ok(projects) => {
            let project_ids: Vec<String> =
                projects.iter().map(|p| p.as_str().to_owned()).collect();
            (
                StatusCode::OK,
                Json(ProjectsResponse {
                    projects: project_ids,
                }),
            )
                .into_response()
        }
        Err(e) => service_err_response(e),
    }
}

/// List commits for a project.
async fn list_commits(
    State(state): State<Arc<AppState>>,
    Path(project_id): Path<String>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| {
        let project = ProjectId::new(&project_id);
        svc.list_commits(&project)
    })
    .await
    .and_then(|r| r)
    {
        Ok(commits) => {
            let commit_infos: Vec<CommitInfo> = commits
                .iter()
                .map(|c| CommitInfo {
                    id: c.commit.as_str().to_owned(),
                    parent: c.parent.as_ref().map(|p| p.as_str().to_owned()),
                    message: c.message.clone(),
                    timestamp: c.timestamp,
                })
                .collect();
            (
                StatusCode::OK,
                Json(CommitsResponse {
                    commits: commit_infos,
                }),
            )
                .into_response()
        }
        Err(e) => service_err_response(e),
    }
}

/// Get a model snapshot.
async fn get_model(
    State(state): State<Arc<AppState>>,
    Path((project_id, commit_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let project = ProjectId::new(&project_id);
    let commit = CommitId::new(&commit_id);

    match run_service_blocking(state, move |svc| svc.load_model(&project, &commit))
        .await
        .and_then(|r| r)
    {
        Ok(Some(graph)) => {
            let json = to_json_string(&graph);
            (StatusCode::OK, [("content-type", "application/json")], json).into_response()
        }
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Snapshot not found".to_owned(),
            }),
        )
            .into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Store a model snapshot.
async fn store_model(
    State(state): State<Arc<AppState>>,
    Path((project_id, commit_id)): Path<(String, String)>,
    Json(request): Json<StoreModelRequest>,
) -> impl IntoResponse {
    // Parse the model
    let model_json = serde_json::to_string(&request.model).unwrap_or_default();
    let graph = match from_json_str(&model_json) {
        Ok(g) => g,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid model: {}", e),
                }),
            )
                .into_response();
        }
    };

    let project = ProjectId::new(&project_id);
    let commit = CommitId::new(&commit_id);

    let mut meta = SnapshotMeta::new(commit, request.message);
    if let Some(parent) = request.parent {
        meta = meta.with_parent(CommitId::new(parent));
    }

    match run_service_blocking(state, move |svc| svc.store_model(&project, meta, &graph))
        .await
        .and_then(|r| r)
    {
        Ok(()) => (
            StatusCode::CREATED,
            Json(StoreModelResponse {
                commit: commit_id,
                project: project_id,
            }),
        )
            .into_response(),
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("Conflict") || msg.contains("conflict") {
                (StatusCode::CONFLICT, Json(ErrorResponse { error: msg })).into_response()
            } else {
                service_err_response(e)
            }
        }
    }
}

// ── New source / query endpoints ────────────────────────────────────────

/// Load SysML source text into the service.
async fn load_source(
    State(state): State<Arc<AppState>>,
    Json(request): Json<LoadSourceRequest>,
) -> impl IntoResponse {
    let uri_for_resp = request.uri.clone();
    match run_service_blocking(state, move |svc| {
        svc.load_source(&request.uri, &request.source)
    })
    .await
    .and_then(|r| r)
    {
        Ok(()) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "uri": uri_for_resp })),
        )
            .into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Run a structured element-list query.
async fn query_engine(
    State(state): State<Arc<AppState>>,
    Json(request): Json<QueryEngineRequest>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| svc.query(&request.uri, &request.spec))
        .await
        .and_then(|r| r)
    {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Find elements by name pattern.
async fn find_elements(
    State(state): State<Arc<AppState>>,
    Path(uri): Path<String>,
    Query(params): Query<FindQuery>,
    Query(readiness): Query<WithReadinessQuery>,
) -> impl IntoResponse {
    let kind = params.kind.as_ref().and_then(|k| parse_element_kind(k));
    match run_service_blocking(state, move |svc| {
        svc.find(&uri, &params.pattern, kind.as_ref())
            .map(|elements| envelope_with_readiness(svc, &uri, &readiness, to_json_value(&elements)))
    })
    .await
    .and_then(|r| r)
    {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Model statistics.
async fn model_stats(
    State(state): State<Arc<AppState>>,
    Path(uri): Path<String>,
    Query(readiness): Query<WithReadinessQuery>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| {
        svc.stats(&uri)
            .map(|stats| envelope_with_readiness(svc, &uri, &readiness, to_json_value(&stats)))
    })
    .await
    .and_then(|r| r)
    {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Query params for `GET /models/:uri/tree`.
///
/// Both fields are optional. `view` accepts `"user_facing"` (default —
/// mirrors what the simulation UI sees) or `"full"` (every kind kept,
/// for AI agents inspecting the raw graph). Anything else (typo, etc.)
/// falls back to `user_facing` server-side.
#[derive(Deserialize, Default)]
struct ModelTreeQuery {
    max_depth: Option<usize>,
    view: Option<String>,
}

/// Model tree.
async fn model_tree(
    State(state): State<Arc<AppState>>,
    Path(uri): Path<String>,
    Query(q): Query<ModelTreeQuery>,
    Query(readiness): Query<WithReadinessQuery>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| {
        svc.model_tree(&uri, q.max_depth, q.view.as_deref())
            .map(|tree| envelope_with_readiness(svc, &uri, &readiness, to_json_value(&tree)))
    })
    .await
    .and_then(|r| r)
    {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Unverified requirements.
async fn unverified(
    State(state): State<Arc<AppState>>,
    Path(uri): Path<String>,
    Query(readiness): Query<WithReadinessQuery>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| {
        svc.unverified(&uri)
            .map(|elements| envelope_with_readiness(svc, &uri, &readiness, to_json_value(&elements)))
    })
    .await
    .and_then(|r| r)
    {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => service_err_response(e),
    }
}

// ── Element & constraint endpoints ──────────────────────────────────────

/// Get a single element by ID.
///
/// The Element struct's serde derive skips `props` when the BTreeMap is empty.
/// For the Element Inspector UI we always want a `props` field present (even
/// if `{}`) so the frontend can render a stable Properties section without
/// needing to special-case missing keys.
async fn get_element(
    State(state): State<Arc<AppState>>,
    Path((uri, id)): Path<(String, String)>,
    Query(readiness): Query<WithReadinessQuery>,
) -> impl IntoResponse {
    let element_id = ElementId::from_string(id);
    let element_id_for_err = element_id.clone();
    match run_service_blocking(state, move |svc| {
        svc.element(&uri, &element_id).map(|opt| {
            opt.map(|elem| {
                let mut value = to_json_value(&elem);
                if let Some(obj) = value.as_object_mut() {
                    obj.entry("props")
                        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
                }
                envelope_with_readiness(svc, &uri, &readiness, value)
            })
        })
    })
    .await
    .and_then(|r| r)
    {
        Ok(Some(payload)) => (StatusCode::OK, Json(payload)).into_response(),
        Ok(None) => (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: format!("element not found: {element_id_for_err}"),
            }),
        )
            .into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Get children of an element.
async fn get_children(
    State(state): State<Arc<AppState>>,
    Path((uri, id)): Path<(String, String)>,
    Query(readiness): Query<WithReadinessQuery>,
) -> impl IntoResponse {
    let element_id = ElementId::from_string(id);
    match run_service_blocking(state, move |svc| {
        svc.children(&uri, &element_id)
            .map(|children| envelope_with_readiness(svc, &uri, &readiness, to_json_value(&children)))
    })
    .await
    .and_then(|r| r)
    {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Check constraints on a loaded model.
async fn check_constraints(
    State(state): State<Arc<AppState>>,
    Path(uri): Path<String>,
    body: Option<Json<CheckConstraintsRequest>>,
) -> impl IntoResponse {
    let overrides = body.map(|b| b.0.overrides).unwrap_or_default();
    match run_service_blocking(state, move |svc| svc.check_constraints(&uri, &overrides))
        .await
        .and_then(|r| r)
    {
        Ok(results) => (StatusCode::OK, Json(to_json_value(&results))).into_response(),
        Err(e) => service_err_response(e),
    }
}

// ── Visualization endpoints ─────────────────────────────────────────────

/// Export model as canonical JSON.
async fn export_json(
    State(state): State<Arc<AppState>>,
    Path(uri): Path<String>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| svc.export_json(&uri))
        .await
        .and_then(|r| r)
    {
        Ok(json_str) => {
            (StatusCode::OK, [("content-type", "application/json")], json_str).into_response()
        }
        Err(e) => service_err_response(e),
    }
}

/// List user-authored ViewUsage / ViewDefinition elements.
async fn views_list(
    State(state): State<Arc<AppState>>,
    Path(uri): Path<String>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| svc.views_list(&uri))
        .await
        .and_then(|r| r)
    {
        Ok(views) => (StatusCode::OK, Json(views)).into_response(),
        Err(e) => service_err_response(e),
    }
}

#[derive(serde::Deserialize, Default)]
struct ViewRenderQuery {
    #[serde(default)]
    expanded: Option<String>,
}

/// Render a user-authored ViewUsage as a diagram.
async fn views_render(
    State(state): State<Arc<AppState>>,
    Path((uri, view_id)): Path<(String, String)>,
    Query(params): Query<ViewRenderQuery>,
) -> impl IntoResponse {
    let element_id = ElementId::from_string(view_id);
    let expanded: HashSet<String> = params
        .expanded
        .as_deref()
        .unwrap_or("")
        .split(',')
        .filter(|s| !s.is_empty())
        .map(|s| s.to_owned())
        .collect();
    match run_service_blocking(state, move |svc| {
        svc.views_render(&uri, &element_id, &expanded)
    })
    .await
    .and_then(|r| r)
    {
        Ok(json) => (StatusCode::OK, Json(json)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// List views that satisfy the given viewpoint.
async fn views_by_viewpoint(
    State(state): State<Arc<AppState>>,
    Path((uri, viewpoint_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let vp_id = ElementId::from_string(viewpoint_id);
    match run_service_blocking(state, move |svc| svc.views_by_viewpoint(&uri, &vp_id))
        .await
        .and_then(|r| r)
    {
        Ok(views) => (StatusCode::OK, Json(views)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// List viewpoints whose StakeholderMembership references the given
/// stakeholder PartUsage.
async fn viewpoints_by_stakeholder(
    State(state): State<Arc<AppState>>,
    Path((uri, stakeholder_id)): Path<(String, String)>,
) -> impl IntoResponse {
    let sh_id = ElementId::from_string(stakeholder_id);
    match run_service_blocking(state, move |svc| svc.viewpoints_by_stakeholder(&uri, &sh_id))
        .await
        .and_then(|r| r)
    {
        Ok(ids) => (StatusCode::OK, Json(to_json_value(&ids))).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Body for the create-scratch endpoint. `expose` is a list of
/// qualified names or element references; one `expose <name>;` line is
/// emitted per entry.
#[derive(Debug, Deserialize)]
pub struct CreateScratchRequest {
    pub expose: Vec<String>,
}

/// Build a `view scratch : InterconnectionView { expose ...; }` source
/// snippet for the editor's "create view def from selection" command.
async fn views_create_scratch(
    State(state): State<Arc<AppState>>,
    Json(body): Json<CreateScratchRequest>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| svc.views_create_scratch(&body.expose))
        .await
        .and_then(|r| r)
    {
        Ok(snippet) => (
            StatusCode::OK,
            [("content-type", "text/plain; charset=utf-8")],
            snippet,
        )
            .into_response(),
        Err(e) => service_err_response(e),
    }
}

// ── Navigation endpoints ────────────────────────────────────────────────

/// Get ancestors of an element.
async fn get_ancestors(
    State(state): State<Arc<AppState>>,
    Path((uri, id)): Path<(String, String)>,
    Query(readiness): Query<WithReadinessQuery>,
) -> impl IntoResponse {
    let element_id = ElementId::from_string(id);
    match run_service_blocking(state, move |svc| {
        svc.ancestors(&uri, &element_id).map(|ancestors| {
            envelope_with_readiness(svc, &uri, &readiness, to_json_value(&ancestors))
        })
    })
    .await
    .and_then(|r| r)
    {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Get descendants of an element.
async fn get_descendants(
    State(state): State<Arc<AppState>>,
    Path((uri, id)): Path<(String, String)>,
    Query(readiness): Query<WithReadinessQuery>,
) -> impl IntoResponse {
    let element_id = ElementId::from_string(id);
    match run_service_blocking(state, move |svc| {
        svc.descendants(&uri, &element_id).map(|descendants| {
            envelope_with_readiness(svc, &uri, &readiness, to_json_value(&descendants))
        })
    })
    .await
    .and_then(|r| r)
    {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Generate a trace matrix.
async fn trace_matrix(
    State(state): State<Arc<AppState>>,
    Path(uri): Path<String>,
    Query(params): Query<TraceQuery>,
) -> impl IntoResponse {
    let Some(source_kind) = parse_element_kind(&params.source_kind) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid source_kind: {}", params.source_kind),
            }),
        )
            .into_response();
    };
    let Some(target_kind) = parse_element_kind(&params.target_kind) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid target_kind: {}", params.target_kind),
            }),
        )
            .into_response();
    };
    let Some(rel_kind) = parse_relationship_kind(&params.relation_kind) else {
        return (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("invalid relation_kind: {}", params.relation_kind),
            }),
        )
            .into_response();
    };
    match run_service_blocking(state, move |svc| {
        svc.trace_matrix(&uri, &source_kind, &rel_kind, &target_kind)
    })
    .await
    .and_then(|r| r)
    {
        Ok(rows) => (StatusCode::OK, Json(to_json_value(&rows))).into_response(),
        Err(e) => service_err_response(e),
    }
}

// ── Analysis endpoints ─────────────────────────────────────────────────

/// Get diagnostics for a loaded model.
async fn get_diagnostics(
    State(state): State<Arc<AppState>>,
    Path(uri): Path<String>,
    Query(readiness): Query<WithReadinessQuery>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| {
        svc.diagnostics(&uri)
            .map(|diags| envelope_with_readiness(svc, &uri, &readiness, to_json_value(&diags)))
    })
    .await
    .and_then(|r| r)
    {
        Ok(payload) => (StatusCode::OK, Json(payload)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Evaluate a standalone expression.
async fn eval_expression(
    State(state): State<Arc<AppState>>,
    Json(request): Json<EvalExpressionRequest>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| {
        svc.eval_expression(&request.expression, &request.context)
    })
    .await
    .and_then(|r| r)
    {
        Ok(value) => (StatusCode::OK, Json(to_json_value(&value))).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// List all loaded model URIs.
async fn list_models(
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    match run_service_blocking(state, |svc| svc.loaded_uris()).await {
        Ok(uris) => (StatusCode::OK, Json(serde_json::json!({ "uris": uris }))).into_response(),
        Err(e) => service_err_response(e),
    }
}

// ── File loading endpoint ──────────────────────────────────────────────

/// Load a file from disk path. Returns the URI and the file's source text.
async fn load_file(
    State(state): State<Arc<AppState>>,
    Json(request): Json<LoadFileRequest>,
) -> impl IntoResponse {
    let request_path = request.path;
    // Read the source text before parsing (so we can return it)
    let source = match std::fs::read_to_string(&request_path) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({ "error": format!("Failed to read file: {}", e) })),
            )
                .into_response();
        }
    };
    match run_service_blocking(state, move |svc| {
        svc.load_file(std::path::Path::new(&request_path))
    })
    .await
    .and_then(|r| r)
    {
        Ok(uri) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "uri": uri, "source": source })),
        )
            .into_response(),
        Err(e) => service_err_response(e),
    }
}

// ── Simulation session endpoints ───────────────────────────────────────

/// Start a simulation session.
async fn simulate_start(
    State(state): State<Arc<AppState>>,
    Json(request): Json<SimulateStartRequest>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| {
        svc.simulate_start(&request.uri, &request.sm_name)
    })
    .await
    .and_then(|r| r)
    {
        Ok((key, initial)) => (
            StatusCode::CREATED,
            Json(serde_json::json!({
                "session_key": key,
                "initial_state": to_json_value(&initial),
            })),
        )
            .into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Step a session forward (any kind) — delegates to unified sessions_step.
/// Accepts optional `event` and `overrides`; the unified backend handles
/// the no-overrides and with-overrides cases through a single command.
async fn simulate_step(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
    body: Option<Json<SimulateStepRequest>>,
) -> impl IntoResponse {
    let (event, overrides) = match body {
        Some(Json(b)) => (b.event, b.overrides),
        None => (None, None),
    };
    match run_service_blocking(state, move |svc| {
        svc.sessions_step(
            &key,
            event.as_deref(),
            overrides.as_deref(),
            // Bulk-step (`ticks`) is exposed through the generic `/api/command`
            // dispatch of `sysml.sessions.step`; this legacy REST alias stays
            // single-tick.
            None,
        )
    })
    .await
    .and_then(|r| r)
    {
        Ok(summary) => (StatusCode::OK, Json(summary)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Stop and clean up a simulation session.
async fn simulate_stop(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| svc.simulate_stop(&key))
        .await
        .and_then(|r| r)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => service_err_response(e),
    }
}

// ── Action session endpoints ───────────────────────────────────────────

/// Start an action execution session.
async fn action_start(
    State(state): State<Arc<AppState>>,
    Json(request): Json<ActionStartRequest>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| {
        svc.action_start(&request.uri, &request.action_name)
    })
    .await
    .and_then(|r| r)
    {
        Ok(key) => (
            StatusCode::CREATED,
            Json(serde_json::json!({ "session_key": key })),
        )
            .into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Step an action session — delegates to unified sessions_step.
async fn action_step(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| svc.sessions_step(&key, None, None, None))
        .await
        .and_then(|r| r)
    {
        Ok(summary) => (StatusCode::OK, Json(summary)).into_response(),
        Err(e) => service_err_response(e),
    }
}

// ── Continuous simulation / orchestrator session endpoints ─────────────

/// Stop an orchestrator session.
async fn orchestrator_stop(
    State(state): State<Arc<AppState>>,
    Path(key): Path<String>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| svc.orchestrate_stop(&key))
        .await
        .and_then(|r| r)
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => service_err_response(e),
    }
}

// ── Meta endpoints ──────────────────────────────────────────────────────

/// Command catalog.
///
/// Left inline (not routed through `run_service_blocking`): `command_catalog`
/// is an associated fn that reads the static inventory registry — it takes no
/// `&self` and never locks the host Mutex, so there is nothing to block on.
async fn commands() -> Json<serde_json::Value> {
    Json(SysmlService::command_catalog())
}

/// Return the [`Readiness`] snapshot for a URI. Companion to the SSE
/// progress stream — clients can poll this any time to learn whether
/// the file's project / library / file are loaded enough for the
/// readiness-gated diagnostic tiers.
async fn readiness_for(
    State(state): State<Arc<AppState>>,
    Path(uri): Path<String>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| svc.readiness_for(&uri)).await {
        Ok(r) => {
            let r: Readiness = r;
            (StatusCode::OK, Json(to_json_value(&r))).into_response()
        }
        Err(e) => service_err_response(e),
    }
}

/// Request body for the generic command dispatch endpoint.
#[derive(Debug, Deserialize)]
pub struct CommandRequest {
    /// Command name (e.g. "sysml.find", "sysml.stats").
    pub command: String,
    /// Parameters for the command as a JSON object.
    #[serde(default = "default_params")]
    pub params: serde_json::Value,
}

fn default_params() -> serde_json::Value {
    serde_json::Value::Object(serde_json::Map::new())
}

/// Generic command dispatch endpoint.
///
/// Accepts any registered service command by name and delegates to the
/// inventory-based command registry via `execute_command`.
///
/// ```text
/// POST /api/command
/// Body: { "command": "sysml.find", "params": { "uri": "...", "pattern": "..." } }
/// ```
async fn dispatch_command(
    State(state): State<Arc<AppState>>,
    Json(request): Json<CommandRequest>,
) -> impl IntoResponse {
    match run_command_blocking(state, request.command, request.params).await {
        Ok(result) => (StatusCode::OK, Json(result)).into_response(),
        Err(e) => service_err_response(e),
    }
}

/// Run a service command on the blocking-thread pool instead of inline on
/// the async worker thread.
///
/// `sysml_service::execute_command` is a plain synchronous `fn` — for heavy
/// commands (e.g. `sysml.sessions.step` with a large `ticks` bulk-step, which
/// can run a non-yielding loop for several seconds) calling it directly from
/// an `async fn` handler monopolizes a tokio worker thread, starving the
/// WebSocket session-events task, health checks, and the session reaper for
/// the duration of the call. Moving the call into
/// [`tokio::task::spawn_blocking`] lets it run on the dedicated blocking pool
/// so the async runtime keeps making progress.
///
/// This is safe to do uniformly (not just for step/step_many): `AppState`'s
/// `service: Arc<SysmlService>` is `Send + Sync + 'static` and clones
/// cheaply; every lock reachable from `execute_command` (the salsa
/// `AnalysisHost`, stores, caches) is a `std::sync::Mutex`/`RwLock` — safe to
/// block on from a blocking-pool thread — never a `tokio::sync::Mutex`, so
/// there's no risk of blocking an async task that's itself waiting on the
/// tokio runtime. `execute_command`'s call chain is fully synchronous (no
/// `.await`), so it can run inside `spawn_blocking`'s synchronous closure
/// unchanged. Behavior and error semantics are preserved exactly; only where
/// the call runs has changed.
async fn run_command_blocking(
    state: Arc<AppState>,
    command: String,
    params: serde_json::Value,
) -> Result<serde_json::Value, sysml_service::ServiceError> {
    match tokio::task::spawn_blocking(move || {
        sysml_service::execute_command(&state.service, &command, params)
    })
    .await
    {
        Ok(result) => result,
        Err(join_err) => Err(sysml_service::ServiceError::Internal(format!(
            "command execution task failed: {join_err}"
        ))),
    }
}

/// Run a synchronous, host-locking `SysmlService` call on the blocking
/// pool. Same rationale as [`run_command_blocking`]: the named REST
/// handlers used to call these sync methods inline on async worker
/// threads, so a parked host Mutex (mutation waiting in a salsa setter,
/// or a long elaboration) blocked one worker per request until the pool
/// was exhausted and the server stopped responding entirely (the
/// 2026-07-17 wedge amplifier). Behavior and error semantics preserved;
/// only where the call runs changes.
async fn run_service_blocking<T: Send + 'static>(
    state: Arc<AppState>,
    f: impl FnOnce(&sysml_service::SysmlService) -> T + Send + 'static,
) -> Result<T, sysml_service::ServiceError> {
    match tokio::task::spawn_blocking(move || f(&state.service)).await {
        Ok(v) => Ok(v),
        Err(join_err) => Err(sysml_service::ServiceError::Internal(format!(
            "service task failed: {join_err}"
        ))),
    }
}

// ── Workspace file listing ──────────────────────────────────────────────

#[derive(Deserialize)]
struct WorkspaceQuery {
    root: String,
}

/// List SysML/KerML files in a workspace directory (recursive).
///
/// `GET /workspace/files?root=/path/to/dir`
///
/// Thin pass-through to the `sysml.workspace.files` service command. The
/// shape — `{ root, entries }` with `entries` as a tree of
/// `{ name, path, type, children? }` — is preserved so existing FE
/// consumers keep working.
async fn workspace_files(
    State(state): State<Arc<AppState>>,
    Query(params): Query<WorkspaceQuery>,
) -> impl IntoResponse {
    match run_service_blocking(state, move |svc| svc.workspace_files(&params.root, None))
        .await
        .and_then(|r| r)
    {
        Ok(result) => (StatusCode::OK, Json(serde_json::to_value(result).unwrap())).into_response(),
        Err(sysml_service::ServiceError::Project(msg)) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({ "error": msg })),
        )
            .into_response(),
        Err(e) => service_err_response(e),
    }
}

// ── Auth middleware ──────────────────────────────────────────────────────

/// Bearer-token auth middleware for mutation endpoints.
///
/// The expected token is fixed when the router is built (see [`ApiConfig`]),
/// not read per request. `None` means mutations are open.
///
/// Reading the environment here instead would make every request depend on
/// mutable process-wide state — which is both a surprising thing for a server
/// to do and, in tests, a race: one test setting the variable changed the
/// answer for every other test running beside it.
async fn require_auth(
    expected: Option<Arc<str>>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> impl IntoResponse {
    if let Some(expected_token) = expected {
        let provided = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        if provided != Some(&*expected_token) {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }
    next.run(request).await.into_response()
}

// ── Inventory-generated routes ──────────────────────────────────────────

/// Generate POST routes for all registered service commands.
///
/// Each route dispatches to the inventory-based command registry via
/// `execute_command`, providing named routes for discoverability alongside
/// the generic `/api/command` endpoint.
fn inventory_routes() -> Router<Arc<AppState>> {
    let mut router = Router::new();
    for reg in sysml_service::registered_commands() {
        let name: &'static str = reg.meta.name;
        let path = format!("/api/commands/{}", name);
        router = router.route(
            &path,
            post(move |State(state): State<Arc<AppState>>, Json(body): Json<serde_json::Value>| async move {
                // See `run_command_blocking` doc comment: dispatch on the
                // blocking-thread pool so a heavy command (e.g. a large
                // `sysml.sessions.step` bulk-step) doesn't monopolize a
                // tokio async worker thread.
                match run_command_blocking(state, name.to_string(), body).await {
                    Ok(result) => (StatusCode::OK, Json(result)).into_response(),
                    Err(e) => service_err_response(e),
                }
            }),
        );
    }
    router
}

// ── CORS policy ─────────────────────────────────────────────────────────

/// Which browser origins may call this server.
///
/// The API is a development tool: writes are unauthenticated unless
/// `SYSML_API_TOKEN` is set, and every command route can load files from disk
/// and execute model behaviour. A permissive CORS policy therefore lets *any*
/// web page the operator happens to visit drive that surface, so it is not the
/// default — it has to be asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CorsPolicy {
    /// Allow only browser origins served from the loopback interface —
    /// `localhost`, `127.0.0.1`, `[::1]`, on any port and either scheme.
    ///
    /// This covers the intended development setup (the simulation app's Vite
    /// server, the VS Code webview) while leaving a page on the open web
    /// unable to read a response from this server.
    #[default]
    LocalhostOnly,
    /// Allow any origin, method, and header.
    ///
    /// Only appropriate behind a trusted proxy, or when the operator has
    /// consciously accepted that any origin may drive the API. Opt in with
    /// `--permissive-cors` / `SYSML_API_CORS=permissive`.
    Permissive,
}

impl CorsPolicy {
    /// Read the policy from `SYSML_API_CORS`. Unset or unrecognised means the
    /// safe default — an operator typo must not silently widen access.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("SYSML_API_CORS").as_deref() {
            Ok("permissive") => Self::Permissive,
            _ => Self::LocalhostOnly,
        }
    }

    /// Build the tower-http layer this policy describes.
    #[must_use]
    pub fn layer(self) -> tower_http::cors::CorsLayer {
        match self {
            Self::Permissive => tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::Any)
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
            Self::LocalhostOnly => tower_http::cors::CorsLayer::new()
                .allow_origin(tower_http::cors::AllowOrigin::predicate(
                    |origin, _req| origin.to_str().is_ok_and(is_loopback_origin),
                ))
                .allow_methods(tower_http::cors::Any)
                .allow_headers(tower_http::cors::Any),
        }
    }
}

/// Is this `Origin` header value served from the loopback interface?
///
/// Matched on the host alone: the port is free (dev servers move around) and
/// the scheme may be `http` or `https`. A host that merely *contains*
/// "localhost" (`localhost.example.com`, `notlocalhost`) is not loopback, so
/// the host is compared exactly after the optional `:port` is stripped.
fn is_loopback_origin(origin: &str) -> bool {
    let rest = match origin.split_once("://") {
        Some(("http" | "https", rest)) => rest,
        _ => return false,
    };
    // `[::1]:3010` — an IPv6 literal keeps its brackets; split after them.
    let host = if let Some(after) = rest.strip_prefix('[') {
        match after.split_once(']') {
            Some((h, tail)) if tail.is_empty() || tail.starts_with(':') => h,
            _ => return false,
        }
    } else {
        match rest.split_once(':') {
            Some((h, port)) if port.chars().all(|c| c.is_ascii_digit()) => h,
            Some(_) => return false,
            None => rest,
        }
    };
    matches!(host, "localhost" | "127.0.0.1" | "::1")
}

// ── Router configuration ────────────────────────────────────────────────

/// Everything about the router that is a deployment decision rather than a
/// property of the model: who may call it from a browser, and what token (if
/// any) mutations must present.
///
/// Both are resolved ONCE, when the router is built. Neither is re-read per
/// request, so a running server's access rules cannot change under it.
#[derive(Debug, Clone, Default)]
pub struct ApiConfig {
    /// Which browser origins may call this server.
    pub cors: CorsPolicy,
    /// Bearer token required by write and command routes. `None` leaves
    /// mutations open, which is the development default.
    pub auth_token: Option<Arc<str>>,
}

impl ApiConfig {
    /// Read the deployment configuration from the environment:
    /// `SYSML_API_TOKEN` and `SYSML_API_CORS`.
    ///
    /// An empty `SYSML_API_TOKEN` is treated as unset. Exporting the variable
    /// with no value reads as "I want auth", and silently accepting the empty
    /// string as the required token would grant everyone access instead.
    #[must_use]
    pub fn from_env() -> Self {
        Self {
            cors: CorsPolicy::from_env(),
            auth_token: std::env::var("SYSML_API_TOKEN")
                .ok()
                .filter(|t| !t.is_empty())
                .map(Arc::from),
        }
    }

    /// Require this bearer token on write and command routes.
    #[must_use]
    pub fn with_auth_token(mut self, token: impl AsRef<str>) -> Self {
        self.auth_token = Some(Arc::from(token.as_ref()));
        self
    }

    /// Use this browser-origin policy.
    #[must_use]
    pub fn with_cors(mut self, cors: CorsPolicy) -> Self {
        self.cors = cors;
        self
    }
}

// ── Router construction ─────────────────────────────────────────────────

/// Create the API router backed by a [`SysmlService`], configured from the
/// environment via [`ApiConfig::from_env`].
///
/// If you need backward-compatible construction (empty in-memory store),
/// use `create_router(Arc::new(AppState::new()))`.
pub fn create_router(state: Arc<AppState>) -> Router {
    create_router_with_config(state, ApiConfig::from_env())
}

/// Create the API router with an explicit configuration, reading nothing from
/// the environment. This is what tests should use.
pub fn create_router_with_config(state: Arc<AppState>, config: ApiConfig) -> Router {
    let ApiConfig { cors, auth_token } = config;
    // Read-only routes (no auth required)
    let read_routes = Router::new()
        .route("/health", get(health))
        .route("/projects", get(list_projects))
        .route("/projects/:project_id/commits", get(list_commits))
        .route(
            "/projects/:project_id/commits/:commit_id/model",
            get(get_model),
        )
        // New query endpoints
        .route("/models/:uri/find", get(find_elements))
        .route("/models/:uri/stats", get(model_stats))
        .route("/models/:uri/tree", get(model_tree))
        .route("/models/:uri/unverified", get(unverified))
        // Element endpoints
        .route("/models/:uri/elements/:id", get(get_element))
        .route("/models/:uri/elements/:id/children", get(get_children))
        // Navigation endpoints
        .route("/models/:uri/elements/:id/ancestors", get(get_ancestors))
        .route("/models/:uri/elements/:id/descendants", get(get_descendants))
        .route("/models/:uri/trace", get(trace_matrix))
        // Analysis endpoints
        .route("/models/:uri/diagnostics", get(get_diagnostics))
        .route("/models", get(list_models))
        // Visualization endpoints
        .route("/models/:uri/export/json", get(export_json))
        .route("/models/:uri/views", get(views_list))
        .route("/models/:uri/views/:view_id/render", get(views_render))
        .route(
            "/models/:uri/views/by_viewpoint/:viewpoint_id",
            get(views_by_viewpoint),
        )
        .route(
            "/models/:uri/viewpoints/by_stakeholder/:stakeholder_id",
            get(viewpoints_by_stakeholder),
        )
        // Workspace
        .route("/workspace/files", get(workspace_files))
        // Meta
        .route("/commands", get(commands))
        // Readiness + progress (P-RA5)
        .route("/v1/progress", get(progress_sse::progress_sse_handler))
        .route("/v1/readiness/:uri", get(readiness_for))
        // WebSocket LSP transport
        .route("/lsp", get(lsp_ws::lsp_ws_handler))
        .route(
            "/api/sessions/:id/events",
            get(session_ws::session_ws_handler),
        );

    // Mutation routes (auth required when SYSML_API_TOKEN is set)
    let write_routes = Router::new()
        .route(
            "/projects/:project_id/commits/:commit_id/model",
            post(store_model),
        )
        .route("/sources", post(load_source))
        .route("/api/query", post(query_engine))
        .route("/models/:uri/check", post(check_constraints))
        .route("/views/scratch", post(views_create_scratch))
        // Expression evaluation
        .route("/eval", post(eval_expression))
        // File loading
        .route("/files", post(load_file))
        // Simulation sessions
        .route("/sessions/simulate/start", post(simulate_start))
        .route("/sessions/:key/step", post(simulate_step))
        .route("/sessions/:key", delete(simulate_stop))
        // Action sessions
        .route("/sessions/action/start", post(action_start))
        .route("/sessions/action/:key/step", post(action_step))
        // Continuous simulation / orchestrator sessions
        // (`/sessions/continuous/start` retired — use `sysml.simulate.continuous.auto`
        //  or `sysml.sessions.create` via the generic command routes.)
        // Orchestrator sessions step through the unified /sessions/:key/step
        // route — `simulate_step` accepts the overrides field for any kind.
        .route("/sessions/orchestrator/:key", delete(orchestrator_stop))
        // Generic command dispatch
        .route("/api/command", post(dispatch_command))
        .layer(middleware::from_fn({
            let token = auth_token.clone();
            move |h, r, n| require_auth(token.clone(), h, r, n)
        }));

    // Auto-generated routes for all registered service commands
    let command_routes = inventory_routes()
        .layer(middleware::from_fn({
            let token = auth_token.clone();
            move |h, r, n| require_auth(token.clone(), h, r, n)
        }));

    read_routes
        .merge(write_routes)
        .merge(command_routes)
        .layer(DefaultBodyLimit::max(50_000_000))
        .layer(cors.layer())
        .with_state(state)
}

/// Run the API server.
pub async fn run_server(addr: &str) -> Result<(), Box<dyn std::error::Error>> {
    let state = Arc::new(AppState::new());
    let app = create_router(state);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

// ── Helpers ─────────────────────────────────────────────────────────────

/// Parse a string into an `ElementKind`. Returns `None` for unrecognized kinds.
fn parse_element_kind(s: &str) -> Option<sysml_core::ElementKind> {
    // Try serde deserialization from a JSON string
    serde_json::from_value(serde_json::Value::String(s.to_owned())).ok()
}

/// Parse a string into a `RelationshipKind`. Returns `None` for unrecognized kinds.
fn parse_relationship_kind(s: &str) -> Option<sysml_core::RelationshipKind> {
    serde_json::from_value(serde_json::Value::String(s.to_owned())).ok()
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    unsafe_code
)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn test_state() -> Arc<AppState> {
        Arc::new(AppState::new())
    }

    fn test_state_with_source(uri: &str, source: &str) -> Arc<AppState> {
        let state = AppState::new();
        state.service.load_source(uri, source).unwrap();
        Arc::new(state)
    }

    // ── CORS policy ─────────────────────────────────────────────────────
    //
    // The API previously allowed every origin. It is a development server
    // whose write and command routes are unauthenticated unless
    // `SYSML_API_TOKEN` is set, so an allow-any header let any page the
    // operator visited read model data and drive commands out of it. These
    // pin the narrowed default and the shape of the opt-out.

    /// Ask the router what it will grant a given browser origin.
    async fn allowed_origin_for(app: Router, origin: &str) -> Option<String> {
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("origin", origin)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        response
            .headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap().to_owned())
    }

    #[tokio::test]
    async fn default_router_admits_loopback_browser_origins() {
        // The intended development setup: the simulation app's dev server and
        // anything else served from this machine must keep working.
        for origin in [
            "http://localhost:3010",
            "http://127.0.0.1:8080",
            "http://[::1]:5173",
            "https://localhost:443",
        ] {
            assert_eq!(
                allowed_origin_for(create_router(test_state()), origin).await,
                Some(origin.to_owned()),
                "{origin} should be allowed by the default policy"
            );
        }
    }

    #[tokio::test]
    async fn default_router_refuses_a_remote_browser_origin() {
        // The whole point of the change: a page on the open web gets no
        // allow-origin header back, so the browser withholds the response.
        for origin in [
            "https://example.com",
            "http://evil.test",
            // Hosts that merely *contain* a loopback name are not loopback.
            "http://localhost.example.com",
            "http://notlocalhost",
            "http://127.0.0.1.example.com",
        ] {
            assert_eq!(
                allowed_origin_for(create_router(test_state()), origin).await,
                None,
                "{origin} must not be granted access by the default policy"
            );
        }
    }

    #[tokio::test]
    async fn permissive_policy_still_admits_any_origin() {
        // The opt-out has to actually restore the old behaviour, or operators
        // behind a trusted proxy have no way back.
        let app = create_router_with_config(
            test_state(),
            ApiConfig::default().with_cors(CorsPolicy::Permissive),
        );
        assert_eq!(
            allowed_origin_for(app, "https://example.com").await,
            Some("*".to_owned())
        );
    }

    #[test]
    fn the_safe_policy_is_the_default() {
        assert_eq!(CorsPolicy::default(), CorsPolicy::LocalhostOnly);
    }

    // ── Auth configuration ──────────────────────────────────────────────
    //
    // The expected token used to be read from the process environment on every
    // request. That made these tests mutate global state, and three unrelated
    // tests running beside them intermittently saw the token and failed with
    // 401 — a flake that made the whole suite unusable as a release gate. The
    // token is now fixed when the router is built.

    /// POST a source-load with whatever auth headers the caller supplies.
    async fn post_source(app: Router, auth: Option<&str>) -> StatusCode {
        let mut req = Request::builder()
            .method("POST")
            .uri("/sources")
            .header("content-type", "application/json");
        if let Some(a) = auth {
            req = req.header("authorization", a);
        }
        app.oneshot(
            req.body(Body::from(r#"{"uri":"t.sysml","source":"package P {}"}"#))
                .unwrap(),
        )
        .await
        .unwrap()
        .status()
    }

    fn token_router() -> Router {
        create_router_with_config(
            test_state(),
            ApiConfig::default().with_auth_token("right-token"),
        )
    }

    #[tokio::test]
    async fn a_correct_bearer_token_is_accepted() {
        // The negative cases below are worthless without this one: middleware
        // that rejected EVERYTHING would satisfy them all.
        assert_ne!(
            post_source(token_router(), Some("Bearer right-token")).await,
            StatusCode::UNAUTHORIZED
        );
    }

    #[tokio::test]
    async fn a_wrong_or_missing_bearer_token_is_rejected() {
        for auth in [None, Some("Bearer wrong-token"), Some("right-token")] {
            assert_eq!(
                post_source(token_router(), auth).await,
                StatusCode::UNAUTHORIZED,
                "auth {auth:?} must not pass"
            );
        }
    }

    #[tokio::test]
    async fn inventory_generated_command_routes_are_gated_too() {
        // The inventory-generated routes carry their own auth layer. A token
        // that only guarded the hand-written writes would leave the whole
        // command surface open.
        let response = token_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/commands/sysml.loaded_uris")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn no_token_configured_leaves_mutations_open() {
        let app = create_router_with_config(test_state(), ApiConfig::default());
        assert_ne!(post_source(app, None).await, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn the_default_config_requires_no_token_and_restricts_origins() {
        let c = ApiConfig::default();
        assert!(c.auth_token.is_none());
        assert_eq!(c.cors, CorsPolicy::LocalhostOnly);
    }


    #[test]
    fn loopback_origin_matching_is_exact_on_the_host() {
        // Scheme and port are free; the host is not.
        assert!(is_loopback_origin("http://localhost"));
        assert!(is_loopback_origin("http://localhost:3010"));
        assert!(is_loopback_origin("https://127.0.0.1:8080"));
        assert!(is_loopback_origin("http://[::1]:5173"));
        assert!(is_loopback_origin("http://[::1]"));

        assert!(!is_loopback_origin("http://localhost.evil.test"));
        assert!(!is_loopback_origin("http://sub.localhost"));
        assert!(!is_loopback_origin("http://127.0.0.1.evil.test"));
        assert!(!is_loopback_origin("http://127.0.0.2"));
        // Non-HTTP schemes never match — a file:// or extension origin is not
        // "this machine's dev server".
        assert!(!is_loopback_origin("file://localhost"));
        assert!(!is_loopback_origin("null"));
        assert!(!is_loopback_origin("localhost:3010"));
    }

    #[tokio::test]
    async fn health_endpoint() {
        let app = create_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn list_projects_empty() {
        let app = create_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/projects")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_nonexistent_model() {
        let app = create_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/projects/test/commits/v1/model")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn post_without_auth_returns_401_when_token_set() {
        let app = create_router_with_config(
            test_state(),
            ApiConfig::default().with_auth_token("test-secret-token"),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/projects/test/commits/v1/model")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"message":"test","model":{}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn get_works_without_auth_when_token_set() {
        let app = create_router_with_config(
            test_state(),
            ApiConfig::default().with_auth_token("test-secret-token-2"),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/projects")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    // ── New endpoint tests ──────────────────────────────────────────────

    #[tokio::test]
    async fn load_source_endpoint() {
        let app = create_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sources")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"uri":"test.sysml","source":"package Vehicle { part engine; }"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
    }

    #[tokio::test]
    async fn load_source_auth_required_when_token_set() {
        let app = create_router_with_config(
            test_state(),
            ApiConfig::default().with_auth_token("test-source-token"),
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/sources")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"uri":"test.sysml","source":"package P {}"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn query_endpoint() {
        let state = test_state_with_source(
            "test.sysml",
            "package Vehicle { part engine; }",
        );
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/query")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"uri":"test.sysml","spec":{"filter":{"type":"kind","kinds":["PartUsage"]},"projection":"summary"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn find_elements_endpoint() {
        let state = test_state_with_source(
            "test.sysml",
            "package Vehicle { part engine; }",
        );
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/test.sysml/find?pattern=Vehicle")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn find_elements_not_loaded() {
        let app = create_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/missing.sysml/find?pattern=X")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn stats_endpoint() {
        let state = test_state_with_source(
            "test.sysml",
            "package Vehicle { part engine; }",
        );
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/test.sysml/stats")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn tree_endpoint() {
        let state = test_state_with_source(
            "test.sysml",
            "package Vehicle { part engine; }",
        );
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/test.sysml/tree")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn unverified_endpoint() {
        let state = test_state_with_source(
            "test.sysml",
            "package P { requirement safetyReq; }",
        );
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/test.sysml/unverified")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn export_json_endpoint() {
        let state = test_state_with_source(
            "test.sysml",
            "package Vehicle { part engine; }",
        );
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/test.sysml/export/json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn views_list_endpoint_surfaces_view_definition_and_expose() {
        let state = test_state_with_source(
            "test.sysml",
            "package P { part engine; view def OverviewView { expose engine; } }",
        );
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/test.sysml/views")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let views = payload.as_array().expect("views payload should be an array");
        let overview = views
            .iter()
            .find(|v| v["name"].as_str() == Some("OverviewView"))
            .expect("OverviewView should be in the views list");
        let exposed = overview["exposed"]
            .as_array()
            .expect("exposed should be an array");
        let engine_expose = exposed
            .iter()
            .find(|e| e["qualified_name"].as_str() == Some("engine"))
            .unwrap_or_else(|| panic!("expected an Expose entry for `engine`, got {exposed:?}"));
        // `engine` is a sibling part in the same package — it should
        // resolve to a concrete element id, not just a raw qname.
        assert!(
            engine_expose["exposed_element_id"].is_string(),
            "expected exposed_element_id to be populated, got {engine_expose:?}"
        );
    }

    #[tokio::test]
    async fn views_render_endpoint_returns_diagram_for_authored_view() {
        let state = test_state_with_source(
            "test.sysml",
            "package P { part engine; view def OverviewView { expose engine; } }",
        );
        // Look up the OverviewView's id via views_list first.
        let views = state.service.views_list("test.sysml").unwrap();
        let view_id = views
            .iter()
            .find(|v| v.name.as_deref() == Some("OverviewView"))
            .expect("OverviewView present")
            .id
            .clone();

        let app = create_router(state);
        let path = format!("/models/test.sysml/views/{}/render", view_id);
        let response = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(payload.get("scene").is_some(), "expected ViewModel, got {payload}");
    }

    #[tokio::test]
    async fn views_render_endpoint_404s_unknown_view_id() {
        let state = test_state_with_source(
            "test.sysml",
            "package P { part engine; }",
        );
        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/test.sysml/views/00000000-0000-0000-0000-000000000000/render")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn views_list_endpoint_returns_empty_for_model_with_no_views() {
        let state = test_state_with_source(
            "test.sysml",
            "package Vehicle { part engine; }",
        );
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/test.sysml/views")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(payload.as_array().map(Vec::len), Some(0));
    }

    #[tokio::test]
    async fn commands_endpoint_advertises_the_named_routes() {
        let app = create_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/commands")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let catalog: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            catalog.as_array().is_some_and(|commands| commands
                .iter()
                .any(|command| command["name"] == "sysml.loaded_uris")),
            "the catalog must advertise routes created from the same command inventory"
        );
    }

    #[tokio::test]
    async fn inventory_named_route_dispatches_a_catalogued_command() {
        let state = test_state_with_source("test.sysml", "package Vehicle { part engine; }");
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/commands/sysml.loaded_uris")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let uris: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(
            uris.as_array()
                .is_some_and(|uris| uris.iter().any(|uri| uri == "test.sysml")),
            "named routes must dispatch the catalogued command, got {uris}"
        );
    }

    #[tokio::test]
    async fn scratch_view_route_is_root_scoped_and_auth_gated() {
        let unauthorized = token_router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/views/scratch")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"expose":["P::engine"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);

        let response = create_router(test_state())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/views/scratch")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"expose":["P::engine"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert!(
            String::from_utf8_lossy(&body).contains("expose P::engine;"),
            "scratch route must return the generated view snippet"
        );
    }

    #[tokio::test]
    async fn generic_command_dispatch() {
        let state = test_state_with_source(
            "test.sysml",
            "package Vehicle { part engine; }",
        );
        let app = create_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/command")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"command":"sysml.loaded_uris","params":{}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn generic_command_dispatch_not_found() {
        let app = create_router(test_state());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/command")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"command":"sysml.nonexistent","params":{}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Task #7 item 1 (P5 escalation): an RS002-class override-resolution
    /// failure (an unknown override target name) is caller-input-shaped, not
    /// an internal invariant violation — `service_err_response` must map it
    /// to 400, not the 500 a plain `ServiceError::Execution` would produce.
    /// Drives the REST transport end-to-end (not just the service layer)
    /// since that's the surface the original bug report was filed against.
    #[tokio::test]
    async fn sessions_step_bogus_override_returns_400_not_500() {
        let state = test_state_with_source(
            "test.sysml",
            r#"
            package Task7Bogus400 {
                state def SM {
                    state A;
                    state B;
                    transition first A accept go then B;
                }
            }
            "#,
        );
        let app = create_router(state);

        let start_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/command")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"command":"sysml.simulate.start","params":{"uri":"test.sysml","sm_name":"SM"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(start_response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(start_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // simulate.start returns [session_key, StepResult].
        let session_id = payload[0].as_str().expect("session key is a string");

        let step_body = serde_json::json!({
            "command": "sysml.sessions.step",
            "params": {
                "session_id": session_id,
                "event": null,
                "overrides": [["rsc2NoSuchVariableXyz", "1.0"]],
            },
        });
        let step_response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/command")
                    .header("content-type", "application/json")
                    .body(Body::from(step_body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(
            step_response.status(),
            StatusCode::BAD_REQUEST,
            "unknown override target must surface as 400, not 500"
        );
    }

    // ── P-RA5: readiness envelope + SSE progress ───────────────────────

    #[tokio::test]
    async fn readiness_endpoint_returns_empty_for_unloaded_uri() {
        let app = create_router(test_state());
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/readiness/nonexistent.sysml")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // The JSON shape matches sysml_service::readiness::Readiness.
        assert!(payload.get("library").is_some());
        assert!(payload.get("project").is_some());
        assert!(payload.get("file").is_some());
    }

    #[tokio::test]
    async fn diagnostics_envelope_wraps_response_when_flag_set() {
        let state =
            test_state_with_source("test.sysml", "package P { part engine; }");
        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/test.sysml/diagnostics?with_readiness=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Enveloped: { data: [..], readiness: {..} }
        assert!(payload.get("data").is_some(), "envelope should carry `data`");
        let readiness = payload
            .get("readiness")
            .expect("envelope should carry `readiness`");
        assert!(readiness.get("library").is_some());
    }

    #[tokio::test]
    async fn diagnostics_without_flag_keeps_legacy_shape() {
        let state =
            test_state_with_source("test.sysml", "package P { part engine; }");
        let app = create_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/models/test.sysml/diagnostics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let payload: serde_json::Value = serde_json::from_slice(&body).unwrap();
        // Bare array (no envelope) — back-compat for existing clients.
        assert!(
            payload.is_array(),
            "diagnostics without with_readiness should be a bare array, got {payload}"
        );
    }

    /// SSE smoke test: hit `GET /v1/progress`, publish a `Ready` event
    /// from another task, assert the stream emits at least one
    /// `event: ready` frame. This is the P-RA5 acceptance criterion —
    /// progress published by the service is reachable over HTTP/SSE.
    #[tokio::test]
    async fn progress_sse_stream_emits_published_events() {
        use sysml_service::progress::ProgressEvent;

        let state = Arc::new(AppState::new());
        let service = Arc::clone(&state.service);
        let app = create_router(Arc::clone(&state));

        // Spawn the publisher AFTER the subscribe (the route handler
        // subscribes synchronously inside `oneshot`). To avoid a race,
        // we use a small delay before publishing.
        let publisher = tokio::spawn(async move {
            // Yield once so the handler's subscribe runs first.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            service.publish_progress(ProgressEvent::Ready);
            // Publish a second event so we get something even if the
            // first one races the subscription.
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            service.publish_progress(ProgressEvent::Refresh { reason: "test" });
        });

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/progress")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let ct = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        assert!(
            ct.contains("text/event-stream"),
            "expected SSE content-type, got {ct}"
        );

        // Pull a bounded prefix of the SSE body and stop once we see
        // the expected event name. SSE bodies are infinite (keep-alive
        // pings), so we MUST cap reads or the test will hang.
        // axum::body::Body implements `http_body::Body`; iterate via
        // `Stream`-style frame polling.
        use futures::StreamExt as _;
        let mut body = response.into_body().into_data_stream();
        let mut bytes_accum = Vec::new();
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
        while tokio::time::Instant::now() < deadline {
            let chunk = match tokio::time::timeout(
                std::time::Duration::from_millis(500),
                body.next(),
            )
            .await
            {
                Ok(Some(Ok(c))) => c,
                _ => continue,
            };
            bytes_accum.extend_from_slice(&chunk);
            let text = String::from_utf8_lossy(&bytes_accum);
            if text.contains("event: ready") || text.contains("event: refresh") {
                break;
            }
        }

        // Wait for publisher to drain so it doesn't outlive the test.
        let _ = publisher.await;

        let text = String::from_utf8_lossy(&bytes_accum);
        assert!(
            text.contains("event: ready") || text.contains("event: refresh"),
            "expected SSE to emit at least one ready/refresh event; got body:\n{text}"
        );
    }
}
